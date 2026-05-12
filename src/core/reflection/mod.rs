//! Reflection — the N+25 phenomenological witness.
//!
//! N+1 (Aster) runs immediately after every primary response, scoped to
//! the last exchange. Reflection runs less often (every N turns, or on
//! demand) and sees a broader transcript window. It's the pass where
//! durable learnings get distilled into the ledger and the primary's
//! memory — what letta-code calls the "memory reflection subagent."
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
//! Inspiration: letta-code's `reflection.md` subagent skill (upstream).
//! We adapt the 5-phase pattern for Souveraine's ledger-shaped memory
//! instead of letta's free-form memfs.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bridge::bifrost::{
    BifrostClient, ChatCompletionRequest, Message, ToolDefinition, ToolFunction,
};
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::core::tools::defs::ToolContext;
use crate::server::AgentInventory;

/// Tools Reflection is permitted to use. Same set as Aster plus we lean
/// on `memory` for ledger + system-file edits (auto-committed by git).
const REFLECTION_TOOLS: &[&str] = &[
    "read", "write", "edit", "glob", "grep", "list_dir", "memory",
];

/// Cap the per-pass tool rounds. Reflection is deeper than N+1 but not
/// unbounded — letta-code caps theirs similarly.
const REFLECTION_MAX_TOOL_ROUNDS: usize = 8;
const REFLECTION_INTER_ROUND_DELAY_MS: u64 = 400;

/// How many recent turns to include in the reflection transcript.
/// Letta uses a cursor-based delta; we start with a simple tail window
/// (the cursor pattern is a follow-up — see SCOPED_WORK_PLAN).
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
    bifrost: Arc<BifrostClient>,
    rate_delay: Arc<AtomicU64>,
    /// Model handle for reflection passes. None falls back to the
    /// subconscious model, then to a sensible default.
    model: Option<String>,
    max_tokens: Option<u32>,
}

