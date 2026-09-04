#![allow(dead_code)] // WIP scaffolding not yet wired
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::ui::chat::ChatPalette;

// ── Panel focus ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Categories,
    Fields,
}

// ── Category groups ────────────────────────────────────────────────────────

/// Logical grouping of settings categories for the left-panel navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryGroup {
    Core,
    Intelligence,
    Presence,
    System,
}

impl CategoryGroup {
    pub fn all() -> &'static [CategoryGroup] {
        &[
            CategoryGroup::Core,
            CategoryGroup::Intelligence,
            CategoryGroup::Presence,
            CategoryGroup::System,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            CategoryGroup::Core => "CORE",
            CategoryGroup::Intelligence => "INTELLIGENCE",
            CategoryGroup::Presence => "PRESENCE",
            CategoryGroup::System => "SYSTEM",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            CategoryGroup::Core => "\u{2699}",         // ⚙
            CategoryGroup::Intelligence => "\u{2727}", // ✧
            CategoryGroup::Presence => "\u{25c6}",     // ◆
            CategoryGroup::System => "\u{2b1a}",       // ⬚
        }
    }

    /// Categories in display order within this group.
    pub fn categories(&self) -> &'static [Category] {
        match self {
            CategoryGroup::Core => &[Category::Agent, Category::Inference, Category::Providers],
            CategoryGroup::Intelligence => &[
                Category::Subconscious,
                Category::Reflection,
                Category::Archivist,
                Category::Subagent,
                Category::Compaction,
            ],
            CategoryGroup::Presence => &[
                Category::Presence,
                Category::Voice,
                Category::Sensorium,
                Category::Federation,
            ],
            CategoryGroup::System => &[
                Category::Memory,
                Category::Server,
                Category::WebSocket,
                Category::Schedules,
                Category::Events,
                Category::Image,
                Category::Tui,
            ],
        }
    }
}

// ── Categories ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Agent,
    Inference,
    Providers,
    Subconscious,
    Reflection,
    Archivist,
    Subagent,
    Memory,
    WebSocket,
    Sensorium,
    Compaction,
    Server,
    Schedules,
    Events,
    Federation,
    Presence,
    Voice,
    Image,
    Tui,
}

impl Category {
    pub fn all() -> &'static [Category] {
        &[
            Category::Agent,
            Category::Inference,
            Category::Providers,
            Category::Subconscious,
            Category::Reflection,
            Category::Archivist,
            Category::Subagent,
            Category::Memory,
            Category::WebSocket,
            Category::Sensorium,
            Category::Compaction,
            Category::Server,
            Category::Schedules,
            Category::Events,
            Category::Federation,
            Category::Presence,
            Category::Voice,
            Category::Image,
            Category::Tui,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::Agent => "Agent",
            Category::Inference => "Inference",
            Category::Providers => "Providers",
            Category::Subconscious => "Subconscious",
            Category::Reflection => "Reflection",
            Category::Archivist => "Archivist",
            Category::Subagent => "Subagent",
            Category::Memory => "Memory",
            Category::WebSocket => "WebSocket",
            Category::Sensorium => "Sensorium",
            Category::Compaction => "Compaction",
            Category::Server => "Server",
            Category::Schedules => "Schedules",
            Category::Events => "Events",
            Category::Federation => "Federation",
            Category::Presence => "Presence",
            Category::Voice => "Voice",
            Category::Image => "Image",
            Category::Tui => "TUI",
        }
    }

    /// The logical group this category belongs to.
    pub fn group(&self) -> CategoryGroup {
        match self {
            Category::Agent | Category::Inference | Category::Providers => CategoryGroup::Core,
            Category::Subconscious
            | Category::Reflection
            | Category::Archivist
            | Category::Subagent
            | Category::Compaction => CategoryGroup::Intelligence,
            Category::Presence | Category::Voice | Category::Sensorium | Category::Federation => {
                CategoryGroup::Presence
            }
            Category::Memory
            | Category::Server
            | Category::WebSocket
            | Category::Schedules
            | Category::Events
            | Category::Image
            | Category::Tui => CategoryGroup::System,
        }
    }

    /// Flat index into `Category::all()`.
    pub fn flat_index(&self) -> usize {
        Self::all().iter().position(|c| c == self).unwrap_or(0)
    }

    /// Category from a flat index into `Category::all()`.
    pub fn from_flat_index(idx: usize) -> Category {
        Self::all().get(idx).copied().unwrap_or(Category::Agent)
    }
}

