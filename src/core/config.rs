use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use crate::core::compact::CompactionConfig;

/// Top-level config — mirrors souveraine.example.toml structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsciousnessConfig {
    /// Bifrost inference gateway config
    #[serde(default)]
    pub bifrost: BifrostConfig,

    /// Per-model physics configs
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,

    /// Subconscious (N+1, inbox)
    #[serde(default)]
    pub subconscious: SubconsciousConfig,

    /// Reflection (N+25)
    #[serde(default)]
    pub reflection: ReflectionConfig,

    /// Archivist (N+100 compression)
    #[serde(default)]
    pub archivist: ArchivistConfig,

    /// In-session message compaction
    #[serde(default)]
    pub compaction: CompactionConfig,

    /// Subagent pool
    #[serde(default)]
    pub subagent: SubagentConfig,

    /// Memory (git-backed)
    #[serde(default)]
    pub memory: MemoryConfig,

    /// WebSocket server
    #[serde(default)]
    pub websocket: WebSocketConfig,

    /// Sensorium (interface abstraction)
    #[serde(default)]
    pub sensorium: SensoriumConfig,

    /// Server bind/port + client connection URL
    #[serde(default)]
    pub server: ServerConfig,

    /// Schedule system (cron sensor)
    #[serde(default)]
    pub schedules: SchedulesConfig,

    /// Event persistence (firehose log)
    #[serde(default)]
    pub events: EventsConfig,

    /// Federation (cross-instance sync)
    #[serde(default)]
    pub federation: FederationConfig,

    /// Self-awareness pulse during long turns (in-turn noticing of time passing).
    #[serde(default)]
    pub presence: PresenceConfig,

    /// Voice channel — STT/TTS services, mic capture, push-to-talk key.
    #[serde(default)]
    pub voice: VoiceConfig,

    /// Top-level agent/substrate settings — platform prompt, etc.
    #[serde(default)]
    pub agent: AgentConfig,

    /// TUI interface settings (stale timeout, etc.)
    #[serde(default)]
    pub tui: TuiConfig,
}

// ── Agent (top-level) ──

/// Substrate-level settings for the primary agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Platform prompt — injected at the very top of the system prompt,
    /// before the agent's own identity files. Operator-level context that
    /// the agent reads but did not write. Editable in Settings > Agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

// ── Server ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Where the server listens (bind address)
    #[serde(default = "default_server_bind")]
    pub bind: String,

    /// TCP port the server listens on
    #[serde(default = "default_server_port")]
    pub port: u16,

    /// URL clients use to reach the server. Env `SOUVERAINE_SERVER_URL` wins
    /// at runtime; this value is the persistent default.
    #[serde(default = "default_server_url")]
    pub url: String,

    /// Auth configuration for the memfs HTTP write path.
    #[serde(default)]
    pub auth: AuthConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_server_bind(),
            port: default_server_port(),
            url: default_server_url(),
            auth: AuthConfig::default(),
        }
    }
}

impl ServerConfig {
    /// Effective URL — env var overrides config.
    pub fn effective_url(&self) -> String {
        std::env::var("SOUVERAINE_SERVER_URL").unwrap_or_else(|_| self.url.clone())
    }
}

// ── Auth ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Require bearer-token authentication for memory routes.
    #[serde(default = "default_true")]
    pub required: bool,

    /// Allow loopback (127.0.0.1 / ::1) requests to bypass auth.
    #[serde(default = "default_true")]
    pub allow_loopback: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            required: true,
            allow_loopback: true,
        }
    }
}

