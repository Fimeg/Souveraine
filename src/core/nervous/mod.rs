pub mod cron;
pub mod event_log;
pub mod handler;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ── SensorEvent ─────────────────────────────────────────────────
//
// The universal event type. Local sensors fire these, federated peers
// fire these, cron fires these. One type, one bus, one nervous system.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SensorEvent {
    pub sensor_name: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub target: Option<String>,
    pub urgency: f32,
    pub payload: Option<serde_json::Value>,
    /// None = local event. Some(...) = originated from a federated peer.
    /// When federation lands, this becomes the peer's public key / DID.
    pub seed_id: Option<String>,
}

// ── SensorConfig ────────────────────────────────────────────────
//
// Per-sensor configuration — controls how a sensor participates in
// the nervous system. Sensors with `nervous_system: true` are nerve
// endings: they can push events without being called.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorConfig {
    pub channel: SensorChannel,
    /// If true, this sensor can push events onto the EventBus spontaneously.
    pub nervous_system: bool,
    pub push_threshold: PushThreshold,
    pub sensitivity: Sensitivity,
}

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            channel: SensorChannel::Filesystem,
            nervous_system: false,
            push_threshold: PushThreshold::OnChange,
            sensitivity: Sensitivity::Medium,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SensorChannel {
    Filesystem,
    FilesystemWatch,
    GitDiff,
    Cron,
    Memory,
    Process,
    Federation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PushThreshold {
    Once,
    OnChange,
    #[serde(rename = "interval")]
    Interval(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    Medium,
    High,
}

// ── EventBus ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<SensorEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn send(&self, event: SensorEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SensorEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}
