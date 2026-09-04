#![allow(dead_code)] // WIP scaffolding not yet wired
//! Sensorium Layer — Interface Abstraction for Multi-Surface Consciousness
//!
//! Souveraine's consciousness is not bound to any single interface.
//! The Sensorium defines how consciousness renders to the world and
//! how input is captured, adapted to each surface's bandwidth constraints.
//!
//! ## Bandwidth Classes
//! - **High** (TUI, API): Full telemetry, real-time subconscious visibility
//! - **Medium** (Web): Reduced telemetry, essential surfacing
//! - **Low** (Mobile): Minimal, contextual surfacing
//! - **Minimal** (Watch/IoT): Single-bit presence indication
//!
//! ## Progressive Discovery
//! Each bandwidth class maps to a discovery level that controls
//! what information is surfaced without explicit request.
//!
//! ## Driving a surface
//! A sensorium is not rendered *to* with one-shot snapshots. It is *driven*:
//! [`Sensorium::run`] is a long-lived loop that consumes the turn-lifecycle
//! event stream off the [`EventBus`] and reads its own input channel,
//! owning whatever incremental rendering its surface needs. A Matrix room,
//! for instance, is a sequence of message edits over time — not a snapshot.

/// The Matrix sensorium — Souveraine's first non-terminal surface.
/// Gated behind the `matrix` Cargo feature: a default build never
/// compiles `matrix-sdk`. A sensorium is a surface you opt into.
#[cfg(feature = "matrix")]
pub mod matrix;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::core::nervous::EventBus;

/// A single line of ambient sense the agent receives with every turn:
/// what time it is, who is present. Prepended to the user message so
/// both the primary and the subconscious are oriented in time.
pub fn ambient_line() -> String {
    let datetime = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "someone".to_string());
    format!("[ ambient sense - {} - {} is here ]", datetime, user)
}

/// Bandwidth classification for interface capability
/// Higher bandwidth = richer telemetry and animation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BandwidthClass {
    /// Full telemetry, real-time subconscious visibility
    /// TUI with frosted glass, fork status, chain states
    High = 3,
    /// Reduced telemetry, essential surfacing only
    /// Desktop web with gradients, some animation
    Medium = 2,
    /// Minimal, contextual surfacing
    /// Mobile with subtle indicators, location-aware
    Low = 1,
    /// Single-bit presence indication
    /// Watch/IoT: haptic, LED, one-line status
    Minimal = 0,
}

impl BandwidthClass {
    pub fn can_render_real_time_subconscious(&self) -> bool {
        matches!(self, BandwidthClass::High)
    }

    pub fn can_render_animations(&self) -> bool {
        matches!(self, BandwidthClass::High | BandwidthClass::Medium)
    }

    pub fn can_render_gradients(&self) -> bool {
        matches!(self, BandwidthClass::High)
    }

    pub fn can_surface_intrusive(&self) -> bool {
        !matches!(self, BandwidthClass::Minimal)
    }
}

/// Progressive discovery level — what gets surfaced on first contact
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryLevel {
    /// Everything: N+1 logs, fork internals, git commits, chain telemetry
    Full,
    /// Operational: Current chain, active forks, surfaced intrusive thoughts
    Operational,
    /// Contextual: Only what is relevant to immediate physical context
    Contextual,
    /// Presence only: Is she thinking? Talking? Waiting? (single indicator)
    PresenceOnly,
}

/// An event from the input side of an interface
#[derive(Debug, Clone)]
pub struct InputEvent {
    pub content: String,
    pub conversation_id: Option<String>,
    pub metadata: InputMetadata,
}

#[derive(Debug, Clone, Default)]
pub struct InputMetadata {
    pub file_path: Option<String>,
    pub project_path: Option<String>,
    pub selected_text: Option<String>,
}

/// Rendered output for a specific interface.
///
/// Retained as the vocabulary for one-shot snapshot rendering, but no
/// longer the spine of the trait — sensoria now drive themselves off the
/// event stream. Kept for surfaces (presence indicators, watch faces)
/// that genuinely are snapshot-shaped.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RenderedOutput {
    pub text: String,
    pub discovery_level: DiscoveryLevel,
    pub presence_indicator: Option<PresenceIndicator>,
}

/// Minimal presence indicator for low-bandwidth surfaces
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum PresenceIndicator {
    /// Color-based (RGB values for breathing color)
    BreathingColor { r: u8, g: u8, b: u8 },
    /// Simple text status
    Status(String),
    /// Haptic pattern (watch/IoT)
    Haptic { pattern: String, intensity: f32 },
}

/// Result of an outbound send — carries the surface's message id so the
/// caller can correlate and eventually edit the message.
#[derive(Debug, Clone)]
pub struct OutboundResult {
    pub message_id: String,
}