// ── Field locations ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldLoc {
    // Agent
    AgSystemPrompt,
    AgModel,
    AgProvider,
    // Inference (global default provider selector)
    IfProvider,
    // Providers (per-provider fields — operates on the selected provider)
    PvName,
    PvType,
    PvBaseUrl,
    PvApiKey,
    PvVirtualKey,
    PvPrimaryModel,
    PvTimeoutSecs,
    PvAddProvider,
    PvRemoveProvider,
    PvSelectProvider,
    // Subconscious
    ScN1Enabled,
    ScN1Trigger,
    ScN1Every,
    ScN1Secs,
    ScInboxEnabled,
    ScModel,
    ScMaxTokens,
    ScSystemPrompt,
    // Reflection
    RfEnabled,
    RfMessageInterval,
    RfTrigger,
    RfModel,
    // Archivist
    ArEnabled,
    ArInterval,
    ArThreshold,
    ArCompressionModel,
    // Subagent
    SaEnabled,
    SaMaxConcurrent,
    SaTimeout,
    SaMaxDepth,
    SaMaxToolRounds,
    SaWarning1Threshold,
    SaWarning2Threshold,
    SaInterRoundDelayMs,
    // Memory
    MeGitEnabled,
    MeAutoCommit,
    MeAutoPush,
    MeBasePath,
    // WebSocket
    WsEnabled,
    WsPort,
    // Sensorium
    SePrimaryBandwidth,
    SeLowUrgencyOnly,
    SeMinimalPresenceMode,
    // Compaction
    CpEnabled,
    CpStrategy,
    CpWarnPressure,
    CpUrgentPressure,
    CpCriticalPressure,
    CpModel,
    // Server
    SvBind,
    SvPort,
    SvUrl,
    // Schedules
    ScdEnabled,
    // Events
    EvEnabled,
    EvRetainDays,
    // Federation
    FdEnabled,
    // Presence
    PrPulseEnabled,
    PrPulseIntervalSecs,
    PrOutfit,
    PrAtmosphere,
    // Voice
    VcEnabled,
    VcSttUrl,
    VcTtsUrl,
    VcVoiceId,
    VcPushToTalkKey,
    // Server auth
    SvAuthRequired,
    SvAuthLoopback,
    // Federation (cont.)
    FdRole,
    FdInstanceLabel,
    FdAutoWake,
    // TUI
    TuShowInterstitial,
    TuCennoThreshold,
    // Image
    ImMaxWidth,
    ImMaxHeight,
    ImMaxPixels,
    ImMaxBytes,
    ImJpegQuality,
    // Agent (read-only diagnostics)
    AgAgentId,
    AgSubconsciousId,
    AgMemoryPath,
    AgSubconsciousPath,
    AgSubconsciousStatus,
}

