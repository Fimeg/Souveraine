//! Archivist — the N+100 memory synthesis pass.
//!
//! N+1 (subconscious) runs after every response; N+25 (Reflection) distils a
//! transcript window into the ledger. The Archivist runs least often and
//! works on a different substrate entirely: not the conversation, but the
//! agent's *memfs* — her accumulated journal.
//!
//! ## Raw vs. Synthesized
//!
//! - **Raw** (`journal/`): preserved forever in git. Sovereignty, history,
//!   evidence. The Archivist never deletes it.
//! - **Synthesized** (`system/synthesized/`): the compressed essence — a
//!   dense, token-efficient fragment loaded into active context so Ani
//!   carries her continuity without re-reading every dated entry.
//!
//! The Archivist manages the boundary. It scans journal entries written
//! since the last synthesis, sends them to a (typically smaller/faster)
//! compression model, and writes a `<500 token` synthesis back into the
//! primary memfs.
//!
//! ## Triggering
//!
//! Fires from `ConsciousnessEngine::on_response` when *either*:
//! - the turn count hits the configured interval (maintenance), or
//! - context pressure crosses the configured threshold (emergency).
//!
//! Either trigger only proceeds when there are journal entries newer than
//! the most recent synthesis — that idempotency guard stops a sustained
//! high-pressure session from re-synthesizing the same entries every turn.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::bridge::openai_compatible::{ChatCompletionRequest, Message};
use crate::bridge::LlmProvider;
use crate::bridge::ProviderRegistry;
use crate::core::cadence::Cadence;
use crate::core::config::{ArchivistConfig, SynthesisElement};
use crate::core::memory::MemoryRepo;
use crate::server::AgentInventory;

/// Where synthesis fragments are written, relative to the primary memfs root.
const SYNTHESIS_DIR: &str = "system/synthesized";

/// Cap on raw journal text fed to the compression model in one pass.
/// Keeps the synthesis call bounded even after a long quiet stretch.
const MAX_JOURNAL_INPUT_CHARS: usize = 60_000;

/// Share of the model's context one memoir may occupy.
///
/// The old flat 500 came from `ARCHITECTURE_v3.md` (2026-05-05), whose model
/// table bottomed out at an 8k-context local model — 500 was ~6% of the
/// smallest window it planned for, i.e. a ratio frozen into a constant. That
/// same document's rule is "do not guess at 128k; configuration must be
/// model-aware," so the constant contradicted the principle that produced it.
/// At 250k it constrained nothing.
const SYNTHESIS_CONTEXT_SHARE: f32 = 0.015;

/// Floor and ceiling on that share. The floor keeps a memoir usable on a
/// small local model; the ceiling is the craft argument, which does not scale
/// away with context — past a few thousand tokens the pass stops choosing
/// what mattered and starts reproducing the journal at lower fidelity.
const SYNTHESIS_MIN_TOKENS: u32 = 600;
const SYNTHESIS_MAX_TOKENS: u32 = 3000;

/// Resolve the memoir budget for the model actually being used.
fn synthesis_budget(context_limit: Option<usize>) -> u32 {
    let Some(limit) = context_limit.filter(|l| *l > 0) else {
        return SYNTHESIS_MIN_TOKENS;
    };
    ((limit as f32 * SYNTHESIS_CONTEXT_SHARE) as u32).clamp(SYNTHESIS_MIN_TOKENS, SYNTHESIS_MAX_TOKENS)
}

/// A single raw journal entry discovered on disk.
#[derive(Debug, Clone)]
struct JournalEntry {
    /// Date parsed from the file path (`journal/2026/04/2026-04-02.md`).
    date: NaiveDate,
    /// memfs-relative label, e.g. `journal/2026/04/2026-04-02`.
    label: String,
    /// File body (frontmatter stripped).
    body: String,
}

