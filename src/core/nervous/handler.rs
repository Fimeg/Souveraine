use tokio::sync::broadcast;
use tracing::{debug, warn};

use super::SensorEvent;

pub struct HeartbeatHandler {
    rx: broadcast::Receiver<SensorEvent>,
}

impl HeartbeatHandler {
    pub fn new(rx: broadcast::Receiver<SensorEvent>) -> Self {
        Self { rx }
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
        let prompt = event
            .payload
            .as_ref()
            .and_then(|p| p.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        debug!(
            schedule = name,
            urgency = event.urgency,
            "heartbeat: schedule due"
        );

        // TODO(phase 3): inject turn through run_turn() when idle,
        // route to subconscious inbox when active session exists.
        // For now, log the event. The wiring into LocalBackend's
        // turn injection path comes in Phase 4 integration.
        let _ = prompt;
    }
}
