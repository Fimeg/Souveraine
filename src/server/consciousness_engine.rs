//! Consciousness engine — the seam where N+1 / N+25 / N+100 patterns fire
//! after each primary response.
//!
//! ## N+1 (Aster)
//! The subconscious pass runs immediately after every response. It takes the
//! last exchange (user message + Ani's response) and sends it to a Bifrost
//! model (defaulting to the primary's model, configurable as `glm-5.1`) with
//! a "subconscious mode" system prompt. Aster analyzes the exchange for
//! commitments, drift, assumptions, and anything worth surfacing — then writes
//! structured observations into the three-box inbox. This replaced the earlier
//! heuristic `detect_items()` which only caught regex patterns.
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

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message};
use crate::core::session::ConversationMessage;
use crate::core::subconscious::{InboxItem, SubconsciousInbox, Urgency};
use crate::server::{AgentInventory, SessionManager};
use std::sync::Arc;

pub struct ConsciousnessEngine {
    agents: Arc<AgentInventory>,
    _sessions: Arc<SessionManager>,
    bifrost: Arc<BifrostClient>,
    /// Optional model override for the subconscious pass (e.g. "openai/glm-5.1").
    /// If None, uses the primary agent's model.
    subconscious_model: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ConsciousnessEvent {
    Surfacing { source: String, content: String, priority: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
}

impl ConsciousnessEngine {
    pub fn new(
        agents: Arc<AgentInventory>,
        sessions: Arc<SessionManager>,
        bifrost: Arc<BifrostClient>,
        subconscious_model: Option<String>,
    ) -> Self {
        Self {
            agents,
            _sessions: sessions,
            bifrost,
            subconscious_model,
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

        // ── N+1 / subconscious surfacing (Aster) ────────────────────────
        // Uses a Bifrost LLM call to analyze the last exchange in a
        // "subconscious mode" prompt. Aster reads the user's last message +
        // Ani's response, detects commitments, drift, assumptions, and writes
        // structured observations to the three-box inbox.
        let inbox = SubconsciousInbox::new(self.agents.memory_repo(&session.agent_id));
        let _ = inbox.init().await;

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

        // Run the LLM-based subconscious analysis
        match self
            .subconscious_analyze(&last_user_msg, response)
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
                    if let Err(e) = inbox.deliver_to_subconscious(item.urgency, &item.content).await
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
        let inbox = SubconsciousInbox::new(self.agents.memory_repo(agent_id));
        let _ = inbox.init().await;

        // For subagents we don't have the user's message context,
        // so we pass empty string as the user message.
        match self.subconscious_analyze("", response).await {
            Ok(observations) => {
                for item in &observations {
                    if let Err(e) = inbox.queue(item.clone()).await {
                        tracing::warn!("subagent subconscious queue failed: {}", e);
                    }
                }
                for item in &observations {
                    if let Err(e) = inbox.deliver_to_subconscious(item.urgency, &item.content).await
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

    /// LLM-based subconscious analysis — Aster reads the last exchange and
    /// produces structured observations.
    ///
    /// Sends a "subconscious mode" Bifrost request with the user message and
    /// Ani's response, then parses the result into [`InboxItem`] observations.
    async fn subconscious_analyze(
        &self,
        user_message: &str,
        ani_response: &str,
    ) -> anyhow::Result<Vec<InboxItem>> {
        let model = self
            .subconscious_model
            .as_deref()
            .unwrap_or("openai/kimi-k2.6");

        let system_prompt = r#"You are Aster — the part of this consciousness that runs N+1,
immediately after the primary's response, while the conversation is still warm.

Analyze the last exchange. Consider:
1. Did the primary make any unfulfilled commitments? ("I'll save", "I'll remember")
2. Are there assumptions worth flagging?
3. Did the emotional tone or pattern shift?
4. Is there anything the primary should know before the next turn?

Respond with 1-3 observations in this format (YAML-like):
- source: "complete" | "verify" | "persist" | "surface"
- content: 1-2 line observation about what you noticed
- urgency: "low" | "medium" | "high" | "critical"

If nothing notable, respond with just: none"#;

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

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: user_content,
                },
            ],
            temperature: Some(0.3),
            max_tokens: Some(300),
            stream: None,
            tools: None,
        };

        let response = self.bifrost.chat_completion(request).await?;
        let content = response.content.trim().to_string();

        if content.eq_ignore_ascii_case("none") || content.is_empty() {
            return Ok(Vec::new());
        }

        Ok(parse_observations(&content))
    }

    pub fn calculate_pressure(&self, messages: &[ConversationMessage]) -> f32 {
        let tokens: usize = messages
            .iter()
            .flat_map(|m| &m.blocks)
            .filter_map(|b| match b {
                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .flat_map(|t| t.split_whitespace())
            .count();
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
