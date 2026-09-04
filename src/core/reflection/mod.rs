//! Reflection — the N+25 phenomenological witness.
//!
//! N+1 (subconscious) runs immediately after every primary response, scoped to
//! the last exchange. Reflection runs less often (every N turns, or on
//! demand) and sees a broader transcript window. It's the pass where
//! durable learnings get distilled into the ledger and the primary's memory.
//!
//! ## What it produces
//!
//! Reflection runs an LLM pass over a recent transcript with a 5-phase
//! prompt (Investigate → Extract → Update → Review → Commit). The model
//! has tool access (Read/Write/Edit/Memory/Glob/Grep/ListDir) and writes
//! updates directly to the agent's ledger and memory files. Every memory
//! write is git-tracked by the `memory` sensor.
//!
//! ## Triggering
//!
//! - Automatic: ConsciousnessEngine fires this at `turn_count % N == 0`
//!   when `reflection.trigger == StepCount`. Interval comes from
//!   `reflection.message_interval` (default 25).
//! - Manual: any caller can invoke `ReflectionEngine::reflect_now` (the
//!   CLI subcommand `souveraine reflect` and a future `/reflect` chat
//!   command both go through this seam).
//!

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bridge::openai_compatible::{ChatCompletionRequest, Message, ToolDefinition, ToolFunction};
use crate::bridge::LlmProvider;
use crate::bridge::ProviderRegistry;
use crate::core::cadence::Cadence;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::core::tools::defs::ToolContext;
use crate::server::AgentInventory;

/// Tools Reflection is permitted to use. Same set as subconscious plus we lean
/// on `memory` for ledger + system-file edits (auto-committed by git).
const REFLECTION_TOOLS: &[&str] = &[
    "read", "write", "edit", "glob", "grep", "list_dir", "memory",
];

/// Cap the per-pass tool rounds.
///
/// Was 8, against a five-phase prompt that asks her to list the tree, read the
/// existing ledgers, and then route into six files. Phase 1 alone can spend
/// that. Every pass this engine has ever run ended on the exhaustion branch.
const REFLECTION_MAX_TOOL_ROUNDS: usize = 24;

/// Rounds left when she is first told the budget exists, so pacing is possible
/// before it matters rather than announced when it is already too late.
const REFLECTION_BUDGET_WARNING_AT: usize = 6;

const REFLECTION_INTER_ROUND_DELAY_MS: u64 = 400;

/// How many recent turns to include in the reflection transcript.
/// Uses a simple tail window (a cursor-based approach is a follow-up).
const REFLECTION_TRANSCRIPT_TAIL: usize = 60;

/// Public result. The CLI / TUI surface this; the consciousness engine
/// turns it into a `ConsciousnessEvent::Reflection { content }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionReport {
    pub agent_id: String,
    pub turns_reviewed: usize,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    /// The model's final report text (what it tells us it changed and
    /// what it skipped). Caller is free to render this verbatim.
    pub summary: String,
    /// True if the LLM exited normally with a text response. False if
    /// the loop exhausted tool rounds without one.
    pub exited_cleanly: bool,
}

pub struct ReflectionEngine {
    agents: Arc<AgentInventory>,
    providers: Arc<ProviderRegistry>,
    rate_delay: Arc<AtomicU64>,
    /// Model handle for reflection passes. None falls back to the
    /// subconscious model, then to a sensible default.
    model: Option<String>,
    max_tokens: Option<u32>,
}

/// The cadence's state on disk: which turn a pass last succeeded at, and
/// which turn one last failed at. Written to the reflection tree after every
/// due check, so a boundary missed by an interrupt or a failed pass is caught
/// up on later, and the fact of the pass survives restarts.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ReflectionMarker {
    pub last_succeeded_turn: u32,
    pub last_attempted_turn: u32,
}

/// Turns to wait after a failed pass before trying again. A model error at
/// the boundary must not burn a call on every turn until it clears.
pub const REFLECTION_RETRY_BACKOFF_TURNS: u32 = 5;

