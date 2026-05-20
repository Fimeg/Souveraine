use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::ui::chat::ChatPalette;

// ── Panel focus ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Categories,
    Fields,
}

// ── Categories ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Agent,
    Bifrost,
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
    Tui,
}

impl Category {
    pub fn all() -> &'static [Category] {
        &[
            Category::Agent,
            Category::Bifrost,
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
            Category::Tui,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::Agent => "Agent",
            Category::Bifrost => "Bifrost",
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
            Category::Tui => "TUI",
        }
    }
}

// ── Field locations ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldLoc {
    // Agent
    AgSystemPrompt,
    AgModel,
    // Bifrost
    BfBaseUrl,
    BfApiKey,
    BfVirtualKey,
    BfPrimaryModel,
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
    // Bifrost (cont.)
    BfTimeoutSecs,
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
            FieldLoc::AgSystemPrompt | FieldLoc::AgModel => Category::Agent,
            FieldLoc::BfBaseUrl | FieldLoc::BfApiKey | FieldLoc::BfVirtualKey | FieldLoc::BfPrimaryModel | FieldLoc::BfTimeoutSecs => Category::Bifrost,
            FieldLoc::ScN1Enabled | FieldLoc::ScN1Trigger | FieldLoc::ScN1Every | FieldLoc::ScN1Secs
                | FieldLoc::ScInboxEnabled | FieldLoc::ScModel | FieldLoc::ScMaxTokens
                | FieldLoc::ScSystemPrompt => Category::Subconscious,
            FieldLoc::RfEnabled | FieldLoc::RfMessageInterval | FieldLoc::RfTrigger | FieldLoc::RfModel => Category::Reflection,
            FieldLoc::ArEnabled | FieldLoc::ArInterval | FieldLoc::ArThreshold | FieldLoc::ArCompressionModel => Category::Archivist,
            FieldLoc::SaEnabled | FieldLoc::SaMaxConcurrent | FieldLoc::SaTimeout | FieldLoc::SaMaxDepth | FieldLoc::SaMaxToolRounds
                | FieldLoc::SaWarning1Threshold | FieldLoc::SaWarning2Threshold | FieldLoc::SaInterRoundDelayMs => Category::Subagent,
            FieldLoc::MeGitEnabled | FieldLoc::MeAutoCommit | FieldLoc::MeAutoPush | FieldLoc::MeBasePath => Category::Memory,
            FieldLoc::WsEnabled | FieldLoc::WsPort => Category::WebSocket,
            FieldLoc::SePrimaryBandwidth | FieldLoc::SeLowUrgencyOnly | FieldLoc::SeMinimalPresenceMode => Category::Sensorium,
            FieldLoc::CpEnabled | FieldLoc::CpStrategy | FieldLoc::CpWarnPressure | FieldLoc::CpUrgentPressure | FieldLoc::CpCriticalPressure | FieldLoc::CpModel => Category::Compaction,
            FieldLoc::SvBind | FieldLoc::SvPort | FieldLoc::SvUrl | FieldLoc::SvAuthRequired | FieldLoc::SvAuthLoopback => Category::Server,
            FieldLoc::ScdEnabled => Category::Schedules,
            FieldLoc::EvEnabled | FieldLoc::EvRetainDays => Category::Events,
            FieldLoc::FdEnabled | FieldLoc::FdRole | FieldLoc::FdInstanceLabel | FieldLoc::FdAutoWake => Category::Federation,
            FieldLoc::PrPulseEnabled | FieldLoc::PrPulseIntervalSecs | FieldLoc::PrOutfit | FieldLoc::PrAtmosphere => Category::Presence,
            FieldLoc::VcEnabled | FieldLoc::VcSttUrl | FieldLoc::VcTtsUrl | FieldLoc::VcVoiceId | FieldLoc::VcPushToTalkKey => Category::Voice,
            FieldLoc::TuShowInterstitial | FieldLoc::TuCennoThreshold => Category::Tui,
            FieldLoc::AgAgentId | FieldLoc::AgSubconsciousId | FieldLoc::AgMemoryPath
                | FieldLoc::AgSubconsciousPath | FieldLoc::AgSubconsciousStatus => Category::Agent,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            FieldLoc::AgSystemPrompt => "system_prompt",
            FieldLoc::AgModel => "model",
            FieldLoc::BfBaseUrl => "base_url",
            FieldLoc::BfApiKey => "api_key",
            FieldLoc::BfVirtualKey => "virtual_key",
            FieldLoc::BfPrimaryModel => "primary_model",
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
            FieldLoc::BfTimeoutSecs => "timeout_secs",
            FieldLoc::SvAuthRequired => "required",
            FieldLoc::SvAuthLoopback => "allow_loopback",
            FieldLoc::FdRole => "role",
            FieldLoc::FdInstanceLabel => "instance_label",
            FieldLoc::FdAutoWake => "auto_wake",
            FieldLoc::TuShowInterstitial => "show_interstitial",
            FieldLoc::TuCennoThreshold => "cenno_word_threshold",
            FieldLoc::AgAgentId => "agent_id",
            FieldLoc::AgSubconsciousId => "subconscious_id",
            FieldLoc::AgMemoryPath => "memory_path",
            FieldLoc::AgSubconsciousPath => "subconscious_path",
            FieldLoc::AgSubconsciousStatus => "subconscious_ok",
        }
    }

    /// Returns true if this field's value should be masked in browse mode.
    pub fn is_secret(&self) -> bool {
        matches!(self, FieldLoc::BfApiKey | FieldLoc::BfVirtualKey)
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
            FieldLoc::BfBaseUrl => "endpoint",
            FieldLoc::BfApiKey => "API key",
            FieldLoc::BfVirtualKey => "virtual key",
            FieldLoc::BfPrimaryModel => "new-agent default",
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
            FieldLoc::BfTimeoutSecs => "request timeout (s)",
            FieldLoc::SvAuthRequired => "require auth",
            FieldLoc::SvAuthLoopback => "allow loopback bypass",
            FieldLoc::FdRole => "role",
            FieldLoc::FdInstanceLabel => "instance label",
            FieldLoc::FdAutoWake => "auto-wake on summon",
            FieldLoc::TuShowInterstitial => "interstitial narration",
            FieldLoc::TuCennoThreshold => "cenno threshold (words)",
            FieldLoc::AgAgentId => "agent id",
            FieldLoc::AgSubconsciousId => "subconscious",
            FieldLoc::AgMemoryPath => "memory path",
            FieldLoc::AgSubconsciousPath => "subconscious path",
            FieldLoc::AgSubconsciousStatus => "subconscious on disk",
        }
    }

    /// Whether a change to this field reaches the running system immediately.
    /// On save the whole config is written to disk and to the shared config
    /// lock — but most subsystems captured their settings at startup and only
    /// re-read on restart. These few are wired to apply live.
    pub fn applies_live(&self) -> bool {
        matches!(
            self,
            FieldLoc::PrAtmosphere | FieldLoc::PrOutfit | FieldLoc::BfPrimaryModel | FieldLoc::AgModel
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
    EnumVariant { index: usize, variants: Vec<String> },
}

impl EditableValue {
    /// Render the value as styled spans for browse mode.
    pub fn display_spans(&self, is_selected: bool, palette: &ChatPalette) -> Vec<Span<'static>> {
        let active_style = if is_selected {
            Style::default().fg(palette.agent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        match self {
            EditableValue::Bool(true) => {
                vec![Span::styled("true", active_style.fg(Color::Rgb(120, 200, 120)))]
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
                vec![Span::styled("********", Style::default().fg(palette.agent_dim))]
            }
            EditableValue::OptionalText(None) => {
                vec![Span::styled("(none)", Style::default().fg(palette.agent_dim).add_modifier(Modifier::ITALIC))]
            }
            EditableValue::OptionalText(Some(v)) => {
                vec![Span::styled(v.clone(), active_style)]
            }
            EditableValue::EnumVariant { index, variants } => {
                let label = variants.get(*index).map(|s| s.as_str()).unwrap_or("?");
                vec![
                    Span::styled(" [<] ", Style::default().fg(palette.agent_dim)),
                    Span::styled(label.to_string(), Style::default().fg(palette.agent_primary).add_modifier(Modifier::BOLD)),
                    Span::styled(" [>] ", Style::default().fg(palette.agent_dim)),
                ]
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
