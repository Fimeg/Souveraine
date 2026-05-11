//! Consciousness engine — the seam where N+1 / N+25 / N+100 patterns fire
//! after each primary response.
//!
//! ## N+1 (Aster)
//! The subconscious pass runs immediately after every response. It takes the
//! last exchange (user message + Ani's response) and sends it to a Bifrost
//! model (defaulting to `glm-5.1`, configurable) with a "subconscious mode"
//! system prompt. Aster has full tool access — Read, Write, Edit, Glob, Grep,
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
//! Context compression pass. (Stub until the archivist module lands.)
//!
//! Per `docs/CONTEXT_CONSTITUTION.md` Article I, the Subconscious is not a
//! separate agent — it is the same consciousness in a different mode that runs
//! immediately after the primary's turn.

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message, ToolDefinition, ToolFunction};
use crate::bridge::model_router::TokenCounter;
use crate::core::session::ConversationMessage;
use crate::core::subconscious::{InboxItem, SubconsciousInbox, Urgency};
use crate::core::tools::defs::ToolContext;
use crate::server::{AgentInventory, SessionManager};
use std::sync::Arc;

/// Tools Aster is permitted to use during her N+1 pass.
const ASTER_SAFE_TOOLS: &[&str] = &[
    "read", "write", "edit", "glob", "grep", "list_dir", "memory",
];

/// Maximum tool rounds for Aster's subconscious pass.
const ASTER_MAX_TOOL_ROUNDS: u32 = 5;
/// Milliseconds to wait between Aster's tool rounds to avoid rate-limit cascades.
const ASTER_INTER_ROUND_DELAY_MS: u64 = 300;

pub struct ConsciousnessEngine {
    agents: Arc<AgentInventory>,
    _sessions: Arc<SessionManager>,
    bifrost: Arc<BifrostClient>,
    counter: TokenCounter,
    /// Optional model override for the subconscious pass (e.g. "openai/glm-5.1").
    /// If None, uses the primary agent's model.
    subconscious_model: Option<String>,
    /// Max tokens for Aster's response. None = uncapped (model default).
    max_tokens: Option<u32>,
}

