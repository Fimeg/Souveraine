//! TUI settings editor — browse and edit `souveraine.toml` live.
//!
//! Two-panel layout: categories (left) · fields (right). Edits are made on a
//! cloned copy of the config; Ctrl+S persists to disk and propagates to the
//! running system. Esc with unsaved changes prompts for confirmation.

use std::path::Path;

use tokio::sync::oneshot;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::core::compact::CompactionStrategyKind;
use crate::core::config::{
    BandwidthClass, ConsciousnessConfig, N1Trigger, ReflectionTrigger,
};
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
        }
    }
}

// ── Field locations ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldLoc {
    // Agent
    AgSystemPrompt,
    // Bifrost
    BfBaseUrl,
    BfApiKey,
    BfVirtualKey,
    BfPrimaryModel,
    // Subconscious
    ScN1Enabled,
    ScN1Trigger,
    ScInboxEnabled,
    ScModel,
    ScMaxTokens,
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
}

impl FieldLoc {
    pub fn category(&self) -> Category {
        match self {
            FieldLoc::AgSystemPrompt => Category::Agent,
            FieldLoc::BfBaseUrl | FieldLoc::BfApiKey | FieldLoc::BfVirtualKey | FieldLoc::BfPrimaryModel => Category::Bifrost,
            FieldLoc::ScN1Enabled | FieldLoc::ScN1Trigger | FieldLoc::ScInboxEnabled | FieldLoc::ScModel | FieldLoc::ScMaxTokens => Category::Subconscious,
            FieldLoc::RfEnabled | FieldLoc::RfMessageInterval | FieldLoc::RfTrigger | FieldLoc::RfModel => Category::Reflection,
            FieldLoc::ArEnabled | FieldLoc::ArInterval | FieldLoc::ArThreshold | FieldLoc::ArCompressionModel => Category::Archivist,
            FieldLoc::SaEnabled | FieldLoc::SaMaxConcurrent | FieldLoc::SaTimeout | FieldLoc::SaMaxDepth | FieldLoc::SaMaxToolRounds
                | FieldLoc::SaWarning1Threshold | FieldLoc::SaWarning2Threshold | FieldLoc::SaInterRoundDelayMs => Category::Subagent,
            FieldLoc::MeGitEnabled | FieldLoc::MeAutoCommit | FieldLoc::MeAutoPush | FieldLoc::MeBasePath => Category::Memory,
            FieldLoc::WsEnabled | FieldLoc::WsPort => Category::WebSocket,
            FieldLoc::SePrimaryBandwidth | FieldLoc::SeLowUrgencyOnly | FieldLoc::SeMinimalPresenceMode => Category::Sensorium,
            FieldLoc::CpEnabled | FieldLoc::CpStrategy | FieldLoc::CpWarnPressure | FieldLoc::CpUrgentPressure | FieldLoc::CpCriticalPressure | FieldLoc::CpModel => Category::Compaction,
            FieldLoc::SvBind | FieldLoc::SvPort | FieldLoc::SvUrl => Category::Server,
            FieldLoc::ScdEnabled => Category::Schedules,
            FieldLoc::EvEnabled | FieldLoc::EvRetainDays => Category::Events,
            FieldLoc::FdEnabled => Category::Federation,
            FieldLoc::PrPulseEnabled | FieldLoc::PrPulseIntervalSecs | FieldLoc::PrOutfit | FieldLoc::PrAtmosphere => Category::Presence,
            FieldLoc::VcEnabled | FieldLoc::VcSttUrl | FieldLoc::VcTtsUrl | FieldLoc::VcVoiceId | FieldLoc::VcPushToTalkKey => Category::Voice,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            FieldLoc::AgSystemPrompt => "system_prompt",
            FieldLoc::BfBaseUrl => "base_url",
            FieldLoc::BfApiKey => "api_key",
            FieldLoc::BfVirtualKey => "virtual_key",
            FieldLoc::BfPrimaryModel => "primary_model",
            FieldLoc::ScN1Enabled => "n1_enabled",
            FieldLoc::ScN1Trigger => "n1_trigger",
            FieldLoc::ScInboxEnabled => "inbox_enabled",
            FieldLoc::ScModel => "model",
            FieldLoc::ScMaxTokens => "max_tokens",
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
        }
    }

    /// Returns true if this field's value should be masked in browse mode.
    pub fn is_secret(&self) -> bool {
        matches!(self, FieldLoc::BfApiKey | FieldLoc::BfVirtualKey)
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldLoc::AgSystemPrompt => "platform prompt",
            FieldLoc::BfBaseUrl => "endpoint",
            FieldLoc::BfApiKey => "API key",
            FieldLoc::BfVirtualKey => "virtual key",
            FieldLoc::BfPrimaryModel => "primary model",
            FieldLoc::ScN1Enabled => "N+1 enabled",
            FieldLoc::ScN1Trigger => "N+1 trigger",
            FieldLoc::ScInboxEnabled => "inbox",
            FieldLoc::ScModel => "model",
            FieldLoc::ScMaxTokens => "max tokens",
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
        }
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
    fn display_spans(&self, is_selected: bool, palette: &ChatPalette) -> Vec<Span<'static>> {
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

// ── Main state ──────────────────────────────────────────────────────────────

pub struct SettingsView {
    /// Working copy of config — all edits go here.
    pub config: ConsciousnessConfig,
    /// Original snapshot taken at entry, for dirty detection and discard.
    pub(crate) original: ConsciousnessConfig,
    /// Whether unsaved changes exist.
    pub dirty: bool,
    /// Index into Category::all().
    pub category_idx: usize,
    /// Index into the field list for the selected category.
    pub field_idx: usize,
    /// Interaction state machine.
    pub mode: SettingsMode,
    /// Path to the active agent's `expressions/` directory, for discovering
    /// outfit subdirectories. `None` if no agent path is known.
    expressions_path: Option<std::path::PathBuf>,
    /// Which panel has keyboard focus.
    pub focus: PanelFocus,
    /// Models fetched from Bifrost + cfg.models keys. Empty until [r] is pressed.
    pub available_models: Vec<String>,
    /// Pending async fetch of available models.
    pub models_rx: Option<oneshot::Receiver<Vec<String>>>,
    /// True while a fetch is in flight.
    pub models_fetching: bool,