// ── Bifrost ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BifrostConfig {
    /// Bifrost API base URL (e.g. "http://10.10.20.120:3360")
    #[serde(default = "default_bifrost_url")]
    pub base_url: String,

    /// Bearer token for auth
    #[serde(default = "default_bifrost_key")]
    pub api_key: String,

    /// Virtual key for x-bf-vk header (required by some providers)
    #[serde(default = "default_bifrost_virtual_key")]
    pub virtual_key: String,

    /// Default model for conversation
    #[serde(default = "default_primary_model")]
    pub primary_model: String,

    /// Per-model overrides
    #[serde(default)]
    pub models: HashMap<String, BifrostModelConfig>,

    /// Request timeout in seconds for each LLM call attempt.
    /// When exceeded, the attempt is treated as a transient error and retried.
    /// Default: 120 (two minutes per attempt, 7 attempts = ~14 min total).
    #[serde(default = "default_bifrost_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BifrostModelConfig {
    #[serde(default = "default_128k")]
    pub context_limit: usize,
    #[serde(default = "default_8k")]
    pub output_limit: usize,
    #[serde(default = "default_threshold_70")]
    pub archivist_threshold: f32,
}

impl Default for BifrostConfig {
    fn default() -> Self {
        Self {
            base_url: default_bifrost_url(),
            api_key: default_bifrost_key(),
            virtual_key: default_bifrost_virtual_key(),
            primary_model: default_primary_model(),
            models: HashMap::new(),
            timeout_secs: default_bifrost_timeout(),
        }
    }
}

// ── Subconscious ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousConfig {
    #[serde(default = "default_true")]
    pub n1_enabled: bool,
    #[serde(default)]
    pub n1_trigger: N1Trigger,
    #[serde(default = "default_true")]
    pub inbox_enabled: bool,
    /// Model handle for the subconscious pass (e.g. "openai/glm-5.1").
    /// Defaults to None — uses the primary agent's model.
    #[serde(default)]
    pub model: Option<String>,
    /// Max tokens for Aster's response. Set to control cost/length.
    /// Defaults to None — let the model use its full output capacity.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Per-agent N+ interval overrides (e.g. Ani=N+1, Helper=N+5)
    #[serde(default)]
    pub per_agent_intervals: HashMap<String, AgentSubconsciousConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSubconsciousConfig {
    pub n_interval: usize,
}

impl Default for SubconsciousConfig {
    fn default() -> Self {
        Self {
            n1_enabled: true,
            n1_trigger: N1Trigger::EveryResponse,
            inbox_enabled: true,
            model: None,
            max_tokens: None,
            per_agent_intervals: HashMap::new(),
        }
    }
}

// ── Reflection ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_25")]
    pub message_interval: usize,
    #[serde(default)]
    pub trigger: ReflectionTrigger,
    /// Model handle for reflection passes. Falls back to subconscious model when unset.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub per_agent: HashMap<String, AgentReflectionSettings>,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            message_interval: 25,
            trigger: ReflectionTrigger::StepCount,
            model: None,
            per_agent: HashMap::new(),
        }
    }
}

// ── Archivist ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivistConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_100")]
    pub interval: usize,
    #[serde(default = "default_threshold_70")]
    pub threshold: f32,
    #[serde(default = "default_auto_model")]
    pub compression_model: String,
    #[serde(default = "default_synthesis_elements")]
    pub synthesis_elements: Vec<SynthesisElement>,
}

impl Default for ArchivistConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: 100,
            threshold: 0.7,
            compression_model: "auto".to_string(),
            synthesis_elements: default_synthesis_elements(),
        }
    }
}

// ── Subagent ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_3")]
    pub max_concurrent: usize,
    #[serde(default = "default_300")]
    pub timeout: u64,
    /// Maximum nesting depth for spawned subagents.
    #[serde(default = "default_3u32")]
    pub max_depth: u32,
    /// Maximum tool rounds per subagent turn.
    #[serde(default = "default_25u32")]
    pub max_tool_rounds: u32,
    /// Fraction of max_tool_rounds at which first warning fires.
    #[serde(default = "default_warning_1_threshold")]
    pub warning_1_threshold: f32,
    /// Fraction of max_tool_rounds at which second warning fires.
    #[serde(default = "default_warning_2_threshold")]
    pub warning_2_threshold: f32,
    /// Milliseconds to wait between subagent tool rounds.
    /// Helps avoid rate-limit cascades from rapid consecutive LLM calls.
    /// Default: 300ms. Set to 0 to disable.
    #[serde(default = "default_sub_inter_round_delay")]
    pub inter_round_delay_ms: u64,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent: 3,
            timeout: 300,
            max_depth: 3,
            max_tool_rounds: 50,
            warning_1_threshold: 0.8,
            warning_2_threshold: 0.95,
            inter_round_delay_ms: 300,
        }
    }
}

