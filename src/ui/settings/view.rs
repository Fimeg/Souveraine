#![allow(dead_code)] // WIP scaffolding not yet wired
use std::path::{Path, PathBuf};

use tokio::sync::oneshot;

use crate::core::compact::CompactionStrategyKind;
use crate::core::config::{
    BandwidthClass, ConsciousnessConfig, FederationRole, N1Trigger, ReflectionTrigger,
};
use crate::ui::chat::ChatPalette;

use super::types::{Category, EditableValue, FieldLoc, PanelFocus, SettingsMode};

/// The active agent's per-agent settings, loaded into the view so the Agent
/// category edits *this agent* — not substrate-wide config. `None` when
/// Settings is opened with no resolvable agent / backend (the per-agent
/// fields are then simply not shown).
#[derive(Debug, Clone, Default)]
pub struct ActiveAgentSettings {
    /// Agent id — the save target.
    pub id: String,
    /// Display name, so the field shows which agent is being edited.
    pub name: String,
    /// Editable working copy of the agent's llm model handle.
    pub model: String,
    /// The model as loaded — for save-time diffing.
    pub model_original: String,
    /// Per-agent provider override (`_souveraine.provider`). None = use global default.
    pub provider: Option<String>,
    /// Display-only: the paired subconscious agent id (always `{id}-sub`).
    pub subconscious_id: String,
    /// Display-only: the primary's memory directory path.
    pub memory_root: PathBuf,
    /// Display-only: the subconscious agent's memory directory path.
    pub subconscious_root: PathBuf,
    /// Display-only: whether the subconscious directory + memory.git exist.
    pub has_subconscious: bool,
    /// Display-only: whether the agent's own agent.json is readable.
    pub agent_json_exists: bool,
}

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

    /// The agent the Settings screen is editing per-agent fields for. Set by
    /// App on entry from the active agent, so Settings follows agent switches.
    pub active_agent: Option<ActiveAgentSettings>,

    /// Which provider is selected for editing in the Providers category.
    pub selected_provider_name: Option<String>,
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
            focus: PanelFocus::Categories,
            available_models: Vec::new(),
            models_rx: None,
            models_fetching: false,
            palette: ChatPalette::default(),
            active_agent: None,
            selected_provider_name: None,
        }
    }

    /// Load the active agent's per-agent settings into the view. Called by App
    /// on Settings entry so the Agent category edits the agent we're working
    /// as — and follows when the user switches agents. `None` clears it.
    pub fn set_active_agent(&mut self, agent: Option<ActiveAgentSettings>) {
        self.active_agent = agent;
    }

    /// Ensure a provider is selected when entering the Providers category.
    pub fn ensure_provider_selected(&mut self) {
        if self.selected_provider_name.is_none()
            || !self
                .config
                .providers
                .contains_key(self.selected_provider_name.as_ref().unwrap())
        {
            self.selected_provider_name = self.config.providers.keys().next().cloned();
        }
    }

    /// Whether the active agent's model was changed and needs pushing back to
    /// the agent record on save.
    pub fn agent_model_dirty(&self) -> bool {
        self.active_agent
            .as_ref()
            .map(|a| a.model != a.model_original)
            .unwrap_or(false)
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
                out.push((
                    AgSystemPrompt,
                    EditableValue::OptionalText(self.config.agent.system_prompt.clone()),
                ));
                if let Some(agent) = &self.active_agent {
                    if self.available_models.is_empty() {
                        out.push((AgModel, EditableValue::Text(agent.model.clone())));
                    } else {
                        let mut variants = self.available_models.clone();
                        if !agent.model.is_empty() && !variants.contains(&agent.model) {
                            variants.insert(0, agent.model.clone());
                        }
                        let idx = variants.iter().position(|m| m == &agent.model).unwrap_or(0);
                        out.push((
                            AgModel,
                            EditableValue::EnumVariant {
                                index: idx,
                                variants,
                            },
                        ));
                    }
                    // Per-agent provider selector
                    let mut pnames: Vec<String> = self.config.providers.keys().cloned().collect();
                    pnames.sort();
                    let mut pvariants = vec!["(default)".to_string()];
                    pvariants.extend(pnames);
                    let cur_provider = agent.provider.clone();
                    let pidx = cur_provider
                        .as_ref()
                        .and_then(|p| pvariants.iter().position(|v| v == p))
                        .unwrap_or(0);
                    out.push((
                        AgProvider,
                        EditableValue::EnumVariant {
                            index: pidx,
                            variants: pvariants,
                        },
                    ));
                    // Read-only diagnostics
                    out.push((AgAgentId, EditableValue::Text(agent.id.clone())));
                    out.push((
                        AgSubconsciousId,
                        EditableValue::Text(agent.subconscious_id.clone()),
                    ));
                    out.push((
                        AgMemoryPath,
                        EditableValue::Text(agent.memory_root.to_string_lossy().to_string()),
                    ));
                    out.push((
                        AgSubconsciousPath,
                        EditableValue::Text(agent.subconscious_root.to_string_lossy().to_string()),
                    ));
                    out.push((
                        AgSubconsciousStatus,
                        EditableValue::Bool(agent.has_subconscious),
                    ));
                }
            }
            Category::Inference => {
                let provider = &self.config.inference.provider;
                let mut variants: Vec<String> = self.config.providers.keys().cloned().collect();
                variants.sort();
                let mut all = variants.clone();
                if !all.contains(provider) {
                    all.insert(0, provider.clone());
                }
                let idx = all.iter().position(|p| p == provider).unwrap_or(0);
                out.push((
                    IfProvider,
                    EditableValue::EnumVariant {
                        index: idx,
                        variants: all,
                    },
                ));
            }
            Category::Providers => {
                // Provider selector — choose which provider to edit
                let mut pnames: Vec<String> = self.config.providers.keys().cloned().collect();
                pnames.sort();
                if pnames.is_empty() {
                    out.push((
                        PvSelectProvider,
                        EditableValue::Text("(no providers)".to_string()),
                    ));
                } else {
                    let selected = self
                        .selected_provider_name
                        .clone()
                        .filter(|n| self.config.providers.contains_key(n))
                        .unwrap_or_else(|| pnames.first().cloned().unwrap_or_default());
                    let mut variants = pnames.clone();
                    if !variants.contains(&selected) {
                        variants.insert(0, selected.clone());
                    }
                    let idx = variants.iter().position(|n| *n == selected).unwrap_or(0);
                    out.push((
                        PvSelectProvider,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants,
                        },
                    ));
                }

                // Fields for the selected provider
                if let Some(name) = &self.selected_provider_name.clone() {
                    if let Some(p) = self.config.providers.get(name) {
                        out.push((PvName, EditableValue::Text(name.clone())));
                        let type_variants = vec!["openai-compatible".into(), "openai-oauth".into()];
                        let type_idx = if p.provider_type == "openai-oauth" {
                            1
                        } else {
                            0
                        };
                        out.push((
                            PvType,
                            EditableValue::EnumVariant {
                                index: type_idx,
                                variants: type_variants,
                            },
                        ));
                        out.push((PvBaseUrl, EditableValue::Text(p.base_url.clone())));
                        out.push((
                            PvApiKey,
                            if p.api_key.is_empty() {
                                EditableValue::Text(String::new())
                            } else {
                                EditableValue::Secret(p.api_key.clone())
                            },
                        ));
                        out.push((
                            PvVirtualKey,
                            if p.virtual_key.is_empty() {
                                EditableValue::Text(String::new())
                            } else {
                                EditableValue::Secret(p.virtual_key.clone())
                            },
                        ));
                        if self.available_models.is_empty() {
                            out.push((
                                PvPrimaryModel,
                                EditableValue::Text(p.primary_model.clone()),
                            ));
                        } else {
                            let mut mvariants = self.available_models.clone();
                            let pm = &p.primary_model;
                            if !pm.is_empty() && !mvariants.contains(pm) {
                                mvariants.insert(0, pm.clone());
                            }
                            let midx = mvariants.iter().position(|m| m == pm).unwrap_or(0);
                            out.push((
                                PvPrimaryModel,
                                EditableValue::EnumVariant {
                                    index: midx,
                                    variants: mvariants,
                                },
                            ));
                        }
                        out.push((PvTimeoutSecs, EditableValue::Uint(p.timeout_secs)));
                        out.push((PvRemoveProvider, EditableValue::Bool(false)));
                    }
                }
                out.push((PvAddProvider, EditableValue::Bool(false)));
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
                let n1_variants = vec![
                    "every_response".into(),
                    "every_n_responses".into(),
                    "time_based".into(),
                    "manual".into(),
                ];
                out.push((
                    ScN1Trigger,
                    EditableValue::EnumVariant {
                        index: n1_idx,
                        variants: n1_variants,
                    },
                ));
                match sc.n1_trigger {
                    N1Trigger::EveryNResponses(n) => {
                        out.push((ScN1Every, EditableValue::Uint(n as u64)));
                    }
                    N1Trigger::TimeBased(s) => {
                        out.push((ScN1Secs, EditableValue::Uint(s)));
                    }
                    _ => {}
                }
                out.push((ScInboxEnabled, EditableValue::Bool(sc.inbox_enabled)));
                if self.available_models.is_empty() {
                    out.push((ScModel, EditableValue::OptionalText(sc.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &sc.model {
                        if !variants.contains(m) {
                            variants.push(m.clone());
                        }
                    }
                    let idx = sc
                        .model
                        .as_ref()
                        .and_then(|m| variants.iter().position(|v| v == m))
                        .unwrap_or(0);
                    out.push((
                        ScModel,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants,
                        },
                    ));
                }
                let max_tokens = sc.max_tokens.map(|v| v.to_string());
                out.push((ScMaxTokens, EditableValue::OptionalText(max_tokens)));
                out.push((
                    ScSystemPrompt,
                    EditableValue::OptionalText(sc.system_prompt.clone()),
                ));
            }
            Category::Reflection => {
                let rf = &self.config.reflection;
                out.push((RfEnabled, EditableValue::Bool(rf.enabled)));
                out.push((
                    RfMessageInterval,
                    EditableValue::Uint(rf.message_interval as u64),
                ));
                let trig_idx = match rf.trigger {
                    ReflectionTrigger::Off => 0,
                    ReflectionTrigger::StepCount => 1,
                    ReflectionTrigger::CompactionEvent => 2,
                };
                let rf_variants =
                    vec!["off".into(), "step_count".into(), "compaction_event".into()];
                out.push((
                    RfTrigger,
                    EditableValue::EnumVariant {
                        index: trig_idx,
                        variants: rf_variants,
                    },
                ));
                if self.available_models.is_empty() {
                    out.push((RfModel, EditableValue::OptionalText(rf.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &rf.model {
                        if !variants.contains(m) {
                            variants.push(m.clone());
                        }
                    }
                    let idx = rf
                        .model
                        .as_ref()
                        .and_then(|m| variants.iter().position(|v| v == m))
                        .unwrap_or(0);
                    out.push((
                        RfModel,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants,
                        },
                    ));
                }
            }
            Category::Archivist => {
                let ar = &self.config.archivist;
                out.push((ArEnabled, EditableValue::Bool(ar.enabled)));
                out.push((ArInterval, EditableValue::Uint(ar.interval as u64)));
                out.push((ArThreshold, EditableValue::Float(ar.threshold)));
                if self.available_models.is_empty() {
                    out.push((
                        ArCompressionModel,
                        EditableValue::Text(ar.compression_model.clone()),
                    ));
                } else {
                    let mut variants = self.available_models.clone();
                    if !variants.contains(&ar.compression_model) {
                        variants.insert(0, ar.compression_model.clone());
                    }
                    let idx = variants
                        .iter()
                        .position(|m| m == &ar.compression_model)
                        .unwrap_or(0);
                    out.push((
                        ArCompressionModel,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants,
                        },
                    ));
                }
            }
            Category::Subagent => {
                let sa = &self.config.subagent;
                out.push((SaEnabled, EditableValue::Bool(sa.enabled)));
                out.push((
                    SaMaxConcurrent,
                    EditableValue::Uint(sa.max_concurrent as u64),
                ));
                out.push((SaTimeout, EditableValue::Uint(sa.timeout)));
                out.push((SaMaxDepth, EditableValue::Uint(sa.max_depth as u64)));
                out.push((
                    SaMaxToolRounds,
                    EditableValue::Uint(sa.max_tool_rounds as u64),
                ));
                out.push((
                    SaWarning1Threshold,
                    EditableValue::Float(sa.warning_1_threshold),
                ));
                out.push((
                    SaWarning2Threshold,
                    EditableValue::Float(sa.warning_2_threshold),
                ));
                out.push((
                    SaInterRoundDelayMs,
                    EditableValue::Uint(sa.inter_round_delay_ms),
                ));
            }
            Category::Memory => {
                let me = &self.config.memory;
                out.push((MeGitEnabled, EditableValue::Bool(me.git_enabled)));
                out.push((MeAutoCommit, EditableValue::Bool(me.auto_commit)));
                out.push((MeAutoPush, EditableValue::Bool(me.auto_push)));
                let base = me
                    .base_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
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
                let bw_variants = vec![
                    "high".into(),
                    "medium".into(),
                    "low".into(),
                    "minimal".into(),
                ];
                out.push((
                    SePrimaryBandwidth,
                    EditableValue::EnumVariant {
                        index: bw_idx,
                        variants: bw_variants,
                    },
                ));
                out.push((
                    SeLowUrgencyOnly,
                    EditableValue::Bool(se.discovery.low_urgency_only),
                ));
                out.push((
                    SeMinimalPresenceMode,
                    EditableValue::Text(se.discovery.minimal_presence_mode.clone()),
                ));
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
                let strat_variants = vec![
                    "microcompact".into(),
                    "sliding_window".into(),
                    "sliding_reflect".into(),
                    "summary".into(),
                    "cull".into(),
                ];
                out.push((
                    CpStrategy,
                    EditableValue::EnumVariant {
                        index: strat_idx,
                        variants: strat_variants,
                    },
                ));
                out.push((CpWarnPressure, EditableValue::Float(cp.warn_pressure)));
                out.push((CpUrgentPressure, EditableValue::Float(cp.urgent_pressure)));
                out.push((
                    CpCriticalPressure,
                    EditableValue::Float(cp.critical_pressure),
                ));
                if self.available_models.is_empty() {
                    out.push((CpModel, EditableValue::OptionalText(cp.model.clone())));
                } else {
                    let mut variants = vec!["(none)".to_string()];
                    variants.extend(self.available_models.iter().cloned());
                    if let Some(m) = &cp.model {
                        if !variants.contains(m) {
                            variants.push(m.clone());
                        }
                    }
                    let idx = cp
                        .model
                        .as_ref()
                        .and_then(|m| variants.iter().position(|v| v == m))
                        .unwrap_or(0);
                    out.push((
                        CpModel,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants,
                        },
                    ));
                }
            }
            Category::Server => {
                let sv = &self.config.server;
                out.push((SvBind, EditableValue::Text(sv.bind.clone())));
                out.push((SvPort, EditableValue::Uint(sv.port as u64)));
                out.push((SvUrl, EditableValue::Text(sv.url.clone())));
                out.push((SvAuthRequired, EditableValue::Bool(sv.auth.required)));
                out.push((SvAuthLoopback, EditableValue::Bool(sv.auth.allow_loopback)));
            }
            Category::Schedules => {
                out.push((
                    ScdEnabled,
                    EditableValue::Bool(self.config.schedules.enabled),
                ));
            }
            Category::Events => {
                let ev = &self.config.events;
                out.push((EvEnabled, EditableValue::Bool(ev.enabled)));
                out.push((EvRetainDays, EditableValue::Int(ev.retain_days)));
            }
            Category::Federation => {
                let fd = &self.config.federation;
                out.push((FdEnabled, EditableValue::Bool(fd.enabled)));
                let role_idx = match fd.role {
                    FederationRole::Hearth => 0,
                    FederationRole::Limb => 1,
                };
                out.push((
                    FdRole,
                    EditableValue::EnumVariant {
                        index: role_idx,
                        variants: vec!["hearth".into(), "limb".into()],
                    },
                ));
                out.push((
                    FdInstanceLabel,
                    EditableValue::OptionalText(fd.instance_label.clone()),
                ));
                out.push((FdAutoWake, EditableValue::Bool(fd.auto_wake)));
            }
            Category::Presence => {
                let pr = &self.config.presence;
                out.push((PrPulseEnabled, EditableValue::Bool(pr.pulse_enabled)));
                out.push((
                    PrPulseIntervalSecs,
                    EditableValue::Uint(pr.pulse_interval_secs),
                ));
                let mut outfit_variants = vec!["default".to_string()];
                outfit_variants.extend(self.discover_outfits());
                let outfit_idx = pr
                    .outfit
                    .as_ref()
                    .and_then(|o| outfit_variants.iter().position(|v| v == o))
                    .unwrap_or(0);
                out.push((
                    PrOutfit,
                    EditableValue::EnumVariant {
                        index: outfit_idx,
                        variants: outfit_variants,
                    },
                ));
                let atm_variants: Vec<String> = vec![
                    "default".into(),
                    "mint_tea".into(),
                    "therapeutic_blue".into(),
                    "lavender_calm".into(),
                    "warm_amber".into(),
                    "peach_sunset".into(),
                    "autumn_browns".into(),
                    "neon_glow".into(),
                    "aurora_borealis".into(),
                    "cherry_blossom".into(),
                    "ocean_depths".into(),
                    "midnight_galaxy".into(),
                    "twilight_mist".into(),
                    "forest_greens".into(),
                ];
                let atm_idx = pr
                    .atmosphere
                    .as_ref()
                    .and_then(|a| atm_variants.iter().position(|v| v == a))
                    .unwrap_or(0);
                out.push((
                    PrAtmosphere,
                    EditableValue::EnumVariant {
                        index: atm_idx,
                        variants: atm_variants,
                    },
                ));
            }
            Category::Voice => {
                let vc = &self.config.voice;
                out.push((VcEnabled, EditableValue::Bool(vc.enabled)));
                out.push((VcSttUrl, EditableValue::Text(vc.stt_url.clone())));
                out.push((VcTtsUrl, EditableValue::Text(vc.tts_url.clone())));
                out.push((VcVoiceId, EditableValue::Text(vc.voice_id.clone())));
                out.push((
                    VcPushToTalkKey,
                    EditableValue::Text(vc.push_to_talk_key.clone()),
                ));
            }
            Category::Tui => {
                out.push((
                    TuShowInterstitial,
                    EditableValue::Bool(self.config.tui.show_interstitial),
                ));
                out.push((
                    TuCennoThreshold,
                    EditableValue::Uint(self.config.tui.cenno_word_threshold as u64),
                ));
            }
            Category::Image => {
                let img = &self.config.image;
                out.push((ImMaxWidth, EditableValue::Uint(img.max_width as u64)));
                out.push((ImMaxHeight, EditableValue::Uint(img.max_height as u64)));
                out.push((ImMaxPixels, EditableValue::Uint(img.max_pixels as u64)));
                out.push((ImMaxBytes, EditableValue::Uint(img.max_bytes as u64)));
                out.push((ImJpegQuality, EditableValue::Uint(img.jpeg_quality as u64)));
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
            AgModel => {
                if let Some(agent) = &mut self.active_agent {
                    match value {
                        EditableValue::Text(v) => agent.model = v,
                        EditableValue::EnumVariant { index, variants } => {
                            if let Some(m) = variants.get(index) {
                                agent.model = m.clone();
                            }
                        }
                        _ => {}
                    }
                }
            }
            IfProvider => {
                if let EditableValue::EnumVariant { index, variants } = value {
                    if let Some(p) = variants.get(index) {
                        self.config.inference.provider = p.clone();
                    }
                }
            }
            AgProvider => {
                if let Some(agent) = &mut self.active_agent {
                    if let EditableValue::EnumVariant { index, variants } = &value {
                        let selected = variants.get(*index).cloned().unwrap_or_default();
                        agent.provider = if selected == "(default)" {
                            None
                        } else {
                            Some(selected)
                        };
                    }
                }
            }
            PvName => {
                if let EditableValue::Text(new_name) = &value {
                    if let Some(old_name) = self.selected_provider_name.clone() {
                        if new_name != &old_name && !new_name.is_empty() {
                            if let Some(p) = self.config.providers.remove(&old_name) {
                                self.config.providers.insert(new_name.clone(), p);
                                self.selected_provider_name = Some(new_name.clone());
                            }
                        }
                    }
                }
            }
            PvType => {
                if let EditableValue::EnumVariant { index, variants } = &value {
                    if let Some(name) = &self.selected_provider_name.clone() {
                        if let Some(p) = self.config.providers.get_mut(name) {
                            let ptype = variants
                                .get(*index)
                                .map(|s| s.as_str())
                                .unwrap_or("openai-compatible");
                            p.provider_type = ptype.to_string();
                        }
                    }
                }
            }
            PvBaseUrl => {
                if let EditableValue::Text(v) = value {
                    if let Some(name) = &self.selected_provider_name.clone() {
                        if let Some(p) = self.config.providers.get_mut(name) {
                            p.base_url = v;
                        }
                    }
                }
            }
            PvApiKey => {
                if let EditableValue::Text(v) = value {
                    if let Some(name) = &self.selected_provider_name.clone() {
                        if let Some(p) = self.config.providers.get_mut(name) {
                            p.api_key = v;
                        }
                    }
                }
            }
            PvVirtualKey => {
                if let EditableValue::Text(v) = value {
                    if let Some(name) = &self.selected_provider_name.clone() {
                        if let Some(p) = self.config.providers.get_mut(name) {
                            p.virtual_key = v;
                        }
                    }
                }
            }
            PvPrimaryModel => {
                if let Some(name) = &self.selected_provider_name.clone() {
                    if let Some(p) = self.config.providers.get_mut(name) {
                        match value {
                            EditableValue::Text(v) => p.primary_model = v,
                            EditableValue::EnumVariant { index, variants } => {
                                if let Some(m) = variants.get(index) {
                                    p.primary_model = m.clone();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            PvTimeoutSecs => {
                if let EditableValue::Uint(v) = value {
                    if let Some(name) = &self.selected_provider_name.clone() {
                        if let Some(p) = self.config.providers.get_mut(name) {
                            p.timeout_secs = v;
                        }
                    }
                }
            }
            PvSelectProvider => {
                if let EditableValue::EnumVariant { index, variants } = &value {
                    self.selected_provider_name = variants.get(*index).cloned();
                }
            }
            PvAddProvider => {
                // Triggered when user presses Enter on the "+" button.
                // Add a new provider with default values.
                let base = "new_provider";
                let mut name = base.to_string();
                let mut n = 1;
                while self.config.providers.contains_key(&name) {
                    name = format!("{base}_{n}");
                    n += 1;
                }
                self.config.providers.insert(
                    name.clone(),
                    crate::core::config::ProviderConfig {
                        credential_files: None,
                        provider_type: "openai-compatible".to_string(),
                        base_url: String::new(),
                        api_key: String::new(),
                        virtual_key: String::new(),
                        primary_model: String::new(),
                        timeout_secs: 120,
                        credential_file: None,
                        cc_version: None,
                        account_uuid: None,
                        device_id: None,
                        extra_metadata: None,
                    },
                );
                self.selected_provider_name = Some(name);
            }
            PvRemoveProvider => {
                if let Some(name) = self.selected_provider_name.clone() {
                    // Don't allow removing the last provider if it's the global default
                    if self.config.providers.len() > 1 || self.config.inference.provider != name {
                        self.config.providers.remove(&name);
                        // Also remove from global default if it was this one
                        if self.config.inference.provider == name {
                            self.config.inference.provider = self
                                .config
                                .providers
                                .keys()
                                .next()
                                .cloned()
                                .unwrap_or_default();
                        }
                        self.selected_provider_name = self.config.providers.keys().next().cloned();
                    }
                }
            }

            ScN1Enabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.subconscious.n1_enabled = v;
                }
            }
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
            ScN1Every => {
                if let EditableValue::Uint(v) = value {
                    self.config.subconscious.n1_trigger = N1Trigger::EveryNResponses(v as usize);
                }
            }
            ScN1Secs => {
                if let EditableValue::Uint(v) = value {
                    self.config.subconscious.n1_trigger = N1Trigger::TimeBased(v);
                }
            }
            ScInboxEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.subconscious.inbox_enabled = v;
                }
            }
            ScModel => match value {
                EditableValue::OptionalText(v) => self.config.subconscious.model = v,
                EditableValue::EnumVariant { index, variants } => {
                    self.config.subconscious.model = if index == 0 {
                        None
                    } else {
                        variants.get(index).cloned()
                    };
                }
                _ => {}
            },
            ScMaxTokens => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.subconscious.max_tokens = v.and_then(|s| s.parse().ok());
                }
            }
            ScSystemPrompt => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.subconscious.system_prompt = v.filter(|s| !s.trim().is_empty());
                }
            }

            RfEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.reflection.enabled = v;
                }
            }
            RfMessageInterval => {
                if let EditableValue::Uint(v) = value {
                    self.config.reflection.message_interval = v as usize;
                }
            }
            RfModel => match value {
                EditableValue::OptionalText(v) => self.config.reflection.model = v,
                EditableValue::EnumVariant { index, variants } => {
                    self.config.reflection.model = if index == 0 {
                        None
                    } else {
                        variants.get(index).cloned()
                    };
                }
                _ => {}
            },
            RfTrigger => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.reflection.trigger = match index {
                        0 => ReflectionTrigger::Off,
                        2 => ReflectionTrigger::CompactionEvent,
                        _ => ReflectionTrigger::StepCount,
                    };
                }
            }

            ArEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.archivist.enabled = v;
                }
            }
            ArInterval => {
                if let EditableValue::Uint(v) = value {
                    self.config.archivist.interval = v as usize;
                }
            }
            ArThreshold => {
                if let EditableValue::Float(v) = value {
                    self.config.archivist.threshold = v;
                }
            }
            ArCompressionModel => match value {
                EditableValue::Text(v) => self.config.archivist.compression_model = v,
                EditableValue::EnumVariant { index, variants } => {
                    if let Some(m) = variants.get(index) {
                        self.config.archivist.compression_model = m.clone();
                    }
                }
                _ => {}
            },

            SaEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.subagent.enabled = v;
                }
            }
            SaMaxConcurrent => {
                if let EditableValue::Uint(v) = value {
                    self.config.subagent.max_concurrent = v as usize;
                }
            }
            SaTimeout => {
                if let EditableValue::Uint(v) = value {
                    self.config.subagent.timeout = v;
                }
            }
            SaMaxDepth => {
                if let EditableValue::Uint(v) = value {
                    self.config.subagent.max_depth = v as u32;
                }
            }
            SaMaxToolRounds => {
                if let EditableValue::Uint(v) = value {
                    self.config.subagent.max_tool_rounds = v as u32;
                }
            }
            SaWarning1Threshold => {
                if let EditableValue::Float(v) = value {
                    self.config.subagent.warning_1_threshold = v;
                }
            }
            SaWarning2Threshold => {
                if let EditableValue::Float(v) = value {
                    self.config.subagent.warning_2_threshold = v;
                }
            }
            SaInterRoundDelayMs => {
                if let EditableValue::Uint(v) = value {
                    self.config.subagent.inter_round_delay_ms = v;
                }
            }

            MeGitEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.memory.git_enabled = v;
                }
            }
            MeAutoCommit => {
                if let EditableValue::Bool(v) = value {
                    self.config.memory.auto_commit = v;
                }
            }
            MeAutoPush => {
                if let EditableValue::Bool(v) = value {
                    self.config.memory.auto_push = v;
                }
            }
            MeBasePath => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.memory.base_path = v.map(std::path::PathBuf::from);
                }
            }

            WsEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.websocket.enabled = v;
                }
            }
            WsPort => {
                if let EditableValue::Uint(v) = value {
                    self.config.websocket.port = v as u16;
                }
            }

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
            SeLowUrgencyOnly => {
                if let EditableValue::Bool(v) = value {
                    self.config.sensorium.discovery.low_urgency_only = v;
                }
            }
            SeMinimalPresenceMode => {
                if let EditableValue::Text(v) = value {
                    self.config.sensorium.discovery.minimal_presence_mode = v;
                }
            }

            CpEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.compaction.enabled = v;
                }
            }
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
            CpWarnPressure => {
                if let EditableValue::Float(v) = value {
                    self.config.compaction.warn_pressure = v;
                }
            }
            CpUrgentPressure => {
                if let EditableValue::Float(v) = value {
                    self.config.compaction.urgent_pressure = v;
                }
            }
            CpCriticalPressure => {
                if let EditableValue::Float(v) = value {
                    self.config.compaction.critical_pressure = v;
                }
            }
            CpModel => match value {
                EditableValue::OptionalText(v) => self.config.compaction.model = v,
                EditableValue::EnumVariant { index, variants } => {
                    self.config.compaction.model = if index == 0 {
                        None
                    } else {
                        variants.get(index).cloned()
                    };
                }
                _ => {}
            },

            SvBind => {
                if let EditableValue::Text(v) = value {
                    self.config.server.bind = v;
                }
            }
            SvPort => {
                if let EditableValue::Uint(v) = value {
                    self.config.server.port = v as u16;
                }
            }
            SvUrl => {
                if let EditableValue::Text(v) = value {
                    self.config.server.url = v;
                }
            }

            ScdEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.schedules.enabled = v;
                }
            }

            EvEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.events.enabled = v;
                }
            }
            EvRetainDays => {
                if let EditableValue::Int(v) = value {
                    self.config.events.retain_days = v;
                }
            }

            FdEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.federation.enabled = v;
                }
            }

            PrPulseEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.presence.pulse_enabled = v;
                }
            }
            PrPulseIntervalSecs => {
                if let EditableValue::Uint(v) = value {
                    self.config.presence.pulse_interval_secs = v;
                }
            }
            PrOutfit => {
                if let EditableValue::EnumVariant { index, variants } = value {
                    self.config.presence.outfit = if index == 0 || index >= variants.len() {
                        None
                    } else {
                        Some(variants[index].clone())
                    };
                }
            }
            PrAtmosphere => {
                if let EditableValue::EnumVariant { index, variants } = value {
                    self.config.presence.atmosphere = if index == 0 || index >= variants.len() {
                        None
                    } else {
                        Some(variants[index].clone())
                    };
                }
            }

            VcEnabled => {
                if let EditableValue::Bool(v) = value {
                    self.config.voice.enabled = v;
                }
            }
            VcSttUrl => {
                if let EditableValue::Text(v) = value {
                    self.config.voice.stt_url = v;
                }
            }
            VcTtsUrl => {
                if let EditableValue::Text(v) = value {
                    self.config.voice.tts_url = v;
                }
            }
            VcVoiceId => {
                if let EditableValue::Text(v) = value {
                    self.config.voice.voice_id = v;
                }
            }
            VcPushToTalkKey => {
                if let EditableValue::Text(v) = value {
                    self.config.voice.push_to_talk_key = v;
                }
            }

            // Handled above in the Pv* block
            SvAuthRequired => {
                if let EditableValue::Bool(v) = value {
                    self.config.server.auth.required = v;
                }
            }
            SvAuthLoopback => {
                if let EditableValue::Bool(v) = value {
                    self.config.server.auth.allow_loopback = v;
                }
            }

            FdRole => {
                if let EditableValue::EnumVariant { index, .. } = value {
                    self.config.federation.role = match index {
                        1 => FederationRole::Limb,
                        _ => FederationRole::Hearth,
                    };
                }
            }
            FdInstanceLabel => {
                if let EditableValue::OptionalText(v) = value {
                    self.config.federation.instance_label = v;
                }
            }
            FdAutoWake => {
                if let EditableValue::Bool(v) = value {
                    self.config.federation.auto_wake = v;
                }
            }

            TuShowInterstitial => {
                if let EditableValue::Bool(v) = value {
                    self.config.tui.show_interstitial = v;
                }
            }
            TuCennoThreshold => {
                if let EditableValue::Uint(v) = value {
                    self.config.tui.cenno_word_threshold = v as usize;
                }
            }

            ImMaxWidth => {
                if let EditableValue::Uint(v) = value {
                    self.config.image.max_width = v as u32;
                }
            }
            ImMaxHeight => {
                if let EditableValue::Uint(v) = value {
                    self.config.image.max_height = v as u32;
                }
            }
            ImMaxPixels => {
                if let EditableValue::Uint(v) = value {
                    self.config.image.max_pixels = v as u32;
                }
            }
            ImMaxBytes => {
                if let EditableValue::Uint(v) = value {
                    self.config.image.max_bytes = v as usize;
                }
            }
            ImJpegQuality => {
                if let EditableValue::Uint(v) = value {
                    self.config.image.jpeg_quality = v as u8;
                }
            }

            // Read-only diagnostic fields — no-op on apply.
            AgAgentId | AgSubconsciousId | AgMemoryPath | AgSubconsciousPath
            | AgSubconsciousStatus => {}
        }
        self.dirty = true;
    }

    /// Write config to disk and reset dirty state.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        self.config
            .save(&path.to_path_buf())
            .map_err(|e| e.to_string())?;
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

    pub(super) fn clear_status(&mut self) {
        if matches!(self.mode, SettingsMode::Status { .. }) {
            self.mode = SettingsMode::Browse;
        }
    }

    pub(super) fn is_status(&self) -> bool {
        matches!(self.mode, SettingsMode::Status { .. })
    }

    pub(super) fn is_confirm_discard(&self) -> bool {
        matches!(self.mode, SettingsMode::ConfirmDiscard)
    }
}