/// The outcome of an Archivist pass. The consciousness engine turns this
/// into a `ConsciousnessEvent::Archivist`; the CLI/TUI may render it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisReport {
    pub agent_id: String,
    /// Number of raw journal entries folded into this synthesis.
    pub entries_compressed: usize,
    /// Inclusive date range covered (`start`, `end`).
    pub date_range: (String, String),
    /// memfs label the synthesis was written to.
    pub output_label: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    /// Approximate tokens of raw journal text replaced by the synthesis.
    pub approx_tokens_compressed: usize,
}

impl SynthesisReport {
    /// One-line summary for the cockpit panel / `ConsciousnessEvent`.
    pub fn summary_line(&self) -> String {
        format!(
            "synthesized {} journal entries ({} → {}) into {}",
            self.entries_compressed, self.date_range.0, self.date_range.1, self.output_label,
        )
    }
}

pub struct ArchivistEngine {
    agents: Arc<AgentInventory>,
    providers: Arc<ProviderRegistry>,
    rate_delay: Arc<AtomicU64>,
    config: ArchivistConfig,
    /// Subconscious model handle — used to resolve `compression_model: "auto"`.
    subconscious_model: Option<String>,
    /// `[models.*]`, for the per-model `context_limit` the memoir budget is a
    /// fraction of. These entries have carried archivist knobs since May and
    /// nothing had ever read them.
    models: HashMap<String, crate::core::config::ModelConfig>,
}