// ── Memory ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_true")]
    pub git_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_commit: bool,
    #[serde(default)]
    pub auto_push: bool,
    #[serde(default)]
    pub base_path: Option<PathBuf>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            git_enabled: true,
            auto_commit: true,
            auto_push: false,
            base_path: None,
        }
    }
}

// ── WebSocket ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_7373")]
    pub port: u16,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 7373,
        }
    }
}

// ── Sensorium ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensoriumConfig {
    #[serde(default = "default_bandwidth_high")]
    pub primary_bandwidth: BandwidthClass,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default = "default_true")]
    pub mobile_context_aware: bool,
}

impl Default for SensoriumConfig {
    fn default() -> Self {
        Self {
            primary_bandwidth: BandwidthClass::High,
            discovery: DiscoveryConfig::default(),
            mobile_context_aware: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_true")]
    pub low_urgency_only: bool,
    #[serde(default = "default_presence_breathing")]
    pub minimal_presence_mode: String,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            low_urgency_only: true,
            minimal_presence_mode: "breathing_color".to_string(),
        }
    }
}

// ── Agent Identity ──

/// Discriminates agent types for directory routing and behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentType {
    Primary,
    Subconscious,
    Subagent,
}

/// Identity metadata carried by every agent at creation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub agent_type: AgentType,
    /// Set for Subconscious and Subagent — links back to the creator.
    pub parent_agent: Option<String>,
    /// Path or generator key for this agent's system prompt.
    pub system_prompt_source: String,
}

// ── Enums & Shared Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum N1Trigger {
    EveryResponse,
    EveryNResponses(usize),
    TimeBased(u64),
    Manual,
}

impl Default for N1Trigger {
    fn default() -> Self { N1Trigger::EveryResponse }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionTrigger {
    Off,
    StepCount,
    CompactionEvent,
}

impl Default for ReflectionTrigger {
    fn default() -> Self { ReflectionTrigger::StepCount }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BandwidthClass {
    High, Medium, Low, Minimal,
}

impl Default for BandwidthClass {
    fn default() -> Self { BandwidthClass::High }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisElement {
    Themes, Emotions, Tensions, Anchors, Evolution, Patterns,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Conversation, Synthesis, Reflection, Research, FastResponse, Coding,
}

// ── Model Physics ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_128k")]
    pub context_limit: usize,
    #[serde(default = "default_8k")]
    pub output_limit: usize,
    #[serde(default = "default_threshold_70")]
    pub archivist_threshold: f32,
    #[serde(default = "default_100")]
    pub archivist_interval: usize,
    #[serde(default)]
    pub preferred_for: Vec<TaskType>,
}

// ── Agent Reflection Settings ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReflectionSettings {
    pub trigger: ReflectionTrigger,
    #[serde(default = "default_25")]
    pub step_count: usize,
}

// ── Defaults ──

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            bifrost: BifrostConfig::default(),
            models: default_models(),
            subconscious: SubconsciousConfig::default(),
            reflection: ReflectionConfig::default(),
            archivist: ArchivistConfig::default(),
            compaction: CompactionConfig::default(),
            subagent: SubagentConfig::default(),
            memory: MemoryConfig::default(),
            websocket: WebSocketConfig::default(),
            sensorium: SensoriumConfig::default(),
            server: ServerConfig::default(),
            schedules: SchedulesConfig::default(),
            events: EventsConfig::default(),
            federation: FederationConfig::default(),
            presence: PresenceConfig::default(),
            voice: VoiceConfig::default(),
            agent: AgentConfig::default(),
            tui: TuiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub schedules_dir: Option<PathBuf>,
}

impl Default for SchedulesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            schedules_dir: None,
        }
    }
}

// ── Events / Firehose ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub events_dir: Option<PathBuf>,
    #[serde(default = "default_retain_days")]
    pub retain_days: i64,
}