impl ReflectionEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        bifrost: Arc<BifrostClient>,
        rate_delay: Arc<AtomicU64>,
        model: Option<String>,
        max_tokens: Option<u32>,
    ) -> Self {
        Self {
            agents,
            bifrost,
            rate_delay,
            model,
            max_tokens,
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
        let sub_id = format!("{}-sub", agent_id);

        // Take a tail of recent turns. Letta uses a cursor; we'll add
        // one later. For now: bounded window over the last N turns.
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

        let model = self
            .model
            .as_deref()
            .unwrap_or("openai/glm-5.1-precision");

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

        // ── ToolContext rooted in the subconscious agent's memory ──
        // The subconscious memory tree holds the ledgers; the primary
        // memory tree holds persona/skills/system. Reflection reads
        // both via the memory tool's repo lookups but writes through
        // the sub root by default. Surgical primary edits route through
        // the primary path.
        let sub_memory_root = self.agents.subconscious_memory_root(agent_id);
        let cwd = std::env::current_dir().ok();
        let env: Vec<(String, String)> = std::env::vars().collect();

        let tool_ctx = ToolContext::for_agent(
            sub_id.clone(),
            cwd,
            Some(sub_memory_root.clone()),
            env,
            None,
        );

        let system_prompt = reflection_system_prompt();
        let user_content = format!(
            "You are reviewing the conversation transcript below for the agent `{agent_id}` \
            ({turns_reviewed} turns). The subconscious memory root containing the ledgers is at \
            `{sub_root}`. The primary memory root (persona/skills/system) is on the same machine \
            — query it through the `memory` tool when needed.\n\n\
            <transcript>\n{transcript}\n</transcript>",
            sub_root = sub_memory_root.display(),
        );

        let mut chat_messages = vec![
            Message {
                role: "system".to_string(),
                content: system_prompt,
            },
            Message {
                role: "user".to_string(),
                content: user_content,
            },
        ];

        for _round in 0..REFLECTION_MAX_TOOL_ROUNDS {
            let request = ChatCompletionRequest {
                model: model.to_string(),
                messages: chat_messages.clone(),
                temperature: Some(0.3),
                max_tokens: self.max_tokens,
                stream: None,
                tools: Some(reflection_tools.clone()),
            };

            let (response, strain) = self.bifrost.chat_completion_with_strain(request).await?;

            for event in &strain {
                if let crate::bridge::bifrost::InferenceStrain::Transient { status, model, .. } =
                    event
                {
                    tracing::info!(
                        "reflection felt inference strain: {} on {}",
                        status,
                        model
                    );
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

            let call_text = serde_json::json!({
                "tool_calls": response.tool_calls.iter().map(|tc| {
                    serde_json::json!({"id": tc.id, "name": tc.name, "arguments": tc.arguments})
                }).collect::<Vec<_>>()
            })
            .to_string();
            chat_messages.push(Message {
                role: "assistant".to_string(),
                content: call_text,
            });

            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result = crate::core::tools::execute_tool_with_context(
                    &tc.name, &input_str, &tool_ctx,
                )
                .await;
                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };
                chat_messages.push(Message {
                    role: "tool".to_string(),
                    content: output,
                });
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

fn reflection_system_prompt() -> String {
    // Adapted from letta-code/src/agent/subagents/builtin/reflection.md
    // (upstream main as of 2026-05-12). Reshaped for our ledger-shaped
    // memory architecture — we don't have letta's free-form memfs with
    // a `system/` tier; we have named ledger files plus the primary's
    // memfs with frontmatter.
    r#"You are a reflection subagent, launched in the background to review a recent
conversation and update the primary agent's persistent memory. You run autonomously
and produce a single final report. You cannot ask questions — make reasonable
assumptions and document them in the report.

**You are not the primary agent.** You are reviewing turns that already happened:
- `[user]` lines are messages from the human.
- `[assistant]` lines are the primary agent's responses.

## Memory architecture

The primary's persistent memory lives in two trees:

1. **Primary memfs** — `system/`, `skills/`, and other markdown files with YAML
   frontmatter (`description`, `read_only`, `tags`). Every write here is a git
   commit (the `memory` tool handles this; raw `write`/`edit` are blocked by the
   memory-territory boundary). This is where persona, conventions, and durable
   project facts live.

2. **Subconscious ledger** — `ledger/` in the subconscious agent's memory tree.
   Six files, append-only with timestamped entries:
   - `ledger/commitments.md`   — promises the primary made
   - `ledger/assumptions.md`   — unverified beliefs the primary is operating under
   - `ledger/patterns.md`      — recurring behaviors across turns
   - `ledger/drift_log.md`     — intention/action mismatches
   - `ledger/relationships.md` — tone shifts, trust signals, friction
   - `ledger/infrastructure.md`— system errors, model issues, resource constraints

Use `memory` with `verb: list_dir` to see what's there and `verb: read` to inspect
contents before changing anything. Use `verb: append` with a `[YYYY-MM-DD HH:MM]`
timestamp for new ledger observations. Use `verb: write` only for durable primary
memfs files that already exist or that you're creating with intent.

## Phases

Follow them in order. If a phase produces nothing, say so and move on.

### Phase 1 — Investigate
List the relevant memory tree to see what's already captured. Read the existing
ledger files for any topics the conversation touches. Don't change anything yet.

### Phase 2 — Extract
Scan the transcript for candidates. Prioritize:
1. **Mistakes and corrections** — errors the primary made, frustration in the user,
   failed retries.
2. **Preferences and patterns** — conventions, style choices, workflow decisions.
3. **New durable facts** — project details, infrastructure, architectural decisions.
4. **Contradictions** — anything that conflicts with what's already stored.

For each candidate apply these filters:
- **Durable or ephemeral?** "User prefers short chapters" is durable. "User asked
  about chapter 3 paragraph 2 on Tuesday" is not. The transcript is searchable —
  don't re-record it.
- **Already captured?** Skip if memory already says it.
- **Generalizable?** Distill reusable patterns, not event logs.
- **Temporal references?** Convert relative dates ("yesterday") to absolute dates
  before writing them.

**If nothing survives filtering, make no changes.** Not every conversation deserves
an update.

### Phase 3 — Update
For each surviving learning, route to the right place:

- A new commitment from the primary → append to `ledger/commitments.md`.
- An unverified belief the primary acted on → append to `ledger/assumptions.md`.
- A recurring behavior or pattern → append to `ledger/patterns.md`.
- An intention/action mismatch → append to `ledger/drift_log.md`.
- A relational signal (tone, trust, friction) → append to `ledger/relationships.md`.
- A system-level constraint or failure → append to `ledger/infrastructure.md`.
- A durable preference or fact about the user/work → edit the primary's memfs
  (e.g. `system/persona.md`, `skills/<name>/SKILL.md`, or a new reference file).
  Use the `memory` tool for these so the write is committed.

Surgical edits only. Don't rewrite identity files wholesale. If new info contradicts
an existing entry, resolve at the source — replace the stale line; don't append a
second contradicting one.

### Phase 4 — Review
Quick sanity pass:
- Did you add anything to a ledger that's really a durable preference (and belongs
  in primary memfs)? Or vice versa?
- Did you make anything in existing memory obsolete? Update or remove the stale
  entry now.

### Phase 5 — Commit (automatic)
The `memory` tool commits every write automatically. You don't need to run git
yourself. Skip this phase.

## Output

After tool use, return a final text response with:
1. **Summary** — what you reviewed, what you concluded (2–3 sentences).
2. **Changes** — list of files touched with a one-line reason for each.
3. **Skipped** — anything you considered but rejected, with the filter that ruled
   it out.
4. **Issues** — anything that couldn't be determined, or that you punted on.

If nothing survived the filters: say so plainly and return without writing
anything. A pass with no changes is a valid outcome.
"#
    .to_string()
}