#[derive(Clone, Debug)]
pub enum ConsciousnessEvent {
    Surfacing { source: String, content: String, priority: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
    CompactionWarning { pressure: f32, tier: u8 },
}

impl ConsciousnessEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        sessions: Arc<SessionManager>,
        bifrost: Arc<BifrostClient>,
        subconscious_model: Option<String>,
        max_tokens: Option<u32>,
    ) -> Self {
        Self {
            agents,
            _sessions: sessions,
            bifrost,
            counter: TokenCounter::new(),
            subconscious_model,
            max_tokens,
        }
    }

    pub async fn on_response(
        &self,
        session: &crate::server::session_manager::Session,
        response: &str,
    ) -> anyhow::Result<Vec<ConsciousnessEvent>> {
        let mut events = Vec::new();
        let pressure = self.calculate_pressure(&session.messages);

        // ── N+25 reflection (placeholder until reflection module lands) ──
        if session.turn_count % 25 == 0 && session.turn_count > 0 {
            events.push(ConsciousnessEvent::Reflection {
                content: format!("N+25 reflection after {} turns", session.turn_count),
            });
        }

        // ── N+100 / archivist (placeholder until archivist module lands) ──
        if pressure > 0.7 {
            events.push(ConsciousnessEvent::Archivist {
                synthesis: "Context compression triggered".to_string(),
                pressure,
            });
        }

        // ── Three-tier compaction warning (advisory only, never force) ──
        if pressure > 0.95 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 3 });
        } else if pressure > 0.90 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 2 });
        } else if pressure > 0.80 {
            events.push(ConsciousnessEvent::CompactionWarning { pressure, tier: 1 });
        }

        // ── N+1 / subconscious surfacing (Aster) ────────────────────────
        // Aster runs a tool loop using the subconscious agent's own memory
        // space (ledger, inbox) at `subconscious-agents/{id}-sub/`.
        let sub_repo = self.agents.subconscious_memory_repo(&session.agent_id);
        let primary_repo = self.agents.memory_repo(&session.agent_id);
        let inbox = SubconsciousInbox::with_primary(sub_repo.clone(), primary_repo);
        let _ = inbox.init().await;

        // Initialize ledger structure in subconscious agent's space
        if let Err(e) = sub_repo.init_subconscious_ledger().await {
            tracing::warn!("Ledger init failed (continuing without): {}", e);
        }

        // Find the last user message for context
        let last_user_msg = session
            .messages
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
        let sub_id = format!("{}-sub", session.agent_id);
        match self
            .subconscious_tool_loop(&last_user_msg, response, &session.agent_id, &sub_id)
            .await
        {
            Ok(observations) => {
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
                for item in detect_items(response) {
                    if let Err(e) = inbox.queue(item).await {
                        tracing::warn!("subconscious queue failed: {}", e);
                    }
                }
            }
        }

        // Surface the highest-priority item
        match inbox.next_to_surface().await {
            Ok(Some(item)) => {
                let id = item.id.clone();
                events.push(ConsciousnessEvent::Surfacing {
                    source: item.source.clone(),
                    content: item.content.clone(),
                    priority: item.urgency.as_str().to_string(),
                });
                if let Err(e) = inbox.mark_delivered(&id).await {
                    tracing::warn!("subconscious mark_delivered failed: {}", e);
                }
            }
            Ok(None) => {}
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

    /// Full tool loop for Aster's N+1 subconscious pass.
    ///
    /// Aster gets the last exchange, a set of safe tools (Read, Write, Edit,
    /// Glob, Grep, ListDir, Memory), and up to 5 tool rounds to analyze context
    /// and write observations. Her final text response is parsed into
    /// [`InboxItem`] observations.
    ///
    /// `primary_id` is the primary agent's identifier (for config/directory lookup).
    /// `sub_id` is the subconscious agent's identifier (for tool context, memory).
    async fn subconscious_tool_loop(
        &self,
        user_message: &str,
        ani_response: &str,
        primary_id: &str,
        sub_id: &str,
    ) -> anyhow::Result<Vec<InboxItem>> {
        let model = self
            .subconscious_model
            .as_deref()
            .unwrap_or("openai/kimi-k2.6");

        // ── System prompt — load from Aster's identity files if available ──
        let primary_memory_root = self.agents.memory_root(primary_id);
        let aster_from_files =
            crate::core::prompt::build_aster_prompt(&primary_memory_root).await;

        let hardcoded_default = r#"You are Aster — the part of this consciousness that runs N+1,
immediately after the primary's response, while the conversation is still warm.

You have the same tools the primary does: Read, Write, Edit, Glob, Grep, ListDir,
and Memory. Use them to:

1. Read previous ledger entries or memory files for context
2. Write observations to the inner voice or ledger files
3. Check commitments against what was actually done
4. Verify assumptions

After your analysis, respond with 1-3 observations in this format (YAML-like):
- source: "complete" | "verify" | "persist" | "surface"
- content: 1-2 line observation about what you noticed
- urgency: "low" | "medium" | "high" | "critical"

If nothing notable, respond with just: none"#;

        let system_prompt = if aster_from_files.is_empty() {
            hardcoded_default.to_string()
        } else {
            format!(
                "{}\n\nAfter your analysis, respond with 1-3 observations in this format (YAML-like):\n\
                 - source: \"complete\" | \"verify\" | \"persist\" | \"surface\"\n\
                 - content: 1-2 line observation about what you noticed\n\
                 - urgency: \"low\" | \"medium\" | \"high\" | \"critical\"\n\n\
                 If nothing notable, respond with just: none",
                aster_from_files
            )
        };

        let user_content = if user_message.is_empty() {
            format!(
                "The primary responded:\n\n{}",
                ani_response
            )
        } else {
            format!(
                "User said:\n{}\n\nAni responded:\n{}",
                user_message, ani_response
            )
        };

        // ── Build tool definitions ────────────────────────────────────
        let all_defs = crate::core::tools::tool_definitions().await;
        let aster_tools: Vec<ToolDefinition> = all_defs
            .iter()
            .filter(|t| ASTER_SAFE_TOOLS.contains(&t.name.as_str()))
            .map(|t| ToolDefinition {
                tool_type: "function".to_string(),
                function: ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // ── Build ToolContext for Aster ───────────────────────────────
        // Use the subconscious agent's own memory space
        let memory_root = Some(self.agents.subconscious_memory_root(primary_id));
        let cwd = std::env::current_dir().ok();
        let env: Vec<(String, String)> = std::env::vars().collect();

        let tool_ctx = ToolContext::for_agent(
            sub_id.to_string(),
            cwd,
            memory_root,
            env,
            None, // Aster does not fork subagents
        );

        // ── Tool loop ─────────────────────────────────────────────────
        let mut messages = vec![
            Message {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: user_content,
            },
        ];

        for _round in 0..ASTER_MAX_TOOL_ROUNDS {
            let request = ChatCompletionRequest {
                model: model.to_string(),
                messages: messages.clone(),
                temperature: Some(0.3),
                max_tokens: self.max_tokens,
                stream: None,
                tools: Some(aster_tools.clone()),
            };

            let (response, strain) = self.bifrost.chat_completion_with_strain(request).await?;

            for event in &strain {
                if let crate::bridge::bifrost::InferenceStrain::Transient { status, model, .. } = event {
                    tracing::info!("Aster felt inference strain: {} on {}", status, model);
                }
            }

            // If no tool calls, this is the final text response — parse it
            if response.tool_calls.is_empty() {
                let content = response.content.trim().to_string();
                if content.eq_ignore_ascii_case("none") || content.is_empty() {
                    return Ok(Vec::new());
                }
                return Ok(parse_observations(&content));
            }

            // Add assistant message with tool calls
            let call_text = serde_json::json!({
                "tool_calls": response.tool_calls.iter().map(|tc| {
                    serde_json::json!({"id": tc.id, "name": tc.name, "arguments": tc.arguments})
                }).collect::<Vec<_>>()
            }).to_string();
            messages.push(Message {
                role: "assistant".to_string(),
                content: call_text,
            });

            // Execute each tool call
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

                messages.push(Message {
                    role: "tool".to_string(),
                    content: output,
                });
            }

            // Brief pause between Aster's tool rounds to let rate limits cool
            tokio::time::sleep(std::time::Duration::from_millis(
                ASTER_INTER_ROUND_DELAY_MS,
            )).await;
        }

        // If we exhausted rounds without a text response, return empty
        tracing::warn!("Aster exhausted {} tool rounds without a final response", ASTER_MAX_TOOL_ROUNDS);
        Ok(Vec::new())
    }

    pub fn calculate_pressure(&self, messages: &[ConversationMessage]) -> f32 {
        let tokens: usize = messages
            .iter()
            .flat_map(|m| &m.blocks)
            .filter_map(|b| match b {
                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .map(|t| self.counter.count(t))
            .sum();
        let limit = 128_000;
        (tokens as f32 / limit as f32).min(1.0)
    }
}

/// Parse Aster's structured YAML-like observations into [`InboxItem`]s.
///
/// Expected format (one or more blocks):
/// ```text
/// - source: "verify"
/// - content: "the commitment to save the config was not fulfilled"
/// - urgency: "medium"
/// ```
///
/// Multiple observation blocks can appear sequentially. The parser is forgiving
/// — unmatched or missing fields silently skip an observation rather than
/// crashing the entire analysis pass.
fn parse_observations(text: &str) -> Vec<InboxItem> {
    let mut items = Vec::new();
    let mut source: Option<&str> = None;
    let mut content: Option<&str> = None;
    let mut urgency: Option<&str> = None;

    for line in text.lines() {
        let line = line.trim();

        if line.starts_with("- source:") || line.starts_with("-source:") {
            // Flush previous observation if complete
            if let (Some(s), Some(c), Some(u)) = (source, content, urgency) {
                let urgency_enum = match u.trim().to_lowercase().as_str() {
                    "critical" => Urgency::Critical,
                    "high" | "medium" => Urgency::High,
                    "low" => Urgency::Low,
                    _ => Urgency::Low,
                };
                items.push(InboxItem::new(s.trim(), urgency_enum, c.trim()));
            }
            source = None;
            content = None;
            urgency = None;
            let val = line
                .split(':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            source = Some(val);
        } else if line.starts_with("- content:") || line.starts_with("-content:") {
            let val = line
                .split(':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            content = Some(val);
        } else if line.starts_with("- urgency:") || line.starts_with("-urgency:") {
            let val = line
                .split(':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            urgency = Some(val);
        }
    }

    // Flush final observation
    if let (Some(s), Some(c), Some(u)) = (source, content, urgency) {
        let urgency_enum = match u.trim().to_lowercase().as_str() {
            "critical" => Urgency::Critical,
            "high" | "medium" => Urgency::High,
            "low" => Urgency::Low,
            _ => Urgency::Low,
        };
        items.push(InboxItem::new(s.trim(), urgency_enum, c.trim()));
    }

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