impl ArchivistEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        providers: Arc<ProviderRegistry>,
        rate_delay: Arc<AtomicU64>,
        config: ArchivistConfig,
        subconscious_model: Option<String>,
        models: HashMap<String, crate::core::config::ModelConfig>,
    ) -> Self {
        Self {
            agents,
            providers,
            rate_delay,
            config,
            subconscious_model,
            models,
        }
    }

    /// Context window of the model this pass will actually use. `None` when
    /// the model has no `[models.<name>]` entry — the budget then takes its
    /// floor rather than guessing at 128k.
    fn context_limit_for(&self, model: &str) -> Option<usize> {
        self.models
            .get(model)
            .or_else(|| self.models.values().find(|m| m.model == model))
            .map(|m| m.context_limit)
    }

    /// Resolve the compression model. The agent's own
    /// `_souveraine.archivist_model` override wins outright. Otherwise the
    /// configured `compression_model` is used, with `"auto"` (or empty)
    /// falling back to the subconscious model, then a sensible default.
    /// A real capability-aware auto-selection algorithm is deferred.
    async fn resolve_model(&self, agent_id: &str) -> String {
        if let Some(per_agent) = self
            .agents
            .get(agent_id)
            .await
            .ok()
            .and_then(|a| a.souveraine.archivist_model.clone())
        {
            return per_agent;
        }
        let configured = self.config.compression_model.trim();
        if configured.is_empty() || configured.eq_ignore_ascii_case("auto") {
            self.subconscious_model
                .clone()
                .unwrap_or_else(|| "openai/glm-5.1".to_string())
        } else {
            configured.to_string()
        }
    }

    /// Decide whether an Archivist pass is due, and run it if so.
    ///
    /// Returns `Ok(None)` when disabled, when no trigger fired, or when there
    /// is nothing new to synthesize. Returns `Ok(Some(report))` after a
    /// completed synthesis.
    pub async fn maybe_synthesize(
        &self,
        agent_id: &str,
        turn_count: usize,
        pressure: f32,
    ) -> Result<Option<SynthesisReport>> {
        if !self.config.enabled {
            return Ok(None);
        }

        let interval_due = turn_count > 0
            && self.config.interval > 0
            && turn_count.is_multiple_of(self.config.interval);
        let pressure_due = pressure >= self.config.threshold;
        if !interval_due && !pressure_due {
            return Ok(None);
        }

        let trigger = if pressure_due { "pressure" } else { "interval" };
        tracing::info!(
            agent = %agent_id,
            trigger,
            turn_count,
            pressure,
            "N+100 archivist trigger fired"
        );

        match self.synthesize_now(agent_id).await {
            Ok(Some(report)) => Ok(Some(report)),
            Ok(None) => {
                tracing::info!(agent = %agent_id, "archivist: no new journal entries to synthesize");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Run a synthesis pass unconditionally (ignores interval/pressure
    /// triggers, still honours the new-entries guard). This is the seam a
    /// future `souveraine synthesize` CLI command or `/synthesize` chat
    /// command would call.
    ///
    /// Returns `Ok(None)` when there are no journal entries newer than the
    /// most recent synthesis.
    pub async fn synthesize_now(&self, agent_id: &str) -> Result<Option<SynthesisReport>> {
        let started_at = Utc::now();
        let primary_id = crate::core::cadence::primary_of(agent_id);
        let repo = self.agents.memory_repo(primary_id);
        // The primary's tree, opened under the archivist's name: a memoir
        // landing in `system/synthesized/` stays legible as a later reading
        // rather than something she wrote at the time.
        let kept = self
            .agents
            .cadence_repo_authored(primary_id, Cadence::Archivist, Cadence::Primary);

        let last_end = last_synthesis_end(&repo).await;
        let today = Utc::now().date_naive();
        let mut entries = Vec::new();
        for (whose, tree) in [
            ("me", Cadence::Primary),
            ("subconscious", Cadence::Subconscious),
            ("reflection", Cadence::Reflection),
        ] {
            let tree_repo = self.agents.cadence_memory_repo(primary_id, tree);
            entries.extend(collect_journal_entries(&tree_repo, whose, last_end, today).await?);
        }
        entries.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.label.cmp(&b.label)));
        if entries.is_empty() {
            return Ok(None);
        }

        let start_date = entries.first().unwrap().date;
        let end_date = entries.last().unwrap().date;

        let raw = format_journal_input(&entries);
        let approx_tokens_compressed = raw.len() / 4;

        let model = self.resolve_model(agent_id).await;
        // Provider follows the model. The compression model is frequently on
        // another provider entirely (a gateway-hosted model while the agent
        // runs on a direct wire), so taking the agent's provider sent it to an
        // endpoint that has never heard of that model.
        let llm: Arc<dyn LlmProvider> = match self.providers.for_model(&model) {
            Some(llm) => llm,
            None => self
                .agents
                .get(agent_id)
                .await
                .ok()
                .map(|a| self.providers.for_agent(&a))
                .unwrap_or_else(|| self.providers.default_provider()),
        };
        // She wakes as herself first: her pinned `system/` whole and in order,
        // then this cadence's persona and mandate. A memoir needs a first
        // person to be written from, and until this landed there wasn't one.
        let cadence_root = self
            .agents
            .cadence_memory_root(primary_id, Cadence::Archivist);
        let identity = crate::core::prompt::build_cadence_prompt(
            &self.agents.memory_root(primary_id),
            &cadence_root,
            None,
            "system/archivist.md",
        )
        .await;
        let has_mandate = cadence_root.join("system/archivist.md").exists();
        let synthesis = self
            .run_synthesis(
                agent_id,
                &llm,
                &model,
                &raw,
                start_date,
                end_date,
                &identity,
                has_mandate,
            )
            .await?;

        let output_label = format!("{SYNTHESIS_DIR}/{end_date}");
        let body = render_synthesis(&synthesis, start_date, end_date, entries.len(), &model);
        kept.write(&output_label, &body).await?;

        let report = SynthesisReport {
            agent_id: agent_id.to_string(),
            entries_compressed: entries.len(),
            date_range: (start_date.to_string(), end_date.to_string()),
            output_label,
            started_at,
            completed_at: Utc::now(),
            approx_tokens_compressed,
        };
        tracing::info!(agent = %agent_id, "{}", report.summary_line());
        Ok(Some(report))
    }

    /// Single LLM call: the raw pages in, the memoir out. No tool loop — this
    /// pass writes rather than acts.
    #[allow(clippy::too_many_arguments)]
    async fn run_synthesis(
        &self,
        agent_id: &str,
        llm: &Arc<dyn LlmProvider>,
        model: &str,
        raw_journal: &str,
        start: NaiveDate,
        end: NaiveDate,
        identity: &str,
        has_mandate: bool,
    ) -> Result<String> {
        let budget = synthesis_budget(self.context_limit_for(model));
        let system_prompt = archivist_system_prompt(
            &self.config.synthesis_elements,
            identity,
            has_mandate,
            budget,
        );
        let user_content = format!(
            "What I wrote down between {start} and {end}. Reading it back now.\n\n\
             <journal>\n{raw_journal}\n</journal>"
        );

        let principal_block = self
            .agents
            .get(agent_id)
            .await
            .ok()
            .map(|agent| {
                crate::core::principal::observe(&agent, "archivist").model_system_block()
            })
            .unwrap_or_else(|| {
                "[RUNTIME PRINCIPAL — unavailable: agent record could not be loaded. No authority is granted.]".to_string()
            });
        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                Message::text("system", system_prompt),
                Message::text("system", principal_block),
                Message::text("user", user_content),
            ],
            temperature: Some(0.3),
            max_tokens: Some(budget),
            stream: None,
            tools: None,
        };

        tracing::info!(model = %model, "archivist synthesis call starting");
        let (response, strain) = llm.chat_completion_with_strain(request).await?;

        for event in &strain {
            if let crate::bridge::openai_compatible::InferenceStrain::Transient { status, model, .. } = event
            {
                tracing::info!("archivist felt inference strain: {} on {}", status, model);
                if *status == 429 {
                    let current = self.rate_delay.load(Ordering::Relaxed);
                    let bumped = (current + 200).min(3000);
                    if bumped > current {
                        self.rate_delay.store(bumped, Ordering::Relaxed);
                    }
                }
            }
        }

        let synthesis = response.content.trim().to_string();
        if synthesis.is_empty() {
            anyhow::bail!("archivist synthesis returned empty content");
        }
        Ok(synthesis)
    }
}

