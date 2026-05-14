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

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Event from the /btw fork background task.
#[derive(Debug, Clone)]
enum BtwForkEvent {
    Forked { id: String },
    Token(String),
    Done,
    Error(String),
}
use tokio_util::sync::CancellationToken;

use crate::backend::{Backend, BackendEvent};
use crate::bridge::bifrost::BifrostClient;
use crate::core::config::ConsciousnessConfig;
use crate::ui::atmosphere::lerp_color;
use crate::ui::markdown;

// Fallback constants — still used for static colors that don't come from
// the atmosphere palette (e.g. white text that should always be white).

/// Live palette derived from the current atmosphere. Updated per-tick so
/// transitions interpolate smoothly. Every message rendering function reads
/// from this instead of hardcoded constants.
#[derive(Debug, Clone, Copy)]
pub struct ChatPalette {
    /// Agent bubble border + label — the most visible accent.
    pub agent_primary: Color,
    /// Agent text accent — dimmer variant of primary.
    pub agent_dim: Color,
    /// User bubble color — primary tinted toward blue.
    pub user_accent: Color,
    /// Tool card border.
    pub tool_accent: Color,
    /// Tool card dimmed text.
    pub tool_dim: Color,
    /// Surfacing bubble (subconscious).
    pub surfacing: Color,
    /// Reflection lines.
    pub reflection: Color,
    /// Archivist lines.
    pub archivist: Color,
    /// Compaction warnings.
    pub compaction: Color,
    /// Background tint for panes.
    pub bg: Color,
}

impl ChatPalette {
    /// Build a palette from an Atmosphere enum (snapshot — no transition).
    pub fn from_atmosphere(atm: crate::ui::atmosphere::Atmosphere) -> Self {
        Self::from_colors(atm.primary(), atm.secondary(), atm.dim(), atm.bg_tint())
    }

    /// Build a palette from raw primary/secondary/dim/bg colors.
    /// Used by the ambient atmosphere lerp path — feed it interpolated colors.
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

    /// Deterministic hash of the palette's channel values. Invalidation key
    /// for caches that depend on palette-derived colors (markdown cache, etc.).
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

    fn color(&self, palette: &ChatPalette) -> Color {
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

const SLASH_COMMANDS: &[SlashDef] = &[
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
];

/// Cached markdown render for an assistant bubble. Key = (text byte-len,
/// inner width). On a frame, if both match the current state, we clone
/// the stored lines instead of re-parsing + re-wrapping the markdown.
/// Pattern from jcode's `IncrementalMarkdownRenderer` (`lib.rs:448`) —
/// the "incremental" path there is actually a text-equality fast path
/// over the same renderer call. We use text length as a cheap proxy:
/// streaming appends always change length, final bubbles never do.
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
        /// Per-message markdown cache. RefCell so `draw_messages(&ChatState)`
        /// can populate it without taking `&mut`.
        rendered_cache: RefCell<Option<MarkdownCache>>,
    },
    Surfacing { source: String, content: String, priority: String, ts: Instant },
    System { text: String, ts: Instant },
    /// User spoke while the agent was working. Queued and prepended to
    /// the agent's context before her next LLM call. Rendered with a
    /// distinct chevron so the user sees their interjection landed in
    /// the stream, separate from a normal /user turn.
    Interjection { text: String, ts: Instant, delivered: bool },
    /// Tool invocation card — name, arguments, round, plus an attached result
    /// once it streams back. `expanded` is reserved for click-to-expand (UI
    /// interactivity lands as part of message-click work).
    Tool {
        id: String,
        name: String,
        arguments: String,
        round: u32,
        result: Option<ToolResultBlock>,
        ts: Instant,
        expanded: bool,
    },
}

/// The agent's current "what is she doing" phase, surfaced in the
/// dedicated phase strip between the message body and the input box.
/// Phase transitions come from BackendEvent observations — pure derived
/// state, no new event types required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    Idle,
    Thinking,
    Tool,
    Streaming,
    Interrupted,
}

/// Render posture for the chat surface. Same conversation, same memfs,
/// same agent — different way of being in the room. Toggled mid-session
/// via `/code` and `/chat`, persists no further than the current process.
///
/// - `Conversation`: prose first. Tool gestures collapse to a single line
///   by default. Assistant text gets the soft bubble. The room is warm.
/// - `Code`: work first. Tool gestures default-expanded so diffs and output
///   are visible without a keystroke. The input prefix shifts to `≡ ›` so
///   the user feels the posture change. Bubble wrapping stays — code mode
///   is a stance, not a separate UI.
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
    /// Cancellation handle for the current in-flight turn. Esc fires this;
    /// the backend treats it as a signal (Constitution VI.1 — substrate, not
    /// harness) — the current tool completes, no further LLM calls, partial
    /// text is preserved with `*[interrupted]*` appended.
    pub cancel_token: Option<CancellationToken>,
    pub busy: bool,
    /// Number of tool calls in the active turn — drives the phase strip's
    /// "N tools used" counter. Reset to zero at every `submit()`.
    pub tool_calls_this_turn: u32,
    /// Current rendering phase (drives the phase strip text/colour).
    /// Derived state that mirrors what BackendEvent we last saw.
    pub phase: TurnPhase,
    /// Messages typed during `busy`. Shared with the backend's turn
    /// loop via `Arc<Mutex<…>>`: chat pushes synchronously from the
    /// input handler; the backend drains between LLM rounds and
    /// prepends each as a `[user interjected]` system note so the
    /// agent reads them in her own voice on her next pass.
    pub pending_interjections: crate::backend::InterjectionQueue,
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
    /// When the last BackendEvent arrived. Compared against `stale_timeout`
    /// in `drain_events` to detect silent hangs — the backend channel stays
    /// open but no events arrive (e.g. provider crash mid-turn).
    last_event_at: Instant,
    /// How long to wait before declaring a turn stalled. Reads from
    /// `[tui] stale_timeout_secs` in config; defaults to 90s.
    stale_timeout: Duration,
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
    /// `/btw` fork state — an ephemeral side-quest conversation running
    /// in parallel to the main chat. Rendered as a floating bordered pane.
    pub btw_state: BtwState,
    /// Receiver for /btw fork stream results (token deltas).
    pub btw_rx: Option<mpsc::Receiver<BtwForkEvent>>,
    /// Session-level toggle for tool card expansion. False (default) renders
    /// each tool call as a single compact line — a gesture by the sensorium,
    /// not a billboard. True restores the full bubble with arguments + result
    /// preview. Per-message `expanded` on individual cards overrides this.
    pub tool_cards_expanded: bool,
    /// Conversation vs Code posture. See [`ChatMode`]. Toggled via /code,
    /// /chat. Code posture defaults tool cards to expanded so the work is
    /// visible without a keystroke.
    pub render_mode: ChatMode,
    /// Live palette derived from the agent's current atmosphere.
    pub palette: ChatPalette,
}

