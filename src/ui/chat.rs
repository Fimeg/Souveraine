//! Wired chat screen — bubbles, streaming, surfacing.
//!
//! The state owns:
//! - A `Box<dyn Backend>` constructed at App startup (typically `LocalBackend`).
//! - A turn-events channel (`mpsc::Receiver<BackendEvent>`) populated by the
//!   currently-running send task; `None` when idle.
//! - A scrollable history of [`ChatMessage`]s.
//!
//! Visual model — jcode rounded-box pattern:
//! - User messages: right-aligned blue bubble.
//! - Assistant messages: left-aligned orange bubble; partial message
//!   appends streaming tokens live.
//! - Surfacing items: centered yellow bubble with `[surfacing]` header
//!   (Constitution Article II.2).

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use futures::StreamExt;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::backend::{Backend, BackendEvent};
use crate::bridge::bifrost::BifrostClient;
use crate::core::config::ConsciousnessConfig;
use crate::ui::markdown;

const SURFACING_YELLOW: Color = Color::Rgb(220, 190, 100);
const USER_BLUE: Color = Color::Rgb(120, 170, 240);
const ANI_ORANGE: Color = Color::Rgb(255, 140, 66);
const ANI_DIM: Color = Color::Rgb(180, 120, 80);
const STATUS_GRAY: Color = Color::Rgb(140, 140, 140);
const REFLECTION_LAVENDER: Color = Color::Rgb(180, 160, 220);
const ARCHIVIST_TEAL: Color = Color::Rgb(120, 190, 180);
const COMPACTION_AMBER: Color = Color::Rgb(240, 180, 60);
const COMPACTION_RED: Color = Color::Rgb(220, 90, 80);
const STRAIN_CRIMSON: Color = Color::Rgb(200, 80, 100);

// ─── Cockpit entry ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CockpitKind {
    Surfacing,
    Reflection,
    Archivist,
    CompactionWarn,
    CompactionUrgent,
    CompactionCritical,
    InferenceStrain,
}

#[derive(Debug, Clone)]
pub struct CockpitEntry {
    pub kind: CockpitKind,
    pub text: String,
}

impl CockpitEntry {
    fn prefix(&self) -> &'static str {
        match self.kind {
            CockpitKind::Surfacing => "◈",
            CockpitKind::Reflection => "◉",
            CockpitKind::Archivist => "◆",
            CockpitKind::CompactionWarn => "▲",
            CockpitKind::CompactionUrgent => "▲▲",
            CockpitKind::CompactionCritical => "▲▲▲",
            CockpitKind::InferenceStrain => "⚡",
        }
    }

    fn color(&self) -> Color {
        match self.kind {
            CockpitKind::Surfacing => SURFACING_YELLOW,
            CockpitKind::Reflection => REFLECTION_LAVENDER,
            CockpitKind::Archivist => ARCHIVIST_TEAL,
            CockpitKind::CompactionWarn => COMPACTION_AMBER,
            CockpitKind::CompactionUrgent => ANI_ORANGE,
            CockpitKind::CompactionCritical => COMPACTION_RED,
            CockpitKind::InferenceStrain => STRAIN_CRIMSON,
        }
    }
}

// ─── Overlay ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    SlashComplete {
        selected: usize,
        matches: Vec<&'static SlashDef>,
    },
    ConversationPicker {
        selected: usize,
        conversations: Vec<crate::backend::ConversationInfo>,
    },
}

#[derive(Debug, Clone)]
pub struct SlashDef {
    pub name: &'static str,
    pub hint: &'static str,
}

const SLASH_COMMANDS: &[SlashDef] = &[
    SlashDef { name: "/help",   hint: "Show this help" },
    SlashDef { name: "/clear",  hint: "Clear chat history" },
    SlashDef { name: "/new",    hint: "New conversation" },
    SlashDef { name: "/resume", hint: "List / switch conversations" },
    SlashDef { name: "/convos", hint: "Alias for /resume" },
    SlashDef { name: "/model",  hint: "List or set model" },
];

#[derive(Debug, Clone)]
pub enum ChatMessage {
    User { text: String, ts: Instant },
    Assistant { text: String, ts: Instant, streaming: bool },
    Surfacing { source: String, content: String, priority: String, ts: Instant },
    System { text: String, ts: Instant },
}

pub struct ChatState {
    pub backend: Arc<dyn Backend>,
    pub mode: String,
    pub agent_name: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub scroll: u16,
    pub turn_rx: Option<mpsc::Receiver<BackendEvent>>,
    pub busy: bool,
    pub pressure: f32,
    pub overlay: Overlay,
    /// Cockpit pane visible (Tab toggles).
    pub cockpit: bool,
    /// Recent thinking/reasoning lines for the cockpit pane.
    pub thinking: Vec<String>,
    /// Recent subconscious surfacings + reflections for the cockpit pane.
    pub cockpit_log: Vec<CockpitEntry>,
    /// Monotonic tick counter for animation timings.
    pub tick: u64,
    /// When the current turn started (for spinner animation).
    pub turn_started: Option<Instant>,
    /// Receiver for `/model` listing results from async Bifrost call.
    pub model_rx: Option<oneshot::Receiver<String>>,
    /// Consciousness events (surfacing, reflection, archivist) since last drain.
    /// Forwarded to the Scene by App after each tick.
    pub pending_consciousness: Vec<BackendEvent>,
    /// Pending /new conversation result.
    pub new_conv_rx: Option<oneshot::Receiver<Result<String>>>,
    /// Pending /resume conversation list result.
    pub convos_rx: Option<oneshot::Receiver<Result<Vec<crate::backend::ConversationInfo>>>>,
    /// Pending conversation switch result (conv_id, messages).
    pub switch_rx: Option<oneshot::Receiver<Result<(String, Vec<crate::core::session::ConversationMessage>)>>>,
}