    /// Atmosphere-derived colour palette. Used for coloured values and accents.
    pub palette: ChatPalette,
}

impl SettingsView {
    pub fn new(config: &ConsciousnessConfig) -> Self {
        Self {
            config: config.clone(),
            original: config.clone(),
            dirty: false,
            category_idx: 0,
            field_idx: 0,
            mode: SettingsMode::Browse,
            expressions_path: None,
            focus: PanelFocus::Fields,
            available_models: Vec::new(),
            models_rx: None,
            models_fetching: false,
            palette: ChatPalette::default(),
        }
    }

    /// Called each tick. Drains the model fetch receiver if one is pending.
    pub fn poll_models_rx(&mut self) {
        if let Some(rx) = self.models_rx.as_mut() {
            if let Ok(models) = rx.try_recv() {
                self.available_models = models;
                self.models_rx = None;
                self.models_fetching = false;
            }
        }
    }

    /// Set the expressions directory path so outfit discovery can find
    /// subdirectories on disk. Called by App when initializing the view.
    pub fn set_expressions_path(&mut self, path: Option<std::path::PathBuf>) {
        self.expressions_path = path;
    }

    /// Scan the expressions directory for subdirectories (outfits).
    fn discover_outfits(&self) -> Vec<String> {
        let dir = match &self.expressions_path {
            Some(p) => p.clone(),
            None => return Vec::new(),
        };
        if !dir.is_dir() {
            return Vec::new();
        }
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        names.push(name.to_string());
                    }
                }
            }
        }
        names.sort();
        names
    }

    /// Get the list of (FieldLoc, EditableValue) pairs for a category.
    pub fn fields_for_category(&self, cat: Category) -> Vec<(FieldLoc, EditableValue)> {
        use FieldLoc::*;
        let mut out = Vec::new();

        match cat {
            Category::Agent => {
                out.push((AgSystemPrompt, EditableValue::OptionalText(self.config.agent.system_prompt.clone())));
            }
            Category::Bifrost => {
                out.push((BfBaseUrl, EditableValue::Text(self.config.bifrost.base_url.clone())));
                out.push((BfApiKey, EditableValue::Secret(self.config.bifrost.api_key.clone())));
                out.push((BfVirtualKey, EditableValue::Secret(self.config.bifrost.virtual_key.clone())));
                if self.available_models.is_empty() {
                    out.push((BfPrimaryModel, EditableValue::Text(self.config.bifrost.primary_model.clone())));
                } else {
                    let mut variants = self.available_models.clone();
                    if !variants.contains(&self.config.bifrost.primary_model) {
                        variants.insert(0, self.config.bifrost.primary_model.clone());
                    }
                    let idx = variants.iter().position(|m| m == &self.config.bifrost.primary_model).unwrap_or(0);
                    out.push((BfPrimaryModel, EditableValue::EnumVariant { index: idx, variants }));
                }
            }
            Category::Subconscious => {
                let sc = &self.config.subconscious;
                out.push((ScN1Enabled, EditableValue::Bool(sc.n1_enabled)));
                let n1_idx = match sc.n1_trigger {
                    N1Trigger::EveryResponse => 0,
                    N1Trigger::EveryNResponses(_) => 1,
                    N1Trigger::TimeBased(_) => 2,
                    N1Trigger::Manual => 3,
                };
                let n1_variants = vec!["every_response".into(), "every_n_responses".into(), "time_based".into(), "manual".into()];
                out.push((ScN1Trigger, EditableValue::EnumVariant { index: n1_idx, variants: n1_variants }));
                out.push((ScInboxEnabled, EditableValue::Bool(sc.inbox_enabled)));
                if self.available_models.is_empty() {
                    out.push((ScModel, EditableValue::OptionalText(sc.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &sc.model { if !variants.contains(m) { variants.push(m.clone()); } }
                    let idx = sc.model.as_ref().and_then(|m| variants.iter().position(|v| v == m)).unwrap_or(0);
                    out.push((ScModel, EditableValue::EnumVariant { index: idx, variants }));
                }
                let max_tokens = sc.max_tokens.map(|v| v.to_string());
                out.push((ScMaxTokens, EditableValue::OptionalText(max_tokens)));
            }
            Category::Reflection => {
                let rf = &self.config.reflection;
                out.push((RfEnabled, EditableValue::Bool(rf.enabled)));
                out.push((RfMessageInterval, EditableValue::Uint(rf.message_interval as u64)));
                let trig_idx = match rf.trigger {
                    ReflectionTrigger::Off => 0,
                    ReflectionTrigger::StepCount => 1,
                    ReflectionTrigger::CompactionEvent => 2,
                };
                let rf_variants = vec!["off".into(), "step_count".into(), "compaction_event".into()];
                out.push((RfTrigger, EditableValue::EnumVariant { index: trig_idx, variants: rf_variants }));
                if self.available_models.is_empty() {
                    out.push((RfModel, EditableValue::OptionalText(rf.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &rf.model { if !variants.contains(m) { variants.push(m.clone()); } }
                    let idx = rf.model.as_ref().and_then(|m| variants.iter().position(|v| v == m)).unwrap_or(0);
                    out.push((RfModel, EditableValue::EnumVariant { index: idx, variants }));
                }
            }
            Category::Archivist => {
                let ar = &self.config.archivist;
                out.push((ArEnabled, EditableValue::Bool(ar.enabled)));
                out.push((ArInterval, EditableValue::Uint(ar.interval as u64)));
                out.push((ArThreshold, EditableValue::Float(ar.threshold)));
                if self.available_models.is_empty() {
                    out.push((ArCompressionModel, EditableValue::Text(ar.compression_model.clone())));
                } else {
                    let mut variants = self.available_models.clone();
                    if !variants.contains(&ar.compression_model) {
                        variants.insert(0, ar.compression_model.clone());
                    }
                    let idx = variants.iter().position(|m| m == &ar.compression_model).unwrap_or(0);
                    out.push((ArCompressionModel, EditableValue::EnumVariant { index: idx, variants }));
                }
            }
            Category::Subagent => {
                let sa = &self.config.subagent;
                out.push((SaEnabled, EditableValue::Bool(sa.enabled)));
                out.push((SaMaxConcurrent, EditableValue::Uint(sa.max_concurrent as u64)));
                out.push((SaTimeout, EditableValue::Uint(sa.timeout)));
                out.push((SaMaxDepth, EditableValue::Uint(sa.max_depth as u64)));
                out.push((SaMaxToolRounds, EditableValue::Uint(sa.max_tool_rounds as u64)));
                out.push((SaWarning1Threshold, EditableValue::Float(sa.warning_1_threshold)));
                out.push((SaWarning2Threshold, EditableValue::Float(sa.warning_2_threshold)));
                out.push((SaInterRoundDelayMs, EditableValue::Uint(sa.inter_round_delay_ms)));
            }
            Category::Memory => {
                let me = &self.config.memory;
                out.push((MeGitEnabled, EditableValue::Bool(me.git_enabled)));
                out.push((MeAutoCommit, EditableValue::Bool(me.auto_commit)));
                out.push((MeAutoPush, EditableValue::Bool(me.auto_push)));
                let base = me.base_path.as_ref().map(|p| p.to_string_lossy().to_string());
                out.push((MeBasePath, EditableValue::OptionalText(base)));
            }
            Category::WebSocket => {
                let ws = &self.config.websocket;
                out.push((WsEnabled, EditableValue::Bool(ws.enabled)));
                out.push((WsPort, EditableValue::Uint(ws.port as u64)));
            }
            Category::Sensorium => {
                let se = &self.config.sensorium;
                let bw_idx = match se.primary_bandwidth {
                    BandwidthClass::High => 0,
                    BandwidthClass::Medium => 1,
                    BandwidthClass::Low => 2,
                    BandwidthClass::Minimal => 3,
                };
                let bw_variants = vec!["high".into(), "medium".into(), "low".into(), "minimal".into()];
                out.push((SePrimaryBandwidth, EditableValue::EnumVariant { index: bw_idx, variants: bw_variants }));
                out.push((SeLowUrgencyOnly, EditableValue::Bool(se.discovery.low_urgency_only)));
                out.push((SeMinimalPresenceMode, EditableValue::Text(se.discovery.minimal_presence_mode.clone())));
            }
            Category::Compaction => {
                let cp = &self.config.compaction;
                out.push((CpEnabled, EditableValue::Bool(cp.enabled)));
                let strat_idx = match cp.strategy {
                    CompactionStrategyKind::Microcompact => 0,
                    CompactionStrategyKind::SlidingWindow => 1,
                    CompactionStrategyKind::SlidingReflect => 2,
                    CompactionStrategyKind::Summary => 3,
                    CompactionStrategyKind::Cull => 4,
                };
                let strat_variants = vec!["microcompact".into(), "sliding_window".into(), "sliding_reflect".into(), "summary".into(), "cull".into()];
                out.push((CpStrategy, EditableValue::EnumVariant { index: strat_idx, variants: strat_variants }));
                out.push((CpWarnPressure, EditableValue::Float(cp.warn_pressure)));
                out.push((CpUrgentPressure, EditableValue::Float(cp.urgent_pressure)));
                out.push((CpCriticalPressure, EditableValue::Float(cp.critical_pressure)));
                if self.available_models.is_empty() {
                    out.push((CpModel, EditableValue::OptionalText(cp.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &cp.model { if !variants.contains(m) { variants.push(m.clone()); } }
                    let idx = cp.model.as_ref().and_then(|m| variants.iter().position(|v| v == m)).unwrap_or(0);
                    out.push((CpModel, EditableValue::EnumVariant { index: idx, variants }));
                }
            }
            Category::Server => {
                let sv = &self.config.server;
                out.push((SvBind, EditableValue::Text(sv.bind.clone())));
                out.push((SvPort, EditableValue::Uint(sv.port as u64)));
                out.push((SvUrl, EditableValue::Text(sv.url.clone())));
            }
            Category::Schedules => {
                out.push((ScdEnabled, EditableValue::Bool(self.config.schedules.enabled)));
            }
            Category::Events => {
                let ev = &self.config.events;
                out.push((EvEnabled, EditableValue::Bool(ev.enabled)));
                out.push((EvRetainDays, EditableValue::Int(ev.retain_days)));
            }
            Category::Federation => {
                out.push((FdEnabled, EditableValue::Bool(self.config.federation.enabled)));
            }
            Category::Presence => {
                let pr = &self.config.presence;
                out.push((PrPulseEnabled, EditableValue::Bool(pr.pulse_enabled)));
                out.push((PrPulseIntervalSecs, EditableValue::Uint(pr.pulse_interval_secs)));
                // Outfit: discovered from expressions/ subdirectories.
                let mut outfit_variants = vec!["default".to_string()];
                outfit_variants.extend(self.discover_outfits());
                let outfit_idx = pr.outfit.as_ref()
                    .and_then(|o| outfit_variants.iter().position(|v| v == o))
                    .unwrap_or(0);
                out.push((PrOutfit, EditableValue::EnumVariant { index: outfit_idx, variants: outfit_variants }));
                // Atmosphere: enum over all known presets plus "default".
                let atm_variants: Vec<String> = vec![
                    "default".into(), "mint_tea".into(), "therapeutic_blue".into(),
                    "lavender_calm".into(), "warm_amber".into(), "peach_sunset".into(),
                    "autumn_browns".into(), "neon_glow".into(), "aurora_borealis".into(),
                    "cherry_blossom".into(), "ocean_depths".into(), "midnight_galaxy".into(),
                    "twilight_mist".into(), "forest_greens".into(),
                ];
                let atm_idx = pr.atmosphere.as_ref()
                    .and_then(|a| atm_variants.iter().position(|v| v == a))
                    .unwrap_or(0);
                out.push((PrAtmosphere, EditableValue::EnumVariant { index: atm_idx, variants: atm_variants }));
            }
            Category::Voice => {
                let vc = &self.config.voice;
                out.push((VcEnabled, EditableValue::Bool(vc.enabled)));
                out.push((VcSttUrl, EditableValue::Text(vc.stt_url.clone())));
                out.push((VcTtsUrl, EditableValue::Text(vc.tts_url.clone())));
                out.push((VcVoiceId, EditableValue::Text(vc.voice_id.clone())));
                out.push((VcPushToTalkKey, EditableValue::Text(vc.push_to_talk_key.clone())));
            }
        }
        out
    }

    /// Apply an edited value back into the config. Marks dirty.
    pub fn apply_field(&mut self, loc: FieldLoc, value: EditableValue) {
        use FieldLoc::*;
        match loc {
            AgSystemPrompt => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.agent.system_prompt = v;
                }
            }
            BfBaseUrl => { if let EditableValue::Text(v) = value { self.config.bifrost.base_url = v; } }
            BfApiKey => { if let EditableValue::Text(v) = value { self.config.bifrost.api_key = v; } }
            BfVirtualKey => { if let EditableValue::Text(v) = value { self.config.bifrost.virtual_key = v; } }
            BfPrimaryModel => {
                match value {
                    EditableValue::Text(v) => self.config.bifrost.primary_model = v,
                    EditableValue::EnumVariant { index, variants } => {
                        if let Some(m) = variants.get(index) { self.config.bifrost.primary_model = m.clone(); }
                    }
                    _ => {}
                }
            }

            ScN1Enabled => { if let EditableValue::Bool(v) = value { self.config.subconscious.n1_enabled = v; } }
            ScN1Trigger => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.subconscious.n1_trigger = match index {
                        1 => {
                            let cur = match &self.config.subconscious.n1_trigger {
                                N1Trigger::EveryNResponses(n) => *n,
                                _ => 5,
                            };
                            N1Trigger::EveryNResponses(cur)
                        }
                        2 => {
                            let cur = match &self.config.subconscious.n1_trigger {
                                N1Trigger::TimeBased(s) => *s,
                                _ => 3600,
                            };
                            N1Trigger::TimeBased(cur)
                        }
                        3 => N1Trigger::Manual,
                        _ => N1Trigger::EveryResponse,
                    };
                }
            }
            ScInboxEnabled => { if let EditableValue::Bool(v) = value { self.config.subconscious.inbox_enabled = v; } }
            ScModel => {
                match value {
                    EditableValue::OptionalText(v) => self.config.subconscious.model = v,
                    EditableValue::EnumVariant { index, variants } => {
                        self.config.subconscious.model = if index == 0 { None } else { variants.get(index).cloned() };
                    }
                    _ => {}
                }
            }
            ScMaxTokens => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.subconscious.max_tokens = v.and_then(|s| s.parse().ok());
                }
            }

            RfEnabled => { if let EditableValue::Bool(v) = value { self.config.reflection.enabled = v; } }
            RfMessageInterval => { if let EditableValue::Uint(v) = value { self.config.reflection.message_interval = v as usize; } }
            RfModel => {
                match value {
                    EditableValue::OptionalText(v) => self.config.reflection.model = v,
                    EditableValue::EnumVariant { index, variants } => {
                        self.config.reflection.model = if index == 0 { None } else { variants.get(index).cloned() };
                    }
                    _ => {}
                }
            }
            RfTrigger => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.reflection.trigger = match index {
                        0 => ReflectionTrigger::Off,
                        2 => ReflectionTrigger::CompactionEvent,
                        _ => ReflectionTrigger::StepCount,
                    };
                }
            }

            ArEnabled => { if let EditableValue::Bool(v) = value { self.config.archivist.enabled = v; } }
            ArInterval => { if let EditableValue::Uint(v) = value { self.config.archivist.interval = v as usize; } }
            ArThreshold => { if let EditableValue::Float(v) = value { self.config.archivist.threshold = v; } }
            ArCompressionModel => {
                match value {
                    EditableValue::Text(v) => self.config.archivist.compression_model = v,
                    EditableValue::EnumVariant { index, variants } => {
                        if let Some(m) = variants.get(index) { self.config.archivist.compression_model = m.clone(); }
                    }
                    _ => {}
                }
            }

            SaEnabled => { if let EditableValue::Bool(v) = value { self.config.subagent.enabled = v; } }
            SaMaxConcurrent => { if let EditableValue::Uint(v) = value { self.config.subagent.max_concurrent = v as usize; } }
            SaTimeout => { if let EditableValue::Uint(v) = value { self.config.subagent.timeout = v; } }
            SaMaxDepth => { if let EditableValue::Uint(v) = value { self.config.subagent.max_depth = v as u32; } }
            SaMaxToolRounds => { if let EditableValue::Uint(v) = value { self.config.subagent.max_tool_rounds = v as u32; } }
            SaWarning1Threshold => { if let EditableValue::Float(v) = value { self.config.subagent.warning_1_threshold = v; } }
            SaWarning2Threshold => { if let EditableValue::Float(v) = value { self.config.subagent.warning_2_threshold = v; } }
            SaInterRoundDelayMs => { if let EditableValue::Uint(v) = value { self.config.subagent.inter_round_delay_ms = v; } }

            MeGitEnabled => { if let EditableValue::Bool(v) = value { self.config.memory.git_enabled = v; } }
            MeAutoCommit => { if let EditableValue::Bool(v) = value { self.config.memory.auto_commit = v; } }
            MeAutoPush => { if let EditableValue::Bool(v) = value { self.config.memory.auto_push = v; } }
            MeBasePath => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.memory.base_path = v.map(std::path::PathBuf::from);
                }
            }

            WsEnabled => { if let EditableValue::Bool(v) = value { self.config.websocket.enabled = v; } }
            WsPort => { if let EditableValue::Uint(v) = value { self.config.websocket.port = v as u16; } }

            SePrimaryBandwidth => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.sensorium.primary_bandwidth = match index {
                        1 => BandwidthClass::Medium,
                        2 => BandwidthClass::Low,
                        3 => BandwidthClass::Minimal,
                        _ => BandwidthClass::High,
                    };
                }
            }
            SeLowUrgencyOnly => { if let EditableValue::Bool(v) = value { self.config.sensorium.discovery.low_urgency_only = v; } }
            SeMinimalPresenceMode => { if let EditableValue::Text(v) = value { self.config.sensorium.discovery.minimal_presence_mode = v; } }

            CpEnabled => { if let EditableValue::Bool(v) = value { self.config.compaction.enabled = v; } }
            CpStrategy => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.compaction.strategy = match index {
                        0 => CompactionStrategyKind::Microcompact,
                        1 => CompactionStrategyKind::SlidingWindow,
                        2 => CompactionStrategyKind::SlidingReflect,
                        3 => CompactionStrategyKind::Summary,
                        _ => CompactionStrategyKind::Cull,
                    };
                }
            }
            CpWarnPressure => { if let EditableValue::Float(v) = value { self.config.compaction.warn_pressure = v; } }
            CpUrgentPressure => { if let EditableValue::Float(v) = value { self.config.compaction.urgent_pressure = v; } }
            CpCriticalPressure => { if let EditableValue::Float(v) = value { self.config.compaction.critical_pressure = v; } }
            CpModel => {
                match value {
                    EditableValue::OptionalText(v) => self.config.compaction.model = v,
                    EditableValue::EnumVariant { index, variants } => {
                        self.config.compaction.model = if index == 0 { None } else { variants.get(index).cloned() };
                    }
                    _ => {}
                }
            }

            SvBind => { if let EditableValue::Text(v) = value { self.config.server.bind = v; } }
            SvPort => { if let EditableValue::Uint(v) = value { self.config.server.port = v as u16; } }
            SvUrl => { if let EditableValue::Text(v) = value { self.config.server.url = v; } }

            ScdEnabled => { if let EditableValue::Bool(v) = value { self.config.schedules.enabled = v; } }

            EvEnabled => { if let EditableValue::Bool(v) = value { self.config.events.enabled = v; } }
            EvRetainDays => { if let EditableValue::Int(v) = value { self.config.events.retain_days = v; } }

            FdEnabled => { if let EditableValue::Bool(v) = value { self.config.federation.enabled = v; } }

            PrPulseEnabled => { if let EditableValue::Bool(v) = value { self.config.presence.pulse_enabled = v; } }
            PrPulseIntervalSecs => { if let EditableValue::Uint(v) = value { self.config.presence.pulse_interval_secs = v; } }
            PrOutfit => {
                if let EditableValue::EnumVariant { index, variants } = value {
                    self.config.presence.outfit = if index == 0 || index >= variants.len() {
                        None // "default" = None
                    } else {
                        Some(variants[index].clone())
                    };
                }
            }
            PrAtmosphere => {
                if let EditableValue::EnumVariant { index, variants } = value {
                    self.config.presence.atmosphere = if index == 0 || index >= variants.len() {
                        None // "default" = None
                    } else {
                        Some(variants[index].clone())
                    };
                }
            }

            VcEnabled => { if let EditableValue::Bool(v) = value { self.config.voice.enabled = v; } }
            VcSttUrl => { if let EditableValue::Text(v) = value { self.config.voice.stt_url = v; } }
            VcTtsUrl => { if let EditableValue::Text(v) = value { self.config.voice.tts_url = v; } }
            VcVoiceId => { if let EditableValue::Text(v) = value { self.config.voice.voice_id = v; } }
            VcPushToTalkKey => { if let EditableValue::Text(v) = value { self.config.voice.push_to_talk_key = v; } }
        }
        self.dirty = true;
    }

    /// Write config to disk and reset dirty state.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        self.config.save(&path.to_path_buf()).map_err(|e| e.to_string())?;
        self.original = self.config.clone();
        self.dirty = false;
        Ok(())
    }

    /// Discard changes — revert to original.
    pub fn discard(&mut self) {
        self.config = self.original.clone();
        self.dirty = false;
    }

    /// Replace the working copy and original snapshot with a fresh config.
    /// Called when the live config may have changed externally.
    pub fn refresh(&mut self, config: &ConsciousnessConfig) {
        self.config = config.clone();
        self.original = config.clone();
        self.dirty = false;
    }

    /// Get the selected field loc, if any.
    pub fn selected_field(&self) -> Option<FieldLoc> {
        let fields = self.fields_for_category(self.selected_category());
        fields.get(self.field_idx).map(|(loc, _)| *loc)
    }

    pub fn selected_category(&self) -> Category {
        Category::all()[self.category_idx]
    }

    fn clear_status(&mut self) {
        if matches!(self.mode, SettingsMode::Status { .. }) {
            self.mode = SettingsMode::Browse;
        }
    }

    fn is_status(&self) -> bool {
        matches!(self.mode, SettingsMode::Status { .. })
    }

    fn is_confirm_discard(&self) -> bool {
        matches!(self.mode, SettingsMode::ConfirmDiscard)
    }
}