/// Ephemeral /btw fork state. Mirrors Letta's BtwPane — a forked conversation
/// streams its response into a floating pane alongside the main transcript.
/// User can jump to the fork ([j]) or dismiss ([esc]).
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
        // Pick the backend: try remote first, fall back to local. Mirrors
        // `main::resolve_backend` but adapted for the TUI (no quiet/json flags).
        let cfg = config.read().await;
        let url = cfg.server.effective_url();
        let stale_timeout = Duration::from_secs(cfg.tui.stale_timeout_secs);
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
            cancel_token: None,
            busy: false,
            tool_calls_this_turn: 0,
            phase: TurnPhase::Idle,
            pending_interjections: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pressure: 0.0,
            overlay: Overlay::None,
            cockpit: false,
            thinking: Vec::new(),
            cockpit_log: Vec::new(),
            tick: 0,
            turn_started: None,
            last_event_at: Instant::now(),
            stale_timeout,
            model_rx: None,
            pending_consciousness: Vec::new(),
            new_conv_rx: None,
            convos_rx: None,
            switch_rx: None,
            btw_state: BtwState::Idle,
            btw_rx: None,
            tool_cards_expanded: false,
            render_mode: ChatMode::Conversation,
            palette: ChatPalette::default(),
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
  /btw <text>        Interject — delivered to her next LLM round
  /code              Shift to code posture (tools expanded, ≡ prompt)
  /chat              Shift to conversation posture (tools collapsed)
  /outfit <name>     Change agent's outfit (empty to reset)
  !<command>         Run a shell command (Linux/macOS)