impl ChatState {
    pub async fn connect(
        config: Arc<RwLock<ConsciousnessConfig>>,
        agent_name_pref: &str,
    ) -> Result<Self> {
        // Pick the backend: try remote first, fall back to local. Mirrors
        // `main::resolve_backend` but adapted for the TUI (no quiet/json flags).
        let cfg = config.read().await;
        let url = cfg.server.effective_url();
        drop(cfg);

        let remote = crate::backend::RemoteBackend::new(&url);
        let (backend, mode): (Arc<dyn Backend>, &'static str) = if remote.health().await {
            (Arc::new(remote), "remote")
        } else {
            let cfg = config.read().await.clone();
            let local = crate::backend::LocalBackend::new(cfg).await?;
            (Arc::new(local), "local")
        };

        let agents = backend.list_agents().await?;
        let agent = agents
            .iter()
            .find(|a| a.name == agent_name_pref || a.id == agent_name_pref)
            .or_else(|| agents.first())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no agents available"))?;

        let conversation_id = backend.ensure_conversation(&agent.id).await?;

        Ok(Self {
            backend,
            mode: mode.to_string(),
            agent_name: agent.name,
            agent_id: agent.id,
            conversation_id,
            messages: vec![ChatMessage::System {
                text: "Souveraine ready. Type to begin.".to_string(),
                ts: Instant::now(),
            }],
            input: String::new(),
            scroll: 0,
            turn_rx: None,
            busy: false,
            pressure: 0.0,
            overlay: Overlay::None,
            cockpit: false,
            thinking: Vec::new(),
            cockpit_log: Vec::new(),
            tick: 0,
            turn_started: None,
            model_rx: None,
            pending_consciousness: Vec::new(),
            new_conv_rx: None,
            convos_rx: None,
            switch_rx: None,
        })
    }

    const HELP_TEXT: &'static str = "Available commands:
  /help              Show this help
  /clear             Clear chat history
  /new               Start a new conversation
  /resume            List and switch conversations
  /convos            Alias for /resume
  /model             List available models
  /model <name>      Set the active model
  !<command>         Run a shell command (Linux/macOS)

Use Tab to toggle the cockpit pane.";

    /// Submit the current input. Returns `true` if the input was handled
    /// (slash command, bang command, or sent to backend).
    pub fn submit(&mut self) -> bool {
        if self.busy || self.input.trim().is_empty() {
            return false;
        }

        let trimmed = self.input.trim().to_string();
        self.input.clear();

        // Slash commands
        if trimmed.starts_with('/') {
            return self.handle_slash_command(&trimmed);
        }

        // Bang commands: !<cmd>
        if trimmed.starts_with('!') {
            let cmd = trimmed[1..].trim();
            if !cmd.is_empty() {
                self.handle_bang_command(cmd);
            }
            return true;
        }

        // Normal chat message
        let text = trimmed;
        let ts = Instant::now();
        self.messages.push(ChatMessage::User { text: text.clone(), ts });
        self.messages.push(ChatMessage::Assistant {
            text: String::new(),
            ts,
            streaming: true,
        });
        self.busy = true;
        self.turn_started = Some(Instant::now());

        let (tx, rx) = mpsc::channel::<BackendEvent>(64);
        self.turn_rx = Some(rx);

        let backend = self.backend.clone();
        let conv_id = self.conversation_id.clone();
        tokio::spawn(async move {
            match backend.send(&conv_id, &text).await {
                Ok(mut stream) => {
                    while let Some(ev) = stream.next().await {
                        match ev {
                            Ok(e) => {
                                if tx.send(e).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = tx
                                    .send(BackendEvent::Token(format!("\n[error] {}\n", err)))
                                    .await;
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(BackendEvent::Token(format!("\n[connect error] {}\n", err)))
                        .await;
                }
            }
            let _ = tx.send(BackendEvent::Done).await;
        });
        true
    }

    fn handle_slash_command(&mut self, input: &str) -> bool {
        let trimmed = input.trim();

        if trimmed == "/help" {
            self.system_message(Self::HELP_TEXT.to_string());
            return true;
        }

        if trimmed == "/clear" {
            self.messages.clear();
            self.system_message("Chat cleared.".to_string());
            return true;
        }

        if trimmed == "/new" {
            self.handle_new_conversation();
            return true;
        }

        if trimmed == "/resume" || trimmed == "/convos" {
            self.handle_list_conversations();
            return true;
        }

        if trimmed.starts_with("/resume ") {
            let conv_id = trimmed.strip_prefix("/resume ").unwrap().trim();
            if !conv_id.is_empty() {
                self.handle_switch_conversation(conv_id.to_string());
            }
            return true;
        }

        if trimmed.starts_with("/model") {
            return self.handle_model_command(trimmed);
        }

        // Unknown command
        let cmd = trimmed.split_whitespace().next().unwrap_or(trimmed);
        self.system_message(format!(
            "Unknown command: {}\nType /help for available commands.",
            cmd
        ));
        true
    }

    fn handle_new_conversation(&mut self) {
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.new_conversation(&agent_id).await;
            let _ = tx.send(result);
        });

        self.system_message("Creating new conversation...".to_string());
        self.new_conv_rx = Some(rx);
    }

    fn handle_list_conversations(&mut self) {
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.list_conversations(&agent_id).await;
            let _ = tx.send(result);
        });