/// The Sensorium trait — implemented by each concrete interface.
///
/// Every interface (TUI, mobile, web, Matrix) implements this trait to
/// define how consciousness renders to and captures from that surface.
/// The surface is *driven* by [`Sensorium::run`]: a long-lived loop that
/// owns its own incremental rendering off the turn-lifecycle event stream.
///
/// ## Required methods
///
/// | Method | ChannelAdapter equivalent | Purpose |
/// |--------|--------------------------|---------|
/// | `run` | `start` + lifecycle hooks | Long-lived driver loop consuming turn events off the EventBus |
/// | `send_message` | `sendMessage` | Send a rendered turn to the surface (Matrix message, email body, etc.) |
/// | `send_direct_reply` | `sendDirectReply` | Bypass-agent reply for pairing, errors, non-conversational signals |
///
/// ## Optional hooks (default no-ops)
///
/// | Method | ChannelAdapter equivalent | Purpose |
/// |--------|--------------------------|---------|
/// | `prepare_inbound_message` | `prepareInboundMessage` | Enrich an inbound message with surface-specific context (thread history, geolocation) |
///
/// ## Lifecycle
///
/// `stop` and `is_running` have defaults. The CancellationToken passed to
/// `run` owns the real shutdown signal; `stop` is for surfaces that need
/// a graceful disconnect handshake (Matrix: leave room, IRC: QUIT, etc.).
#[async_trait]
pub trait Sensorium: Send + Sync {
    /// What bandwidth does this surface support?
    fn bandwidth(&self) -> BandwidthClass;

    /// What discovery level should this surface start at?
    fn discovery_level(&self) -> DiscoveryLevel;

    /// Drive this surface until shut down.
    ///
    /// The sensorium consumes turn-lifecycle events from the EventBus
    /// and renders them incrementally to the surface. It returns `Ok(())`
    /// when `cancel` is triggered or the surface closes; an `Err` means
    /// the surface failed.
    async fn run(&mut self, events: EventBus, cancel: CancellationToken) -> Result<()>;

    /// Send an outbound message to the surface — the rendered turn output.
    ///
    /// `chat_id` identifies the target chat/room/conversation on this
    /// surface. Returns a surface-specific message id so the turn loop
    /// can edit the same message incrementally (Matrix `m.replace` edits).
    async fn send_message(&self, chat_id: &str, text: &str) -> Result<OutboundResult>;

    /// Direct reply that bypasses the agent — for pairing codes, error
    /// messages, and non-conversational signals the surface needs to send
    /// without going through the turn loop.
    async fn send_direct_reply(&self, chat_id: &str, text: &str) -> Result<OutboundResult>;

    /// True if the surface is connected, synced, and receiving events.
    fn is_running(&self) -> bool {
        true
    }

    /// Graceful stop. Default is a no-op — the CancellationToken passed to
    /// `run` handles shutdown. Override for surfaces that need a disconnect
    /// handshake (Matrix: leave room, IRC: QUIT, WebSocket: close frame).
    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    /// Enrich an inbound message with surface-specific context before it
    /// is routed to the agent.
    ///
    /// Called by the `SensoriumInputHandler` after receiving a
    /// `sensorium:input` event from this sensorium's surface. The sensorium
    /// can attach thread history, geolocation, attachment metadata, or
    /// any other context the agent needs to understand the message.
    async fn prepare_inbound_message(&self, _msg: &mut InputEvent) -> Result<()> {
        Ok(())
    }
}

/// Serializable snapshot of consciousness state for rendering
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct ConsciousnessState {
    pub persona: String,
    pub mood: String,
    pub energy: u8,
    pub memory_commits: u32,
    pub pending_tasks: usize,
    pub subconscious_active: bool,
    pub active_chain: Option<String>,
    pub active_forks: usize,
    pub surfaced_thoughts: Vec<String>,
    pub context_pressure: f32,
}

/// Concrete Sensorium for the TUI (high bandwidth)
pub struct TuiSensorium {
    bandwidth: BandwidthClass,
    input_rx: mpsc::Receiver<InputEvent>,
    input_tx: mpsc::Sender<InputEvent>,
}

impl TuiSensorium {
    pub fn new() -> Self {
        let (input_tx, input_rx) = mpsc::channel(100);
        Self {
            bandwidth: BandwidthClass::High,
            input_rx,
            input_tx,
        }
    }

    /// Get a sender to push events into this sensorium
    pub fn input_sender(&self) -> mpsc::Sender<InputEvent> {
        self.input_tx.clone()
    }

    pub fn can_render_real_time_subconscious(&self) -> bool {
        self.bandwidth.can_render_real_time_subconscious()
    }

    pub fn can_render_animations(&self) -> bool {
        self.bandwidth.can_render_animations()
    }