Esc during a turn interrupts (signal, not kill — she sees *[interrupted]*).
You can also just keep typing while she works — Enter queues an interjection.
Tab toggles the cockpit pane. `t` (on empty input) toggles tool expansion.";

    /// Submit the current input. Returns `true` if the input was handled
    /// (slash command, bang command, sent to backend, or queued as an
    /// interjection while the agent was already mid-turn).
    ///
    /// When `busy=true`, the message is **not** rejected — it's wrapped
    /// as a [`ChatMessage::Interjection`] for visual feedback and pushed
    /// onto `pending_interjections`. The backend drains that queue
    /// before its next Bifrost call. This is the substrate path for
    /// "talk while she's working" — the user keeps presence in the
    /// conversation; the agent decides when to read it.
    pub fn submit(&mut self) -> bool {
        if self.input.trim().is_empty() {
            return false;
        }

        let trimmed = self.input.trim().to_string();
        self.input.clear();

        // /btw <text> — fork the conversation into a side-quest.
        // The main chat is untouched; the forked conversation streams
        // its response into an ephemeral BtwPane. User can [j]ump or
        // [esc] dismiss.
        if let Some(rest) = trimmed.strip_prefix("/btw ") {
            let question = rest.trim().to_string();
            if !question.is_empty() && !self.btw_active() {
                self.start_btw_fork(question);
            }
            return true;
        }

        // Slash commands route through their own handler. Most are
        // metadata commands (/clear, /help, /new) safe to run any time.
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

        // Typing while busy → queued as an interjection rather than a
        // new turn. The agent sees it in her context on her next pass.
        if self.busy {
            self.enqueue_interjection(trimmed);
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
            rendered_cache: RefCell::new(None),
        });
        self.busy = true;
        self.tool_calls_this_turn = 0;
        self.phase = TurnPhase::Thinking;
        self.turn_started = Some(Instant::now());
        self.last_event_at = Instant::now();

        let (tx, rx) = mpsc::channel::<BackendEvent>(64);
        self.turn_rx = Some(rx);

        // Cancellation token for this turn — Esc fires it.
        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        let backend = self.backend.clone();
        let conv_id = self.conversation_id.clone();
        let interject_queue = self.pending_interjections.clone();
        tokio::spawn(async move {
            match backend.send_with_signals(&conv_id, &text, cancel, interject_queue).await {
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

        if trimmed == "/code" {
            self.render_mode = ChatMode::Code;
            self.system_message(
                "Code posture. Tool gestures expand; the prompt becomes ≡. Same conversation."
                    .to_string(),
            );
            return true;
        }

        if trimmed == "/chat" {
            self.render_mode = ChatMode::Conversation;
            self.system_message(
                "Conversation posture. Tool gestures collapse; the prompt returns to ›.".to_string(),
            );
            return true;
        }

        if trimmed == "/outfit" || trimmed.starts_with("/outfit ") {
            let name = trimmed.strip_prefix("/outfit")
                .map(|s| s.trim())
                .unwrap_or("")
                .to_string();
            self.pending_consciousness.push(BackendEvent::Outfit(name.clone()));
            if name.is_empty() {
                self.system_message("Returned to default appearance.".to_string());
            } else {
                self.system_message(format!("Changed to **{name}** outfit."));
            }
            return true;
        }

        if trimmed == "/btw" {
            self.system_message(
                "Usage: /btw <text> — fork a side-quest conversation.\nThe main chat carries on; the fork streams into a floating pane. Press j to jump to the fork, Esc to dismiss."
                    .to_string(),
            );
            return true;
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
        // If a turn is in flight, cancel it cleanly before switching. The
        // cancel token propagates to the backend which appends *[interrupted]*
        // and commits the partial output to git — so nothing is lost.
        if self.busy {
            self.interrupt();
            self.system_message("Interrupted active turn — partial output saved.".to_string());
        }

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
                        cfg.bifrost.timeout_secs,
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
    /// Walk the message list and mark any interjections as `delivered`
    /// once the shared queue has been drained by the backend. Called at
    /// the top of `drain_events` so the UI flips from amber `⏳ /btw`
    /// to grey `↳ /btw` as soon as the agent has read the interruption.
    fn flush_delivered_interjections(&mut self) {
        let queue_empty = self
            .pending_interjections
            .lock()
            .ok()
            .map(|q| q.is_empty())
            .unwrap_or(true);
        if !queue_empty { return; }
        for msg in self.messages.iter_mut() {
            if let ChatMessage::Interjection { delivered, .. } = msg {
                *delivered = true;
            }
        }
    }

    pub fn drain_events(&mut self) {
        // Any interjections the backend just consumed should flip to the
        // delivered (dim grey) state.
        self.flush_delivered_interjections();

        // Drain /btw fork stream events.
        self.drain_btw();

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
                                    self.messages.push(ChatMessage::Assistant {
                                        text,
                                        ts: Instant::now(),
                                        streaming: false,
                                        rendered_cache: RefCell::new(None),
                                    });
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

        if !drained.is_empty() {
            self.last_event_at = Instant::now();
        }

        for ev in drained {
            match ev {
                BackendEvent::Token(t) => {
                    self.phase = TurnPhase::Streaming;
                    self.append_streaming(&t);
                }
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
                BackendEvent::ToolCall { id, name, arguments, round } => {
                    // If a streaming assistant block is open, finalize it
                    // first so the card lands beneath the just-said text.
                    self.finalize_streaming();
                    self.phase = TurnPhase::Tool;
                    self.tool_calls_this_turn = self.tool_calls_this_turn.saturating_add(1);
                    // Add a round separator to the thinking pane when a new round starts.
                    if !self.thinking.is_empty() {
                        let sep = format!("──── r{} ────", round);
                        let last_is_sep = self.thinking.last().map_or(false, |s| s.starts_with("────"));
                        if !last_is_sep {
                            self.thinking.push(sep);
                        }
                    }
                    // Tool calls are surfaced in the main message stream only;
                    // the subconscious pane is Aster's window, not a tool log.
                    self.messages.push(ChatMessage::Tool {
                        id,
                        name,
                        arguments,
                        round,
                        result: None,
                        ts: Instant::now(),
                        expanded: false,
                    });
                }
                BackendEvent::ToolResult { id, name: _, output, is_error } => {
                    // Attach to the matching tool card by id. If we don't
                    // find one (unusual), append a System line so it's not lost.
                    let mut bound = false;
                    for msg in self.messages.iter_mut().rev() {
                        if let ChatMessage::Tool { id: tid, result, .. } = msg {
                            if tid == &id && result.is_none() {
                                *result = Some(ToolResultBlock {
                                    output: output.clone(),
                                    is_error,
                                });
                                bound = true;
                                break;
                            }
                        }
                    }
                    if !bound {
                        let prefix = if is_error { "[tool error] " } else { "[tool] " };
                        self.messages.push(ChatMessage::System {
                            text: format!("{}{}", prefix, output),
                            ts: Instant::now(),
                        });
                    }
                }
                BackendEvent::Atmosphere(preset) => {
                    self.pending_consciousness.push(BackendEvent::Atmosphere(preset));
                }
                BackendEvent::SubconsciousPass(active) => {
                    self.pending_consciousness.push(BackendEvent::SubconsciousPass(active));
                }
                BackendEvent::Outfit(name) => {
                    self.pending_consciousness.push(BackendEvent::Outfit(name));
                }
                BackendEvent::Done => {
                    self.finalize_streaming();
                    self.busy = false;
                    self.turn_started = None;
                    self.turn_rx = None;
                    self.cancel_token = None;
                    self.phase = TurnPhase::Idle;
                    self.tool_calls_this_turn = 0;
                    return;
                }
            }
        }

        if closed {
            self.finalize_streaming();
            self.busy = false;
            self.turn_started = None;
            self.turn_rx = None;
            self.cancel_token = None;
            self.phase = TurnPhase::Idle;
            self.tool_calls_this_turn = 0;
        }

        // Staleness guard: if the backend channel is open but nothing has
        // arrived for `stale_timeout`, the turn silently hung (provider crash,
        // channel leak). Reset so the UI doesn't display "Streaming" forever.
        if self.busy && self.last_event_at.elapsed() >= self.stale_timeout {
            let secs = self.stale_timeout.as_secs();
            tracing::warn!(
                elapsed = ?self.last_event_at.elapsed(),
                phase = ?self.phase,
                tool_calls = self.tool_calls_this_turn,
                "turn stalled — resetting"
            );
            self.finalize_streaming();
            self.messages.push(ChatMessage::System {
                text: format!(
                    "*[turn stalled — backend went silent after {}s (phase: {:?}, tools: {}). \
                    Your last message may not have been processed. Send it again to retry, or Esc → reconnect. \
                    See souveraine.log for details.]*",
                    secs, self.phase, self.tool_calls_this_turn
                ),
                ts: Instant::now(),
            });
            self.busy = false;
            self.turn_started = None;
            self.turn_rx = None;
            self.cancel_token = None;
            self.phase = TurnPhase::Idle;
            self.tool_calls_this_turn = 0;
        }
    }

    /// User pressed Esc during a turn. Fire the cancel token — the backend
    /// reads it as a signal, lets the current tool complete, stops making
    /// new LLM calls, and commits partial text with `*[interrupted]*` so
    /// the agent reads it on her next turn. Not a hard kill.
    pub fn interrupt(&mut self) {
        if let Some(token) = &self.cancel_token {
            if !token.is_cancelled() {
                token.cancel();
                self.phase = TurnPhase::Interrupted;
            }
        }
    }

    /// Queue text as an interjection (mid-turn user message). Pushes a
    /// visual `ChatMessage::Interjection` so the user sees it landed, and
    /// appends to the shared `pending_interjections` queue. The backend's
    /// turn loop drains the queue on its next round and prepends each as
    /// a `[user interjected]` system message. If no turn is running, we
    /// deliver it immediately as a normal message so a stray `/btw`
    /// doesn't get queued and forgotten.
    fn enqueue_interjection(&mut self, text: String) {
        if !self.busy {
            self.input = text;
            self.submit();
            return;
        }
        self.messages.push(ChatMessage::Interjection {
            text: text.clone(),
            ts: Instant::now(),
            delivered: false,
        });
        if let Ok(mut q) = self.pending_interjections.lock() {
            q.push(text);
        }
    }

    /// Is a /btw fork currently active (forking/streaming)?
    pub fn btw_active(&self) -> bool {
        !matches!(self.btw_state, BtwState::Idle)
    }

    /// Fork the conversation for a /btw side-quest.
    fn start_btw_fork(&mut self, question: String) {
        self.btw_state = BtwState::Forking { question: question.clone() };
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let conv_id = self.conversation_id.clone();
        let (tx, rx) = mpsc::channel::<BtwForkEvent>(64);
        self.btw_rx = Some(rx);

        tokio::spawn(async move {
            let forked_id = match backend.fork_conversation(&agent_id, &conv_id).await {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx.send(BtwForkEvent::Error(e.to_string())).await;
                    return;
                }
            };
            let _ = tx.send(BtwForkEvent::Forked { id: forked_id.clone() }).await;

            let mut stream = match backend.send(&forked_id, &question).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(BtwForkEvent::Error(e.to_string())).await;
                    return;
                }
            };

            use futures::StreamExt;
            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(crate::backend::BackendEvent::Token(t)) => {
                        if tx.send(BtwForkEvent::Token(t)).await.is_err() { break; }
                    }
                    Ok(crate::backend::BackendEvent::Done) => break,
                    _ => {}
                }
            }
            let _ = tx.send(BtwForkEvent::Done).await;
        });
    }

    /// Drain pending /btw fork stream events. Call once per tick.
    pub fn drain_btw(&mut self) {
        // Track forked_id through the state machine
        let mut pending_forked_id: Option<String> = None;

        let Some(rx) = &mut self.btw_rx else { return };
        loop {
            match rx.try_recv() {
                Ok(BtwForkEvent::Forked { id }) => {
                    pending_forked_id = Some(id);
                }
                Ok(BtwForkEvent::Token(token)) => {
                    match &mut self.btw_state {
                        BtwState::Forking { question } => {
                            let q = std::mem::take(question);
                            self.btw_state = BtwState::Streaming {
                                question: q,
                                response_so_far: token,
                            };
                        }
                        BtwState::Streaming { response_so_far, .. } => {
                            response_so_far.push_str(&token);
                        }
                        _ => {}
                    }
                }
                Ok(BtwForkEvent::Done) => {
                    let forked_id = pending_forked_id.take().unwrap_or_default();
                    if let BtwState::Streaming { question, response_so_far } =
                        std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Complete {
                            question,
                            response: response_so_far,
                            forked_id,
                        };
                    }
                    self.btw_rx = None;
                    break;
                }
                Ok(BtwForkEvent::Error(e)) => {
                    if let BtwState::Forking { question } =
                        std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Error { question, error: e };
                    }
                    self.btw_rx = None;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    let forked_id = pending_forked_id.take().unwrap_or_default();
                    if let BtwState::Streaming { question, response_so_far } =
                        std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Complete {
                            question,
                            response: response_so_far,
                            forked_id,
                        };
                    }
                    self.btw_rx = None;
                    break;
                }
            }
        }
    }

    /// Dismiss the /btw fork pane.
    pub fn btw_dismiss(&mut self) {
        self.btw_state = BtwState::Idle;
        self.btw_rx = None;
    }

    /// Jump to the forked conversation — replaces the conversation_id and
    /// backfills messages so the main chat switches to the fork. Caller must
    /// trigger a conversation reload (load_conversation) after this.
    pub fn btw_jump(&mut self) -> Option<String> {
        if let BtwState::Complete { forked_id, .. } = &self.btw_state {
            if !forked_id.is_empty() {
                let id = forked_id.clone();
                self.btw_state = BtwState::Idle;
                self.btw_rx = None;
                return Some(id);
            }
        }
        None
    }

    /// Toggle the cockpit side-pane.
    pub fn toggle_cockpit(&mut self) {
        self.cockpit = !self.cockpit;
    }

    /// Update slash-command completion state based on current input.
    /// Call after each input mutation. Stays live during `busy` so the
    /// user can autocomplete `/btw` mid-turn.
    pub fn update_completion(&mut self) {
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
            rendered_cache: RefCell::new(None),
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
    // Input is now ALWAYS a real input — the "thinking…" spinner has been
    // lifted into its own phase strip above the input, so the user can keep
    // typing (and use /btw) while the agent works.
    let input_inner_width = (area.width as usize).saturating_sub(5).max(1);
    let input_visual_lines = count_visual_lines(&state.input, input_inner_width);
    let max_input_lines = ((area.height as usize) * 40 / 100).max(1);
    let input_height = (input_visual_lines.min(max_input_lines) as u16) + 2; // +2 for borders

    // Phase strip: 1 row when a turn is in flight, 0 rows when idle.
    let phase_height: u16 = if state.busy || state.phase == TurnPhase::Interrupted { 1 } else { 0 };

    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // header
            Constraint::Min(5),                // body (messages + optional cockpit)
            Constraint::Length(phase_height),  // phase strip (0 when idle)
            Constraint::Length(input_height),  // input (dynamic, always live)
            Constraint::Length(1),             // status footer
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

    if phase_height > 0 {
        draw_phase(f, state, vchunks[2]);
    }
    draw_input(f, state, vchunks[3]);
    draw_footer(f, state, vchunks[4]);

    // Overlays render last — anchor them above the input (vchunks[3]) so
    // slash-completion and other popups still line up with the prompt.
    draw_overlay(f, state, area, vchunks[3]);

    // /btw fork pane renders on top of everything, floating over the body.
    if !matches!(state.btw_state, BtwState::Idle) {
        draw_btw_pane(f, state, area);
    }
}