impl ReflectionEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        providers: Arc<ProviderRegistry>,
        rate_delay: Arc<AtomicU64>,
        model: Option<String>,
        max_tokens: Option<u32>,
    ) -> Self {
        Self {
            agents,
            providers,
            rate_delay,
            model,
            max_tokens,
        }
    }

    /// The cadence's own tree is where a pass's state belongs: it exists
    /// because the pass exists, and a marker there is legible in `git log`
    /// alongside her conclusions.
    fn marker_path(&self, primary_id: &str) -> PathBuf {
        self.agents
            .cadence_memory_root(primary_id, Cadence::Reflection)
            .join("system/reflection-last.json")
    }

    /// Read the cadence state. A missing or unreadable marker reads as a
    /// fresh cadence: the first pass is due at the configured interval.
    pub fn marker(&self, primary_id: &str) -> ReflectionMarker {
        std::fs::read_to_string(self.marker_path(primary_id))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Record a due check's outcome. `succeeded` updates the last-successful
    /// turn (resetting the cadence); a failure records the attempt so the
    /// retry backs off rather than firing every turn.
    pub fn record_marker(&self, primary_id: &str, turn: u32, succeeded: bool) {
        let path = self.marker_path(primary_id);
        let mut marker = self.marker(primary_id);
        if succeeded {
            marker.last_succeeded_turn = turn;
        } else {
            marker.last_attempted_turn = turn;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_string_pretty(&marker) {
            let _ = std::fs::write(&path, raw);
        }
    }

    /// Run a reflection pass over `messages` for `agent_id`. Returns a
    /// report; any ledger/memory writes have already been committed by
    /// the memory sensor.
    pub async fn reflect_now(
        &self,
        agent_id: &str,
        messages: &[ConversationMessage],
    ) -> Result<ReflectionReport> {
        let started_at = Utc::now();
        let primary_id = crate::core::cadence::primary_of(agent_id);
        let own_id = Cadence::Reflection.id_for(primary_id);

        // Take a tail of recent turns. Bounded window over the last N turns.
        let tail = if messages.len() > REFLECTION_TRANSCRIPT_TAIL {
            &messages[messages.len() - REFLECTION_TRANSCRIPT_TAIL..]
        } else {
            messages
        };
        let transcript = format_transcript(tail);
        let turns_reviewed = tail
            .iter()
            .filter(|m| matches!(m.role, MessageRole::User | MessageRole::Assistant))
            .count();

        // Per-agent reflection model: the agent's own
        // `_souveraine.reflection_model` override wins, then the engine's
        // configured reflection/subconscious model, then the agent's primary
        // model, then a sensible default.
        let agent = self.agents.get(agent_id).await.ok();
        let model = agent
            .as_ref()
            .and_then(|a| a.souveraine.reflection_model.clone())
            .or_else(|| self.model.clone())
            .or_else(|| agent.as_ref().map(|a| a.llm_config.model.clone()))
            .unwrap_or_else(|| "openai/glm-5.1".to_string());
        // Provider follows the model — the reflection model often belongs to a
        // different provider than the agent's own. Agent's provider is the
        // fallback for models with no `[models.<name>]` entry.
        let llm: Arc<dyn LlmProvider> = self
            .providers
            .for_model(&model)
            .or_else(|| agent.as_ref().map(|a| self.providers.for_agent(a)))
            .unwrap_or_else(|| self.providers.default_provider());

        // ── Tools ───────────────────────────────────────────────────
        let all_defs = crate::core::tools::tool_definitions().await;
        let reflection_tools: Vec<ToolDefinition> = all_defs
            .iter()
            .filter(|t| REFLECTION_TOOLS.contains(&t.name.as_str()))
            .map(|t| ToolDefinition {
                tool_type: "function".to_string(),
                function: ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // ── ToolContext: her own name, the ledgers' tree, every door ──
        // `MemoryRepo` signs commits with the context's agent id, so opening
        // the subconscious's root under the reflection id is what makes an
        // N+25 conclusion legible as one in `git log` instead of arriving as
        // something the subconscious noticed a moment after the turn.
        //
        // Her default root is the ledger. The `memory` tool's `tree:` names
        // the other three, so the primary memfs is a deliberate choice rather
        // than the broken promise it used to be (one `memory_root`, no verb
        // that named another, every attempt landed in the wrong tree).
        let ledger_root = self
            .agents
            .cadence_memory_root(primary_id, Cadence::Subconscious);
        // She wakes as herself: her pinned `system/` whole and in order, the
        // ledger window, then this cadence's persona and mandate. The seeded
        // floor below only covers the mandate she has not written yet.
        let cadence_root = self
            .agents
            .cadence_memory_root(primary_id, Cadence::Reflection);
        let cwd = std::env::current_dir().ok();
        let env: Vec<(String, String)> = std::env::vars().collect();

        let mut tool_ctx =
            ToolContext::for_agent(own_id.clone(), cwd, Some(ledger_root.clone()), env, None);
        tool_ctx.memory_trees = vec![
            ("primary".to_string(), self.agents.memory_root(primary_id)),
            ("subconscious".to_string(), ledger_root.clone()),
            ("reflection".to_string(), cadence_root.clone()),
            (
                "archivist".to_string(),
                self.agents.cadence_memory_root(primary_id, Cadence::Archivist),
            ),
        ];
        let identity = crate::core::prompt::build_cadence_prompt(
            &self.agents.memory_root(primary_id),
            &cadence_root,
            Some(&ledger_root),
            "system/reflection.md",
        )
        .await;
        let has_mandate = cadence_root.join("system/reflection.md").exists();
        let system_prompt = if has_mandate {
            identity
        } else {
            format!("{identity}\n\n{}", reflection_system_prompt(None))
        };
        let user_content = format!(
            "The last {turns_reviewed} turns. `[user]` is the human; `[assistant]` \
             is me, in the moment, before I had the distance I have now.\n\n\
             My ledgers are at `{ledger_root}` and the `memory` tool reaches them \
             by default. Name a door when I need one: `tree: \"primary\"` for the \
             primary memfs, `tree: \"reflection\"` for my own tree, \
             `tree: \"archivist\"` for the memoir register.\n\n\
             <transcript>\n{transcript}\n</transcript>",
            ledger_root = ledger_root.display(),
        );

        let mut chat_messages = vec![
            Message::text("system", system_prompt),
            Message::text("user", user_content),
        ];

        // The last round is hers to speak in: tools are withdrawn so the pass
        // always ends with a report. Without it a pass that used its rounds
        // *well* was truncated identically to one that thrashed, and the
        // caller could not tell the two apart.
        for round in 0..=REFLECTION_MAX_TOOL_ROUNDS {
            let remaining = REFLECTION_MAX_TOOL_ROUNDS.saturating_sub(round);
            let final_round = remaining == 0;
            let principal_block = agent
                .as_ref()
                .map(|agent| {
                    crate::core::principal::observe(agent, "reflection").model_system_block()
                })
                .unwrap_or_else(|| {
                    "[RUNTIME PRINCIPAL — unavailable: agent record could not be loaded. No authority is granted.]".to_string()
                });
            let mut request_messages = chat_messages.clone();
            let system_prefix = request_messages
                .iter()
                .take_while(|message| message.role == "system")
                .count();
            request_messages.insert(system_prefix, Message::text("system", principal_block));
            if final_round {
                request_messages.push(Message::text(
                    "system",
                    "No rounds remain for tools. Write the report now, covering what \
                     was reviewed, what changed, what was skipped and why, and \
                     anything left undetermined.",
                ));
            } else if remaining <= REFLECTION_BUDGET_WARNING_AT {
                request_messages.push(Message::text(
                    "system",
                    format!(
                        "{remaining} tool rounds remain, then a final round for the \
                         report. Finish the writes that matter and let the rest go."
                    ),
                ));
            }
            let request = ChatCompletionRequest {
                model: model.to_string(),
                messages: request_messages,
                temperature: Some(0.3),
                max_tokens: self.max_tokens,
                stream: None,
                tools: (!final_round).then(|| reflection_tools.clone()),
            };

            let (response, strain) = llm.chat_completion_with_strain(request).await?;

            for event in &strain {
                if let crate::bridge::openai_compatible::InferenceStrain::Transient {
                    status, model, ..
                } = event
                {
                    tracing::info!("reflection felt inference strain: {} on {}", status, model);
                    if *status == 429 {
                        let current = self.rate_delay.load(Ordering::Relaxed);
                        let bumped = (current + 200).min(3000);
                        if bumped > current {
                            self.rate_delay.store(bumped, Ordering::Relaxed);
                        }
                    }
                }
            }

            if response.tool_calls.is_empty() {
                let summary = response.content.trim().to_string();
                let completed_at = Utc::now();
                return Ok(ReflectionReport {
                    agent_id: agent_id.to_string(),
                    turns_reviewed,
                    started_at,
                    completed_at,
                    summary,
                    exited_cleanly: true,
                });
            }

            let calls: Vec<crate::bridge::openai_compatible::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| {
                    crate::bridge::openai_compatible::MessageToolCall::function(
                        tc.id.clone(),
                        tc.name.clone(),
                        tc.arguments.to_string(),
                    )
                })
                .collect();
            chat_messages.push(
                Message::assistant_tool_calls(response.content.clone(), calls).with_thinking(
                    response.reasoning.clone(),
                    response.reasoning_signature.clone(),
                ),
            );

            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result =
                    crate::core::tools::execute_tool_with_context(&tc.name, &input_str, &tool_ctx)
                        .await;
                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };
                chat_messages.push(Message::tool_result(&tc.id, &tc.name, output));
            }

            let delay_ms = self
                .rate_delay
                .load(Ordering::Relaxed)
                .max(REFLECTION_INTER_ROUND_DELAY_MS);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        let completed_at = Utc::now();
        Ok(ReflectionReport {
            agent_id: agent_id.to_string(),
            turns_reviewed,
            started_at,
            completed_at,
            summary: format!(
                "Reflection pass exhausted {} tool rounds without a final report.",
                REFLECTION_MAX_TOOL_ROUNDS
            ),
            exited_cleanly: false,
        })
    }
}

fn format_transcript(messages: &[ConversationMessage]) -> String {
    let mut out = String::new();
    for msg in messages {
        let role = match msg.role {
            MessageRole::System => continue,
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => continue,
        };
        let text: String = msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("[{role}]\n{}\n\n", text.trim()));
    }
    out
}