        self.system_message("Loading conversations...".to_string());
        self.convos_rx = Some(rx);
    }

    fn handle_switch_conversation(&mut self, conversation_id: String) {
        let backend = self.backend.clone();
        let conv_id = conversation_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.load_conversation(&conv_id).await;
            let _ = tx.send(result.map(|msgs| (conv_id, msgs)));
        });

        self.system_message(format!("Switching to {}...", conversation_id));
        self.switch_rx = Some(rx);
    }

    fn handle_model_command(&mut self, input: &str) -> bool {
        let rest = input.strip_prefix("/model").unwrap_or("").trim();

        // /model <name> — set model (synchronous, fast)
        if !rest.is_empty() && !rest.starts_with('-') {
            let model_name = rest.to_string();
            let cfg_path = std::env::current_dir()
                .map(|d| d.join("souveraine.toml"))
                .unwrap_or_else(|_| std::path::PathBuf::from("souveraine.toml"));

            match ConsciousnessConfig::load(&cfg_path) {
                Ok(mut cfg) => {
                    cfg.bifrost.primary_model = model_name.clone();
                    match cfg.save(&cfg_path) {
                        Ok(()) => self.system_message(format!("Set model to: {}", model_name)),
                        Err(e) => self.error_message(format!("Failed to save config: {}", e)),
                    }
                }
                Err(e) => self.error_message(format!("Failed to load config: {}", e)),
            }
            return true;
        }

        // /model — list models (async, uses oneshot to get result back)
        self.system_message("Fetching models from Bifrost…".to_string());

        let cfg_path = std::env::current_dir()
            .map(|d| d.join("souveraine.toml"))
            .unwrap_or_else(|_| std::path::PathBuf::from("souveraine.toml"));

        let (tx, rx) = oneshot::channel();
        self.model_rx = Some(rx);

        tokio::spawn(async move {
            let result = match ConsciousnessConfig::load(&cfg_path) {
                Ok(cfg) => {
                    let bifrost = BifrostClient::new(
                        &cfg.bifrost.base_url,
                        &cfg.bifrost.api_key,
                        &cfg.bifrost.virtual_key,
                        &cfg.bifrost.primary_model,
                    );
                    let bifrost_models = bifrost.list_models().await.unwrap_or_default();
                    let mut all_models = bifrost_models.clone();
                    for name in cfg.models.keys() {
                        if !all_models.contains(name) {
                            all_models.push(name.clone());
                        }
                    }
                    let mut text = format!(
                        "Selected: {}\nAvailable ({}):\n",
                        cfg.bifrost.primary_model,
                        all_models.len()
                    );
                    for m in &all_models {
                        let marker = if bifrost_models.contains(&m) { "⚡" } else { "⚙" };
                        text.push_str(&format!("  {} {}\n", marker, m));
                    }
                    text
                }
                Err(e) => format!("✕ Failed to load config: {}", e),
            };
            let _ = tx.send(result);
        });

        true
    }

    fn handle_bang_command(&mut self, cmd: &str) {
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(stdout.trim());
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(stderr.trim());
                }
                if result.is_empty() {
                    result = format!("[exit code {}]", out.status.code().unwrap_or(-1));
                }
                self.system_message(format!("$ {}\n{}", cmd, result));
            }
            Err(e) => {
                self.error_message(format!("Shell command failed: {}", e));
            }
        }
    }

    /// Push a system message for display (from slash commands, etc.)
    pub fn system_message(&mut self, text: String) {
        self.messages.push(ChatMessage::System {
            text,
            ts: Instant::now(),
        });
    }

    /// Push an error message for display
    pub fn error_message(&mut self, text: String) {
        self.messages.push(ChatMessage::System {
            text: format!("✕ {}", text),
            ts: Instant::now(),
        });
    }

    /// Drain pending events from the active turn channel (non-blocking).
    /// Call once per UI tick.
    pub fn drain_events(&mut self) {
        // Check for /model listing result
        if let Some(rx) = self.model_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                self.messages.push(ChatMessage::System {
                    text: result,
                    ts: Instant::now(),
                });
                self.model_rx = None;
            }
        }

        // Check for /new conversation result
        if let Some(rx) = self.new_conv_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(conv_id) => {
                        self.conversation_id = conv_id.clone();
                        self.messages.clear();
                        self.system_message(format!(
                            "New conversation started: {}",
                            &conv_id[..8.min(conv_id.len())]
                        ));
                    }
                    Err(e) => {
                        self.system_message(format!("Failed to create conversation: {}", e));
                    }
                }
                self.new_conv_rx = None;
            }
        }

        // Check for /resume conversation list result → show picker overlay
        if let Some(rx) = self.convos_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(convos) => {
                        if convos.is_empty() {
                            self.system_message("No saved conversations.".to_string());
                        } else {
                            self.overlay = Overlay::ConversationPicker {
                                selected: 0,
                                conversations: convos,
                            };
                        }
                    }
                    Err(e) => {
                        self.system_message(format!("Failed to list conversations: {}", e));
                    }
                }
                self.convos_rx = None;
            }
        }

        // Check for conversation switch result
        if let Some(rx) = self.switch_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok((conv_id, messages)) => {
                        self.conversation_id = conv_id.clone();
                        self.messages.clear();
                        // Backfill from persisted messages
                        for msg in &messages {
                            let text = msg.blocks.iter().filter_map(|b| match b {
                                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            }).collect::<Vec<_>>().join("\n");
                            if text.is_empty() { continue; }
                            match msg.role {
                                crate::core::session::MessageRole::User => {
                                    self.messages.push(ChatMessage::User { text, ts: Instant::now() });
                                }
                                crate::core::session::MessageRole::Assistant => {
                                    self.messages.push(ChatMessage::Assistant { text, ts: Instant::now(), streaming: false });
                                }
                                crate::core::session::MessageRole::System => {
                                    self.messages.push(ChatMessage::System { text, ts: Instant::now() });
                                }
                                _ => {}
                            }
                        }
                        self.system_message(format!(
                            "Resumed conversation {} ({} messages)",
                            &conv_id[..8.min(conv_id.len())],
                            messages.len()
                        ));
                    }
                    Err(e) => {
                        self.system_message(format!("Failed to switch: {}", e));
                    }
                }
                self.switch_rx = None;
            }
        }

        // Two-phase to avoid double-borrowing self: drain into a Vec, then process.
        let mut drained: Vec<BackendEvent> = Vec::new();
        let mut closed = false;
        if let Some(rx) = self.turn_rx.as_mut() {
            loop {
                match rx.try_recv() {
                    Ok(ev) => drained.push(ev),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }
        } else {
            return;
        }

        for ev in drained {
            match ev {
                BackendEvent::Token(t) => self.append_streaming(&t),
                BackendEvent::Reasoning(r) => {
                    self.thinking.push(r.clone());
                    if self.thinking.len() > 200 {
                        self.thinking.drain(..self.thinking.len() - 200);
                    }
                }
                BackendEvent::Surfacing { source, content, priority } => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Surfacing,
                        text: format!("{} · {} — {}", source, priority, content),
                    });
                    if self.cockpit_log.len() > 200 {
                        self.cockpit_log.drain(..self.cockpit_log.len() - 200);
                    }
                    self.messages.push(ChatMessage::Surfacing {
                        source: source.clone(),
                        content: content.clone(),
                        priority: priority.clone(),
                        ts: Instant::now(),
                    });
                    // Buffer for scene dispatch
                    self.pending_consciousness.push(BackendEvent::Surfacing { source, content, priority });
                }
                BackendEvent::Reflection(content) => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Reflection,
                        text: content.clone(),
                    });
                    self.messages.push(ChatMessage::System {
                        text: format!("reflection: {}", content),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness.push(BackendEvent::Reflection(content));
                }
                BackendEvent::Archivist { synthesis, pressure } => {
                    self.pressure = pressure;
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Archivist,
                        text: format!("{:.0}% — {}", pressure * 100.0, synthesis),
                    });
                    self.messages.push(ChatMessage::System {
                        text: format!("archivist: {} (pressure {:.0}%)", synthesis, pressure * 100.0),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness.push(BackendEvent::Archivist { synthesis, pressure });
                }
                BackendEvent::CompactionWarning { pressure, tier } => {
                    self.pressure = pressure;
                    let label = match tier { 3 => "critical", 2 => "urgent", _ => "warn" };
                    let kind = match tier {
                        3 => CockpitKind::CompactionCritical,
                        2 => CockpitKind::CompactionUrgent,
                        _ => CockpitKind::CompactionWarn,
                    };
                    self.cockpit_log.push(CockpitEntry {
                        kind,
                        text: format!("{label} · {:.0}%", pressure * 100.0),
                    });
                    self.messages.push(ChatMessage::System {
                        text: format!("context pressure {:.0}% ({label}) — consider `memory compact`", pressure * 100.0),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness.push(BackendEvent::CompactionWarning { pressure, tier });
                }
                BackendEvent::ContextPressure(p) => {
                    self.pressure = p;
                    // Forward continuous pressure so Presence can yawn at tier 3.
                    self.pending_consciousness.push(BackendEvent::ContextPressure(p));
                }
                BackendEvent::InferenceStrain { attempt, status, model } => {
                    let text = if status == 0 {
                        format!("{} unreachable (attempt {})", model, attempt + 1)
                    } else {
                        format!("{} returned {} (attempt {})", model, status, attempt + 1)
                    };
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::InferenceStrain,
                        text,
                    });
                    // Forward to Presence — the body channel needs to feel this.
                    self.pending_consciousness.push(BackendEvent::InferenceStrain {
                        attempt, status, model: String::new(),
                    });
                }
                BackendEvent::ScheduleActive { name } => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Reflection,
                        text: format!("schedule: {}", name),
                    });
                }
                BackendEvent::ScheduleComplete { name, silent } => {
                    if !silent {
                        self.cockpit_log.push(CockpitEntry {
                            kind: CockpitKind::Reflection,
                            text: format!("schedule done: {}", name),
                        });
                    }
                }
                BackendEvent::Done => {
                    self.finalize_streaming();
                    self.busy = false;
                    self.turn_started = None;
                    self.turn_rx = None;
                    return;
                }
            }
        }

        if closed {
            self.finalize_streaming();
            self.busy = false;
            self.turn_started = None;
            self.turn_rx = None;
        }
    }

    /// Toggle the cockpit side-pane.
    pub fn toggle_cockpit(&mut self) {
        self.cockpit = !self.cockpit;
    }

    /// Update slash-command completion state based on current input.
    /// Call after each input mutation.
    pub fn update_completion(&mut self) {
        if self.busy {
            self.overlay = Overlay::None;
            return;
        }
        let trimmed = self.input.trim_start();
        if trimmed.starts_with('/') && !trimmed.contains(' ') && !trimmed.contains('\n') {
            let query = trimmed;
            let matches: Vec<&'static SlashDef> = SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.name.starts_with(query))
                .collect();
            if matches.is_empty() || (matches.len() == 1 && matches[0].name == query) {
                self.overlay = Overlay::None;
            } else {
                let selected = match &self.overlay {
                    Overlay::SlashComplete { selected, .. } => (*selected).min(matches.len().saturating_sub(1)),
                    _ => 0,
                };
                self.overlay = Overlay::SlashComplete { selected, matches };
            }
        } else if matches!(self.overlay, Overlay::SlashComplete { .. }) {
            self.overlay = Overlay::None;
        }
    }

    /// Accept the currently selected slash completion into the input.
    pub fn accept_completion(&mut self) {
        if let Overlay::SlashComplete { selected, ref matches } = self.overlay {
            if let Some(cmd) = matches.get(selected) {
                self.input = cmd.name.to_string();
            }
        }
        self.overlay = Overlay::None;
    }

    /// Accept the currently selected conversation from the picker.
    pub fn accept_conversation_pick(&mut self) {
        if let Overlay::ConversationPicker { selected, ref conversations } = self.overlay {
            if let Some(conv) = conversations.get(selected) {
                let conv_id = conv.id.clone();
                self.overlay = Overlay::None;
                self.handle_switch_conversation(conv_id);
                return;
            }
        }
        self.overlay = Overlay::None;
    }

    /// Returns true if the overlay is currently capturing input.
    pub fn overlay_active(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    /// Bump the animation tick. Called once per UI frame.
    pub fn advance_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn append_streaming(&mut self, t: &str) {
        if let Some(ChatMessage::Assistant { text, streaming, .. }) = self.messages.last_mut() {
            if *streaming {
                text.push_str(t);
                return;
            }
        }
        self.messages.push(ChatMessage::Assistant {
            text: t.to_string(),
            ts: Instant::now(),
            streaming: true,
        });
    }

    fn finalize_streaming(&mut self) {
        if let Some(ChatMessage::Assistant { streaming, .. }) = self.messages.last_mut() {
            *streaming = false;
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, state: &ChatState) {
    let area = f.size();

    // Dynamic input height: grows with content, capped at 40% of terminal.
    let input_inner_width = (area.width as usize).saturating_sub(5).max(1);
    let input_visual_lines = if state.busy {
        1
    } else {
        count_visual_lines(&state.input, input_inner_width)
    };
    let max_input_lines = ((area.height as usize) * 40 / 100).max(1);
    let input_height = (input_visual_lines.min(max_input_lines) as u16) + 2; // +2 for borders

    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),            // header
            Constraint::Min(5),              // body (messages + optional cockpit)
            Constraint::Length(input_height), // input (dynamic)
            Constraint::Length(1),            // status footer
        ])
        .split(area);

    draw_header(f, state, vchunks[0]);

    if state.cockpit {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(36)])
            .split(vchunks[1]);
        draw_messages(f, state, body[0]);
        draw_cockpit(f, state, body[1]);
    } else {
        draw_messages(f, state, vchunks[1]);
    }

    draw_input(f, state, vchunks[2]);
    draw_footer(f, state, vchunks[3]);

    // Overlays render last — on top of everything.
    draw_overlay(f, state, area, vchunks[2]);
}

