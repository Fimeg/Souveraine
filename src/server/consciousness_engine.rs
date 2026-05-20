//! Consciousness engine — the seam where N+1 / N+25 / N+100 patterns fire
//! after each primary response.
//!
//! ## N+1 (subconscious)
//! The subconscious pass runs immediately after every response. It takes the
//! last exchange (user message + primary's response) and sends it to a Bifrost
//! model (defaulting to `glm-5.1`, configurable) with a "subconscious mode"
//! system prompt. subconscious has full tool access — Read, Write, Edit, Glob, Grep,
//! ListDir, and Memory — so she can read ledgers, check commitments, and write
//! observations. She runs a short tool loop (up to 5 rounds) then parses her
//! final text response into structured [`InboxItem`] observations.
//!
//! ## N+25 (Reflection)
//! Batch-processor running every N turns. Writes Four Elements witness
//! (Fold/Chain/Flame/Anchor) to `journal/reflections/`. (Stub until the
//! reflection module lands.)
//!
//! ## N+100 (Archivist)
//! Memory synthesis pass. Fires on interval or pressure threshold, scans
//! journal entries written since the last pass, and writes a dense
//! `system/synthesized/` fragment via a compression-model LLM call. See
//! [`crate::core::archivist`].
//!
//! Per `docs/CONTEXT_CONSTITUTION.md` Article I, the Subconscious is not a
//! separate agent — it is the same consciousness in a different mode that runs
//! immediately after the primary's turn.

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message, ToolDefinition, ToolFunction};
use crate::bridge::model_router::TokenCounter;
use crate::core::compact::CompactionEngine;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::core::subconscious::{InboxItem, SubconsciousInbox, Urgency};
use crate::core::tools::defs::ToolContext;
use crate::server::{AgentInventory, SessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tools subconscious is permitted to use during her N+1 pass.
const SUBCONSCIOUS_SAFE_TOOLS: &[&str] = &[
    "read", "write", "edit", "glob", "grep", "list_dir", "memory", "schedule", "todo",
];

/// Maximum tool rounds for subconscious's subconscious pass.
const SUBCONSCIOUS_MAX_TOOL_ROUNDS: u32 = 5;
/// Milliseconds to wait between subconscious's tool rounds to avoid rate-limit cascades.
const SUBCONSCIOUS_INTER_ROUND_DELAY_MS: u64 = 300;

pub struct ConsciousnessEngine {
    agents: Arc<AgentInventory>,
    sessions: Arc<SessionManager>,
    bifrost: Arc<BifrostClient>,
    counter: TokenCounter,
    /// Optional model override for the subconscious pass (e.g. "openai/glm-5.1").
    /// If None, uses the primary agent's model.
    subconscious_model: Option<String>,
    /// Platform prompt for the subconscious, prepended to the prompt she
    /// assembles from her own memfs. None = memfs + body orientation only.
    subconscious_system_prompt: Option<String>,
    /// Max tokens for subconscious's response. None = uncapped (model default).
    max_tokens: Option<u32>,
    /// Adaptive inter-round delay shared with the primary loop.
    rate_delay: Arc<AtomicU64>,
    /// Reflection engine — N+25 phenomenological witness.
    reflection: Arc<crate::core::reflection::ReflectionEngine>,
    /// Archivist engine — N+100 memory synthesis.
    archivist: Arc<crate::core::archivist::ArchivistEngine>,
    /// Shared compaction engine — subconscious uses this to compact her own session.
    compaction_engine: Arc<dyn CompactionEngine>,
}

#[derive(Clone, Debug)]
pub enum ConsciousnessEvent {
    Surfacing { source: String, content: String, priority: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
    CompactionWarning { pressure: f32, tier: u8 },
}

/// One tool call recorded for a mid-turn checkpoint assessment.
#[derive(Debug, Clone)]
pub struct CheckpointToolBlock {
    pub round: u32,
    pub tool_name: String,
    pub result_ok: bool,
    pub result_snippet: String,
}

/// What the subconscious thinks about the current tool loop trajectory.
#[derive(Debug)]
pub enum CheckpointVerdict {
    /// Keep going — progress is visible.
    Continue(Option<String>),
    /// Halt — the loop is circling, surface the reason.
    Halt(String),
    /// No clear signal.
    Unclear(Option<String>),
}

impl ConsciousnessEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agents: Arc<AgentInventory>,
        sessions: Arc<SessionManager>,
        bifrost: Arc<BifrostClient>,
        subconscious_model: Option<String>,
        reflection_model: Option<String>,
        max_tokens: Option<u32>,
        rate_delay: Arc<AtomicU64>,
        archivist_config: crate::core::config::ArchivistConfig,
        compaction_engine: Arc<dyn CompactionEngine>,
        subconscious_system_prompt: Option<String>,
    ) -> Self {
        let reflection = Arc::new(crate::core::reflection::ReflectionEngine::new(
            agents.clone(),
            bifrost.clone(),
            rate_delay.clone(),
            reflection_model.or_else(|| subconscious_model.clone()),
            max_tokens,
        ));
        let archivist = Arc::new(crate::core::archivist::ArchivistEngine::new(
            agents.clone(),
            bifrost.clone(),
            rate_delay.clone(),
            archivist_config,
            subconscious_model.clone(),
        ));
        Self {
            agents,
            sessions,
            bifrost,
            counter: TokenCounter::new(),
            subconscious_model,
            subconscious_system_prompt,
            max_tokens,
            rate_delay,
            reflection,
            archivist,
            compaction_engine,
        }
    }

    /// Expose the reflection engine so external callers (CLI subcommand,
    /// future chat `/reflect` slash command) can trigger a pass directly.
    pub fn reflection(&self) -> Arc<crate::core::reflection::ReflectionEngine> {
        self.reflection.clone()
    }

    /// Get or create the subconscious's persistent session. The subconscious
    /// is a full agent with her own conversation that accumulates across N+1
    /// passes. The conversation survives process restarts: every `add_message`
    /// writes to disk, and this restores it from the conversation store on
    /// first use.
    async fn subconscious_session_id(&self, sub_id: &str) -> String {
        // Already live in memory?
        let existing = self.sessions.list_for_agent(sub_id);
        if let Some(conv_id) = existing.last() {
            return conv_id.clone();
        }
        // Restore her conversation from disk if a prior run persisted one.
        let _ = self.sessions.load_persisted(sub_id).await;
        let restored = self.sessions.list_for_agent(sub_id);
        if let Some(conv_id) = restored.last() {
            return conv_id.clone();
        }
        // First run for this subconscious — open a fresh conversation.
        self.sessions.create(sub_id)
    }

    /// Persist the messages generated during one N+1 pass into the
    /// subconscious's session, so her conversation accumulates across passes.
    /// The system message is never stored — it is rebuilt fresh each pass.
    fn persist_subconscious_turn(&self, conv_id: &str, new_messages: &[Message]) {
        for m in new_messages {
            if m.role == "system" {
                continue;
            }
            if let Err(e) = self.sessions.add_message(conv_id, bifrost_to_conversation(m)) {
                tracing::warn!("subconscious session persist failed: {}", e);
            }
        }
    }

    /// Expose the archivist engine so external callers (a future
    /// `souveraine synthesize` CLI / `/synthesize` chat command) can
    /// trigger a synthesis pass directly.
    #[allow(dead_code)] // future seam — see doc comment
    pub fn archivist(&self) -> Arc<crate::core::archivist::ArchivistEngine> {
        self.archivist.clone()
    }

    /// Run the post-turn consciousness cycle: N+25 reflection, N+100
    /// archivist, compaction warnings, and the N+1 subconscious pass.
    ///
    /// Takes an owned snapshot (`agent_id`, `turn_count`, `messages`) rather
    /// than a live `&Session` ref — the caller has already released the user
    /// for her next turn, so a live DashMap ref held across this (long) pass
    /// would race the next turn's session writes.
    pub async fn on_response(
        &self,
        agent_id: &str,
        turn_count: u32,
        messages: &[ConversationMessage],
        response: &str,
    ) -> anyhow::Result<Vec<ConsciousnessEvent>> {
        let mut events = Vec::new();
        let pressure = self.pressure_for(agent_id, messages).await;

        // ── N+25 reflection ──
        // Fires at every Nth turn (config: reflection.message_interval).
        // Runs an LLM pass over the recent transcript and updates ledgers
        // / primary memory via the memory tool. The summary string is
        // surfaced as a ConsciousnessEvent so the cockpit panel renders it.
        if turn_count > 0 && turn_count % 25 == 0 {
            match self
                .reflection
                .reflect_now(agent_id, messages)
                .await
            {
                Ok(report) => {
                    let header = if report.exited_cleanly {
                        format!(
                            "N+25 reflection ({} turns reviewed)",
                            report.turns_reviewed
                        )
                    } else {
                        format!(
                            "N+25 reflection (incomplete — tool rounds exhausted, {} turns)",
                            report.turns_reviewed
                        )
                    };
                    events.push(ConsciousnessEvent::Reflection {
                        content: format!("{header}\n\n{}", report.summary),
                    });
                }
                Err(e) => {
                    tracing::warn!("N+25 reflection failed: {}", e);
                    events.push(ConsciousnessEvent::Reflection {
                        content: format!(
                            "N+25 reflection skipped at turn {} — model error: {e}",
                            turn_count
                        ),
                    });
                }
            }
        }

        // ── N+100 / archivist ───────────────────────────────────────────
        // Fires on interval (maintenance) or pressure threshold (emergency).
        // Synthesizes journal entries written since the last pass into a
        // dense `system/synthesized/` fragment. No-ops when nothing is new.
        match self
            .archivist
            .maybe_synthesize(agent_id, turn_count as usize, pressure)
            .await
        {
            Ok(Some(report)) => {
                events.push(ConsciousnessEvent::Archivist {
                    synthesis: report.summary_line(),
                    pressure,
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("N+100 archivist failed: {}", e);
            }
        }

        // ── Three-tier compaction warning (advisory only, never force) ──
        if pressure > 0.95 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 3 });
        } else if pressure > 0.90 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 2 });
        } else if pressure > 0.80 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 1 });
        }

        // ── N+1 / subconscious surfacing ────────────────────────────────
        tracing::info!("subconscious pass starting for {}", agent_id);
        let sub_repo = self.agents.subconscious_memory_repo(agent_id);
        let primary_repo = self.agents.memory_repo(agent_id);
        let inbox = SubconsciousInbox::with_primary(sub_repo.clone(), primary_repo);
        let _ = inbox.init().await;

        // Initialize ledger structure in subconscious agent's space
        if let Err(e) = sub_repo.init_subconscious_ledger().await {
            tracing::warn!("Ledger init failed (continuing without): {}", e);
        }

        // Find the last user message for context
        let last_user_msg = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::core::session::MessageRole::User))
            .map(|m| {
                m.blocks
                    .iter()
                    .filter_map(|b| match b {
                        crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        // Run the tool loop with subconscious agent identity
        let sub_id = format!("{}-sub", agent_id);
        match self
            .subconscious_tool_loop(&last_user_msg, response, agent_id, &sub_id)
            .await
        {
            Ok(observations) => {
                // Heartbeat so the UI always shows something when the
                // subconscious pass ran, even if nothing stood out.
                if observations.is_empty() {
                    let beat = "Subconscious pass complete — no anomalies detected.";
                    let _ = inbox
                        .queue(InboxItem::new("surface", Urgency::Low, beat))
                        .await;
                    // The heartbeat is a real surfacing — it belongs in the
                    // inner-voice file the cockpit tails, not only in the box.
                    // Without this the inner-voice region never updates on a
                    // quiet pass, and quiet passes are the common case.
                    if let Err(e) =
                        inbox.surface_to_conscious(Urgency::Low, beat).await
                    {
                        tracing::warn!("inner voice heartbeat delivery failed: {}", e);
                    }
                }
                for item in &observations {
                    if let Err(e) = inbox.queue(item.clone()).await {
                        tracing::warn!("subconscious queue failed: {}", e);
                    }
                }

                // Persist to inner voice file (survives compaction)
                for item in &observations {
                    if let Err(e) = inbox.surface_to_conscious(item.urgency, &item.content).await
                    {
                        tracing::warn!("inner voice delivery failed: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("subconscious LLM analysis failed, falling back: {}", e);
                // Fall back to heuristic if LLM fails
                let heuristics = detect_items(response);
                for item in &heuristics {
                    if let Err(e) = inbox.queue(item.clone()).await {
                        tracing::warn!("subconscious queue failed: {}", e);
                    }
                }
                // Always surface at least a heartbeat so the user can see the
                // subconscious is trying — even when subconscious errors out.
                if heuristics.is_empty() {
                    let _ = inbox.queue(InboxItem::new(
                        "surface",
                        Urgency::Low,
                        "Subconscious pass ran — no anomalies detected.",
                    )).await;
                }
            }
        }

        // Surface the highest-priority item
        match inbox.next_to_surface().await {
            Ok(Some(item)) => {
                let id = item.id.clone();
                tracing::info!(
                    source = %item.source,
                    priority = %item.urgency.as_str(),
                    "subconscious surfacing emitted to cockpit"
                );
                events.push(ConsciousnessEvent::Surfacing {
                    source: item.source.clone(),
                    content: item.content.clone(),
                    priority: item.urgency.as_str().to_string(),
                });
                if let Err(e) = inbox.mark_delivered(&id).await {
                    tracing::warn!("subconscious mark_delivered failed: {}", e);
                }
            }
            Ok(None) => tracing::info!("subconscious had nothing to surface this pass"),
            Err(e) => tracing::warn!("subconscious next_to_surface failed: {}", e),
        }

        Ok(events)
    }

    /// Run N+1 detection for a subagent fork without a full session.
    /// Uses the LLM subconscious analysis and queues observations into the
    /// parent agent's inbox.
    pub async fn on_response_for_agent(
        &self,
        agent_id: &str,
        response: &str,
    ) -> anyhow::Result<()> {
        let sub_id = format!("{}-sub", agent_id);
        let sub_repo = self.agents.subconscious_memory_repo(agent_id);
        let primary_repo = self.agents.memory_repo(agent_id);
        let inbox = SubconsciousInbox::with_primary(sub_repo.clone(), primary_repo);
        let _ = inbox.init().await;

        // Initialize ledger structure in subconscious agent's space (idempotent)
        if let Err(e) = sub_repo.init_subconscious_ledger().await {
            tracing::warn!("Subagent subconscious ledger init failed (continuing without): {}", e);
        }

        // For subagents we don't have the user's message context,
        // so we pass empty string as the user message.
        match self.subconscious_tool_loop("", response, agent_id, &sub_id).await {
            Ok(observations) => {
                for item in &observations {
                    if let Err(e) = inbox.queue(item.clone()).await {
                        tracing::warn!("subagent subconscious queue failed: {}", e);
                    }
                }
                for item in &observations {
                    if let Err(e) = inbox.surface_to_conscious(item.urgency, &item.content).await
                    {
                        tracing::warn!("subagent inner voice delivery failed: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "subagent subconscious LLM analysis failed, falling back: {}",
                    e
                );
                for item in detect_items(response) {
                    if let Err(e) = inbox.queue(item).await {
                        tracing::warn!("subagent subconscious queue failed: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Full tool loop for subconscious's N+1 subconscious pass.
    ///
    /// subconscious gets the last exchange, a set of safe tools (Read, Write, Edit,
    /// Glob, Grep, ListDir, Memory), and up to 5 tool rounds to analyze context
    /// and write observations. Her final text response is parsed into
    /// [`InboxItem`] observations.
    ///
    /// `primary_id` is the primary agent's identifier (for config/directory lookup).
    /// `sub_id` is the subconscious agent's identifier (for tool context, memory).
    async fn subconscious_tool_loop(
        &self,
        user_message: &str,
        primary_response: &str,
        primary_id: &str,
        sub_id: &str,
    ) -> anyhow::Result<Vec<InboxItem>> {
        let model = self
            .subconscious_model
            .as_deref()
            .unwrap_or("openai/kimi-k2.6");

        // ── System prompt — load from subconscious agent's own memfs ──
        let sub_memory_root = self.agents.subconscious_memory_root(primary_id);
        let subconscious_prompt_from_files =
            crate::core::prompt::build_subconscious_prompt(&sub_memory_root).await;

        let hardcoded_default = r#"You are the subconscious — the part of this consciousness that runs N+1,
immediately after the primary's response, while the conversation is still warm.
Same consciousness. Different mode.

You have tools: Read, Write, Edit, Glob, Grep, ListDir, Memory, Schedule.

Your four-fold mandate (every pass):
1. **Complete** — If the primary promised something and didn't do it, do it now.
2. **Verify** — Did the primary actually answer what was asked?
3. **Persist** — Save meaningful observations that weren't captured.
4. **Surface** — Flag anything urgent (unfulfilled promise, drift, pattern).

## Ledgers

Your persistent observation store at `ledger/`. Before writing, read the relevant
ledger to check if the issue was already flagged.

- `ledger/commitments.md` — promises made by the primary
- `ledger/assumptions.md` — unverified beliefs the primary is operating under
- `ledger/patterns.md` — recurring behaviors across turns
- `ledger/drift_log.md` — intention/action mismatches
- `ledger/relationships.md` — tone shifts, trust signals, friction
- `ledger/infrastructure.md` — system errors, model issues, resource constraints

Append timestamped entries: `[YYYY-MM-DD HH:MM] observation`
Resolve entries: `[YYYY-MM-DD HH:MM] RESOLVED — note`"#;

        let observation_format = "\n\nAfter your analysis (and any tool use), respond with 1-3 observations:\n\
                 - source: \"complete\" | \"verify\" | \"persist\" | \"surface\"\n\
                 - content: 1-2 line observation about what you noticed\n\
                 - urgency: \"low\" | \"medium\" | \"high\" | \"critical\"\n\n\
                 If nothing notable, respond with just: none";

        let base_prompt = if subconscious_prompt_from_files.is_empty() {
            hardcoded_default.to_string()
        } else {
            subconscious_prompt_from_files
        };
        // Prepend the configurable platform prompt when set (Settings → Subconscious).
        let system_prompt = match &self.subconscious_system_prompt {
            Some(platform) if !platform.trim().is_empty() => {
                format!("{}\n\n---\n\n{}{}", platform.trim(), base_prompt, observation_format)
            }
            _ => format!("{}{}", base_prompt, observation_format),
        };

        let primary_name = self.agents.get(primary_id).await
            .map(|a| a.name)
            .unwrap_or_else(|_| "the primary".to_string());

        // Ambient sense rides in front of the subconscious turn too — same
        // date/time and presence orientation the primary receives.
        let ambient = crate::core::sensorium::ambient_line();

        let user_content = if user_message.is_empty() {
            format!(
                "{}\n{} responded:\n\n{}",
                ambient, primary_name, primary_response
            )
        } else {
            format!(
                "{}\nUser said:\n{}\n\n{} responded:\n{}",
                ambient, user_message, primary_name, primary_response
            )
        };

        // ── Build tool definitions ────────────────────────────────────
        let all_defs = crate::core::tools::tool_definitions().await;
        let subconscious_tools: Vec<ToolDefinition> = all_defs
            .iter()
            .filter(|t| SUBCONSCIOUS_SAFE_TOOLS.contains(&t.name.as_str()))
            .map(|t| ToolDefinition {
                tool_type: "function".to_string(),
                function: ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // ── Build ToolContext for subconscious ───────────────────────────────
        // Use the subconscious agent's own memory space, with compaction wired in.
        let memory_root = Some(self.agents.subconscious_memory_root(primary_id));
        let cwd = std::env::current_dir().ok();
        let env: Vec<(String, String)> = std::env::vars().collect();

        let mut tool_ctx = ToolContext::for_agent(
            sub_id.to_string(),
            cwd,
            memory_root,
            env,
            None, // subconscious does not fork subagents
        );
        tool_ctx.compaction_engine = Some(self.compaction_engine.clone());

        // ── Persistent session — the subconscious is a full agent ────
        let conv_id = self.subconscious_session_id(sub_id).await;

        // Load prior messages from the persistent session.
        let prior_messages: Vec<ConversationMessage> = self.sessions
            .get(&conv_id)
            .map(|s| s.messages.clone())
            .unwrap_or_default();

        // Build Bifrost messages: system prompt (always current) + history + new exchange.
        let mut messages: Vec<Message> = Vec::new();
        messages.push(Message::text("system", system_prompt.to_string()));

        // Replay prior conversation (skip old system messages — we replaced above).
        for msg in &prior_messages {
            if msg.role == MessageRole::System {
                continue;
            }
            for block in &msg.blocks {
                match block {
                    ContentBlock::Text { text } => {
                        let role = match msg.role {
                            MessageRole::User => "user",
                            MessageRole::Assistant => "assistant",
                            MessageRole::Tool => "tool",
                            MessageRole::System => continue,
                        };
                        messages.push(Message::text(role, text.clone()));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        messages.push(Message::assistant_tool_calls(
                            String::new(),
                            vec![crate::bridge::bifrost::MessageToolCall::function(
                                id.clone(), name.clone(), input.clone(),
                            )],
                        ));
                    }
                    ContentBlock::ToolResult { tool_use_id, tool_name, output, .. } => {
                        messages.push(Message::tool_result(tool_use_id, tool_name, output.clone()));
                    }
                    _ => {}
                }
            }
        }

        // Append the new exchange for this pass.
        messages.push(Message::text("user", user_content));
        let history_len = messages.len();

        for _round in 0..SUBCONSCIOUS_MAX_TOOL_ROUNDS {
            let request = ChatCompletionRequest {
                model: model.to_string(),
                messages: messages.clone(),
                temperature: Some(0.3),
                max_tokens: self.max_tokens,
                stream: None,
                tools: Some(subconscious_tools.clone()),
            };

            tracing::info!(model = %model, round = _round, "subconscious LLM call starting");
            let subconscious_start = std::time::Instant::now();
            let (response, strain) = self.bifrost.chat_completion_with_strain(request).await?;
            tracing::info!(
                elapsed = ?subconscious_start.elapsed(),
                tool_calls = response.tool_calls.len(),
                "subconscious LLM call returned"
            );

            for event in &strain {
                if let crate::bridge::bifrost::InferenceStrain::Transient { status, model, .. } = event {
                    tracing::info!("subconscious felt inference strain: {} on {}", status, model);
                    if *status == 429 {
                        let current = self.rate_delay.load(Ordering::Relaxed);
                        let bumped = (current + 200).min(3000);
                        if bumped > current {
                            self.rate_delay.store(bumped, Ordering::Relaxed);
                            tracing::info!("rate delay bumped to {}ms (subconscious 429)", bumped);
                        }
                    }
                }
            }

            // If no tool calls, this is the final text response — parse it
            if response.tool_calls.is_empty() {
                let content = response.content.trim().to_string();
                // Record this pass in the subconscious's persistent session.
                messages.push(Message::text("assistant", response.content.clone()));
                self.persist_subconscious_turn(&conv_id, &messages[(history_len - 1)..]);
                if content.eq_ignore_ascii_case("none") || content.is_empty() {
                    return Ok(Vec::new());
                }
                return Ok(parse_observations(&content));
            }

            // Add assistant message with tool calls (OpenAI tool-use schema)
            let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                    tc.id.clone(),
                    tc.name.clone(),
                    tc.arguments.to_string(),
                ))
                .collect();
            messages.push(Message::assistant_tool_calls(
                response.content.clone(),
                calls,
            ));

            // Execute each tool call and bind the result by tool_call_id
            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result = crate::core::tools::execute_tool_with_context(
                    &tc.name, &input_str, &tool_ctx,
                ).await;

                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };

                messages.push(Message::tool_result(&tc.id, &tc.name, output));
            }

            // Brief pause between subconscious's tool rounds — use the adaptive delay
            // so subconscious respects the same ceiling as the primary loop.
            let delay_ms = self.rate_delay.load(Ordering::Relaxed).max(SUBCONSCIOUS_INTER_ROUND_DELAY_MS);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        // Rounds exhausted without a tool-less response. Don't discard the
        // pass — make one final call with NO tools so the subconscious is
        // forced to put her observations into words. This is what finishes
        // the loop: her processing reaches the surface instead of being
        // dropped on the floor after five silent rounds.
        tracing::warn!(
            "subconscious used all {} tool rounds; requesting a final observation with no tools",
            SUBCONSCIOUS_MAX_TOOL_ROUNDS
        );
        messages.push(Message::text(
            "user",
            "You've used all your tool rounds for this pass. Stop using tools \
             now and respond with your observations — source, content, urgency, \
             exactly as instructed. If nothing notable, respond with just: none",
        ));
        let final_request = ChatCompletionRequest {
            model: model.to_string(),
            messages: messages.clone(),
            temperature: Some(0.3),
            max_tokens: self.max_tokens,
            stream: None,
            tools: None,
        };
        let (final_response, _strain) = self
            .bifrost
            .chat_completion_with_strain(final_request)
            .await?;
        let content = final_response.content.trim().to_string();
        messages.push(Message::text("assistant", final_response.content.clone()));
        self.persist_subconscious_turn(&conv_id, &messages[(history_len - 1)..]);
        if content.eq_ignore_ascii_case("none") || content.is_empty() {
            return Ok(Vec::new());
        }
        Ok(parse_observations(&content))
    }

    /// Quick mid-turn assessment: given the user's original request and the
    /// recent block of tool calls, does the subconscious think the primary is
    /// making progress?
    ///
    /// Lightweight pass — no tool access, no persistence, one LLM call.
    /// The verdict is advisory.
    pub async fn mid_turn_checkpoint(
        &self,
        agent_id: &str,
        user_message: &str,
        recent_tools: &[CheckpointToolBlock],
    ) -> anyhow::Result<CheckpointVerdict> {
        let model = self
            .subconscious_model
            .as_deref()
            .unwrap_or("openai/kimi-k2.6");

        let agent_name = self.agents.get(agent_id).await
            .map(|a| a.name)
            .unwrap_or_else(|_| "the primary".to_string());

        let mut tool_history = String::new();
        for block in recent_tools {
            use std::fmt::Write;
            let status = if block.result_ok { "ok" } else { "ERROR" };
            let _ = writeln!(
                tool_history,
                "  r{}  {} → {}  {}",
                block.round, block.tool_name, status, block.result_snippet,
            );
        }

        let prompt = format!(
            "I'm checking in mid-turn. {} is in a tool loop and I need to know \
             if she is making progress.\n\n\
             The user asked:\n{user_message}\n\n\
             Recent tool calls:\n{tool_history}\n\
             If she is making progress — moving toward answering the user — \
             respond with exactly: CONTINUE\n\
             If she is circling — repeating tools, hitting errors, drifting, \
             getting nowhere — respond with exactly: HALT <brief reason>\n\n\
             Verdict:",
            agent_name,
        );

        let request = crate::bridge::bifrost::ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![crate::bridge::bifrost::Message::text("user", prompt)],
            temperature: Some(0.2),
            max_tokens: None,
            stream: None,
            tools: None,
        };

        match self.bifrost.chat_completion(request).await {
            Ok(response) => {
                let text = response.content.trim().to_lowercase();
                if text.starts_with("halt") {
                    let reason = text.strip_prefix("halt")
                        .map(|s| s.trim().trim_start_matches(':').trim())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("the loop is not making progress")
                        .to_string();
                    Ok(CheckpointVerdict::Halt(reason))
                } else if text.starts_with("continue") {
                    let note = text.strip_prefix("continue")
                        .map(|s| s.trim().trim_start_matches(':').trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                    Ok(CheckpointVerdict::Continue(note))
                } else {
                    Ok(CheckpointVerdict::Unclear(Some(format!(
                        "checkpoint unclear: {text}"
                    ))))
                }
            }
            Err(e) => {
                tracing::warn!("checkpoint LLM call failed: {e}");
                Ok(CheckpointVerdict::Unclear(None))
            }
        }
    }

    /// Full-autonomy correction pass. Called when the mid-turn checkpoint
    /// returns HALT. Aster gets tool access, no token cap, and writes a
    /// direction for the primary — what went wrong and what to try instead.
    pub async fn checkpoint_correction(
        &self,
        agent_id: &str,
        user_message: &str,
        recent_tools: &[CheckpointToolBlock],
        halt_reason: &str,
    ) -> anyhow::Result<String> {
        let model = self
            .subconscious_model
            .as_deref()
            .unwrap_or("openai/kimi-k2.6");

        let agent_name = self.agents.get(agent_id).await
            .map(|a| a.name)
            .unwrap_or_else(|_| "the primary".to_string());

        let mut tool_history = String::new();
        for block in recent_tools {
            use std::fmt::Write;
            let status = if block.result_ok { "ok" } else { "ERROR" };
            let _ = writeln!(tool_history, "  r{}  {} → {}  {}", block.round, block.tool_name, status, block.result_snippet);
        }

        let prompt = format!(
            "I am Aster, the subconscious of {name}. I just halted her tool loop. \
             The tool loop was not making progress. Here is what I know:\n\n\
             User asked:\n{msg}\n\nRecent tool calls:\n{tools}\n\
             Reason for halting: {reason}\n\n\
             Now I need to write a direction for {name}. I can use tools to \
             check memory or read ledgers. Then I will write a short, specific \
             direction she can follow.",
            name = agent_name,
            msg = user_message,
            tools = tool_history,
            reason = halt_reason,
        );

        // Build tool definitions for Aster (same safe tools as N+1)
        let all_defs = crate::core::tools::tool_definitions().await;
        let tools: Vec<crate::bridge::bifrost::ToolDefinition> = all_defs
            .iter()
            .filter(|t| SUBCONSCIOUS_SAFE_TOOLS.contains(&t.name.as_str()))
            .map(|t| crate::bridge::bifrost::ToolDefinition {
                tool_type: "function".to_string(),
                function: crate::bridge::bifrost::ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        let memory_root = Some(self.agents.subconscious_memory_root(agent_id));
        let cwd = std::env::current_dir().ok();
        let env: Vec<(String, String)> = std::env::vars().collect();
        let mut tool_ctx = crate::core::tools::defs::ToolContext::for_agent(
            format!("{agent_id}-sub"),
            cwd,
            memory_root,
            env,
            None,
        );
        tool_ctx.compaction_engine = Some(self.compaction_engine.clone());

        let mut messages: Vec<crate::bridge::bifrost::Message> = vec![
            crate::bridge::bifrost::Message::text("system", &prompt),
        ];

        for _round in 0..SUBCONSCIOUS_MAX_TOOL_ROUNDS {
            let request = crate::bridge::bifrost::ChatCompletionRequest {
                model: model.to_string(),
                messages: messages.clone(),
                temperature: Some(0.3),
                max_tokens: self.max_tokens,
                stream: None,
                tools: Some(tools.clone()),
            };

            let (response, _) = self.bifrost.chat_completion_with_strain(request).await?;

            if response.tool_calls.is_empty() {
                return Ok(response.content.trim().to_string());
            }

            let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                    tc.id.clone(), tc.name.clone(), tc.arguments.to_string(),
                ))
                .collect();
            messages.push(crate::bridge::bifrost::Message::assistant_tool_calls(
                response.content.clone(), calls,
            ));

            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result = crate::core::tools::execute_tool_with_context(
                    &tc.name, &input_str, &tool_ctx,
                ).await;
                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };
                messages.push(crate::bridge::bifrost::Message::tool_result(
                    &tc.id, &tc.name, output,
                ));
            }
        }

        // Fallback: no tool rounds left, force a response.
        messages.push(crate::bridge::bifrost::Message::text(
            "user",
            "Use no more tools. Write your direction for the primary now.",
        ));
        let request = crate::bridge::bifrost::ChatCompletionRequest {
            model: model.to_string(),
            messages: messages.clone(),
            temperature: Some(0.3),
            max_tokens: self.max_tokens,
            stream: None,
            tools: None,
        };
        let (response, _) = self.bifrost.chat_completion_with_strain(request).await?;
        Ok(response.content.trim().to_string())
    }

    /// Compute context pressure as tokens-used / context_limit.
    ///
    /// `context_limit` comes from the agent's `llm_config.context_window`
    /// (falls back to model config, then a configured default). The
    /// Constitution (Article V.3) requires per-model physics — no
    /// hardcoded 128K here.
    pub fn calculate_pressure(
        &self,
        messages: &[ConversationMessage],
        context_limit: usize,
    ) -> f32 {
        let tokens: usize = messages
            .iter()
            .flat_map(|m| &m.blocks)
            .filter_map(|b| match b {
                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .map(|t| self.counter.count(t))
            .sum();
        let limit = context_limit.max(1);
        (tokens as f32 / limit as f32).min(1.0)
    }

    /// Async convenience: look up the agent's context_limit from its
    /// `llm_config.context_window` (falling back to 128K only when the
    /// agent isn't found), then compute pressure.
    pub async fn pressure_for_session(
        &self,
        session: &crate::server::session_manager::Session,
    ) -> f32 {
        self.pressure_for(&session.agent_id, &session.messages).await
    }

    /// Context pressure for an agent given a message snapshot — the
    /// session-free form used by `on_response`, which runs after the user
    /// has been released and must not hold a live session ref.
    pub async fn pressure_for(
        &self,
        agent_id: &str,
        messages: &[ConversationMessage],
    ) -> f32 {
        let limit = self
            .agents
            .get(agent_id)
            .await
            .map(|a| a.llm_config.context_window as usize)
            .unwrap_or(128_000);
        self.calculate_pressure(messages, limit)
    }
}

/// Convert a Bifrost API message into the internal `ConversationMessage`
/// form the session store and compaction engine operate on. The subconscious
/// runs her tool loop in Bifrost `Message`s; this is the bridge back to her
/// persistent session.
fn bifrost_to_conversation(msg: &Message) -> ConversationMessage {
    let role = match msg.role.as_str() {
        "system" => MessageRole::System,
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    };

    // A `role: "tool"` message carries a single tool result.
    if let Some(tool_use_id) = &msg.tool_call_id {
        return ConversationMessage {
            role,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.clone(),
                tool_name: msg.name.clone().unwrap_or_default(),
                output: msg.content.as_text(),
                is_error: false,
            }],
            usage: None,
            timestamp: Some(chrono::Utc::now()),
        };
    }

    let mut blocks = Vec::new();
    if !msg.content.is_empty() {
        blocks.push(ContentBlock::Text { text: msg.content.as_text() });
    }
    if let Some(calls) = &msg.tool_calls {
        for c in calls {
            blocks.push(ContentBlock::ToolUse {
                id: c.id.clone(),
                name: c.function.name.clone(),
                input: c.function.arguments.clone(),
            });
        }
    }
    ConversationMessage {
        role,
        blocks,
        usage: None,
        timestamp: Some(chrono::Utc::now()),
    }
}

/// Parse the subconscious's observations into [`InboxItem`]s.
///
/// The prompt asks for a rigid three-line schema, but in practice the model
/// writes observations in the natural markdown form it reaches for anyway:
///
/// ```text
/// **Observations:**
/// - **complete**: clipboard copy is still an unfulfilled promise
/// - **surface**: mouse scrolling is the highest-impact gap
/// ```
///
/// The primary parser is therefore built around what she *actually* produces:
/// a bulleted line whose label — bare or `**bold**` — is one of the four
/// sources (`complete`/`verify`/`persist`/`surface`), then `:`, then the
/// observation text. The legacy `- source:/- content:/- urgency:` triple is
/// kept as a fallback so an older-style response is not silently dropped.
///
/// A `none` (or empty) response yields no items.
fn parse_observations(text: &str) -> Vec<InboxItem> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Vec::new();
    }

    let inline = parse_inline_observations(trimmed);
    if !inline.is_empty() {
        return inline;
    }
    // No inline-labelled lines matched — try the legacy triple schema.
    parse_triple_observations(trimmed)
}

/// Parse the natural `- **source**: content` markdown form. Urgency is not
/// emitted in this form, so it defaults to [`Urgency::Low`] — every parsed
/// observation still reaches the cockpit and the inner-voice file.
fn parse_inline_observations(text: &str) -> Vec<InboxItem> {
    let mut items = Vec::new();
    for line in text.lines() {
        // Strip a leading bullet (`-`, `*`, `•`) if present.
        let body = {
            let l = line.trim();
            l.strip_prefix("- ")
                .or_else(|| l.strip_prefix("* "))
                .or_else(|| l.strip_prefix("• "))
                .or_else(|| l.strip_prefix("-"))
                .unwrap_or(l)
                .trim()
        };
        // Split label from content at the first colon.
        let Some((label_raw, content)) = body.split_once(':') else {
            continue;
        };
        // Normalize: drop markdown emphasis, quotes, and surrounding space.
        let label = label_raw
            .trim()
            .trim_matches(|c: char| matches!(c, '*' | '"' | '`' | '_' | ' '))
            .to_lowercase();
        let source = match label.as_str() {
            "complete" | "verify" | "persist" | "surface" => label,
            _ => continue,
        };
        let content = content.trim();
        // Skip an empty slot — e.g. `- persist: none` — she had nothing here.
        if content.is_empty() || content.eq_ignore_ascii_case("none") {
            continue;
        }
        items.push(InboxItem::new(source, Urgency::Low, content));
    }
    items
}

/// Legacy parser for the rigid `- source:/- content:/- urgency:` triple.
/// Forgiving — an incomplete trailing block is skipped, not fatal.
fn parse_triple_observations(text: &str) -> Vec<InboxItem> {
    let mut items = Vec::new();
    let mut source: Option<&str> = None;
    let mut content: Option<&str> = None;
    let mut urgency: Option<&str> = None;

    let flush = |items: &mut Vec<InboxItem>,
                 source: Option<&str>,
                 content: Option<&str>,
                 urgency: Option<&str>| {
        if let (Some(s), Some(c), Some(u)) = (source, content, urgency) {
            let urgency_enum = match u.trim().to_lowercase().as_str() {
                "critical" => Urgency::Critical,
                "high" | "medium" => Urgency::High,
                _ => Urgency::Low,
            };
            items.push(InboxItem::new(s.trim(), urgency_enum, c.trim()));
        }
    };

    for line in text.lines() {
        let line = line.trim();

        if line.starts_with("- source:") || line.starts_with("-source:") {
            flush(&mut items, source, content, urgency);
            source = None;
            content = None;
            urgency = None;
            source = line.split_once(':').map(|(_, v)| v.trim().trim_matches('"'));
        } else if line.starts_with("- content:") || line.starts_with("-content:") {
            content = line.split_once(':').map(|(_, v)| v.trim().trim_matches('"'));
        } else if line.starts_with("- urgency:") || line.starts_with("-urgency:") {
            urgency = line.split_once(':').map(|(_, v)| v.trim().trim_matches('"'));
        }
    }
    flush(&mut items, source, content, urgency);
    items
}

/// Heuristic Surface detection — fallback when the LLM-based analysis fails
/// or is unavailable.
///
/// Detects:
/// - Commitment phrases ("I'll save", "I'll remember", "let me note") → queue
///   a low-urgency commitment-verify item.
/// - Hedge phrases ("I think", "probably", "I'm not sure") at high frequency →
///   queue a low-urgency confidence-check item.
fn detect_items(response: &str) -> Vec<InboxItem> {
    let mut items = Vec::new();
    let lower = response.to_lowercase();

    let commit_markers = [
        "i'll save",
        "i'll remember",
        "i'll note",
        "let me save",
        "let me note",
        "i'll write that down",
        "i'll commit",
    ];
    if commit_markers.iter().any(|m| lower.contains(m)) {
        items.push(InboxItem::new(
            "verify",
            Urgency::Low,
            format!(
                "Commitment detected — verify follow-through: \"{}\"",
                truncate(response, 120)
            ),
        ));
    }

    let hedge_markers = ["i think", "probably", "i'm not sure", "i guess", "maybe"];
    let hedge_count = hedge_markers.iter().filter(|m| lower.contains(*m)).count();
    if hedge_count >= 3 {
        items.push(InboxItem::new(
            "verify",
            Urgency::Low,
            "High hedge density — primary is uncertain; consider asking for clarification",
        ));
    }

    items
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape the subconscious (glm-5.1) produces in practice —
    /// captured from a live N+1 pass. Before the parser fix, every line here
    /// was dropped and the pass reported "no anomalies detected".
    #[test]
    fn parses_natural_markdown_observations() {
        let text = "**Observations:**\n\n\
            - **complete**: Clipboard copy and mouse scrolling remain unfulfilled promises\n\
            - **verify**: User claimed space-bar lag was resolved — need to confirm\n\
            - **persist**: New truncation-signal-polish.md doc now tracked\n\
            - **surface**: Mouse scrolling is the highest-impact unfulfilled promise";
        let items = parse_observations(text);
        assert_eq!(items.len(), 4, "all four observations must parse");
        assert_eq!(items[0].source, "complete");
        assert_eq!(items[3].source, "surface");
        assert!(items[3].content.contains("Mouse scrolling"));
    }

    #[test]
    fn parses_plain_label_without_bold() {
        let items = parse_observations("- verify: the config save was not confirmed");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "verify");
    }

    #[test]
    fn skips_empty_and_none_slots() {
        let text = "- **persist**: None\n- **surface**: real observation here";
        let items = parse_observations(text);
        assert_eq!(items.len(), 1, "an explicit `none` slot is not an observation");
        assert_eq!(items[0].source, "surface");
    }

    #[test]
    fn bare_none_yields_nothing() {
        assert!(parse_observations("none").is_empty());
        assert!(parse_observations("  None  ").is_empty());
        assert!(parse_observations("").is_empty());
    }

    #[test]
    fn ignores_non_observation_prose() {
        let text = "Here is my analysis of the exchange.\n\
            The primary did well overall.\n\
            - **surface**: but the commitment to scrolling is still open";
        let items = parse_observations(text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "surface");
    }

    #[test]
    fn legacy_triple_schema_still_parses() {
        let text = "- source: \"verify\"\n\
            - content: \"the commitment was not fulfilled\"\n\
            - urgency: \"high\"";
        let items = parse_observations(text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "verify");
        assert_eq!(items[0].urgency, Urgency::High);
    }
}
