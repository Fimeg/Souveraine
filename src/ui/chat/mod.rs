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

mod cockpit;
mod commands;
mod events;
mod footer;
mod overlays;
mod render;
pub mod wrap;

pub use render::draw;
pub use footer::short;

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::Color,
    text::Line,
};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;

use crate::backend::{Backend, BackendEvent};
use crate::core::config::ConsciousnessConfig;
pub use crate::core::session::ImageAttachment;

#[derive(Debug, Clone)]
enum BtwForkEvent {
    Forked { id: String },
    Token(String),
    Done,
    Error(String),
}

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Copy)]
pub struct ChatPalette {
    pub agent_primary: Color,
    pub agent_dim: Color,
    pub user_accent: Color,
    pub tool_accent: Color,
    pub tool_dim: Color,
    pub surfacing: Color,
    pub reflection: Color,
    pub archivist: Color,
    pub compaction: Color,
    pub bg: Color,
}

impl ChatPalette {
    pub fn from_atmosphere(atm: crate::ui::atmosphere::Atmosphere) -> Self {
        Self::from_colors(atm.primary(), atm.secondary(), atm.dim(), atm.bg_tint())
    }

    pub fn from_colors(
        primary: Color, secondary: Color, dim: Color, bg: Color,
    ) -> Self {
        let (pr, pg, pb) = match primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let (sr, sg, sb) = match secondary { Color::Rgb(r, g, b) => (r, g, b), _ => (180, 120, 80) };
        Self {
            agent_primary: primary,
            agent_dim: dim,
            user_accent: Color::Rgb(
                (sr / 3).wrapping_add(80),
                (sg / 3).wrapping_add(100),
                (sb / 3).wrapping_add(160).min(240),
            ),
            tool_accent: Color::Rgb(
                (pr / 3).wrapping_add(80),
                (pg / 3).wrapping_add(150).min(220),
                (pb / 3).wrapping_add(160).min(230),
            ),
            tool_dim: Color::Rgb(
                (pr / 4).wrapping_add(60),
                (pg / 4).wrapping_add(100),
                (pb / 4).wrapping_add(110),
            ),
            surfacing: Color::Rgb(
                (pr / 3).wrapping_add(150).min(230),
                (pg / 3).wrapping_add(140).min(210),
                (pb / 6).wrapping_add(80),
            ),
            reflection: Color::Rgb(
                (sr / 3).wrapping_add(120),
                (sg / 4).wrapping_add(110),
                (sb / 3).wrapping_add(160).min(230),
            ),
            archivist: Color::Rgb(
                (sr / 4).wrapping_add(80),
                (sg / 3).wrapping_add(140).min(210),
                (sb / 3).wrapping_add(130).min(200),
            ),
            compaction: Color::Rgb(
                (pr / 3).wrapping_add(170).min(245),
                (pg / 3).wrapping_add(130).min(200),
                (pb / 6).wrapping_add(40),
            ),
            bg,
        }
    }

    pub fn hash(&self) -> u64 {
        let into = |c: Color| match c {
            Color::Rgb(r, g, b) => (r as u64, g as u64, b as u64),
            _ => (0, 0, 0),
        };
        let (apr, apg, apb) = into(self.agent_primary);
        let (upr, upg, upb) = into(self.user_accent);
        let (tar, tag, tab) = into(self.tool_accent);
        let (sur, sug, sub) = into(self.surfacing);
        apr.wrapping_mul(31)
            .wrapping_add(apg).wrapping_mul(37)
            .wrapping_add(apb).wrapping_mul(41)
            .wrapping_add(upr as u64).wrapping_mul(43)
            .wrapping_add(upg as u64).wrapping_mul(47)
            .wrapping_add(upb as u64).wrapping_mul(53)
            .wrapping_add(tar as u64).wrapping_mul(59)
            .wrapping_add(tag as u64).wrapping_mul(61)
            .wrapping_add(tab as u64).wrapping_mul(67)
            .wrapping_add(sur as u64).wrapping_mul(71)
            .wrapping_add(sug as u64).wrapping_mul(73)
            .wrapping_add(sub as u64).wrapping_mul(79)
    }
}

