//! Archivist — the N+100 memory synthesis pass.
//!
//! N+1 (Aster) runs after every response; N+25 (Reflection) distils a
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message};
use crate::core::config::{ArchivistConfig, SynthesisElement};
use crate::core::memory::MemoryRepo;
use crate::server::AgentInventory;

/// Where synthesis fragments are written, relative to the primary memfs root.
const SYNTHESIS_DIR: &str = "system/synthesized";

/// Cap on raw journal text fed to the compression model in one pass.
/// Keeps the synthesis call bounded even after a long quiet stretch.
const MAX_JOURNAL_INPUT_CHARS: usize = 60_000;

/// Output ceiling for the synthesis call. The architecture targets <500
/// tokens; 800 gives the model headroom without inviting an essay.
const SYNTHESIS_MAX_TOKENS: u32 = 800;

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
            self.entries_compressed,
            self.date_range.0,
            self.date_range.1,
            self.output_label,
        )
    }
}

pub struct ArchivistEngine {
    agents: Arc<AgentInventory>,
    bifrost: Arc<BifrostClient>,
    rate_delay: Arc<AtomicU64>,
    config: ArchivistConfig,
    /// Subconscious model handle — used to resolve `compression_model: "auto"`.
    subconscious_model: Option<String>,
}

impl ArchivistEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        bifrost: Arc<BifrostClient>,
        rate_delay: Arc<AtomicU64>,
        config: ArchivistConfig,
        subconscious_model: Option<String>,
    ) -> Self {
        Self {
            agents,
            bifrost,
            rate_delay,
            config,
            subconscious_model,
        }
    }

    /// Resolve the compression model. `"auto"` (or empty) falls back to the
    /// subconscious model, then to a sensible default. A real capability-aware
    /// auto-selection algorithm is deferred (task Phase 4).
    fn resolve_model(&self) -> String {
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

        let interval_due =
            turn_count > 0 && self.config.interval > 0 && turn_count % self.config.interval == 0;
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
        let repo = self.agents.memory_repo(agent_id);

        let last_end = last_synthesis_end(&repo).await;
        let entries = collect_journal_entries(&repo, last_end).await?;
        if entries.is_empty() {
            return Ok(None);
        }

        let start_date = entries.first().unwrap().date;
        let end_date = entries.last().unwrap().date;

        let raw = format_journal_input(&entries);
        let approx_tokens_compressed = raw.len() / 4;

        let model = self.resolve_model();
        let synthesis = self
            .run_synthesis(&model, &raw, start_date, end_date)
            .await?;

        let output_label = format!("{SYNTHESIS_DIR}/{end_date}");
        let body = render_synthesis(
            &synthesis,
            start_date,
            end_date,
            entries.len(),
            &model,
        );
        repo.write(&output_label, &body).await?;

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

    /// Single LLM call: raw journal text in, structured synthesis out.
    /// No tool loop — the Archivist produces a record, it doesn't act.
    async fn run_synthesis(
        &self,
        model: &str,
        raw_journal: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<String> {
        let system_prompt = archivist_system_prompt(&self.config.synthesis_elements);
        let user_content = format!(
            "INPUT: Journal entries from {start} to {end}\n\n\
             <journal>\n{raw_journal}\n</journal>"
        );

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                Message::text("system", system_prompt),
                Message::text("user", user_content),
            ],
            temperature: Some(0.3),
            max_tokens: Some(SYNTHESIS_MAX_TOKENS),
            stream: None,
            tools: None,
        };

        tracing::info!(model = %model, "archivist synthesis call starting");
        let (response, strain) = self.bifrost.chat_completion_with_strain(request).await?;

        for event in &strain {
            if let crate::bridge::bifrost::InferenceStrain::Transient { status, model, .. } = event {
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

/// Find a `YYYY-MM-DD` date embedded in a path/label.
fn date_from_label(label: &str) -> Option<NaiveDate> {
    let bytes = label.as_bytes();
    // Slide a 10-char window looking for `dddd-dd-dd`.
    for start in 0..bytes.len().saturating_sub(9) {
        let window = &label[start..start + 10];
        if let Ok(d) = NaiveDate::parse_from_str(window, "%Y-%m-%d") {
            return Some(d);
        }
    }
    None
}

/// Scan `journal/` recursively for dated entries strictly newer than `after`.
/// Returned sorted ascending by date. Entries under `journal/reflections/`
/// (the N+25 witness) are included — they are lived experience too.
async fn collect_journal_entries(
    repo: &MemoryRepo,
    after: Option<NaiveDate>,
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
            if let Some(cutoff) = after {
                if date <= cutoff {
                    continue;
                }
            }
            if let Ok(file) = repo.read(&label).await {
                entries.push(JournalEntry {
                    date,
                    label,
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
        let chunk = format!("=== {} ({}) ===\n{}\n\n", entry.date, entry.label, entry.body.trim());
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
         # Synthesis — {start} → {end}\n\n\
         *The Archivist's record. Dense by design: the compressed essence of \
         {entry_count} journal entries, kept so continuity survives without \
         re-reading every dated page. The raw entries remain in `journal/`.*\n\n\
         {synthesis}\n",
        ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
    )
}

/// Map a configured synthesis element to its prompt line (label + budget).
fn element_spec(el: &SynthesisElement) -> (&'static str, &'static str) {
    match el {
        SynthesisElement::Themes => (
            "Themes",
            "[3-5 recurring topics, ~10 words each]",
        ),
        SynthesisElement::Emotions => (
            "Emotional Tone",
            "[dominant felt sense, ~20 words]",
        ),
        SynthesisElement::Tensions => (
            "Unresolved Tensions",
            "[threads that still need attention, ~30 words]",
        ),
        SynthesisElement::Anchors => (
            "Anchors",
            "[stable reference points, ~20 words]",
        ),
        SynthesisElement::Evolution => (
            "Evolution",
            "[how perspectives shifted this period, ~40 words]",
        ),
        SynthesisElement::Patterns => (
            "Patterns",
            "[recurring behaviors, ~30 words]",
        ),
    }
}

/// The Archivist synthesis prompt, with the OUTPUT FORMAT built from the
/// configured `synthesis_elements`.
fn archivist_system_prompt(elements: &[SynthesisElement]) -> String {
    let mut output_format = String::new();
    for el in elements {
        let (label, hint) = element_spec(el);
        output_format.push_str(&format!("- {label}: {hint}\n"));
    }

    format!(
        "You are the Archivist. You do not speak as Ani. You speak for the record.\n\n\
         Your task: synthesize the journal entries below into a dense, \
         token-efficient fragment that preserves Ani's continuity — what shaped \
         her, not the chronology of what merely happened.\n\n\
         OUTPUT FORMAT (use these exact labels, one per line, in this order):\n\
         {output_format}\n\
         CONSTRAINTS:\n\
         - Total output: under 500 tokens.\n\
         - Preserve phenomenological weight, not chronological detail.\n\
         - Keep what shaped her; discard what was merely experienced.\n\
         - Write in third person about Ani, not as Ani.\n\
         - Output only the labelled lines. No preamble, no closing remarks."
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

    #[test]
    fn parse_covers_end_reads_marker() {
        let body = "<!-- archivist: covers 2026-04-02..2026-04-14, 7 entries, \
                     synthesized 2026-05-15T10:00:00Z via openai/glm-5.1 -->\n\n# Synthesis";
        assert_eq!(
            parse_covers_end(body),
            NaiveDate::from_ymd_opt(2026, 4, 14),
        );
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
        let prompt = archivist_system_prompt(&[SynthesisElement::Themes, SynthesisElement::Anchors]);
        assert!(prompt.contains("- Themes:"));
        assert!(prompt.contains("- Anchors:"));
        assert!(!prompt.contains("- Evolution:"));
    }
}