/// Single-line phase strip that lives between the message body and the
/// input box during an active turn. The reader tells the story:
///   `⏣ Thinking… 12s`
///   `⏣ Running tool: bash · 4 tools used · 23s`
///   `⏣ Streaming · 31s`
///   `× Interrupted · 35s`
/// No box, no border — it reads as a status line, not another widget.
fn draw_phase(f: &mut Frame, state: &ChatState, area: Rect) {
    let elapsed = state
        .turn_started
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    let spinner = SPINNER[(state.tick as usize / 2) % SPINNER.len()];

    let (glyph, label, color) = match state.phase {
        TurnPhase::Thinking | TurnPhase::Idle => (spinner, "Thinking".to_string(), state.palette.agent_primary),
        TurnPhase::Tool => {
            let label = if state.tool_calls_this_turn == 1 {
                "Running tool · 1 tool used".to_string()
            } else {
                format!("Running tool · {} tools used", state.tool_calls_this_turn)
            };
            (spinner, label, state.palette.tool_accent)
        }
        TurnPhase::Streaming => (spinner, "Streaming".to_string(), state.palette.agent_primary),
        TurnPhase::Interrupted => ("×", "Interrupted".to_string(), state.palette.compaction),
    };
    let queued = state
        .pending_interjections
        .lock()
        .ok()
        .map(|q| q.len())
        .unwrap_or(0);

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!(" {} ", glyph), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}", label), Style::default().fg(color)),
        Span::styled(format!("  ·  {}s", elapsed), Style::default().fg(state.palette.agent_dim)),
    ];
    if queued > 0 {
        spans.push(Span::styled(
            format!("  ·  /btw queued: {}", queued),
            Style::default().fg(state.palette.surfacing).add_modifier(Modifier::ITALIC),
        ));
    }
    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Left), area);
}

