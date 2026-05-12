use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::SensorEvent;

/// The seam between the nervous system (which knows *when* to fire) and
/// the backend (which knows *how* to start a turn). The handler doesn't
/// know about LocalBackend; LocalBackend implements this trait. Keeps the
/// dependency direction sane and lets remote/federated agents inject too.
#[async_trait]
pub trait TurnInjector: Send + Sync {
    /// Start a background turn for `agent_id` with `text` as the user
    /// message. The stream of events is drained inside; callers don't
    /// see them — heartbeats are silent unless the agent surfaces.
    async fn inject_background_turn(
        &self,
        agent_id: &str,
        text: &str,
    ) -> anyhow::Result<()>;
}

pub struct HeartbeatHandler {
    rx: broadcast::Receiver<SensorEvent>,
    injector: Arc<dyn TurnInjector>,
}

impl HeartbeatHandler {
    pub fn new(rx: broadcast::Receiver<SensorEvent>, injector: Arc<dyn TurnInjector>) -> Self {
        Self { rx, injector }
    }

    pub async fn run(&mut self) {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if event.event_type == "schedule_due" {
                        self.handle_schedule_event(&event).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "heartbeat handler lagged, skipped events");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("event bus closed, heartbeat handler exiting");
                    break;
                }
            }
        }
    }

    async fn handle_schedule_event(&self, event: &SensorEvent) {
        let name = event.target.as_deref().unwrap_or("unknown");
        let agent_id = event
            .payload
            .as_ref()
            .and_then(|p| p.get("agent_id"))
            .and_then(|v| v.as_str());
        let prompt = event
            .payload
            .as_ref()
            .and_then(|p| p.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let Some(agent_id) = agent_id else {
            warn!(schedule = name, "schedule_due missing agent_id in payload");
            return;
        };

        if prompt.is_empty() {
            warn!(schedule = name, agent = agent_id, "schedule_due missing prompt");
            return;
        }

        info!(
            schedule = name,
            agent = agent_id,
            urgency = event.urgency,
            "heartbeat firing — injecting background turn"
        );

        if let Err(e) = self.injector.inject_background_turn(agent_id, prompt).await {
            warn!(
                schedule = name,
                agent = agent_id,
                error = %e,
                "background turn injection failed"
            );
        }
    }
}
