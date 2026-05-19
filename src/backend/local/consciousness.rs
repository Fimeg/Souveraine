use anyhow::Result;
use futures::stream::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::bridge::bifrost::Message as BifrostMessage;
use crate::core::nervous::EventBus;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::server::{ConsciousnessEvent, SouveraineServer};

use crate::backend::{Backend, BackendEvent};

use super::LocalBackend;

#[async_trait::async_trait]
impl crate::core::nervous::handler::TurnInjector for LocalBackend {
    /// Heartbeat-driven turn injection. The cron loop pauses while
    /// `active_sessions > 0`, so by the time we get here the agent is
    /// idle. We grab the most recent conversation (or create a fresh one
    /// if the agent has none), append the scheduled prompt as a user
    /// message, and drain the resulting stream — the turn runs silently
    /// in the background. Anything subconscious surfaces lands in the inbox.
    async fn inject_background_turn(
        &self,
        agent_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let conv_id = match self.server.sessions.list_for_agent(agent_id).last().cloned() {
            Some(id) => id,
            None => self.ensure_conversation(agent_id).await?,
        };
        let stream = self.send(&conv_id, text).await?;
        // Drain the stream in the background — no UI is listening. But the
        // subconscious's N+1 pass runs inside this turn, and what she
        // surfaces (a commitment, a reflection, an archivist synthesis)
        // would otherwise vanish with the drained events. Collect those and
        // stash them so the next TUI/CLI session shows the human what
        // happened during the autonomous cycle.
        let server = self.server.clone();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            use crate::core::nervous::pending::PendingSurfacing;
            let mut s = stream;
            let mut stashed: Vec<PendingSurfacing> = Vec::new();
            while let Some(ev) = s.next().await {
                match ev {
                    Ok(BackendEvent::Surfacing { source, content, priority }) => {
                        // Skip the no-op heartbeat sentinel — the subconscious
                        // always queues a low "pass complete, no anomalies"
                        // item so the UI shows the pass ran. That is noise to
                        // resurface on connect; only stash real observations.
                        if priority.eq_ignore_ascii_case("low")
                            && content.contains("no anomalies detected")
                        {
                            continue;
                        }
                        stashed.push(PendingSurfacing {
                            kind: "surfacing".to_string(),
                            source,
                            content,
                            priority,
                            at: chrono::Utc::now(),
                        });
                    }
                    Ok(BackendEvent::Reflection(content)) => {
                        stashed.push(PendingSurfacing {
                            kind: "reflection".to_string(),
                            source: String::new(),
                            content,
                            priority: String::new(),
                            at: chrono::Utc::now(),
                        });
                    }
                    Ok(BackendEvent::Archivist { synthesis, .. }) => {
                        stashed.push(PendingSurfacing {
                            kind: "archivist".to_string(),
                            source: String::new(),
                            content: synthesis,
                            priority: String::new(),
                            at: chrono::Utc::now(),
                        });
                    }
                    _ => {}
                }
            }
            if !stashed.is_empty() {
                let dir = server.agents.agent_data_dir(&agent_id);
                match crate::core::nervous::pending::append(&dir, &stashed).await {
                    Ok(()) => tracing::info!(
                        agent = %agent_id,
                        count = stashed.len(),
                        "stashed heartbeat surfacings for next session"
                    ),
                    Err(e) => tracing::warn!(
                        "pending heartbeat surfacings stash failed: {}", e
                    ),
                }
            }
        });
        Ok(())
    }

    /// Surface-initiated turn injection. Called by the
    /// [`SensoriumInputHandler`] when a `sensorium:input` event arrives.
    ///
    /// Unlike background turns, the turn's output events are NOT drained
    /// here — `run_turn` already fires them onto the EventBus as `turn:*`
    /// events (via `TurnEventDispatcher`). The originating sensorium's
    /// `run` loop consumes those events for incremental rendering.
    ///
    /// We drain the stream only to prevent backpressure on the mpsc
    /// channel. The EventBus is the public event system; the stream is
    /// a TUI-internal detail.
    async fn inject_surface_turn(
        &self,
        agent_id: &str,
        conversation_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        // Resolve the surface chat ID to a Souveraine conversation ID.
        // Matrix room IDs are not Souveraine conversation IDs — the
        // mapping survives for the lifetime of the surface session so
        // subsequent messages in the same room route to the same
        // conversation. The lock scope is carefully bounded to avoid
        // holding a !Send MutexGuard across the .await below.
        let conv_id = {
            let map = self.surface_conversations.lock().unwrap();
            if let Some(id) = map.get(conversation_id) {
                Some(id.clone())
            } else {
                None
            }
        };
        let conv_id = match conv_id {
            Some(id) => id,
            None => {
                let id = self.ensure_conversation(agent_id).await?;
                self.surface_conversations
                    .lock()
                    .unwrap()
                    .insert(conversation_id.to_string(), id.clone());
                id
            }
        };

        let stream = self.send(&conv_id, text).await?;
        // Drain the stream in the background — the EventBus already carries
        // every `turn:*` event via TurnEventDispatcher. The sensorium
        // renders from the bus. We drain here so the mpsc channel doesn't
        // back up.
        tokio::spawn(async move {
            let mut s = stream;
            while let Some(ev) = s.next().await {
                if ev.is_err() {
                    break;
                }
            }
        });
        Ok(())
    }
}

/// Drain subconscious's intrusive box for the given agent and return formatted
/// `[ surfacing: ... ]` lines ready to prepend to the user's next message.
/// Marks each drained item as delivered (moved to `sent.md`). Mirrors
/// lettabot-v017's `readSurfacingThoughts` + `clearSurfacingThoughts` pair
/// (`~/Projects/lettabot-v017/src/core/prompts.ts:64-91`) — the substrate
/// reads the channel subconscious wrote to and lets the conscious mind see it
/// before she reads Casey.
///
/// Critical urgency gets `[ surfacing — CRITICAL: ... ]`. High becomes
/// `[ surfacing — !: ... ]`. Low/none keep the bare form. The shape is a
/// gradient the agent feels, not a number she has to read.
pub(super) async fn drain_intrusive_surfacings(
    server: &Arc<SouveraineServer>,
    agent_id: &str,
) -> Vec<String> {
    use crate::core::subconscious::{SubconsciousInbox, Urgency};

    let sub_repo = server.agents.subconscious_memory_repo(agent_id);
    let primary_repo = server.agents.memory_repo(agent_id);
    let inbox = SubconsciousInbox::with_primary(sub_repo, primary_repo);

    let items = match inbox.get_intrusive().await {
        Ok(items) => items,
        Err(e) => {
            tracing::debug!("intrusive surfacing read failed (continuing without): {}", e);
            return Vec::new();
        }
    };

    if items.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(items.len());
    for item in &items {
        let prefix = match item.urgency {
            Urgency::Critical => "[ surfacing — CRITICAL:",
            Urgency::High => "[ surfacing — !:",
            Urgency::Low => "[ surfacing:",
        };
        let content = item.content.trim();
        lines.push(format!("{} {} ]", prefix, content));

        if let Err(e) = inbox.mark_delivered(&item.id).await {
            tracing::debug!("mark_delivered failed for {}: {}", item.id, e);
        }
    }
    lines
}