impl FieldLoc {
    pub fn category(&self) -> Category {
        match self {
            FieldLoc::AgSystemPrompt | FieldLoc::AgModel | FieldLoc::AgProvider => Category::Agent,
            FieldLoc::IfProvider => Category::Inference,
            FieldLoc::PvName
            | FieldLoc::PvType
            | FieldLoc::PvBaseUrl
            | FieldLoc::PvApiKey
            | FieldLoc::PvVirtualKey
            | FieldLoc::PvPrimaryModel
            | FieldLoc::PvTimeoutSecs
            | FieldLoc::PvAddProvider
            | FieldLoc::PvRemoveProvider
            | FieldLoc::PvSelectProvider => Category::Providers,
            FieldLoc::ScN1Enabled
            | FieldLoc::ScN1Trigger
            | FieldLoc::ScN1Every
            | FieldLoc::ScN1Secs
            | FieldLoc::ScInboxEnabled
            | FieldLoc::ScModel
            | FieldLoc::ScMaxTokens
            | FieldLoc::ScSystemPrompt => Category::Subconscious,
            FieldLoc::RfEnabled
            | FieldLoc::RfMessageInterval
            | FieldLoc::RfTrigger
            | FieldLoc::RfModel => Category::Reflection,
            FieldLoc::ArEnabled
            | FieldLoc::ArInterval
            | FieldLoc::ArThreshold
            | FieldLoc::ArCompressionModel => Category::Archivist,
            FieldLoc::SaEnabled
            | FieldLoc::SaMaxConcurrent
            | FieldLoc::SaTimeout
            | FieldLoc::SaMaxDepth
            | FieldLoc::SaMaxToolRounds
            | FieldLoc::SaWarning1Threshold
            | FieldLoc::SaWarning2Threshold
            | FieldLoc::SaInterRoundDelayMs => Category::Subagent,
            FieldLoc::MeGitEnabled
            | FieldLoc::MeAutoCommit
            | FieldLoc::MeAutoPush
            | FieldLoc::MeBasePath => Category::Memory,
            FieldLoc::WsEnabled | FieldLoc::WsPort => Category::WebSocket,
            FieldLoc::SePrimaryBandwidth
            | FieldLoc::SeLowUrgencyOnly
            | FieldLoc::SeMinimalPresenceMode => Category::Sensorium,
            FieldLoc::CpEnabled
            | FieldLoc::CpStrategy
            | FieldLoc::CpWarnPressure
            | FieldLoc::CpUrgentPressure
            | FieldLoc::CpCriticalPressure
            | FieldLoc::CpModel => Category::Compaction,
            FieldLoc::SvBind
            | FieldLoc::SvPort
            | FieldLoc::SvUrl
            | FieldLoc::SvAuthRequired
            | FieldLoc::SvAuthLoopback => Category::Server,
            FieldLoc::ScdEnabled => Category::Schedules,
            FieldLoc::EvEnabled | FieldLoc::EvRetainDays => Category::Events,
            FieldLoc::FdEnabled
            | FieldLoc::FdRole
            | FieldLoc::FdInstanceLabel
            | FieldLoc::FdAutoWake => Category::Federation,
            FieldLoc::PrPulseEnabled
            | FieldLoc::PrPulseIntervalSecs
            | FieldLoc::PrOutfit
            | FieldLoc::PrAtmosphere => Category::Presence,
            FieldLoc::VcEnabled
            | FieldLoc::VcSttUrl
            | FieldLoc::VcTtsUrl
            | FieldLoc::VcVoiceId
            | FieldLoc::VcPushToTalkKey => Category::Voice,
            FieldLoc::TuShowInterstitial | FieldLoc::TuCennoThreshold => Category::Tui,
            FieldLoc::ImMaxWidth
            | FieldLoc::ImMaxHeight
            | FieldLoc::ImMaxPixels
            | FieldLoc::ImMaxBytes
            | FieldLoc::ImJpegQuality => Category::Image,
            FieldLoc::AgAgentId
            | FieldLoc::AgSubconsciousId
            | FieldLoc::AgMemoryPath
            | FieldLoc::AgSubconsciousPath
            | FieldLoc::AgSubconsciousStatus => Category::Agent,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            FieldLoc::AgSystemPrompt => "system_prompt",
            FieldLoc::AgModel => "model",
            FieldLoc::AgProvider => "provider",
            FieldLoc::IfProvider => "provider",
            FieldLoc::PvName => "name",
            FieldLoc::PvType => "type",
            FieldLoc::PvBaseUrl => "base_url",
            FieldLoc::PvApiKey => "api_key",
            FieldLoc::PvVirtualKey => "virtual_key",
            FieldLoc::PvPrimaryModel => "primary_model",
            FieldLoc::PvTimeoutSecs => "timeout_secs",
            FieldLoc::PvAddProvider => "add_provider",
            FieldLoc::PvRemoveProvider => "remove_provider",
            FieldLoc::PvSelectProvider => "select_provider",
            FieldLoc::ScN1Enabled => "n1_enabled",
            FieldLoc::ScN1Trigger => "n1_trigger",
            FieldLoc::ScN1Every => "n1_every_n_responses",
            FieldLoc::ScN1Secs => "n1_interval_secs",
            FieldLoc::ScInboxEnabled => "inbox_enabled",
            FieldLoc::ScModel => "model",
            FieldLoc::ScMaxTokens => "max_tokens",
            FieldLoc::ScSystemPrompt => "system_prompt",
            FieldLoc::RfEnabled => "enabled",
            FieldLoc::RfMessageInterval => "message_interval",
            FieldLoc::RfTrigger => "trigger",
            FieldLoc::RfModel => "model",
            FieldLoc::ArEnabled => "enabled",
            FieldLoc::ArInterval => "interval",
            FieldLoc::ArThreshold => "threshold",
            FieldLoc::ArCompressionModel => "compression_model",
            FieldLoc::SaEnabled => "enabled",
            FieldLoc::SaMaxConcurrent => "max_concurrent",
            FieldLoc::SaTimeout => "timeout",
            FieldLoc::SaMaxDepth => "max_depth",
            FieldLoc::SaMaxToolRounds => "max_tool_rounds",
            FieldLoc::SaWarning1Threshold => "warning_1_threshold",
            FieldLoc::SaWarning2Threshold => "warning_2_threshold",
            FieldLoc::SaInterRoundDelayMs => "inter_round_delay_ms",
            FieldLoc::MeGitEnabled => "git_enabled",
            FieldLoc::MeAutoCommit => "auto_commit",
            FieldLoc::MeAutoPush => "auto_push",
            FieldLoc::MeBasePath => "base_path",
            FieldLoc::WsEnabled => "enabled",
            FieldLoc::WsPort => "port",
            FieldLoc::SePrimaryBandwidth => "primary_bandwidth",
            FieldLoc::SeLowUrgencyOnly => "low_urgency_only",
            FieldLoc::SeMinimalPresenceMode => "minimal_presence_mode",
            FieldLoc::CpEnabled => "enabled",
            FieldLoc::CpStrategy => "strategy",
            FieldLoc::CpWarnPressure => "warn_pressure",
            FieldLoc::CpUrgentPressure => "urgent_pressure",
            FieldLoc::CpCriticalPressure => "critical_pressure",
            FieldLoc::CpModel => "model",
            FieldLoc::SvBind => "bind",
            FieldLoc::SvPort => "port",
            FieldLoc::SvUrl => "url",
            FieldLoc::ScdEnabled => "enabled",
            FieldLoc::EvEnabled => "enabled",
            FieldLoc::EvRetainDays => "retain_days",
            FieldLoc::FdEnabled => "enabled",
            FieldLoc::PrPulseEnabled => "pulse_enabled",
            FieldLoc::PrPulseIntervalSecs => "pulse_interval_secs",
            FieldLoc::PrOutfit => "outfit",
            FieldLoc::PrAtmosphere => "atmosphere",
            FieldLoc::VcEnabled => "enabled",
            FieldLoc::VcSttUrl => "stt_url",
            FieldLoc::VcTtsUrl => "tts_url",
            FieldLoc::VcVoiceId => "voice_id",
            FieldLoc::VcPushToTalkKey => "push_to_talk_key",
            FieldLoc::SvAuthRequired => "required",
            FieldLoc::SvAuthLoopback => "allow_loopback",
            FieldLoc::FdRole => "role",
            FieldLoc::FdInstanceLabel => "instance_label",
            FieldLoc::FdAutoWake => "auto_wake",
            FieldLoc::TuShowInterstitial => "show_interstitial",
            FieldLoc::TuCennoThreshold => "cenno_word_threshold",
            FieldLoc::ImMaxWidth => "max_width",
            FieldLoc::ImMaxHeight => "max_height",
            FieldLoc::ImMaxPixels => "max_pixels",
            FieldLoc::ImMaxBytes => "max_bytes",
            FieldLoc::ImJpegQuality => "jpeg_quality",
            FieldLoc::AgAgentId => "agent_id",
            FieldLoc::AgSubconsciousId => "subconscious_id",
            FieldLoc::AgMemoryPath => "memory_path",
            FieldLoc::AgSubconsciousPath => "subconscious_path",
            FieldLoc::AgSubconsciousStatus => "subconscious_ok",
        }
    }

