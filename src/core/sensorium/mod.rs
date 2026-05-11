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

use tokio::sync::mpsc;
use tracing::debug;

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

/// Rendered output for a specific interface
#[derive(Debug, Clone)]
pub struct RenderedOutput {
    pub text: String,
    pub discovery_level: DiscoveryLevel,
    pub presence_indicator: Option<PresenceIndicator>,
}

/// Minimal presence indicator for low-bandwidth surfaces
#[derive(Debug, Clone)]
pub enum PresenceIndicator {
    /// Color-based (RGB values for breathing color)
    BreathingColor { r: u8, g: u8, b: u8 },
    /// Simple text status
    Status(String),
    /// Haptic pattern (watch/IoT)
    Haptic { pattern: String, intensity: f32 },
}

/// The Sensorium trait — implemented by each concrete interface
///
/// Every interface (TUI, mobile, web, API) implements this trait
/// to define how consciousness renders to and captures from that surface.
pub trait Sensorium: Send + Sync {
    /// What bandwidth does this surface support?
    fn bandwidth(&self) -> BandwidthClass;

    /// What discovery level should this surface start at?
    fn discovery_level(&self) -> DiscoveryLevel;

    /// Render consciousness state for this specific interface
    fn render(&self, state: &ConsciousnessState) -> RenderedOutput;

    /// Get the receiver for input events
    fn input_receiver(&mut self) -> &mut mpsc::Receiver<InputEvent>;
}

/// Serializable snapshot of consciousness state for rendering
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
}

impl Default for TuiSensorium {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensorium for TuiSensorium {
    fn bandwidth(&self) -> BandwidthClass {
        self.bandwidth
    }

    fn discovery_level(&self) -> DiscoveryLevel {
        DiscoveryLevel::Full
    }

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

    fn input_receiver(&mut self) -> &mut mpsc::Receiver<InputEvent> {
        &mut self.input_rx
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

    fn render(&self, state: &ConsciousnessState) -> RenderedOutput {
        // Mobile: minimal text, presence indicator only
        let text = if state.subconscious_active && !state.surfaced_thoughts.is_empty() {
            format!("💭 {}", state.surfaced_thoughts[0])
        } else {
            format!("{} — {}", state.persona, state.mood)
        };

        RenderedOutput {
            text,
            discovery_level: DiscoveryLevel::Contextual,
            presence_indicator: Some(PresenceIndicator::BreathingColor {
                r: 255,
                g: 140,
                b: 66,
            }),
        }
    }

    fn input_receiver(&mut self) -> &mut mpsc::Receiver<InputEvent> {
        &mut self.input_rx
    }
}

/// SensoriumCoordinator — routes state to all active sensoria
///
/// Each connected interface gets its own bandwidth-appropriate rendering.
/// Consciousness state updates once; each Sensorium decides how to present it.
pub struct SensoriumCoordinator {
    sensoria: Vec<Box<dyn Sensorium>>,
}

impl SensoriumCoordinator {
    pub fn new() -> Self {
        Self {
            sensoria: Vec::new(),
        }
    }

    /// Register a sensorium
    pub fn register(&mut self, sensorium: Box<dyn Sensorium>) {
        debug!(
            "📡 Sensorium registered — bandwidth: {:?}, discovery: {:?}",
            sensorium.bandwidth(),
            sensorium.discovery_level()
        );
        self.sensoria.push(sensorium);
    }

    /// Broadcast state to all registered sensoria
    pub fn broadcast(&self, state: &ConsciousnessState) -> Vec<RenderedOutput> {
        self.sensoria.iter().map(|s| s.render(state)).collect()
    }

    /// Get the highest bandwidth among all sensoria
    pub fn max_bandwidth(&self) -> BandwidthClass {
        self.sensoria
            .iter()
            .map(|s| s.bandwidth())
            .max()
            .unwrap_or(BandwidthClass::Minimal)
    }

    /// Number of connected interfaces
    pub fn interface_count(&self) -> usize {
        self.sensoria.len()
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
        let mut sensorium = TuiSensorium::new();
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
    fn test_sensorium_coordinator() {
        let mut coord = SensoriumCoordinator::new();
        coord.register(Box::new(TuiSensorium::new()));
        coord.register(Box::new(MobileSensorium::new(true)));
        assert_eq!(coord.interface_count(), 2);
        assert_eq!(coord.max_bandwidth(), BandwidthClass::High);
    }
}