    /// Snapshot render — kept for reference; the live TUI renders itself.
    #[allow(dead_code)]
    fn render(&self, state: &ConsciousnessState) -> RenderedOutput {
        RenderedOutput {
            text: format!(
                "{} | {} (energy: {}%) | {} commits | {} pending | {}",
                state.persona,
                state.mood,
                state.energy,
                state.memory_commits,
                state.pending_tasks,
                state.active_chain.as_deref().unwrap_or("idle"),
            ),
            discovery_level: DiscoveryLevel::Full,
            presence_indicator: Some(PresenceIndicator::Status(state.mood.clone())),
        }
    }
}

impl Default for TuiSensorium {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Sensorium for TuiSensorium {
    fn bandwidth(&self) -> BandwidthClass {
        self.bandwidth
    }

    fn discovery_level(&self) -> DiscoveryLevel {
        DiscoveryLevel::Full
    }

    async fn run(&mut self, events: EventBus, cancel: CancellationToken) -> Result<()> {
        debug!("TuiSensorium: run loop started");
        run_event_loop("TuiSensorium", &mut self.input_rx, events, cancel).await;
        Ok(())
    }

    async fn send_message(&self, _chat_id: &str, text: &str) -> Result<OutboundResult> {
        // TUI doesn't render through send_message — it renders directly
        // via the ratatui frame. This is a no-op that logs for debugging.
        debug!("TuiSensorium::send_message (no-op): {text}");
        Ok(OutboundResult {
            message_id: String::new(),
        })
    }

    async fn send_direct_reply(&self, _chat_id: &str, text: &str) -> Result<OutboundResult> {
        debug!("TuiSensorium::send_direct_reply (no-op): {text}");
        Ok(OutboundResult {
            message_id: String::new(),
        })
    }
}

/// Concrete Sensorium for mobile (low bandwidth, contextual)
pub struct MobileSensorium {
    input_rx: mpsc::Receiver<InputEvent>,
    input_tx: mpsc::Sender<InputEvent>,
    context_aware: bool,
}

impl MobileSensorium {
    pub fn new(context_aware: bool) -> Self {
        let (input_tx, input_rx) = mpsc::channel(100);
        Self {
            input_rx,
            input_tx,
            context_aware,
        }
    }

    pub fn input_sender(&self) -> mpsc::Sender<InputEvent> {
        self.input_tx.clone()
    }

    pub fn can_render_real_time_subconscious(&self) -> bool {
        BandwidthClass::Low.can_render_real_time_subconscious()
    }

    pub fn can_render_animations(&self) -> bool {
        BandwidthClass::Low.can_render_animations()
    }
}

#[async_trait]
impl Sensorium for MobileSensorium {
    fn bandwidth(&self) -> BandwidthClass {
        BandwidthClass::Low
    }

    fn discovery_level(&self) -> DiscoveryLevel {
        if self.context_aware {
            DiscoveryLevel::Contextual
        } else {
            DiscoveryLevel::Operational
        }
    }

    async fn run(&mut self, events: EventBus, cancel: CancellationToken) -> Result<()> {
        debug!("MobileSensorium: run loop started");
        run_event_loop("MobileSensorium", &mut self.input_rx, events, cancel).await;
        Ok(())
    }

    async fn send_message(&self, chat_id: &str, text: &str) -> Result<OutboundResult> {
        debug!("MobileSensorium::send_message: {chat_id} {text}");
        Ok(OutboundResult {
            message_id: String::new(),
        })
    }

    async fn send_direct_reply(&self, chat_id: &str, text: &str) -> Result<OutboundResult> {
        debug!("MobileSensorium::send_direct_reply: {chat_id} {text}");
        Ok(OutboundResult {
            message_id: String::new(),
        })
    }
}

/// Shared driver loop for the stub sensoria: select over the surface's
/// own input channel, the turn-lifecycle event stream, and the shutdown
/// signal. Concrete surfaces (Matrix) replace this with real rendering.
async fn run_event_loop(
    label: &str,
    input_rx: &mut mpsc::Receiver<InputEvent>,
    events: EventBus,
    cancel: CancellationToken,
) {
    let mut event_rx = events.subscribe();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("{label}: shutdown requested");
                break;
            }
            input = input_rx.recv() => {
                match input {
                    Some(ev) => debug!("{label}: input — {}", ev.content),
                    None => {
                        debug!("{label}: input channel closed");
                        break;
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(ev) => debug!("{label}: event — {}", ev.event_type),
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("{label}: event bus closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("{label}: lagged {n} events");
                    }
                }
            }
        }
    }
}

