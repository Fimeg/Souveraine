//! TurnEventDispatcher — turn-lifecycle events onto the nervous system.
//!
//! A turn is not a black box that yields one final answer. It is a
//! sequence: reasoning chunks, tool calls starting and finishing, output
//! arriving in segments, the primary pass completing, sometimes an
//! interrupt. Every non-terminal surface — a Matrix room, a mobile
//! screen — needs to *see* that sequence to render it incrementally.
//!
//! The dispatcher is the single seam where a turn announces what is
//! happening. It fires [`SensorEvent`]s namespaced under `turn:` so a
//! sensorium can subscribe to the [`EventBus`] and drive itself, exactly
//! as `event_log` and `HeartbeatHandler` already consume the same bus.
//!
//! One dispatcher is constructed per turn. `target` carries the turn id
//! so a surface tracking several concurrent turns (one per room) routes
//! each event to the right place.

use serde_json::json;

use crate::core::nervous::{EventBus, SensorEvent};

/// Fires turn-lifecycle events for a single turn.
#[allow(dead_code)]
pub struct TurnEventDispatcher {
    bus: EventBus,
    /// Identifies the turn — conversation/room id. Stamped into `target`.
    turn_id: String,
    /// None = local turn. Some = the turn belongs to a federated peer.
    seed_id: Option<String>,
}

#[allow(dead_code)]
impl TurnEventDispatcher {
    /// A segment of streamed output text. payload: `{ "text": String }`.
    pub const EVT_SEGMENT: &'static str = "turn:segment";
    /// A reasoning / thinking chunk. payload: `{ "text": String }`.
    pub const EVT_REASONING: &'static str = "turn:reasoning";
    /// A tool call has started. payload: `{ "tool": String, "call_id": String }`.
    pub const EVT_TOOL_START: &'static str = "turn:tool_start";
    /// A tool call has finished. payload: `{ "tool", "call_id", "is_error": bool }`.
    pub const EVT_TOOL_END: &'static str = "turn:tool_end";
    /// A still-running tool's liveness tick. payload: `{ "call_id", "elapsed_secs": u64 }`.
    pub const EVT_TOOL_TICK: &'static str = "turn:tool_tick";
    /// The turn is finished. payload: `{}`.
    pub const EVT_TURN_FINISH: &'static str = "turn:finish";
    /// The primary pass is complete (the subconscious pass may follow). payload: `{}`.
    pub const EVT_PRIMARY_COMPLETE: &'static str = "turn:primary_complete";
    /// The turn was interrupted. payload: `{ "reason": String }`.
    pub const EVT_INTERRUPTED: &'static str = "turn:interrupted";

    /// Construct a dispatcher for one turn.
    pub fn new(bus: EventBus, turn_id: impl Into<String>, seed_id: Option<String>) -> Self {
        Self {
            bus,
            turn_id: turn_id.into(),
            seed_id,
        }
    }

    /// The turn this dispatcher speaks for.
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    fn fire(&self, event_type: &str, urgency: f32, payload: serde_json::Value) {
        self.bus.send(SensorEvent {
            sensor_name: "turn".into(),
            timestamp: chrono::Utc::now(),
            event_type: event_type.into(),
            target: Some(self.turn_id.clone()),
            urgency,
            payload: Some(payload),
            seed_id: self.seed_id.clone(),
            reply_to: None,
        });
    }

    /// A chunk of streamed output text arrived.
    pub fn emit_segment(&self, text: &str) {
        self.fire(Self::EVT_SEGMENT, 0.1, json!({ "text": text }));
    }

    /// A chunk of reasoning / thinking arrived.
    pub fn emit_reasoning(&self, text: &str) {
        self.fire(Self::EVT_REASONING, 0.1, json!({ "text": text }));
    }

    /// A tool call began.
    pub fn emit_tool_start(&self, tool: &str, call_id: &str) {
        self.fire(
            Self::EVT_TOOL_START,
            0.2,
            json!({ "tool": tool, "call_id": call_id }),
        );
    }

    /// A tool call finished — `is_error` distinguishes a failure.
    pub fn emit_tool_end(&self, tool: &str, call_id: &str, is_error: bool) {
        self.fire(
            Self::EVT_TOOL_END,
            0.2,
            json!({ "tool": tool, "call_id": call_id, "is_error": is_error }),
        );
    }

    /// A still-running tool is alive — keeps a live ticker honest.
    pub fn emit_tool_tick(&self, call_id: &str, elapsed_secs: u64) {
        self.fire(
            Self::EVT_TOOL_TICK,
            0.05,
            json!({ "call_id": call_id, "elapsed_secs": elapsed_secs }),
        );
    }

    /// The turn is finished — a surface can finalise its rendering.
    pub fn emit_turn_finish(&self) {
        self.fire(Self::EVT_TURN_FINISH, 0.3, json!({}));
    }

    /// The primary pass is complete; a subconscious pass may still follow.
    pub fn emit_primary_complete(&self) {
        self.fire(Self::EVT_PRIMARY_COMPLETE, 0.3, json!({}));
    }

    /// The turn was interrupted before finishing.
    pub fn emit_interrupted(&self, reason: &str) {
        self.fire(Self::EVT_INTERRUPTED, 0.5, json!({ "reason": reason }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    fn dispatcher() -> (TurnEventDispatcher, tokio::sync::broadcast::Receiver<SensorEvent>) {
        let bus = EventBus::new(64);
        let rx = bus.subscribe();
        (TurnEventDispatcher::new(bus, "conv-1", None), rx)
    }

    #[test]
    fn segment_event_carries_text_and_turn_id() {
        let (d, mut rx) = dispatcher();
        d.emit_segment("hello");
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.event_type, TurnEventDispatcher::EVT_SEGMENT);
        assert_eq!(ev.sensor_name, "turn");
        assert_eq!(ev.target.as_deref(), Some("conv-1"));
        assert_eq!(ev.payload.unwrap()["text"], "hello");
    }

    #[test]
    fn tool_lifecycle_events_carry_call_id() {
        let (d, mut rx) = dispatcher();
        d.emit_tool_start("bash", "call-7");
        d.emit_tool_tick("call-7", 5);
        d.emit_tool_end("bash", "call-7", true);

        let start = rx.try_recv().unwrap();
        assert_eq!(start.event_type, TurnEventDispatcher::EVT_TOOL_START);
        assert_eq!(start.payload.unwrap()["call_id"], "call-7");

        let tick = rx.try_recv().unwrap();
        assert_eq!(tick.event_type, TurnEventDispatcher::EVT_TOOL_TICK);
        assert_eq!(tick.payload.unwrap()["elapsed_secs"], 5);

        let end = rx.try_recv().unwrap();
        assert_eq!(end.event_type, TurnEventDispatcher::EVT_TOOL_END);
        assert_eq!(end.payload.unwrap()["is_error"], true);
    }

    #[test]
    fn finish_and_interrupt_events() {
        let (d, mut rx) = dispatcher();
        d.emit_primary_complete();
        d.emit_interrupted("user raised a hand");
        d.emit_turn_finish();

        assert_eq!(
            rx.try_recv().unwrap().event_type,
            TurnEventDispatcher::EVT_PRIMARY_COMPLETE
        );
        let interrupted = rx.try_recv().unwrap();
        assert_eq!(interrupted.event_type, TurnEventDispatcher::EVT_INTERRUPTED);
        assert_eq!(interrupted.payload.unwrap()["reason"], "user raised a hand");
        assert_eq!(
            rx.try_recv().unwrap().event_type,
            TurnEventDispatcher::EVT_TURN_FINISH
        );
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
}