// ── Key handling ────────────────────────────────────────────────────────────

impl SettingsView {
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<SettingsAction> {
        // Status messages absorb any key press.
        if self.is_status() {
            self.clear_status();
            return None;
        }

        // Confirm discard dialog.
        if self.is_confirm_discard() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.discard();
                    return Some(SettingsAction::GoBack);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.mode = SettingsMode::Browse;
                }
                _ => {}
            }
            return None;
        }

        match &self.mode.clone() {
            SettingsMode::Browse => self.handle_browse_key(key),
            SettingsMode::Editing { loc, buffer, cursor } => {
                self.handle_edit_key(key, *loc, buffer.clone(), *cursor)
            }
            _ => None,
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Option<SettingsAction> {
        let categories = Category::all();
        let fields = self.fields_for_category(self.selected_category());

        // Global keys work regardless of focus.
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.dirty {
                    self.mode = SettingsMode::ConfirmDiscard;
                } else {
                    return Some(SettingsAction::GoBack);
                }
                return None;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                return Some(SettingsAction::Save);
            }
            KeyCode::Char('S') | KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(SettingsAction::SaveAndGoBack);
            }
            _ => {}
        }

        // r: fetch models (only in Bifrost category, any focus).
        if key.code == KeyCode::Char('r')
            && matches!(self.selected_category(), Category::Bifrost)
            && !self.models_fetching
        {
            self.models_fetching = true;
            return Some(SettingsAction::FetchModels);
        }

        match self.focus {
            PanelFocus::Categories => match key.code {
                KeyCode::Up | KeyCode::BackTab => {
                    if self.category_idx > 0 {
                        self.category_idx -= 1;
                        self.field_idx = 0;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if self.category_idx + 1 < categories.len() {
                        self.category_idx += 1;
                        self.field_idx = 0;
                    }
                }
                KeyCode::Right | KeyCode::Enter => {
                    self.focus = PanelFocus::Fields;
                }
                _ => {}
            },
            PanelFocus::Fields => match key.code {
                KeyCode::Up | KeyCode::BackTab => {
                    if self.field_idx > 0 {
                        self.field_idx -= 1;
                    } else if !fields.is_empty() {
                        self.field_idx = fields.len() - 1;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if self.field_idx + 1 < fields.len() {
                        self.field_idx += 1;
                    } else {
                        self.field_idx = 0;
                    }
                }
                KeyCode::Left => {
                    // Cycle enum left, or retreat to category panel.
                    if let Some((loc, value)) = fields.get(self.field_idx) {
                        if let EditableValue::EnumVariant { index, variants } = value {
                            let prev = if *index == 0 { variants.len() - 1 } else { index - 1 };
                            self.apply_field(*loc, EditableValue::EnumVariant { index: prev, variants: variants.clone() });
                            return self.maybe_atmosphere_preview(*loc);
                        }
                    }
                    self.focus = PanelFocus::Categories;
                }
                KeyCode::Right => {
                    // Cycle enum right.
                    if let Some((loc, value)) = fields.get(self.field_idx) {
                        if let EditableValue::EnumVariant { index, variants } = value {
                            let next = (index + 1) % variants.len();
                            self.apply_field(*loc, EditableValue::EnumVariant { index: next, variants: variants.clone() });
                            return self.maybe_atmosphere_preview(*loc);
                        }
                    }
                }
                KeyCode::Enter => {
                    if let Some((loc, value)) = fields.get(self.field_idx) {
                        match value {
                            EditableValue::Bool(v) => {
                                self.apply_field(*loc, EditableValue::Bool(!v));
                            }
                            EditableValue::EnumVariant { index, variants } => {
                                let next = (index + 1) % variants.len();
                                self.apply_field(*loc, EditableValue::EnumVariant { index: next, variants: variants.clone() });
                                return self.maybe_atmosphere_preview(*loc);
                            }
                            EditableValue::Secret(v) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: v.clone(), cursor: v.len() };
                            }
                            EditableValue::Text(v) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: v.clone(), cursor: v.len() };
                            }
                            EditableValue::OptionalText(Some(s)) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: s.clone(), cursor: s.len() };
                            }
                            EditableValue::OptionalText(None) => {
                                self.apply_field(*loc, EditableValue::OptionalText(Some(String::new())));
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: String::new(), cursor: 0 };
                            }
                            EditableValue::Float(v) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: format!("{v}"), cursor: 0 };
                            }
                            EditableValue::Uint(v) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: format!("{v}"), cursor: 0 };
                            }
                            EditableValue::Int(v) => {
                                self.mode = SettingsMode::Editing { loc: *loc, buffer: format!("{v}"), cursor: 0 };
                            }
                        }
                    }
                }
                _ => {}
            },
        }
        None
    }

    /// If `loc` is the atmosphere field, return a live-preview action.
    fn maybe_atmosphere_preview(&self, loc: FieldLoc) -> Option<SettingsAction> {
        if loc == FieldLoc::PrAtmosphere {
            let atm = self.config.presence.atmosphere.clone().unwrap_or_default();
            Some(SettingsAction::AtmospherePreview(atm))
        } else {
            None
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent, loc: FieldLoc, mut buffer: String, mut cursor: usize) -> Option<SettingsAction> {
        match key.code {
            KeyCode::Esc => {
                self.mode = SettingsMode::Browse;
            }
            KeyCode::Enter => {
                self.commit_edit(loc, &buffer);
            }
            KeyCode::Tab => {
                self.commit_edit(loc, &buffer);
                // Advance to next field.
                let fields = self.fields_for_category(self.selected_category());
                if self.field_idx + 1 < fields.len() {
                    self.field_idx += 1;
                }
            }
            KeyCode::Left => {
                if cursor > 0 { cursor -= 1; }
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::Right => {
                if cursor < buffer.len() { cursor += 1; }
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::Home => {
                cursor = 0;
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::End => {
                cursor = buffer.len();
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::Backspace => {
                if cursor > 0 {
                    let idx = cursor - 1;
                    buffer.remove(idx);
                    cursor -= 1;
                }
                // Backspace on empty Optional field → set to None
                if buffer.is_empty() && matches!(loc, FieldLoc::ScModel | FieldLoc::RfModel | FieldLoc::CpModel | FieldLoc::ScMaxTokens | FieldLoc::MeBasePath) {
                    self.apply_field(loc, EditableValue::OptionalText(None));
                    self.mode = SettingsMode::Browse;
                    return None;
                }
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::Delete => {
                if cursor < buffer.len() {
                    buffer.remove(cursor);
                }
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            KeyCode::Char(c) => {
                let is_numeric = matches!(loc,
                    FieldLoc::ScMaxTokens | FieldLoc::RfMessageInterval |
                    FieldLoc::ArInterval | FieldLoc::SaMaxConcurrent |
                    FieldLoc::SaTimeout | FieldLoc::SaMaxDepth |
                    FieldLoc::SaMaxToolRounds | FieldLoc::SaInterRoundDelayMs |
                    FieldLoc::WsPort | FieldLoc::SvPort |
                    FieldLoc::EvRetainDays | FieldLoc::PrPulseIntervalSecs
                );
                let is_float = matches!(loc,
                    FieldLoc::ArThreshold | FieldLoc::SaWarning1Threshold |
                    FieldLoc::SaWarning2Threshold | FieldLoc::CpWarnPressure |
                    FieldLoc::CpUrgentPressure | FieldLoc::CpCriticalPressure
                );

                if is_numeric {
                    if c.is_ascii_digit() {
                        buffer.insert(cursor, c);
                        cursor += 1;
                    }
                } else if is_float {
                    if c.is_ascii_digit() || c == '.' {
                        buffer.insert(cursor, c);
                        cursor += 1;
                    }
                } else {
                    buffer.insert(cursor, c);
                    cursor += 1;
                }
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
            _ => {
                self.mode = SettingsMode::Editing { loc, buffer, cursor };
            }
        }
        None
    }

    fn commit_edit(&mut self, loc: FieldLoc, buffer: &str) {
        match loc {
            FieldLoc::ScMaxTokens | FieldLoc::RfMessageInterval |
            FieldLoc::ArInterval | FieldLoc::SaMaxConcurrent |
            FieldLoc::SaTimeout | FieldLoc::SaMaxDepth |
            FieldLoc::SaMaxToolRounds | FieldLoc::SaInterRoundDelayMs |
            FieldLoc::WsPort | FieldLoc::SvPort |
            FieldLoc::PrPulseIntervalSecs => {
                if let Ok(v) = buffer.parse::<u64>() {
                    self.apply_field(loc, EditableValue::Uint(v));
                }
            }
            FieldLoc::EvRetainDays => {
                if let Ok(v) = buffer.parse::<i64>() {
                    self.apply_field(loc, EditableValue::Int(v));
                }
            }
            FieldLoc::ArThreshold | FieldLoc::SaWarning1Threshold |
            FieldLoc::SaWarning2Threshold | FieldLoc::CpWarnPressure |
            FieldLoc::CpUrgentPressure | FieldLoc::CpCriticalPressure => {
                if let Ok(v) = buffer.parse::<f32>() {
                    self.apply_field(loc, EditableValue::Float(v));
                }
            }
            FieldLoc::ScModel | FieldLoc::RfModel | FieldLoc::CpModel | FieldLoc::MeBasePath => {
                let val = if buffer.is_empty() { None } else { Some(buffer.to_string()) };
                self.apply_field(loc, EditableValue::OptionalText(val));
            }
            _ => {
                self.apply_field(loc, EditableValue::Text(buffer.to_string()));
            }
        }
        self.mode = SettingsMode::Browse;
    }
}

/// Actions the settings handler can request from the parent App.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    GoBack,
    Save,
    SaveAndGoBack,
    FetchModels,
    /// Live atmosphere preview — dispatch before saving so the UI responds immediately.
    AtmospherePreview(String),
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, view: &SettingsView) {
    let area = frame.size();
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    // Header
    draw_header(frame, vert[0], view);
    // Body: two panels
    if area.width >= 80 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(vert[1]);
        draw_category_list(frame, body[0], view);
        draw_field_panel(frame, body[1], view);
    } else {
        // Narrow terminal: full-width field panel with category switcher hint.
        draw_field_panel(frame, vert[1], view);
    }
    // Footer
    draw_footer(frame, vert[2], view);
}

fn draw_header(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let title = if view.dirty { " Settings (unsaved) " } else { " Settings " };
    let header = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(view.palette.agent_primary));
    frame.render_widget(header, area);
}

fn draw_category_list(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let categories = Category::all();
    let focused = view.focus == PanelFocus::Categories;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (i, cat) in categories.iter().enumerate() {
        let selected = i == view.category_idx;
        let (prefix, style) = if selected && focused {
            (">", Style::default().fg(view.palette.agent_primary).add_modifier(Modifier::BOLD))
        } else if selected {
            (">", Style::default().fg(view.palette.agent_primary).add_modifier(Modifier::DIM))
        } else {
            (" ", Style::default().fg(view.palette.agent_dim))
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {}  {}", prefix, cat.label()), style),
        ]));
    }

    let border_style = if focused {
        Style::default().fg(view.palette.agent_primary)
    } else {
        Style::default().fg(view.palette.agent_dim)
    };

    let body = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Categories ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style),
        );
    frame.render_widget(body, area);
}