impl Default for ChatPalette {
    fn default() -> Self {
        Self::from_atmosphere(crate::ui::atmosphere::Atmosphere::Default)
    }
}


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
    pub fn prefix(&self) -> &'static str {
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

    pub fn color(&self, palette: &ChatPalette) -> Color {
        match self.kind {
            CockpitKind::Surfacing => palette.surfacing,
            CockpitKind::Reflection => palette.reflection,
            CockpitKind::Archivist => palette.archivist,
            CockpitKind::CompactionWarn => palette.compaction,
            CockpitKind::CompactionUrgent => palette.agent_primary,
            CockpitKind::CompactionCritical => palette.compaction,
            CockpitKind::InferenceStrain => palette.compaction,
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

pub const SLASH_COMMANDS: &[SlashDef] = &[
    SlashDef { name: "/help",   hint: "Show this help" },
    SlashDef { name: "/clear",  hint: "Clear chat history" },
    SlashDef { name: "/new",    hint: "New conversation" },
    SlashDef { name: "/resume", hint: "List / switch conversations" },
    SlashDef { name: "/convos", hint: "Alias for /resume" },
    SlashDef { name: "/model",  hint: "List or set model" },
    SlashDef { name: "/btw",    hint: "Interject — deliver text mid-turn" },
    SlashDef { name: "/code",   hint: "Shift to code posture (tools expanded, ≡ prompt)" },
    SlashDef { name: "/chat",   hint: "Shift to conversation posture (tools collapsed)" },
    SlashDef { name: "/outfit", hint: "Change agent appearance (outfit name)" },
    SlashDef { name: "/attach", hint: "Attach an image file" },
];

#[derive(Debug, Clone)]
pub struct MarkdownCache {
    pub text_len: usize,
    pub inner_width: usize,
    pub palette_hash: u64,
    pub lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone)]
pub enum ChatMessage {
    User { text: String, ts: Instant },
    Assistant {
        text: String,
        ts: Instant,
        streaming: bool,
        rendered_cache: RefCell<Option<MarkdownCache>>,
    },
    Surfacing { source: String, content: String, priority: String, ts: Instant },
    System { text: String, ts: Instant },
    Interjection { text: String, ts: Instant, delivered: bool },
    Interstitial { text: String, register: crate::backend::Register },
    Tool {
        id: String,
        name: String,
        arguments: String,
        round: u32,
        result: Option<ToolResultBlock>,
        ts: Instant,
        expanded: bool,
    },
    Image {
        media_type: String,
        label: String,
        data: String,
        dimensions: Option<(u32, u32)>,
        ts: Instant,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    Idle,
    Thinking,
    Tool,
    Streaming,
    Interrupted,
    Subconscious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMode {
    Conversation,
    Code,
}

#[derive(Debug, Clone)]
pub struct ToolResultBlock {
    pub output: String,
    pub is_error: bool,
}

#[derive(Default)]
pub struct MsgLayout {
    pub area: Rect,
    pub offset: u16,
    pub spans: Vec<(usize, usize, usize)>,
}

#[derive(Default)]
pub struct CockpitLayout {
    pub thinking: Rect,
    pub subconscious: Rect,
}

pub struct ChatState {
    pub backend: Arc<dyn Backend>,
    pub mode: String,
    pub agent_name: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_cursor: usize,
    /// Images attached to the current input, not yet submitted.
    pub attached_images: Vec<ImageAttachment>,
    pub scroll: u16,
    pub msg_layout: RefCell<MsgLayout>,
    pub copy_flash: Option<Instant>,
    pub turn_rx: Option<mpsc::Receiver<BackendEvent>>,
    pub cancel_token: Option<CancellationToken>,
    pub busy: bool,
    pub show_esc_overlay: bool,
    pub tool_calls_this_turn: u32,
    pub phase: TurnPhase,
    pub pending_interjections: crate::backend::InterjectionQueue,
    pub pressure: f32,
    /// Total context window size (e.g. 262_000 for kimi-k2.6).
    /// None until the first pressure event arrives.
    pub context_limit: Option<usize>,
    pub overlay: Overlay,
    pub cockpit: bool,
    pub thinking: Vec<String>,
    pub cockpit_log: Vec<CockpitEntry>,
    pub thinking_scroll: Cell<u16>,
    pub subconscious_scroll: Cell<u16>,
    pub cockpit_layout: RefCell<CockpitLayout>,
    pub tick: u64,
    pub turn_started: Option<Instant>,
    pub last_event_at: Instant,
    pub model_rx: Option<oneshot::Receiver<String>>,
    pub pending_consciousness: Vec<BackendEvent>,
    pub new_conv_rx: Option<oneshot::Receiver<Result<String>>>,
    pub convos_rx: Option<oneshot::Receiver<Result<Vec<crate::backend::ConversationInfo>>>>,
    pub switch_rx: Option<oneshot::Receiver<Result<(String, Vec<crate::core::session::ConversationMessage>)>>>,
    pub switch_pending: Option<String>,
    pub resume_offer: bool,
    pub btw_state: BtwState,
    pub btw_rx: Option<mpsc::Receiver<BtwForkEvent>>,
    pub tool_cards_expanded: bool,
    pub render_mode: ChatMode,
    pub palette: ChatPalette,
    pub stream_buffer: String,
}

#[derive(Debug, Clone)]
pub enum BtwState {
    Idle,
    Forking { question: String },
    Streaming { question: String, response_so_far: String },
    Complete { question: String, response: String, forked_id: String },
    Error { question: String, error: String },
}

impl ChatState {
    pub async fn connect(
        config: Arc<RwLock<ConsciousnessConfig>>,
        agent_name_pref: &str,
    ) -> Result<Self> {
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

        let backend: Arc<dyn Backend> = if mode == "remote" {
            Arc::new(crate::backend::RemoteBackend::with_agent(&url, &agent.id))
        } else {
            backend
        };

        let pending = backend.take_pending_surfacings(&agent.id).await;
        let mut messages: Vec<ChatMessage> = vec![ChatMessage::System {
            text: "Souveraine ready. Type to begin.".to_string(),
            ts: Instant::now(),
        }];
        let mut cockpit_log: Vec<CockpitEntry> = Vec::new();
        if !pending.is_empty() {
            messages.push(ChatMessage::System {
                text: format!(
                    "{} observation{} surfaced while you were away (Tab for cockpit).",
                    pending.len(),
                    if pending.len() == 1 { "" } else { "s" },
                ),
                ts: Instant::now(),
            });
            for p in &pending {
                let (kind, label) = match p.kind.as_str() {
                    "reflection" => (CockpitKind::Reflection, "reflection"),
                    "archivist" => (CockpitKind::Archivist, "archivist"),
                    _ => (CockpitKind::Surfacing, "surfacing"),
                };
                cockpit_log.push(CockpitEntry {
                    kind,
                    text: format!("(while away) {}", p.content),
                });
                messages.push(ChatMessage::Surfacing {
                    source: if p.source.is_empty() {
                        label.to_string()
                    } else {
                        p.source.clone()
                    },
                    content: p.content.clone(),
                    priority: if p.priority.is_empty() {
                        "heartbeat".to_string()
                    } else {
                        p.priority.clone()
                    },
                    ts: Instant::now(),
                });
            }
        }

        Ok(Self {
            backend,
            mode: mode.to_string(),
            agent_name: agent.name,
            agent_id: agent.id,
            conversation_id,
            messages,
            input: String::new(),
            input_cursor: 0,
            attached_images: Vec::new(),
            scroll: 0,
            msg_layout: RefCell::new(MsgLayout::default()),
            copy_flash: None,
            turn_rx: None,
            cancel_token: None,
            busy: false,
            tool_calls_this_turn: 0,
            phase: TurnPhase::Idle,
            pending_interjections: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pressure: 0.0,
            context_limit: None,
            overlay: Overlay::None,
            cockpit: false,
            thinking: Vec::new(),
            cockpit_log,
            thinking_scroll: Cell::new(0),
            subconscious_scroll: Cell::new(0),
            cockpit_layout: RefCell::new(CockpitLayout::default()),
            tick: 0,
            turn_started: None,
            last_event_at: Instant::now(),
            model_rx: None,
            pending_consciousness: Vec::new(),
            new_conv_rx: None,
            convos_rx: None,
            switch_rx: None,
            switch_pending: None,
            resume_offer: false,
            btw_state: BtwState::Idle,
            btw_rx: None,
            tool_cards_expanded: false,
            show_esc_overlay: false,
            render_mode: ChatMode::Conversation,
            palette: ChatPalette::default(),
            stream_buffer: String::new(),
        })
    }
}