fn draw_header(f: &mut Frame, state: &ChatState, area: Rect) {
    let mode_color = match state.mode.as_str() {
        "local" => Color::Rgb(120, 200, 140),
        "remote" => Color::Rgb(180, 180, 220),
        _ => STATUS_GRAY,
    };
    let title = Line::from(vec![
        Span::styled("✦ Souveraine ", Style::default().fg(ANI_ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {} ", state.agent_name), Style::default().fg(Color::White)),
        Span::styled(format!("[{} mode]", state.mode), Style::default().fg(mode_color)),
    ]);
    f.render_widget(Paragraph::new(title).alignment(Alignment::Center), area);
}

fn draw_messages(f: &mut Frame, state: &ChatState, area: Rect) {
    let max_bubble = ((area.width as usize).saturating_sub(8) * 70 / 100).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in &state.messages {
        match msg {
            ChatMessage::User { text, .. } => {
                lines.extend(bubble(
                    "you",
                    text,
                    max_bubble,
                    Style::default().fg(USER_BLUE),
                    BubbleAlign::Right,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::Assistant { text, streaming, .. } => {
                let label = if *streaming { format!("{} ◦", state.agent_name) } else { state.agent_name.clone() };
                let body_lines = if text.is_empty() && *streaming {
                    vec![Line::from("…")]
                } else {
                    markdown::render(text, ANI_ORANGE)
                };
                lines.extend(bubble_rendered(
                    &label,
                    &body_lines,
                    max_bubble,
                    Style::default().fg(ANI_ORANGE),
                    BubbleAlign::Left,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::Surfacing { source, content, priority, .. } => {
                let label = format!("surfacing · {} · {}", source, priority);
                lines.extend(bubble(
                    &label,
                    content,
                    max_bubble.min(60),
                    Style::default().fg(SURFACING_YELLOW),
                    BubbleAlign::Center,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::System { text, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  · {}", text),
                    Style::default().fg(STATUS_GRAY).add_modifier(Modifier::ITALIC),
                )));
                lines.push(Line::from(""));
            }
        }
    }

    // Auto-scroll to bottom unless the user has manually scrolled up.
    let total = lines.len() as u16;
    let view = area.height.saturating_sub(2);
    let scroll = total.saturating_sub(view).saturating_sub(state.scroll);

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(ANI_DIM))
                .border_type(BorderType::Plain),
        );
    f.render_widget(para, area);
}

#[derive(Clone, Copy)]
enum BubbleAlign {
    Left,
    Right,
    Center,
}

/// Build a rounded-box bubble (jcode pattern). Returns a vector of styled lines.
fn bubble(
    title: &str,
    body: &str,
    max_width: usize,
    border: Style,
    align: BubbleAlign,
    container_width: u16,
) -> Vec<Line<'static>> {
    let max_inner = max_width.saturating_sub(4).max(8);
    let wrapped = wrap_words(body, max_inner);
    let widest = wrapped
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 2);
    let inner = widest.min(max_inner);
    let outer = inner + 4;

    let title_text = format!(" {} ", title);
    let dashes = outer.saturating_sub(2 + title_text.chars().count());
    let left_dash = "─".repeat(dashes / 2);
    let right_dash = "─".repeat(dashes - dashes / 2);

    let pad = match align {
        BubbleAlign::Left => 2,
        BubbleAlign::Right => (container_width as usize).saturating_sub(outer + 2),
        BubbleAlign::Center => (container_width as usize).saturating_sub(outer) / 2,
    };
    let pad_str = " ".repeat(pad);

    let mut lines = Vec::new();

    let top = format!("{}╭{}{}{}╮", pad_str, left_dash, title_text, right_dash);
    lines.push(Line::from(Span::styled(top, border)));

    for chunk in &wrapped {
        let chunk_width = chunk.chars().count();
        let inner_pad = inner.saturating_sub(chunk_width);
        let line_str = format!("{}│ {}{} │", pad_str, chunk, " ".repeat(inner_pad));
        let mut spans = Vec::new();
        spans.push(Span::raw(pad_str.clone()));
        spans.push(Span::styled("│ ", border));
        spans.push(Span::raw(chunk.clone()));
        if inner_pad > 0 {
            spans.push(Span::raw(" ".repeat(inner_pad)));
        }
        spans.push(Span::styled(" │", border));
        let _ = line_str;
        lines.push(Line::from(spans));
    }

    let bottom = format!("{}╰{}╯", pad_str, "─".repeat(outer - 2));
    lines.push(Line::from(Span::styled(bottom, border)));

    lines
}

/// Build a rounded-box bubble around pre-rendered markdown lines.
///
/// Like [`bubble`] but accepts `Vec<Line<'static>>` (from the markdown
/// renderer) instead of a plain `&str`.  Each line keeps its styled spans
/// (bold, code, headings, etc.) inside the box-drawing borders.
fn bubble_rendered(
    title: &str,
    body_lines: &[Line<'static>],
    max_width: usize,
    border: Style,
    align: BubbleAlign,
    container_width: u16,
) -> Vec<Line<'static>> {
    let max_inner = max_width.saturating_sub(4).max(8);
    let widest = body_lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 2);
    let inner = widest.min(max_inner);
    let outer = inner + 4;

    let title_text = format!(" {} ", title);
    let dashes = outer.saturating_sub(2 + title_text.chars().count());
    let left_dash = "─".repeat(dashes / 2);
    let right_dash = "─".repeat(dashes - dashes / 2);

    let pad = match align {
        BubbleAlign::Left => 2,
        BubbleAlign::Right => (container_width as usize).saturating_sub(outer + 2),
        BubbleAlign::Center => (container_width as usize).saturating_sub(outer) / 2,
    };
    let pad_str = " ".repeat(pad);

    let mut lines = Vec::new();

    let top = format!("{}╭{}{}{}╮", pad_str, left_dash, title_text, right_dash);
    lines.push(Line::from(Span::styled(top, border)));

    for line in body_lines {
        let chunk_width = line.width();
        let inner_pad = inner.saturating_sub(chunk_width);
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::raw(pad_str.clone()));
        spans.push(Span::styled("│ ", border));
        spans.extend(line.spans.iter().cloned());
        if inner_pad > 0 {
            spans.push(Span::raw(" ".repeat(inner_pad)));
        }
        spans.push(Span::styled(" │", border));
        lines.push(Line::from(spans));
    }

    let bottom = format!("{}╰{}╯", pad_str, "─".repeat(outer - 2));
    lines.push(Line::from(Span::styled(bottom, border)));

    lines
}

fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let w = word.chars().count();
            if w >= width {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                // Long word — chunk it.
                let mut buf = String::new();
                for ch in word.chars() {
                    if buf.chars().count() + 1 > width {
                        out.push(std::mem::take(&mut buf));
                    }
                    buf.push(ch);
                }
                if !buf.is_empty() {
                    out.push(buf);
                }
                continue;
            }
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + w <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn count_visual_lines(text: &str, wrap_width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let w = wrap_width.max(1);
    let mut count = 0;
    for line in text.split('\n') {
        let chars = line.chars().count();
        if chars == 0 {
            count += 1;
        } else {
            count += (chars + w - 1) / w;
        }
    }
    count.max(1)
}

fn draw_input(f: &mut Frame, state: &ChatState, area: Rect) {
    let border_color = if state.busy {
        let phase = (state.tick as f32 / 8.0).sin().abs();
        let r = (180.0 + (255.0 - 180.0) * phase) as u8;
        let g = (120.0 + (140.0 - 120.0) * phase) as u8;
        let b = (80.0 + (66.0 - 80.0) * phase) as u8;
        Color::Rgb(r, g, b)
    } else {
        ANI_ORANGE
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    if state.busy {
        let spinner = SPINNER[(state.tick as usize / 2) % SPINNER.len()];
        let elapsed = state
            .turn_started
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let line = Line::from(vec![
            Span::styled(format!(" {} ", spinner), Style::default().fg(ANI_ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("thinking… {}s", elapsed),
                Style::default().fg(ANI_DIM).add_modifier(Modifier::ITALIC),
            ),
        ]);
        f.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    let cursor_visible = (state.tick / 5) % 2 == 0;
    let cursor_ch: &str = if cursor_visible { "▏" } else { " " };
    let inner_width = (area.width as usize).saturating_sub(5).max(1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let logical: Vec<&str> = state.input.split('\n').collect();

    for (li, logical_line) in logical.iter().enumerate() {
        let wrapped = wrap_words(logical_line, inner_width);
        for (wi, chunk) in wrapped.iter().enumerate() {
            let prefix: Span<'static> = if li == 0 && wi == 0 {
                Span::styled(" › ", Style::default().fg(ANI_ORANGE).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("   ")
            };
            let is_last = li == logical.len() - 1 && wi == wrapped.len() - 1;
            let mut spans = vec![prefix, Span::styled(chunk.clone(), Style::default().fg(Color::White))];
            if is_last {
                spans.push(Span::styled(cursor_ch.to_string(), Style::default().fg(ANI_ORANGE)));
            }
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" › ", Style::default().fg(ANI_ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(cursor_ch.to_string(), Style::default().fg(ANI_ORANGE)),
        ]));
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll = if lines.len() > visible_height {
        (lines.len() - visible_height) as u16
    } else {
        0
    };

    let para = Paragraph::new(lines).scroll((scroll, 0)).block(block);
    f.render_widget(para, area);
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn draw_overlay(f: &mut Frame, state: &ChatState, full_area: Rect, input_area: Rect) {
    match &state.overlay {
        Overlay::None => {}
        Overlay::SlashComplete { selected, matches } => {
            let count = matches.len().min(8);
            let height = count as u16 + 2; // +2 for border
            let width = 40u16.min(full_area.width.saturating_sub(4));
            let x = input_area.x + 1;
            let y = input_area.y.saturating_sub(height);
            let area = Rect { x, y, width, height };

            f.render_widget(Clear, area);

            let items: Vec<Line<'static>> = matches.iter().enumerate().take(count).map(|(i, cmd)| {
                let sel = i == *selected;
                let style = if sel {
                    Style::default().fg(Color::Rgb(255, 200, 100)).bg(Color::Rgb(60, 40, 20)).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let hint_style = if sel {
                    Style::default().fg(Color::Rgb(180, 150, 80)).bg(Color::Rgb(60, 40, 20))
                } else {
                    Style::default().fg(STATUS_GRAY)
                };
                Line::from(vec![
                    Span::styled(format!(" {} ", cmd.name), style),
                    Span::styled(format!(" {}", cmd.hint), hint_style),
                ])
            }).collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ANI_DIM));
            let para = Paragraph::new(items).block(block);
            f.render_widget(para, area);
        }
        Overlay::ConversationPicker { selected, conversations } => {
            let count = conversations.len();
            let visible = count.min(12);
            let height = visible as u16 + 4; // border + header + footer
            let width = (full_area.width * 3 / 4).max(40).min(full_area.width.saturating_sub(4));
            let x = (full_area.width.saturating_sub(width)) / 2;
            let y = (full_area.height.saturating_sub(height)) / 2;
            let area = Rect { x, y, width, height };

            f.render_widget(Clear, area);

            let inner_width = (width as usize).saturating_sub(4);
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                " Conversations — ↑↓ select · Enter switch · Esc cancel",
                Style::default().fg(STATUS_GRAY).add_modifier(Modifier::ITALIC),
            )));

            let scroll_offset = if *selected >= visible { selected + 1 - visible } else { 0 };
            for (i, conv) in conversations.iter().enumerate().skip(scroll_offset).take(visible) {
                let sel = i == *selected;
                let short_id = &conv.id[..8.min(conv.id.len())];
                let summary = conv.summary.as_deref().unwrap_or("(no summary)");
                let label = format!(
                    " {} · {} msgs · {}",
                    short_id, conv.message_count, summary,
                );
                let truncated = if label.chars().count() > inner_width {
                    let mut s: String = label.chars().take(inner_width.saturating_sub(1)).collect();
                    s.push('…');
                    s
                } else {
                    label
                };

                let style = if sel {
                    Style::default().fg(Color::Rgb(255, 200, 100)).bg(Color::Rgb(60, 40, 20)).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(truncated, style)));
            }

            let block = Block::default()
                .title(Span::styled(
                    " Resume ",
                    Style::default().fg(ANI_ORANGE).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ANI_ORANGE));
            let para = Paragraph::new(lines).block(block);
            f.render_widget(para, area);
        }
    }
}