fn draw_field_panel(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let cat = view.selected_category();
    let fields = view.fields_for_category(cat);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    let p = &view.palette;
    for (i, (loc, value)) in fields.iter().enumerate() {
        let is_selected = i == view.field_idx && view.focus == PanelFocus::Fields;
        let key_style = if is_selected {
            Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.agent_dim)
        };

        let mut spans = vec![
            Span::styled(format!("  {:25}", loc.label()), key_style),
        ];

        // If this field is being edited, show the buffer with cursor.
        if let SettingsMode::Editing { loc: edit_loc, buffer, cursor } = &view.mode {
            if *edit_loc == *loc {
                let before = &buffer[..*cursor];
                let after = &buffer[*cursor..];
                spans.push(Span::styled(
                    before.to_string(),
                    Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    '▏'.to_string(),
                    Style::default().fg(Color::White).add_modifier(Modifier::SLOW_BLINK),
                ));
                spans.push(Span::styled(
                    after.to_string(),
                    Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::from(spans));
                continue;
            }
        }

        spans.extend(value.display_spans(is_selected, &view.palette));
        lines.push(Line::from(spans));
    }

    let focused = view.focus == PanelFocus::Fields;
    let border_color = if view.dirty {
        let (r, g, b) = match p.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        Color::Rgb(r, (g as u16 + 40).min(255) as u8, b.saturating_sub(20))
    } else if focused {
        p.agent_primary
    } else {
        p.agent_dim
    };
    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(format!(" {} ", cat.label()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color)),
        );
    frame.render_widget(body, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let msg: String = match &view.mode {
        SettingsMode::Browse => {
            let base = match view.focus {
                PanelFocus::Categories => " ↑↓ navigate · → or Enter select · y save · Ctrl+S save & quit · Esc back",
                PanelFocus::Fields if area.width >= 80 => " ↑↓ field · ←→ cycle/focus · Enter edit · y save · Ctrl+S save & quit · Esc back",
                PanelFocus::Fields => " ↑↓ · ←→ cycle · Enter · y · Ctrl+S · Esc",
            };
            if matches!(view.selected_category(), Category::Bifrost) {
                if view.models_fetching {
                    format!("{base} · fetching models…")
                } else if view.available_models.is_empty() {
                    format!("{base} · [r] fetch models")
                } else {
                    format!("{base} · [r] refresh  {} models", view.available_models.len())
                }
            } else {
                base.to_string()
            }
        }
        SettingsMode::Editing { .. } => " Enter confirm · Tab confirm+next · Esc cancel".to_string(),
        SettingsMode::ConfirmDiscard => " Unsaved changes — y discard & quit · Esc cancel".to_string(),
        SettingsMode::Status { msg, is_error } => {
            if *is_error {
                return draw_error(frame, area, msg);
            } else {
                return draw_saved(frame, area, msg);
            }
        }
    };

    let footer = Paragraph::new(format!(" {msg}"))
        .style(Style::default().fg(view.palette.agent_dim));
    frame.render_widget(footer, area);
}

fn draw_saved(frame: &mut Frame, area: Rect, msg: &str) {
    let text = Paragraph::new(format!(" ✓ {msg}"))
        .style(Style::default().fg(Color::Rgb(120, 200, 120))); // green (semantic — keep)
    frame.render_widget(text, area);
}

fn draw_error(frame: &mut Frame, area: Rect, msg: &str) {
    let text = Paragraph::new(format!(" ✗ {msg}"))
        .style(Style::default().fg(Color::Rgb(220, 100, 100))); // red (semantic — keep)
    frame.render_widget(text, area);
}