fn draw_header(f: &mut Frame, state: &ChatState, area: Rect) {
    let mode_color = match state.mode.as_str() {
        "local" => state.palette.tool_accent,
        "remote" => state.palette.agent_primary,
        _ => state.palette.agent_dim,
    };
    let title = Line::from(vec![
        Span::styled("✦ Souveraine ", Style::default().fg(state.palette.agent_primary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {} ", state.agent_name), Style::default().fg(Color::White)),
        Span::styled(format!("[{} mode]", state.mode), Style::default().fg(mode_color)),
    ]);
    f.render_widget(Paragraph::new(title).alignment(Alignment::Center), area);
}

fn draw_messages(f: &mut Frame, state: &ChatState, area: Rect) {
    let mdpal = crate::ui::markdown::MarkdownPalette::from_chat_palette(&state.palette);
    let max_bubble = ((area.width as usize).saturating_sub(8) * 70 / 100).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in &state.messages {
        match msg {
            ChatMessage::User { text, .. } => {
                lines.extend(bubble(
                    "you",
                    text,
                    max_bubble,
                    Style::default().fg(state.palette.user_accent),
                    BubbleAlign::Right,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::Assistant { text, streaming, rendered_cache, .. } => {
                let label = if *streaming { format!("{} ◦", state.agent_name) } else { state.agent_name.clone() };
                // Pre-wrap to the bubble's inner width — no line should exit
                // the bubble's borders, and Paragraph's later re-wrap becomes
                // a no-op (preserves scroll line-count math).
                let inner_width = max_bubble.saturating_sub(4).max(8);
                let body_lines = if text.is_empty() && *streaming {
                    vec![Line::from("…")]
                } else {
                    // Cache hit: same text length + width = same render. For
                    // finalized bubbles this is every subsequent frame; for
                    // streaming bubbles each token append invalidates by
                    // changing text.len(). jcode IncrementalMarkdownRenderer
                    // pattern — text-equality fast path, full re-render
                    // otherwise. (jcode/crates/jcode-tui-markdown/src/lib.rs:448)
                    let key_len = text.len();
                    let palette_hash = state.palette.hash();
                    let agent_color = state.palette.agent_primary;
                    let cached = rendered_cache.borrow();
                    if let Some(c) = &*cached {
                        if c.text_len == key_len && c.inner_width == inner_width && c.palette_hash == palette_hash {
                            c.lines.clone()
                        } else {
                            drop(cached);
                            let lines = markdown::render_with_width(text, agent_color, Some(inner_width), &mdpal);
                            *rendered_cache.borrow_mut() = Some(MarkdownCache {
                                text_len: key_len,
                                inner_width,
                                palette_hash,
                                lines: lines.clone(),
                            });
                            lines
                        }
                    } else {
                        drop(cached);
                        let lines = markdown::render_with_width(text, agent_color, Some(inner_width), &mdpal);
                        *rendered_cache.borrow_mut() = Some(MarkdownCache {
                            text_len: key_len,
                            inner_width,
                            palette_hash,
                            lines: lines.clone(),
                        });
                        lines
                    }
                };
                lines.extend(bubble_rendered(
                    &label,
                    &body_lines,
                    max_bubble,
                    Style::default().fg(state.palette.agent_primary),
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
                    Style::default().fg(state.palette.surfacing),
                    BubbleAlign::Center,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::System { text, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  · {}", text),
                    Style::default().fg(state.palette.agent_dim).add_modifier(Modifier::ITALIC),
                )));
                lines.push(Line::from(""));
            }
            ChatMessage::Tool { name, arguments, round, result, expanded, .. } => {
                let code_posture = state.render_mode == ChatMode::Code;
                let expand = *expanded || state.tool_cards_expanded || code_posture;
                if expand {
                    lines.extend(render_tool_card(
                        name,
                        arguments,
                        *round,
                        result.as_ref(),
                        max_bubble,
                        area.width,
                        &state.palette,
                    ));
                    lines.push(Line::from(""));
                } else {
                    lines.extend(render_tool_card_compact(
                        name,
                        arguments,
                        *round,
                        result.as_ref(),
                        area.width,
                        &state.palette,
                    ));
                }
            }
            ChatMessage::Interjection { text, delivered, .. } => {
                // User spoke while the agent was working. Rendered as a
                // compact single-line note so it's visible in the stream
                // without competing with normal user bubbles. Dims after
                // the backend has delivered it on the next LLM round.
                let glyph = if *delivered { "↳" } else { "⏳" };
                let color = if *delivered {
                    state.palette.agent_dim
                } else {
                    state.palette.surfacing
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} /btw  ", glyph),
                        Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::styled(text.clone(), Style::default().fg(color).add_modifier(Modifier::ITALIC)),
                ]));
                lines.push(Line::from(""));
            }
        }
    }

    // Final pre-wrap: anything still wider than the visible area (system
    // notices, raw text, anything that bypassed bubble pre-wrap) gets
    // wrapped here. After this, `lines.len()` equals the visible line
    // count — Paragraph's wrap becomes a no-op and scroll math holds.
    let visible_width = area.width.saturating_sub(0) as usize;
    let lines = markdown::wrap_lines(lines, visible_width);

    // Trim trailing empty lines from the count (each bubble appends a
    // blank separator; the last one shouldn't push the final real line
    // off the bottom). jcode pattern — count the tail-strip, don't drop
    // the lines themselves so the visual rhythm is preserved.
    let trailing_empty = lines
        .iter()
        .rev()
        .take_while(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        .count();
    let effective_total = lines.len().saturating_sub(trailing_empty);

    // Auto-scroll to bottom unless the user has manually scrolled up.
    // `state.scroll` is *lines scrolled up from the bottom* (jcode pattern,
    // `single_session.rs:1223`). Zero means pinned to the tail; growing
    // content with scroll=0 always shows the newest tail without overshoot.
    let view = area.height.saturating_sub(2) as usize;
    let max_scroll = effective_total.saturating_sub(view);
    let user_scroll = (state.scroll as usize).min(max_scroll);
    let offset = max_scroll.saturating_sub(user_scroll) as u16;

    let para = Paragraph::new(lines)
        .scroll((offset, 0))
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(state.palette.agent_dim))
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

/// Render a tool invocation card. Header shows `▶ name (round N)`, then
/// the arguments (truncated/wrapped), then — once a result has streamed
/// back — a `└─ result` section rendered as markdown so output reads like
/// code or prose, not a single jammed line. Errors paint the border red.
fn render_tool_card(
    name: &str,
    arguments: &str,
    round: u32,
    result: Option<&ToolResultBlock>,
    max_width: usize,
    container_width: u16,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let mdpal = crate::ui::markdown::MarkdownPalette::from_chat_palette(palette);
    let is_err = result.map(|r| r.is_error).unwrap_or(false);
    let border_color = if is_err { palette.compaction } else { palette.tool_accent };
    let dim_color = if is_err { palette.compaction } else { palette.tool_dim };
    let border = Style::default().fg(border_color);

    // Header marker + status glyph: pending=⟳, ok=✓, err=⚠
    let glyph = match result {
        None => '⟳',
        Some(r) if r.is_error => '⚠',
        Some(_) => '✓',
    };
    let title = format!("{} {}  ·  round {}", glyph, name, round);

    // Body: arguments (compact one-line summary), then result if present.
    let mut body_lines: Vec<Line<'static>> = Vec::new();
    let inner_width = max_width.saturating_sub(4).max(8);

    let args_summary = summarize_tool_args(arguments);
    let args_line = Line::from(vec![Span::styled(
        args_summary,
        Style::default().fg(dim_color),
    )]);
    body_lines.extend(markdown::wrap_line(args_line, inner_width));

    if let Some(r) = result {
        body_lines.push(Line::from(""));
        let preview = preview_output(&r.output, 12);
        let inner_width = max_width.saturating_sub(4).max(8);
        let rendered = markdown::render_with_width(
            &preview,
            if r.is_error { palette.compaction } else { palette.agent_primary },
            Some(inner_width),
            &mdpal,
        );
        body_lines.extend(rendered);
        if r.output.lines().count() > 12 {
            body_lines.push(Line::from(Span::styled(
                format!("  … ({} more lines)", r.output.lines().count() - 12),
                Style::default().fg(dim_color).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    bubble_rendered(&title, &body_lines, max_width, border, BubbleAlign::Left, container_width)
}

/// Compact single-line tool render — the sensorium signalling a gesture, not
/// announcing one. A status glyph, the sensor name, a clipped argument
/// summary, and (on error) the first line of the failure inline. No box
/// borders, no result preview. Toggle expand-all with `t` to surface
/// the full content when witnessing matters more than the gesture.
fn render_tool_card_compact(
    name: &str,
    arguments: &str,
    round: u32,
    result: Option<&ToolResultBlock>,
    container_width: u16,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let is_err = result.map(|r| r.is_error).unwrap_or(false);
    let pending = result.is_none();
    let (glyph, glyph_color) = match (pending, is_err) {
        (true, _) => ("⟳", palette.tool_accent),
        (false, true) => ("⚠", palette.compaction),
        (false, false) => ("✓", palette.tool_accent),
    };

    let name_color = if is_err { palette.compaction } else { palette.tool_accent };
    let dim = if is_err { palette.compaction } else { palette.tool_dim };

    // Argument summary clipped tight — we want the gesture readable, not
    // the API surface. Reserve room for glyph + name + round suffix.
    let reserved = name.chars().count() + 14;
    let arg_budget = (container_width as usize)
        .saturating_sub(reserved + 6)
        .max(20)
        .min(120);
    let args_summary = clip(&summarize_tool_args(arguments), arg_budget);

    let mut spans: Vec<Span<'static>> = vec![
        Span::raw("  "),
        Span::styled(glyph.to_string(), Style::default().fg(glyph_color)),
        Span::raw(" "),
        Span::styled(
            name.to_string(),
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
    ];
    if !args_summary.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(dim)));
        spans.push(Span::styled(args_summary, Style::default().fg(dim)));
    }
    if round > 1 {
        spans.push(Span::styled(
            format!("  ·  r{}", round),
            Style::default().fg(dim).add_modifier(Modifier::DIM),
        ));
    }

    let mut out = vec![Line::from(spans)];

    // On error, surface the first line of the failure inline — the agent
    // (and the user) need to feel that the gesture didn't land.
    if let Some(r) = result {
        if r.is_error {
            if let Some(first_line) = r.output.lines().next() {
                let trimmed = first_line.trim();
                if !trimmed.is_empty() {
                    let inner = (container_width as usize).saturating_sub(8).max(20);
                    let preview = clip(trimmed, inner);
                    out.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            preview,
                            Style::default().fg(palette.compaction).add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            }
        }
    }

    out
}

/// Compact one-line summary of tool arguments. Keys are kept, long string
/// values are clipped to 60 chars with an ellipsis. Falls back to the raw
/// string if parsing fails.
fn summarize_tool_args(arguments: &str) -> String {
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(arguments);
    match parsed {
        Ok(serde_json::Value::Object(map)) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => clip(s, 60),
                        other => clip(&other.to_string(), 60),
                    };
                    format!("{}: {}", k, s)
                })
                .collect();
            parts.join("  ·  ")
        }
        Ok(other) => clip(&other.to_string(), 120),
        Err(_) => clip(arguments, 120),
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Take the first `n` lines verbatim for in-card preview.
fn preview_output(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().take(n).collect();
    lines.join("\n")
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
    // Border colour subtly shifts when the agent is busy so the user sees the
    // chat is "warm" without losing the ability to type. The actual phase
    // status (Thinking / Tool / Streaming) lives in `draw_phase()` above.
    let border_color = if state.busy {
        let phase = (state.tick as f32 / 8.0).sin().abs();
        lerp_color(state.palette.agent_dim, state.palette.agent_primary, phase)
    } else {
        state.palette.agent_primary
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    let cursor_visible = (state.tick / 5) % 2 == 0;
    let cursor_ch: &str = if cursor_visible { "▏" } else { " " };
    let inner_width = (area.width as usize).saturating_sub(5).max(1);

    let (prefix_str, prefix_color) = match state.render_mode {
        ChatMode::Conversation => (" › ", state.palette.agent_primary),
        ChatMode::Code => (" ≡ ", state.palette.tool_accent),
    };
    let prefix_style = Style::default().fg(prefix_color).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let logical: Vec<&str> = state.input.split('\n').collect();

    for (li, logical_line) in logical.iter().enumerate() {
        let wrapped = wrap_words(logical_line, inner_width);
        for (wi, chunk) in wrapped.iter().enumerate() {
            let prefix: Span<'static> = if li == 0 && wi == 0 {
                Span::styled(prefix_str.to_string(), prefix_style)
            } else {
                Span::raw("   ")
            };
            let is_last = li == logical.len() - 1 && wi == wrapped.len() - 1;
            let mut spans = vec![prefix, Span::styled(chunk.clone(), Style::default().fg(Color::White))];
            if is_last {
                spans.push(Span::styled(cursor_ch.to_string(), Style::default().fg(prefix_color)));
            }
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(prefix_str.to_string(), prefix_style),
            Span::styled(cursor_ch.to_string(), Style::default().fg(prefix_color)),
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

/// Draw the /btw fork pane — a floating bordered panel in the center of the
/// screen showing the forked conversation's streaming response. Mirrors
/// Letta's BtwPane component.
fn draw_btw_pane(f: &mut Frame, state: &ChatState, area: Rect) {
    let pane_w = (area.width * 3 / 4).max(40).min(area.width.saturating_sub(6));
    let pane_h = (area.height * 3 / 5).max(12).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(pane_w)) / 2;
    let y = (area.height.saturating_sub(pane_h)) / 2;
    let pane_area = Rect { x, y, width: pane_w, height: pane_h };

    f.render_widget(Clear, pane_area);

    let (title, body, border_color) = match &state.btw_state {
        BtwState::Forking { question } => {
            let spinner = SPINNER[(state.tick as usize / 2) % SPINNER.len()];
            (
                format!(" btw — {} ", question.chars().take(40).collect::<String>()),
                vec![Line::from(vec![
                    Span::styled(format!(" {} forking...", spinner), Style::default().fg(state.palette.agent_dim)),
                ])],
                state.palette.agent_primary,
            )
        }
        BtwState::Streaming { question, response_so_far } => {
            let truncated: String = response_so_far.chars().take(800).collect();
            let q_label = question.chars().take(40).collect::<String>();
            (
                format!(" btw — {} ", q_label),
                vec![Line::from(Span::styled(
                    truncated,
                    Style::default().fg(Color::White),
                ))],
                state.palette.tool_accent,
            )
        }
        BtwState::Complete { question, response, forked_id } => {
            let truncated: String = response.chars().take(800).collect();
            let q_label = question.chars().take(40).collect::<String>();
            let fork_label = if forked_id.is_empty() {
                String::new()
            } else {
                format!(" fork: {}", &forked_id[..forked_id.len().min(8)])
            };
            (
                format!(" btw — {} {}", q_label, fork_label),
                vec![
                    Line::from(Span::styled(
                        truncated,
                        Style::default().fg(Color::White),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "[esc] dismiss  ·  [j] jump to fork",
                        Style::default().fg(state.palette.agent_dim),
                    )),
                ],
                state.palette.surfacing,
            )
        }
        BtwState::Error { question, error } => {
            let q_label = question.chars().take(40).collect::<String>();
            (
                format!(" btw — {} ", q_label),
                vec![Line::from(Span::styled(
                    format!(" Error: {}", error),
                    Style::default().fg(state.palette.compaction),
                ))],
                state.palette.compaction,
            )
        }
        BtwState::Idle => unreachable!(), // draw_btw_pane is only called when non-idle
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color).add_modifier(Modifier::BOLD));

    let para = Paragraph::new(body).block(block).alignment(Alignment::Left);
    f.render_widget(para, pane_area);
}

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
                let sel_fg = state.palette.agent_primary;
                let sel_bg = state.palette.bg;
                let style = if sel {
                    Style::default().fg(sel_fg).bg(sel_bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let hint_style = if sel {
                    Style::default().fg(state.palette.agent_dim).bg(sel_bg)
                } else {
                    Style::default().fg(state.palette.agent_dim)
                };
                Line::from(vec![
                    Span::styled(format!(" {} ", cmd.name), style),
                    Span::styled(format!(" {}", cmd.hint), hint_style),
                ])
            }).collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_dim));
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
                Style::default().fg(state.palette.agent_dim).add_modifier(Modifier::ITALIC),
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
                    Style::default().fg(state.palette.agent_primary).bg(state.palette.bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(truncated, style)));
            }

            let block = Block::default()
                .title(Span::styled(
                    " Resume ",
                    Style::default().fg(state.palette.agent_primary).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_primary));
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

    // Thinking pane — interleave blank separators between entries so
    // consecutive reasoning blocks don't run into each other visually.
    // Round-separator sentinel lines (pushed by ToolCall handler) render
    // dimmer so the eye finds the break without it being loud.
    let thinking_entries: Vec<&String> = state
        .thinking
        .iter()
        .rev()
        .take(panes[0].height as usize)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut thinking_view: Vec<Line<'static>> = Vec::with_capacity(thinking_entries.len() * 2);
    for (i, t) in thinking_entries.iter().enumerate() {
        if t.starts_with("────") {
            if i > 0 {
                thinking_view.push(Line::from(""));
            }
            thinking_view.push(Line::from(Span::styled(
                t.to_string(),
                Style::default().fg(state.palette.agent_dim),
            )));
            thinking_view.push(Line::from(""));
        } else {
            thinking_view.push(Line::from(Span::styled(
                format!("· {}", t),
                Style::default().fg(state.palette.agent_dim),
            )));
        }
    }
    let thinking = Paragraph::new(thinking_view)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_dim))
                .title(Span::styled(" thinking ", Style::default().fg(state.palette.agent_dim).add_modifier(Modifier::BOLD))),
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
            let base = entry.color(&state.palette);
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
                .border_style(Style::default().fg(state.palette.surfacing))
                .title(Span::styled(" subconscious ", Style::default().fg(state.palette.surfacing).add_modifier(Modifier::BOLD))),
        );
    f.render_widget(subconscious, panes[1]);
}