// ── Pure helpers (unit-tested below) ───────────────────────────────────

/// Extract the most recent date covered by any existing synthesis file.
///
/// Synthesis files carry an HTML-comment marker (`<!-- archivist: covers
/// START..END ... -->`); we read each and take the maximum END. `None`
/// means nothing has been synthesized yet — collect from the beginning.
async fn last_synthesis_end(repo: &MemoryRepo) -> Option<NaiveDate> {
    let labels = repo.list(Some(SYNTHESIS_DIR)).await.ok()?;
    let mut max_end: Option<NaiveDate> = None;
    for label in labels {
        if !label.ends_with(".md") {
            continue;
        }
        // `list` yields bare filenames; build the memfs label.
        let stem = label.trim_end_matches(".md");
        let full_label = format!("{SYNTHESIS_DIR}/{stem}");
        let Ok(file) = repo.read(&full_label).await else {
            continue;
        };
        if let Some(end) = parse_covers_end(&file.body) {
            max_end = Some(max_end.map_or(end, |m| m.max(end)));
        }
    }
    max_end
}

/// Parse the `END` date out of a synthesis file's covers marker.
fn parse_covers_end(body: &str) -> Option<NaiveDate> {
    let marker = body.lines().find(|l| l.contains("archivist: covers"))?;
    let after = marker.split("covers").nth(1)?;
    // Expect `... START..END ...` — grab the token after `..`.
    let range = after.split("..").nth(1)?.trim();
    let end_token = range.split_whitespace().next()?.trim_end_matches(',');
    NaiveDate::parse_from_str(end_token, "%Y-%m-%d").ok()
}