fn default_retain_days() -> i64 {
    30
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            events_dir: None,
            retain_days: 30,
        }
    }
}

// ── Presence (self-awareness pulse) ──

/// During a long turn, every `interval_secs`, the body injects a brief
/// system message in the agent's own register — a beat of self-awareness,
/// not a verdict. She reads it, decides what to do. Substrate, not harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceConfig {
    #[serde(default = "default_true")]
    pub pulse_enabled: bool,
    #[serde(default = "default_pulse_interval")]
    pub pulse_interval_secs: u64,
    /// Current outfit name (subdirectory under `expressions/`).
    /// Empty or None means root-level expressions.
    #[serde(default)]
    pub outfit: Option<String>,
    /// Current atmosphere preset name. Empty defaults to posture-linked.
    #[serde(default)]
    pub atmosphere: Option<String>,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            pulse_enabled: true,
            pulse_interval_secs: default_pulse_interval(),
            outfit: None,
            atmosphere: None,
        }
    }
}

fn default_pulse_interval() -> u64 { 600 }

// ── TUI ──

/// TUI-specific settings. None of these are load-bearing for the agent; they
/// tune the interface layer only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Deprecated — the auto-reset stall guard has been removed. The TUI
    /// now trusts the backend stream and shows a liveness label instead.
    /// Kept for config backwards compatibility.
    #[serde(default = "default_stale_timeout_secs")]
    pub stale_timeout_secs: u64,
    /// When the model produces text alongside tool calls, surface it in the
    /// chat stream as italic interstitial narration. Off = silent tool chains.
    #[serde(default = "default_true")]
    pub show_interstitial: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            stale_timeout_secs: default_stale_timeout_secs(),
            show_interstitial: true,
        }
    }
}

fn default_stale_timeout_secs() -> u64 { 90 }

// ── Federation ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub instance_label: Option<String>,
    /// Peers this instance federates with. Each peer connection is a signed
    /// WS stream to the peer's federation endpoint.
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    /// Seed_ids permitted to `consult` an agent on this instance. The basic
    /// consent floor — empty plus a missing `authorized-summoners.md` means
    /// permissive; richer per-arena gating is the agent's memfs concern.
    #[serde(default)]
    pub authorized_summoners: Vec<String>,
    /// When running as a lite listener, spawn the full engine on an
    /// authorized summon rather than only parking it.
    #[serde(default)]
    pub auto_wake: bool,
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            instance_label: None,
            peers: Vec::new(),
            authorized_summoners: Vec::new(),
            auto_wake: false,
        }
    }
}

/// A federated peer. `pubkey` is the peer's Ed25519 public key (hex) — the
/// trust root for verifying every event the peer sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Peer's federation endpoint, e.g. `ws://10.10.20.50:8484`.
    pub url: String,
    /// Peer's Ed25519 public key, hex-encoded.
    pub pubkey: String,
    /// Sensor-name subscriptions — which events this peer should receive.
    /// `*` = all; a bare name = exact; `name*` = prefix. Empty = none.
    #[serde(default)]
    pub subscriptions: Vec<String>,
}

// ── Voice channel ──

/// Voice channel configuration: STT (Faster-Whisper) + TTS (VibeVoice).
///
/// When `enabled = false`, the Presence screen behaves as today; no audio
/// devices are opened. The agent doesn't know or care whether voice is on —
/// voice is a sensorium channel, not a tool she calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Opt-in; default off. Audio devices not touched when false.
    #[serde(default)]
    pub enabled: bool,
    /// Faster-Whisper HTTP base URL.
    #[serde(default = "default_stt_url")]
    pub stt_url: String,
    /// VibeVoice HTTP base URL.
    #[serde(default = "default_tts_url")]
    pub tts_url: String,
    /// Voice ID passed to VibeVoice (`/audio/speech`).
    #[serde(default = "default_voice_id")]
    pub voice_id: String,
    /// Push-to-talk key label (informational only — the TUI uses Space).
    #[serde(default = "default_ptt_key")]
    pub push_to_talk_key: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stt_url: default_stt_url(),
            tts_url: default_tts_url(),
            voice_id: default_voice_id(),
            push_to_talk_key: default_ptt_key(),
        }
    }
}

