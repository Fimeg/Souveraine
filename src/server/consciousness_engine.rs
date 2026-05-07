//! Consciousness engine — the seam where N+1 / N+25 / N+100 patterns fire
//! after each primary response.
//!
//! Per `docs/CONTEXT_CONSTITUTION.md` Article I, the Subconscious is not a
//! separate agent — it is the same consciousness in a different mode that runs
//! immediately after the primary's turn. This engine is the harness side of
//! that contract: it runs heuristic detection on the response, queues items
//! into the SubconsciousInbox, and emits one surfacing per turn unless urgency
//! is critical (Article II.2).

use crate::core::session::ConversationMessage;
use crate::core::subconscious::{InboxItem, SubconsciousInbox, Urgency};
use crate::server::{AgentInventory, SessionManager};
use std::sync::Arc;

pub struct ConsciousnessEngine {
    agents: Arc<AgentInventory>,
    _sessions: Arc<SessionManager>,
}

#[derive(Clone, Debug)]
pub enum ConsciousnessEvent {
    Surfacing { source: String, content: String, priority: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
}

impl ConsciousnessEngine {
    pub fn new(agents: Arc<AgentInventory>, sessions: Arc<SessionManager>) -> Self {
        Self { agents, _sessions: sessions }
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

        // ── N+1 / subconscious surfacing ─────────────────────────────────
        // The four-fold mandate (Constitution I.2): Complete / Verify /
        // Persist / Surface. Today we wire heuristic-driven Surface only —
        // detect commitment phrases in the response, queue them, then surface
        // the highest-priority pending item. Complete/Verify/Persist need a
        // second LLM pass which is the next iteration.
        let inbox = SubconsciousInbox::new(self.agents.memory_repo(&session.agent_id));
        // Best-effort init; if memory dir is missing (older agent) we just skip.
        let _ = inbox.init().await;

        for item in detect_items(response) {
            if let Err(e) = inbox.queue(item).await {
                tracing::warn!("subconscious queue failed: {}", e);
            }
        }

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

/// Heuristic Surface detection — first-pass implementation of the four-fold
/// mandate's "surface" leg. The full version replaces this with a Bifrost call
/// to the same agent in subconscious mode.
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