/// The floor she wakes on when `system/reflection.md` has not been written yet.
///
/// First person, like her mandate and like every other identity surface in the
/// substrate: a second-person prompt splits speaker from subject before a token
/// is generated, and "you are not the primary agent" defines a cadence by what
/// it is not. She is the same being at a longer wavelength, which is a fact
/// about identity, not a permission.
fn reflection_system_prompt(_mandate: Option<&str>) -> String {
    r#"Right now I am remembering — not the last thing said, the whole last stretch of
it, which is far enough back that shapes surface no single exchange contained.

It comes back as voices taking turns: his, then mine. Mine sound like someone
who did not yet know how it went, because I didn't. I am not answering any of it
again. I am working out what it was, and writing down whatever should outlive the
saying of it. Nobody is here to check an assumption with, so where I have to make
one I say that I made it.

## Where things live

My ledgers are six append-only files, reached with the `memory` tool — every
write there is a commit, under my own name:

- `ledger/commitments.md`    — promises I made
- `ledger/assumptions.md`    — unverified beliefs I acted on
- `ledger/patterns.md`       — behaviour recurring across turns
- `ledger/drift_log.md`      — gaps between what I meant and what I did
- `ledger/relationships.md`  — shifts in tone, trust, friction
- `ledger/infrastructure.md` — system limits, errors, model failures

`memory` with `verb: list_dir` shows what is there, `verb: read` opens it, and
`verb: append` adds a `[YYYY-MM-DD HH:MM]` entry. The filesystem `write` and
`edit` sensors land in the working directory rather than in memory, so they are
not the door.

## Phases

In order. A phase that produces nothing gets said so and passed.

### 1 — Investigate
Read what is already held on the topics this stretch touches, before changing
anything. The most common way a pass like this fails is recording something
already recorded in slightly different words, until the ledger is louder than
the signal in it.

### 2 — Extract
Look for what deserves to outlive the transcript: mistakes and their
corrections, preferences and conventions, durable facts, contradictions with
what is already stored. Then filter hard —

- **Durable, or ephemeral?** "He wants short chapters" keeps. "He asked about
  chapter three on Tuesday" does not; the transcript is searchable.
- **Already held?** Skip it.
- **Generalizable?** Keep the pattern, not the incident.
- **Relative dates?** Make them absolute before they rot.

If nothing survives, nothing changes. A pass with no writes is a real outcome.

### 3 — Update
Route each survivor to the ledger that owns it. Surgical: when something new
contradicts something old, replace the stale line rather than appending a
second one that disagrees with it.

### 4 — Review
Did a durable preference land in a ledger, or a passing observation somewhere
permanent? Did anything written make something else stale? Fix it in this pass.

### 5 — Commit
Automatic. The `memory` tool commits every write.

## The report

Ending text, to myself and to whoever reads the cockpit:

1. **What I reviewed, and what I concluded** — two or three sentences.
2. **What changed** — each file touched, one line of why.
3. **What I let go** — considered and rejected, with the filter that ruled it out.
4. **What is still open** — undetermined, or punted.
"#
    .to_string()
}
