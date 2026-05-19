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

    /// Start a turn for `agent_id` in a specific conversation, with
    /// `text` as the user message. Unlike `inject_background_turn`,
    /// the resulting stream events ARE forwarded to the EventBus as
    /// `turn:*` events so the originating sensorium can render them.
    async fn inject_surface_turn(
        &self,
        agent_id: &str,
        conversation_id: &str,
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

/// Subscribes to `sensorium:input` events on the EventBus and injects
/// turns on behalf of non-terminal surfaces (Matrix, email, federation).
///
/// A surface receives an inbound message (room message, email, federated
/// query), fires a `sensorium:input` SensorEvent with the agent_id,
/// conversation_id, and text. This handler picks it up and routes it
/// into the backend via [`TurnInjector::inject_surface_turn`].
///
/// The resulting `turn:*` events flow back onto the EventBus so that
/// surface's sensorium (`MatrixSensorium::handle_turn_event`, etc.)
/// can render the response incrementally.
///
/// ## Event contract
///
/// | Field | Required | Source |
/// |-------|----------|--------|
/// | `event_type` | `"sensorium:input"` | Set by the firing surface |
/// | `target` | conversation/room id | Identifies the chat context |
/// | `payload.agent_id` | Yes | Which agent to route to |
/// | `payload.text` | Yes | User message content |
/// | `payload.surface` | No | Surface identifier for routing |
/// | `seed_id` | Yes (federated) | Federation origin DID |
///
/// ## Federation
///
/// When `seed_id` is set, the event originated from a federated peer.
/// The handler passes it through unchanged — the backend's turn loop
/// stamps it into the `TurnEventDispatcher` so outgoing `turn:*`
/// events carry the origin seed_id back to the right peer.
pub struct SensoriumInputHandler {
    rx: broadcast::Receiver<SensorEvent>,
    injector: Arc<dyn TurnInjector>,
}

impl SensoriumInputHandler {
    pub fn new(rx: broadcast::Receiver<SensorEvent>, injector: Arc<dyn TurnInjector>) -> Self {
        Self { rx, injector }
    }

    pub async fn run(&mut self) {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if event.event_type == "sensorium:input" {
                        self.handle_input_event(&event).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "sensorium input handler lagged, skipped events");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("event bus closed, sensorium input handler exiting");
                    break;
                }
            }
        }
    }

    async fn handle_input_event(&self, event: &SensorEvent) {
        let conversation_id = event.target.as_deref().unwrap_or("");
        let agent_id = event
            .payload
            .as_ref()
            .and_then(|p| p.get("agent_id"))
            .and_then(|v| v.as_str());
        let text = event
            .payload
            .as_ref()
            .and_then(|p| p.get("text"))
            .and_then(|v| v.as_str());

        let Some(agent_id) = agent_id else {
            warn!("sensorium:input missing agent_id in payload");
            return;
        };

        let text = text.unwrap_or("");
        if text.is_empty() && conversation_id.is_empty() {
            debug!("sensorium:input empty text and no conversation — ignoring");
            return;
        }

        info!(
            agent = agent_id,
            conversation = conversation_id,
            text_len = text.len(),
            seed = ?event.seed_id,
            "sensorium:input — injecting surface turn"
        );

        if let Err(e) = self
            .injector
            .inject_surface_turn(agent_id, conversation_id, text)
            .await
        {
            warn!(
                agent = agent_id,
                conversation = conversation_id,
                error = %e,
                "sensorium:input turn injection failed"
            );
        }
    }
}