    /// Returns true if this field's value should be masked in browse mode.
    pub fn is_secret(&self) -> bool {
        matches!(self, FieldLoc::PvApiKey | FieldLoc::PvVirtualKey)
    }

    /// Returns true for informational fields that cannot be edited.
    pub fn is_readonly(&self) -> bool {
        matches!(
            self,
            FieldLoc::AgAgentId
                | FieldLoc::AgSubconsciousId
                | FieldLoc::AgMemoryPath
                | FieldLoc::AgSubconsciousPath
                | FieldLoc::AgSubconsciousStatus
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldLoc::AgSystemPrompt => "platform prompt",
            FieldLoc::AgModel => "agent model",
            FieldLoc::AgProvider => "provider",
            FieldLoc::IfProvider => "default provider",
            FieldLoc::PvName => "name",
            FieldLoc::PvType => "type",
            FieldLoc::PvBaseUrl => "endpoint",
            FieldLoc::PvApiKey => "API key",
            FieldLoc::PvVirtualKey => "virtual key",
            FieldLoc::PvPrimaryModel => "default model",
            FieldLoc::PvTimeoutSecs => "request timeout (s)",
            FieldLoc::PvAddProvider => "+ add provider",
            FieldLoc::PvRemoveProvider => "- remove",
            FieldLoc::PvSelectProvider => "provider",
            FieldLoc::ScN1Enabled => "N+1 enabled",
            FieldLoc::ScN1Trigger => "N+1 trigger",
            FieldLoc::ScN1Every => "  └ every N responses",
            FieldLoc::ScN1Secs => "  └ interval (s)",
            FieldLoc::ScInboxEnabled => "inbox",
            FieldLoc::ScModel => "model",
            FieldLoc::ScMaxTokens => "max tokens",
            FieldLoc::ScSystemPrompt => "subconscious prompt",
            FieldLoc::RfEnabled => "enabled",
            FieldLoc::RfMessageInterval => "every N msgs",
            FieldLoc::RfTrigger => "trigger",
            FieldLoc::RfModel => "model",
            FieldLoc::ArEnabled => "enabled",
            FieldLoc::ArInterval => "every N msgs",
            FieldLoc::ArThreshold => "threshold",
            FieldLoc::ArCompressionModel => "model",
            FieldLoc::SaEnabled => "enabled",
            FieldLoc::SaMaxConcurrent => "max concurrent",
            FieldLoc::SaTimeout => "timeout (s)",
            FieldLoc::SaMaxDepth => "max depth",
            FieldLoc::SaMaxToolRounds => "tool rounds",
            FieldLoc::SaWarning1Threshold => "warn 1 threshold",
            FieldLoc::SaWarning2Threshold => "warn 2 threshold",
            FieldLoc::SaInterRoundDelayMs => "inter-round delay",
            FieldLoc::MeGitEnabled => "git tracking",
            FieldLoc::MeAutoCommit => "auto commit",
            FieldLoc::MeAutoPush => "auto push",
            FieldLoc::MeBasePath => "base path",
            FieldLoc::WsEnabled => "enabled",
            FieldLoc::WsPort => "port",
            FieldLoc::SePrimaryBandwidth => "bandwidth",
            FieldLoc::SeLowUrgencyOnly => "low urgency only",
            FieldLoc::SeMinimalPresenceMode => "minimal mode",
            FieldLoc::CpEnabled => "enabled",
            FieldLoc::CpStrategy => "strategy",
            FieldLoc::CpWarnPressure => "warn at",
            FieldLoc::CpUrgentPressure => "urgent at",
            FieldLoc::CpCriticalPressure => "critical at",
            FieldLoc::CpModel => "summary model",
            FieldLoc::SvBind => "bind address",
            FieldLoc::SvPort => "port",
            FieldLoc::SvUrl => "public URL",
            FieldLoc::ScdEnabled => "enabled",
            FieldLoc::EvEnabled => "enabled",
            FieldLoc::EvRetainDays => "retain (days)",
            FieldLoc::FdEnabled => "enabled",
            FieldLoc::PrPulseEnabled => "pulse",
            FieldLoc::PrPulseIntervalSecs => "pulse interval",
            FieldLoc::PrOutfit => "outfit",
            FieldLoc::PrAtmosphere => "atmosphere",
            FieldLoc::VcEnabled => "enabled",
            FieldLoc::VcSttUrl => "STT endpoint",
            FieldLoc::VcTtsUrl => "TTS endpoint",
            FieldLoc::VcVoiceId => "voice ID",
            FieldLoc::VcPushToTalkKey => "push-to-talk",
            FieldLoc::SvAuthRequired => "require auth",
            FieldLoc::SvAuthLoopback => "allow loopback bypass",
            FieldLoc::FdRole => "role",
            FieldLoc::FdInstanceLabel => "instance label",
            FieldLoc::FdAutoWake => "auto-wake on summon",
            FieldLoc::TuShowInterstitial => "interstitial narration",
            FieldLoc::TuCennoThreshold => "cenno threshold (words)",
            FieldLoc::ImMaxWidth => "max width (px)",
            FieldLoc::ImMaxHeight => "max height (px)",
            FieldLoc::ImMaxPixels => "max pixels",
            FieldLoc::ImMaxBytes => "max file size (bytes)",
            FieldLoc::ImJpegQuality => "JPEG quality",
            FieldLoc::AgAgentId => "agent id",
            FieldLoc::AgSubconsciousId => "subconscious",
            FieldLoc::AgMemoryPath => "memory path",
            FieldLoc::AgSubconsciousPath => "subconscious path",
            FieldLoc::AgSubconsciousStatus => "subconscious on disk",
        }
    }