fn default_stt_url() -> String { "http://10.10.20.19:7862".to_string() }
fn default_tts_url() -> String { "http://10.10.20.19:7861".to_string() }
fn default_voice_id() -> String { "en-Soother_woman".to_string() }
fn default_ptt_key() -> String { "Space".to_string() }

impl ConsciousnessConfig {
    /// Walk the standard config paths and return the first existing one.
    pub fn discover_path() -> Option<PathBuf> {
        let candidates = [
            "souveraine.toml",
            "souveraine.yaml",
            "~/.config/souveraine/config.toml",
            "~/.config/souveraine/config.yaml",
        ];
        for path_str in &candidates {
            let expanded = shellexpand::tilde(path_str);
            let path = PathBuf::from(expanded.as_ref());
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = if path.extension().map(|e| e == "toml").unwrap_or(false) {
            toml::from_str(&content)?
        } else {
            serde_yaml::from_str(&content)?
        };
        Ok(config)
    }

    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        let content = if path.extension().map(|e| e == "toml").unwrap_or(false) {
            toml::to_string_pretty(self)?
        } else {
            serde_yaml::to_string(self)?
        };
        std::fs::write(path, content)?;
        Ok(())
    }
}

// ── Default helper fns ──

fn default_true() -> bool { true }
fn default_3() -> usize { 3 }
fn default_25() -> usize { 25 }
fn default_3u32() -> u32 { 3 }
fn default_25u32() -> u32 { 50 }  // default subagent max tool rounds
fn default_100() -> usize { 100 }
fn default_300() -> u64 { 300 }
fn default_7373() -> u16 { 7373 }
fn default_128k() -> usize { 128000 }
fn default_8k() -> usize { 8192 }
fn default_threshold_70() -> f32 { 0.7 }
fn default_subconscious_max_tokens() -> u32 { 8192 }
fn default_warning_1_threshold() -> f32 { 0.8 }
fn default_warning_2_threshold() -> f32 { 0.95 }
fn default_sub_inter_round_delay() -> u64 { 300 }
fn default_auto_model() -> String { "auto".to_string() }
fn default_bifrost_url() -> String { "http://10.10.20.120:3360".to_string() }
fn default_server_bind() -> String { "127.0.0.1".to_string() }
fn default_server_port() -> u16 { 8484 }
fn default_server_url() -> String { "http://127.0.0.1:8484".to_string() }
fn default_bifrost_key() -> String {
    crate::core::credentials::get_bifrost_key()
}

fn default_bifrost_virtual_key() -> String {
    std::env::var("BIFROST_VIRTUAL_KEY").unwrap_or_else(|_| String::new())
}
fn default_primary_model() -> String { "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo".to_string() }
fn default_bifrost_timeout() -> u64 { 120 }
fn default_bandwidth_high() -> BandwidthClass { BandwidthClass::High }
fn default_presence_breathing() -> String { "breathing_color".to_string() }

fn default_synthesis_elements() -> Vec<SynthesisElement> {
    vec![SynthesisElement::Themes, SynthesisElement::Emotions, SynthesisElement::Tensions, SynthesisElement::Anchors, SynthesisElement::Evolution]
}

fn default_models() -> HashMap<String, ModelConfig> {
    let mut m = HashMap::new();
    m.insert("kimi-k2p5-turbo".to_string(), ModelConfig {
        provider: "bifrost".to_string(),
        model: "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo".to_string(),
        context_limit: 128000,
        output_limit: 8192,
        archivist_threshold: 0.7,
        archivist_interval: 100,
        preferred_for: vec![TaskType::Conversation],
    });
    m.insert("deepseek-v4-pro".to_string(), ModelConfig {
        provider: "bifrost".to_string(),
        model: "openai/deepseek-v4-pro".to_string(),
        context_limit: 128000,
        output_limit: 8192,
        archivist_threshold: 0.7,
        archivist_interval: 75,
        preferred_for: vec![TaskType::Conversation, TaskType::Reflection],
    });
    m
}