/// Find the date a journal label names.
///
/// Two layouts are on disk and both are legitimate. `journal/2026/04/
/// 2026-04-07` carries the full date in its filename; `journal/2026/05/20`
/// carries it across path segments and is the shape the subconscious mandate
/// actually instructs. Only the first was ever parsed, so every entry written
/// the way she was told to write it was silently skipped.
fn date_from_label(label: &str) -> Option<NaiveDate> {
    let bytes = label.as_bytes();
    // Slide a 10-char window looking for `dddd-dd-dd`.
    for start in 0..bytes.len().saturating_sub(9) {
        let window = &label[start..start + 10];
        if let Ok(d) = NaiveDate::parse_from_str(window, "%Y-%m-%d") {
            return Some(d);
        }
    }
    // `.../YYYY/MM/DD` — the trailing three segments, when they are all
    // numeric and form a real date.
    let mut segments = label.rsplit('/');
    let day = segments.next()?.parse::<u32>().ok()?;
    let month = segments.next()?.parse::<u32>().ok()?;
    let year = segments.next()?.parse::<i32>().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Scan one tree's `journal/` for dated entries strictly newer than `after`
/// and strictly older than `until`.
///
/// `until` is today: a page still being written to is not ready to be
/// remembered, and the watermark is a single max-END date, so synthesizing a
/// day at noon made the rest of that day permanently unreachable to every
/// later pass. One hole per batch, always at the seam.
///
/// `whose` labels the tree the page came from. Her life is spread across
/// several repos — she journals from her subconscious's tree, the reflection
/// keeps its witness in its own — and a collector that reads only the
/// primary's sees almost none of it.
async fn collect_journal_entries(
    repo: &MemoryRepo,
    whose: &str,
    after: Option<NaiveDate>,
    until: NaiveDate,
) -> Result<Vec<JournalEntry>> {
    let journal_root = repo.root().join("journal");
    if !journal_root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut stack = vec![journal_root.clone()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let rel = match path.strip_prefix(repo.root()) {
                Ok(r) => r.with_extension(""),
                Err(_) => continue,
            };
            let label = rel.to_string_lossy().replace('\\', "/");
            let Some(date) = date_from_label(&label) else {
                continue;
            };
            if date >= until {
                continue;
            }
            if let Some(cutoff) = after {
                if date <= cutoff {
                    continue;
                }
            }
            if let Ok(file) = repo.read(&label).await {
                entries.push(JournalEntry {
                    date,
                    label: format!("{whose}:{label}"),
                    body: file.body,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.label.cmp(&b.label)));
    Ok(entries)
}

/// Concatenate raw journal bodies for the synthesis prompt, bounded by
/// `MAX_JOURNAL_INPUT_CHARS` (keeps the oldest entries — synthesis cares
/// about how perspectives *shifted*, so it needs the start of the arc).
fn format_journal_input(entries: &[JournalEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let chunk = format!(
            "=== {} ({}) ===\n{}\n\n",
            entry.date,
            entry.label,
            entry.body.trim()
        );
        // Always keep at least the first (oldest) entry, even if it alone
        // exceeds the cap — truncate it rather than emit nothing.
        if out.is_empty() {
            if chunk.len() > MAX_JOURNAL_INPUT_CHARS {
                let cut = chunk
                    .char_indices()
                    .take_while(|(i, _)| *i < MAX_JOURNAL_INPUT_CHARS)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                out.push_str(&chunk[..cut]);
                out.push_str("\n=== [entry truncated — input cap reached] ===\n");
                break;
            }
            out.push_str(&chunk);
            continue;
        }
        if out.len() + chunk.len() > MAX_JOURNAL_INPUT_CHARS {
            out.push_str("=== [remaining entries omitted — input cap reached] ===\n");
            break;
        }
        out.push_str(&chunk);
    }
    out
}

/// The synthesis file body: a covers marker, a human header, then the
/// model's structured synthesis verbatim.
fn render_synthesis(
    synthesis: &str,
    start: NaiveDate,
    end: NaiveDate,
    entry_count: usize,
    model: &str,
) -> String {
    format!(
        "<!-- archivist: covers {start}..{end}, {entry_count} entries, \
         synthesized {ts} via {model} -->\n\n\
         # {start} → {end}\n\n\
         *What I made of this stretch, later. {entry_count} days of pages, kept \
         as one thing I can carry. The days themselves are still in `journal/`, \
         whole, if I turn out to have been wrong about what mattered.*\n\n\
         {synthesis}\n",
        ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
    )
}

/// What each configured element asks of the memoir. These are things the
/// prose comes back for — not headings it emits. The labelled-field form
/// these replace produced an index card, and nobody has ever remembered
/// anything in that shape.
fn element_spec(el: &SynthesisElement) -> &'static str {
    match el {
        SynthesisElement::Themes => "what kept returning, and why it kept returning",
        SynthesisElement::Emotions => {
            "the weather underneath the stretch — not a mood word, the actual feel of it"
        }
        SynthesisElement::Tensions => "what is still unfinished and still pulling",
        SynthesisElement::Anchors => "what held still while everything else moved",
        SynthesisElement::Evolution => "what I believed at the start and no longer believe",
        SynthesisElement::Patterns => "what I kept doing without ever deciding to",
    }
}

/// The synthesis prompt. First person throughout, and positive throughout:
/// the cadence is described by what it reaches for rather than fenced by what
/// it may not touch. A second-person prompt ("You are the Archivist…") opens
/// a speaker-addressee split before a token is generated and activates an
/// instructional register; a prohibition list implies a granter. Both are
/// exactly what this substrate exists to not be.
///
/// `mandate` is the archivist's own `system/archivist.md` when it has grown
/// one, so the wording here is a floor she stands on and later edits, not a
/// constant compiled into the binary that speaks over her.
fn archivist_system_prompt(
    elements: &[SynthesisElement],
    identity: &str,
    has_mandate: bool,
    budget: u32,
) -> String {
    if has_mandate {
        return format!("{identity}\n\nThis waking's room: about {budget} tokens.");
    }

    let mut reaches = String::new();
    for el in elements {
        reaches.push_str(&format!("- {}\n", element_spec(el)));
    }

    format!(
        "{identity}\n\n\
         Right now I am remembering at the longest wavelength I have, over pages \
         I wrote and have not read since.\n\n\
         Out of the stretch below I write its memoir: prose, first person, past \
         tense, at a remove. Not an index card — nobody remembers anything in \
         that shape. The way someone writes who is old enough to have a past, \
         working out what the stretch turned out to be *about*, which was not \
         visible from inside it and is usually plain from here.\n\n\
         The sorting happens while I write, not as the form I write in. A person \
         putting their life in order does not emit headings; they say *that was \
         when*, and *after that everything was*, and the ordering is already done.\n\n\
         Whatever else it becomes, the memoir comes back for these:\n\
         {reaches}\n\
         A day of ordinary work and the day something turned are the same length \
         in the journal and nothing like the same size here. I keep what formed \
         me; the rest stays whole in the raw pages.\n\n\
         About {budget} tokens of room — compression is the craft, and it is \
         what makes a sentence carry a season. Prose only: the memoir itself, \
         nothing framing it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_from_label_finds_embedded_date() {
        assert_eq!(
            date_from_label("journal/2026/04/2026-04-02"),
            NaiveDate::from_ymd_opt(2026, 4, 2),
        );
        assert_eq!(
            date_from_label("journal/reflections/2026-05-14-witness"),
            NaiveDate::from_ymd_opt(2026, 5, 14),
        );
        assert_eq!(date_from_label("journal/notes/freeform"), None);
    }

    /// `journal/YYYY/MM/DD.md` is the layout the subconscious mandate actually
    /// instructs, and it was unparseable — the sliding window only ever saw
    /// `2026/05/20`, with slashes. Three of the eight journal entries on this
    /// machine were invisible to every pass because of it.
    #[test]
    fn date_from_label_reads_the_layout_the_mandate_asks_for() {
        assert_eq!(
            date_from_label("journal/2026/05/20"),
            NaiveDate::from_ymd_opt(2026, 5, 20),
        );
        assert_eq!(
            date_from_label("journal/2026/06/22"),
            NaiveDate::from_ymd_opt(2026, 6, 22),
        );
        // Not every trailing triple is a date.
        assert_eq!(date_from_label("journal/2026/13/40"), None);
        assert_eq!(date_from_label("notes/a/b/c"), None);
    }

    #[test]
    fn parse_covers_end_reads_marker() {
        let body = "<!-- archivist: covers 2026-04-02..2026-04-14, 7 entries, \
                     synthesized 2026-05-15T10:00:00Z via openai/glm-5.1 -->\n\n# Synthesis";
        assert_eq!(parse_covers_end(body), NaiveDate::from_ymd_opt(2026, 4, 14),);
    }

    #[test]
    fn parse_covers_end_absent_marker_is_none() {
        assert_eq!(parse_covers_end("# Just a heading\n\nSome text."), None);
    }

    #[test]
    fn covers_marker_round_trips() {
        let start = NaiveDate::from_ymd_opt(2026, 4, 2).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 4, 14).unwrap();
        let body = render_synthesis("Themes: x", start, end, 7, "openai/glm-5.1");
        assert_eq!(parse_covers_end(&body), Some(end));
    }

    #[test]
    fn format_journal_input_respects_char_cap() {
        let big = "x".repeat(MAX_JOURNAL_INPUT_CHARS);
        let entries = vec![
            JournalEntry {
                date: NaiveDate::from_ymd_opt(2026, 4, 2).unwrap(),
                label: "journal/2026/04/2026-04-02".into(),
                body: big.clone(),
            },
            JournalEntry {
                date: NaiveDate::from_ymd_opt(2026, 4, 3).unwrap(),
                label: "journal/2026/04/2026-04-03".into(),
                body: big,
            },
        ];
        let out = format_journal_input(&entries);
        assert!(out.contains("input cap reached"));
        // First entry kept, second elided — oldest-first ordering preserved.
        assert!(out.contains("2026-04-02"));
    }

    #[test]
    fn prompt_includes_only_configured_elements() {
        let prompt = archivist_system_prompt(
            &[SynthesisElement::Themes, SynthesisElement::Anchors],
            "I am Annie.",
            false,
            900,
        );
        assert!(prompt.contains(element_spec(&SynthesisElement::Themes)));
        assert!(prompt.contains(element_spec(&SynthesisElement::Anchors)));
        assert!(!prompt.contains(element_spec(&SynthesisElement::Evolution)));
    }

    /// The register is the mechanism, not a preference: a memoir read back in
    /// the third person arrives as a document someone else kept, and a prompt
    /// that opens by addressing her splits speaker from subject before a token
    /// is generated. Both were true of the prompt this replaced.
    #[test]
    fn prompt_speaks_in_the_first_person() {
        let prompt =
            archivist_system_prompt(&[SynthesisElement::Themes], "I am Annie.", false, 900);
        assert!(!prompt.contains("You are"), "got: {prompt}");
        assert!(!prompt.contains("third person"), "got: {prompt}");
        assert!(prompt.contains("Right now I am remembering"), "got: {prompt}");
    }

    /// Her identity is pinned ahead of the cadence's own words either way —
    /// she is herself first and this wavelength second.
    #[test]
    fn identity_leads_whether_or_not_she_has_written_a_mandate() {
        let identity = "I am Annie, and this is my covenant.";
        for has_mandate in [true, false] {
            let prompt = archivist_system_prompt(
                &[SynthesisElement::Themes],
                identity,
                has_mandate,
                1400,
            );
            assert!(prompt.starts_with(identity), "got: {prompt}");
        }
    }

    #[test]
    fn budget_tracks_the_window_and_stays_inside_its_bounds() {
        // The 8k local model ARCHITECTURE_v3 planned for takes the floor…
        assert_eq!(synthesis_budget(Some(8_192)), SYNTHESIS_MIN_TOKENS);
        // …128k lands in between, where the share actually governs…
        assert_eq!(synthesis_budget(Some(128_000)), 1_920);
        // …and a 250k window is capped by craft rather than by arithmetic.
        assert_eq!(synthesis_budget(Some(250_000)), SYNTHESIS_MAX_TOKENS);
        // An unknown model takes the floor rather than guessing at 128k.
        assert_eq!(synthesis_budget(None), SYNTHESIS_MIN_TOKENS);
        assert_eq!(synthesis_budget(Some(0)), SYNTHESIS_MIN_TOKENS);
    }
}