    /// One-line help description shown in the settings UI.
    pub fn description(&self) -> &'static str {
        match self {
            // Agent
            FieldLoc::AgSystemPrompt => "System-level instructions prepended to every conversation",
            FieldLoc::AgModel => "Primary LLM model used by the agent",
            FieldLoc::AgProvider => "Inference provider the agent routes requests through",
            FieldLoc::AgAgentId => "Unique identifier for this agent instance",
            FieldLoc::AgSubconsciousId => "Unique identifier for the subconscious agent",
            FieldLoc::AgMemoryPath => "On-disk path to the agent memory directory",
            FieldLoc::AgSubconsciousPath => "On-disk path to the subconscious agent directory",
            FieldLoc::AgSubconsciousStatus => "Whether the subconscious agent has been initialized on disk",
            // Inference
            FieldLoc::IfProvider => "Default inference provider used when no per-agent override is set",
            // Providers
            FieldLoc::PvName => "Short name for this provider (used in config and agent settings)",
            FieldLoc::PvType => "Provider type — openai-compatible for standard endpoints, openai-oauth for ChatGPT login",
            FieldLoc::PvBaseUrl => "API base URL for /v1/chat/completions",
            FieldLoc::PvApiKey => "Bearer token or API key for this provider",
            FieldLoc::PvVirtualKey => "Optional virtual-key header (x-bf-vk) for gateway routing",
            FieldLoc::PvPrimaryModel => "Default model used when this provider is active",
            FieldLoc::PvTimeoutSecs => "HTTP request timeout in seconds for LLM calls",
            FieldLoc::PvAddProvider => "Add a new inference provider from presets or custom endpoint",
            FieldLoc::PvRemoveProvider => "Remove this provider from the configuration",
            FieldLoc::PvSelectProvider => "Select which provider to edit in this category",
            // Subconscious
            FieldLoc::ScN1Enabled => "Allow the subconscious agent to respond after every reply",
            FieldLoc::ScN1Trigger => "Condition that activates an N+1 subconscious response",
            FieldLoc::ScN1Every => "Respond after every N user messages",
            FieldLoc::ScN1Secs => "Minimum seconds between N+1 subconscious responses",
            FieldLoc::ScInboxEnabled => "Allow the subconscious agent to process its inbox",
            FieldLoc::ScModel => "LLM model used by the subconscious agent",
            FieldLoc::ScMaxTokens => "Maximum token budget for a single subconscious response",
            FieldLoc::ScSystemPrompt => "System prompt used by the subconscious agent",
            // Reflection
            FieldLoc::RfEnabled => "Enable periodic reflection over recent conversation",
            FieldLoc::RfMessageInterval => "Reflect every N user messages",
            FieldLoc::RfTrigger => "Condition that triggers a reflection pass",
            FieldLoc::RfModel => "LLM model used for reflection summaries",
            // Archivist
            FieldLoc::ArEnabled => "Enable the archivist agent for long-term memory curation",
            FieldLoc::ArInterval => "Archive every N messages",
            FieldLoc::ArThreshold => "Relevance score threshold for archiving a memory",
            FieldLoc::ArCompressionModel => "LLM model used to compress and summarize memories",
            // Subagent
            FieldLoc::SaEnabled => "Allow the agent to spawn subagent tasks",
            FieldLoc::SaMaxConcurrent => "Maximum number of subagents running in parallel",
            FieldLoc::SaTimeout => "Seconds before a subagent task is cancelled",
            FieldLoc::SaMaxDepth => "Maximum nesting depth for recursive subagent spawns",
            FieldLoc::SaMaxToolRounds => "Maximum tool-call rounds per subagent turn",
            FieldLoc::SaWarning1Threshold => "Subagent cost fraction that triggers an early warning",
            FieldLoc::SaWarning2Threshold => "Subagent cost fraction that triggers a critical warning",
            FieldLoc::SaInterRoundDelayMs => "Milliseconds to wait between subagent tool-call rounds",
            // Memory
            FieldLoc::MeGitEnabled => "Track memory files with git version control",
            FieldLoc::MeAutoCommit => "Automatically git-commit after memory changes",
            FieldLoc::MeAutoPush => "Automatically git-push after committing",
            FieldLoc::MeBasePath => "Root directory for memory storage",
            // WebSocket
            FieldLoc::WsEnabled => "Enable the WebSocket server for real-time events",
            FieldLoc::WsPort => "Port the WebSocket server listens on",
            // Sensorium
            FieldLoc::SePrimaryBandwidth => "Maximum bandwidth for the primary sensorium channel",
            FieldLoc::SeLowUrgencyOnly => "Restrict sensorium to low-urgency signals only",
            FieldLoc::SeMinimalPresenceMode => "Reduce sensorium output to minimal presence data",
            // Compaction
            FieldLoc::CpEnabled => "Enable automatic conversation compaction when context is full",
            FieldLoc::CpStrategy => "Algorithm used to select and compress old messages",
            FieldLoc::CpWarnPressure => "Context usage fraction (0.0\u{2013}1.0) that triggers a warning",
            FieldLoc::CpUrgentPressure => "Context usage fraction that triggers urgent compaction",
            FieldLoc::CpCriticalPressure => "Context usage fraction that forces immediate compaction",
            FieldLoc::CpModel => "LLM model used to generate conversation summaries",
            // Server
            FieldLoc::SvBind => "Network address the HTTP server binds to",
            FieldLoc::SvPort => "Port the HTTP server listens on",
            FieldLoc::SvUrl => "Public-facing URL advertised to clients",
            FieldLoc::SvAuthRequired => "Require authentication for incoming requests",
            FieldLoc::SvAuthLoopback => "Allow loopback connections to bypass authentication",
            // Schedules
            FieldLoc::ScdEnabled => "Enable scheduled and cron-like tasks",
            // Events
            FieldLoc::EvEnabled => "Enable the event logging subsystem",
            FieldLoc::EvRetainDays => "Number of days to retain event log entries",
            // Federation
            FieldLoc::FdEnabled => "Join the federation network for multi-instance coordination",
            FieldLoc::FdRole => "Role of this instance within the federation",
            FieldLoc::FdInstanceLabel => "Human-readable label for this federation instance",
            FieldLoc::FdAutoWake => "Automatically wake this instance when summoned by the federation",
            // Presence
            FieldLoc::PrPulseEnabled => "Send periodic presence pulse signals",
            FieldLoc::PrPulseIntervalSecs => "Seconds between presence pulse signals",
            FieldLoc::PrOutfit => "Current visual outfit or avatar name",
            FieldLoc::PrAtmosphere => "Ambient mood or atmosphere descriptor",
            // Voice
            FieldLoc::VcEnabled => "Enable voice input and output",
            FieldLoc::VcSttUrl => "Speech-to-text service endpoint URL",
            FieldLoc::VcTtsUrl => "Text-to-speech service endpoint URL",
            FieldLoc::VcVoiceId => "Identifier for the selected TTS voice",
            FieldLoc::VcPushToTalkKey => "Keyboard key used for push-to-talk activation",
            // TUI
            FieldLoc::TuShowInterstitial => "Show narration panels between conversation turns",
            FieldLoc::TuCennoThreshold => "Word count that triggers a cenno (brief summary) display",
            // Image
            FieldLoc::ImMaxWidth => "Maximum image width in pixels after resize",
            FieldLoc::ImMaxHeight => "Maximum image height in pixels after resize",
            FieldLoc::ImMaxPixels => "Maximum total pixel count (width x height) for images",
            FieldLoc::ImMaxBytes => "Maximum file size in bytes for processed images",
            FieldLoc::ImJpegQuality => "JPEG compression quality (0\u{2013}100) for output images",
        }
    }

    /// Whether a change to this field reaches the running system immediately.
    /// On save the whole config is written to disk and to the shared config
    /// lock — but most subsystems captured their settings at startup and only
    /// re-read on restart. These few are wired to apply live.
    pub fn applies_live(&self) -> bool {
        matches!(
            self,
            FieldLoc::PrAtmosphere
                | FieldLoc::PrOutfit
                | FieldLoc::PvPrimaryModel
                | FieldLoc::AgModel
        )
    }
}