fn draw_footer(f: &mut Frame, state: &ChatState, area: Rect) {
    let pressure_pct = (state.pressure * 100.0) as u16;
    let pressure_label = format!("ctx {}%", pressure_pct);
    let cockpit_hint = if state.cockpit { "Tab close cockpit" } else { "Tab cockpit" };
    let tool_hint = if state.tool_cards_expanded { "t collapse tools" } else { "t expand tools" };
    let esc_hint = if state.busy { "Esc interrupt" } else { "Esc menu" };
    let posture_label = match state.render_mode {
        ChatMode::Conversation => "chat",
        ChatMode::Code => "code",
    };
    let mut spans = vec![
        Span::styled(
            format!(" {esc_hint} · Enter send · S-Ret ↵ · ↑↓ scroll · {cockpit_hint} · {tool_hint} "),
            Style::default().fg(state.palette.agent_dim),
        ),
        Span::raw("│  "),
        Span::styled(
            format!("posture {posture_label}"),
            Style::default().fg(
                if state.render_mode == ChatMode::Code { state.palette.tool_accent } else { state.palette.agent_dim },
            ),
        ),
        Span::raw("│  "),
        Span::styled(format!("conv {}", short(&state.conversation_id)), Style::default().fg(state.palette.agent_dim)),
        Span::raw("  │  "),
        Span::styled(pressure_label, Style::default().fg(state.palette.agent_dim)),
    ];
    if state.scroll > 0 {
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled(
            format!("↓ {} below", state.scroll),
            Style::default().fg(state.palette.agent_primary),
        ));
    }
    let footer = Line::from(spans);
    f.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

fn short(s: &str) -> String {
    if s.len() <= 8 { s.to_string() } else { s[..8].to_string() }
}