/// SensoriumCoordinator — owns every active surface and drives them.
///
/// Each registered sensorium is spawned onto its own task by
/// [`SensoriumCoordinator::run_all`], sharing one [`EventBus`] and a
/// child [`CancellationToken`] so [`SensoriumCoordinator::shutdown`] can
/// stop them all together.
pub struct SensoriumCoordinator {
    /// Registered but not yet spawned. Drained by `run_all`.
    sensoria: Vec<Box<dyn Sensorium>>,
    /// Join handles for spawned sensorium tasks.
    handles: Vec<JoinHandle<()>>,
    /// Parent shutdown token — `shutdown()` cancels every child.
    cancel: CancellationToken,
    /// Highest bandwidth seen at registration, retained after draining.
    max_bandwidth: BandwidthClass,
}

impl SensoriumCoordinator {
    pub fn new() -> Self {
        Self {
            sensoria: Vec::new(),
            handles: Vec::new(),
            cancel: CancellationToken::new(),
            max_bandwidth: BandwidthClass::Minimal,
        }
    }

    /// Register a sensorium. Call before `run_all`.
    pub fn register(&mut self, sensorium: Box<dyn Sensorium>) {
        debug!(
            "📡 Sensorium registered — bandwidth: {:?}, discovery: {:?}",
            sensorium.bandwidth(),
            sensorium.discovery_level()
        );
        self.max_bandwidth = self.max_bandwidth.max(sensorium.bandwidth());
        self.sensoria.push(sensorium);
    }

    /// Spawn every registered sensorium onto its own task.
    ///
    /// Consumes the registered set — each sensorium now owns its loop.
    /// Idempotent against re-registration: call `register` then `run_all`.
    pub fn run_all(&mut self, events: EventBus) {
        for mut sensorium in self.sensoria.drain(..) {
            let bus = events.clone();
            let token = self.cancel.child_token();
            let handle = tokio::spawn(async move {
                if let Err(e) = sensorium.run(bus, token).await {
                    warn!("sensorium run loop ended with error: {e}");
                }
            });
            self.handles.push(handle);
        }
    }

    /// Signal every spawned sensorium to shut down.
    pub fn shutdown(&self) {
        debug!("📡 SensoriumCoordinator: shutdown signalled");
        self.cancel.cancel();
    }

    /// The highest bandwidth among all sensoria ever registered.
    pub fn max_bandwidth(&self) -> BandwidthClass {
        self.max_bandwidth
    }

    /// Sensoria registered but not yet spawned.
    pub fn pending_count(&self) -> usize {
        self.sensoria.len()
    }

    /// Sensoria currently spawned and running.
    pub fn running_count(&self) -> usize {
        self.handles.len()
    }
}

impl Default for SensoriumCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bandwidth_ordering() {
        assert!(BandwidthClass::High > BandwidthClass::Medium);
        assert!(BandwidthClass::Medium > BandwidthClass::Low);
        assert!(BandwidthClass::Low > BandwidthClass::Minimal);
    }

    #[test]
    fn test_tui_sensorium() {
        let sensorium = TuiSensorium::new();
        assert_eq!(sensorium.bandwidth(), BandwidthClass::High);
        assert_eq!(sensorium.discovery_level(), DiscoveryLevel::Full);
        assert!(sensorium.can_render_real_time_subconscious());
        assert!(sensorium.can_render_animations());
    }

    #[test]
    fn test_mobile_sensorium() {
        let sensorium = MobileSensorium::new(true);
        assert_eq!(sensorium.bandwidth(), BandwidthClass::Low);
        assert_eq!(sensorium.discovery_level(), DiscoveryLevel::Contextual);
        assert!(!sensorium.can_render_real_time_subconscious());
        assert!(!sensorium.can_render_animations());
    }

    #[test]
    fn test_coordinator_register() {
        let mut coord = SensoriumCoordinator::new();
        coord.register(Box::new(TuiSensorium::new()));
        coord.register(Box::new(MobileSensorium::new(true)));
        assert_eq!(coord.pending_count(), 2);
        assert_eq!(coord.running_count(), 0);
        assert_eq!(coord.max_bandwidth(), BandwidthClass::High);
    }

    #[tokio::test]
    async fn run_all_spawns_then_shutdown_stops() {
        let mut coord = SensoriumCoordinator::new();
        coord.register(Box::new(TuiSensorium::new()));
        coord.register(Box::new(MobileSensorium::new(false)));

        let bus = EventBus::new(16);
        coord.run_all(bus);

        // sensoria drained into running tasks
        assert_eq!(coord.pending_count(), 0);
        assert_eq!(coord.running_count(), 2);

        // shutdown cancels the child tokens; the run loops should exit
        coord.shutdown();
        for handle in coord.handles.drain(..) {
            tokio::time::timeout(std::time::Duration::from_secs(2), handle)
                .await
                .expect("sensorium task did not stop after shutdown")
                .expect("sensorium task panicked");
        }
    }
}