// ── Editable value ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EditableValue {
    Bool(bool),
    Float(f32),
    Uint(u64),
    Int(i64),
    Text(String),
    Secret(String),
    OptionalText(Option<String>),
    /// Enumerable variants with runtime-discovered names.
    EnumVariant {
        index: usize,
        variants: Vec<String>,
    },
    /// 2D point (x, y) — rendered as a PointPicker grid.
    Point2D {
        x: u64,
        y: u64,
    },
}

impl EditableValue {
    /// Render the value as styled spans for browse mode.
    pub fn display_spans(&self, is_selected: bool, palette: &ChatPalette) -> Vec<Span<'static>> {
        let active_style = if is_selected {
            Style::default()
                .fg(palette.agent_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        match self {
            EditableValue::Bool(true) => {
                vec![Span::styled(
                    "true",
                    active_style.fg(Color::Rgb(120, 200, 120)),
                )]
            }
            EditableValue::Bool(false) => {
                vec![Span::styled("false", active_style.fg(palette.agent_dim))]
            }
            EditableValue::Float(v) => {
                vec![Span::styled(format!("{v}"), active_style)]
            }
            EditableValue::Uint(v) => {
                vec![Span::styled(format!("{v}"), active_style)]
            }
            EditableValue::Int(v) => {
                vec![Span::styled(format!("{v}"), active_style)]
            }
            EditableValue::Text(v) => {
                vec![Span::styled(v.clone(), active_style)]
            }
            EditableValue::Secret(_) => {
                vec![Span::styled(
                    "********",
                    Style::default().fg(palette.agent_dim),
                )]
            }
            EditableValue::OptionalText(None) => {
                vec![Span::styled(
                    "(none)",
                    Style::default()
                        .fg(palette.agent_dim)
                        .add_modifier(Modifier::ITALIC),
                )]
            }
            EditableValue::OptionalText(Some(v)) => {
                vec![Span::styled(v.clone(), active_style)]
            }
            EditableValue::EnumVariant { index, variants } => {
                let label = variants.get(*index).map(|s| s.as_str()).unwrap_or("?");
                vec![
                    Span::styled(" [<] ", Style::default().fg(palette.agent_dim)),
                    Span::styled(
                        label.to_string(),
                        Style::default()
                            .fg(palette.agent_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" [>] ", Style::default().fg(palette.agent_dim)),
                ]
            }
            EditableValue::Point2D { x, y } => {
                vec![Span::styled(format!(" ({x}, {y})"), active_style)]
            }
        }
    }
}

// ── Interaction mode ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SettingsMode {
    /// Navigating categories and fields.
    Browse,
    /// Editing a specific field's value.
    Editing {
        loc: FieldLoc,
        buffer: String,
        cursor: usize,
    },
    /// Unsaved changes — confirm save/discard.
    ConfirmDiscard,
    /// Transient status message.
    Status { msg: String, is_error: bool },
}