fn draw_cockpit(f: &mut Frame, state: &ChatState, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Thinking pane
    let thinking_view = state
        .thinking
        .iter()
        .rev()
        .take(panes[0].height as usize)
        .rev()
        .map(|t| Line::from(Span::styled(format!("· {}", t), Style::default().fg(STATUS_GRAY))))
        .collect::<Vec<_>>();
    let thinking = Paragraph::new(thinking_view)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ANI_DIM))
                .title(Span::styled(" thinking ", Style::default().fg(ANI_DIM).add_modifier(Modifier::BOLD))),
        );
    f.render_widget(thinking, panes[0]);

    // Subconscious pane (surfacings, reflections, archivist)
    let visible_height = panes[1].height.saturating_sub(2) as usize;
    let visible_entries: Vec<&CockpitEntry> = state
        .cockpit_log
        .iter()
        .rev()
        .take(visible_height)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let entry_count = visible_entries.len();
    let log_view: Vec<Line<'static>> = visible_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let base = entry.color();
            let dim = if entry_count > 1 {
                let age = 1.0 - (i as f32 / (entry_count - 1) as f32);
                0.4 + 0.6 * (1.0 - age)
            } else {
                1.0
            };
            let Color::Rgb(r, g, b) = base else { unreachable!() };
            let fg = Color::Rgb(
                (r as f32 * dim) as u8,
                (g as f32 * dim) as u8,
                (b as f32 * dim) as u8,
            );
            Line::from(vec![
                Span::styled(
                    format!(" {} ", entry.prefix()),
                    Style::default().fg(fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(entry.text.clone(), Style::default().fg(fg)),
            ])
        })
        .collect();
    let subconscious = Paragraph::new(log_view)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(SURFACING_YELLOW))
                .title(Span::styled(" subconscious ", Style::default().fg(SURFACING_YELLOW).add_modifier(Modifier::BOLD))),
        );
    f.render_widget(subconscious, panes[1]);
}

fn draw_footer(f: &mut Frame, state: &ChatState, area: Rect) {
    let pressure_pct = (state.pressure * 100.0) as u16;
    let pressure_label = format!("ctx {}%", pressure_pct);
    let cockpit_hint = if state.cockpit { "Tab close cockpit" } else { "Tab cockpit" };
    let footer = Line::from(vec![
        Span::styled(
            format!(" Esc menu · Enter send · S-Ret ↵ · ↑↓ scroll · {} ", cockpit_hint),
            Style::default().fg(STATUS_GRAY),
        ),
        Span::raw("│  "),
        Span::styled(format!("conv {}", short(&state.conversation_id)), Style::default().fg(STATUS_GRAY)),
        Span::raw("  │  "),
        Span::styled(pressure_label, Style::default().fg(STATUS_GRAY)),
    ]);
    f.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

fn short(s: &str) -> String {
    if s.len() <= 8 { s.to_string() } else { s[..8].to_string() }
}
