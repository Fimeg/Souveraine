//! Souveraine - Full Terminal UI
//! Splash → Welcome → Dashboard / Chat / etc.
//!
//! The `App` holds a `Scene` which dispatches `TuiEvent` variants to all
//! registered `Component`s. Components are extracted here incrementally.
//! Existing draw methods remain until their panels become proper Components.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::core::config::ConsciousnessConfig;
use crate::ui::chat::{ChatState, draw as draw_chat};
use crate::ui::cockpit_panel::CockpitPane;
use crate::ui::health_panel::HealthPane;
use crate::ui::presence::{Posture, Presence, draw_overlay as draw_presence_overlay};
use crate::ui::color_support::rgb;
use crate::ui::component::{Component, Scene, SceneLayout, TuiEvent};
use crate::ui::setup::{SetupFlow, SetupState};
use crate::backend::BackendEvent;
use crate::ui::settings::SettingsAction;

use ratatui_image::{picker::Picker, protocol::{Protocol, StatefulProtocol}, Image, Resize, StatefulImage};

#[cfg(feature = "figlet-rs")]
use figlet_rs::FIGlet;

pub struct App {
    current_screen: Screen,
    splash_start: Instant,
    menu_selected: usize,
    /// Setup wizard state (None = wizard not active).
    setup_state: Option<SetupState>,
    /// Advisory hint shown on the Welcome screen (from BootstrapPlan).
    welcome_hint: Option<String>,
    agent_status: AgentStatus,
    should_quit: bool,
    config: Arc<RwLock<ConsciousnessConfig>>,
    chat: Option<ChatState>,
    /// Set when chat connect fails so we can surface the error in the menu.
    chat_error: Option<String>,
    /// Schedules editor (Cron screen). Lazily constructed on first entry.
    schedules: Option<crate::ui::schedules::SchedulesView>,
    /// Agent name preference (from `--agent` CLI flag).
    agent_pref: String,
    /// Annie Composite made felt — body channel for the running agent.
    /// See `src/ui/presence.rs` for the state model and event subscriptions.
    presence: Presence,
    /// Available agent names (for the manager / selection).
    available_agents: Vec<String>,
    /// The component scene — owns event dispatch and layout.
    scene: Scene,
    /// Monotonic tick counter, incremented each frame.
    tick: u64,
    /// Splash bloom animation state.
    bloom: crate::ui::animation::bloom::BloomState,
    /// Terminal image renderer (kitty/sixel/halfblock). Queried once after
    /// entering alternate screen. None before initialization.
    image_picker: Option<Picker>,
    /// Pre-processed portrait for the current agent. Loaded alongside the
    /// half-block PortraitSource; when set, Welcome / Presence / Dashboard
    /// render the real photo instead of pixel art.
    image_protocol: Option<Protocol>,
    /// Currently highlighted card index in the Agent Manager grid.
    manager_selected: usize,
    /// Column count last computed by draw_agent_cards_mut — used by key handler.
    manager_cols: usize,
    /// Per-agent cards for the AgentsManager screen.
    agent_cards: Vec<AgentCard>,
    /// Per-agent stateful image protocols for the Agent Manager card grid.
    /// Keyed by agent id (the dir name under `~/.souveraine/agents/`). Each
    /// protocol owns its own resize state so multiple cards can render at
    /// different sizes simultaneously without conflicting. Lazily populated
    /// when the manager is opened; survives Esc → reopen.
    card_images: HashMap<String, StatefulProtocol>,
    /// Raw source images for card portraits, used for cover-fill scaling
    /// at render time. Stores the decoded DynamicImage after 2:3 crop.
    /// The lifecyle matches `card_images` — populated by `load_card_image`
    /// and trimmed by `refresh_card_images`.
    raw_card_images: HashMap<String, image::DynamicImage>,
    /// Cover-scaled protocols keyed by (agent_id, area_width, area_height).
    /// Lazily created from `raw_card_images` and reused across frames when
    /// the card area hasn't changed. Cleared when the source image changes.
    cover_protocols: HashMap<String, (u16, u16, StatefulProtocol)>,
    /// Expression image cache for the animated portrait system.
    /// A zero-cost abstraction: only hit when `expressions/` directory exists
    /// in the agent's assets. Otherwise slides silently to portrait fallback.
    expression_cache: crate::ui::expressions::ExpressionCache,
    /// RGP 3D portrait — active only when running inside ratty terminal.
    rgp_portrait: Option<crate::ui::rgp::Graphic>,
    /// Whether the Ratty Graphics Protocol is available.
    rgp_available: bool,
    /// Settings editor state. Lazily constructed on first Settings screen entry.
    settings: Option<crate::ui::settings::SettingsView>,
    /// Path to the loaded config file, for save-back. Discovered at startup.
    config_path: Option<PathBuf>,

    // ── Voice channel ────────────────────────────────────────────────────
    // Present only when voice.enabled = true in config. None otherwise.

    /// HTTP client for STT/TTS services. Created on Presence entry when
    /// voice is enabled; shared across turns within a Presence session.
    voice_client: Option<crate::core::voice::VoiceClient>,
    /// Active mic capture while the user holds Space. Dropped on release.
    voice_capture: Option<crate::ui::voice::MicCapture>,
    /// Rodio player for TTS audio. Held for the lifetime of the Presence
    /// session so the output device stays open between turns.
    voice_player: Option<crate::ui::voice::VoicePlayer>,
    /// Pending STT + submit + TTS pipeline result. Receives the mp3 bytes
    /// (or an error string to inject as the transcription).
    voice_tts_rx: Option<tokio::sync::oneshot::Receiver<Result<Vec<u8>, String>>>,
    /// Pending STT transcription result (received before TTS starts).
    voice_stt_rx: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    /// Track which reply text we last submitted to TTS, so we don't
    /// synthesize the same reply twice if the tick loop fires multiple times
    /// before the TTS receiver is polled.
    voice_last_synthesized: Option<String>,
    /// Rolling waveform buffer — keeps the last N mic level samples
    /// for the scrolling waveform visualizer.
    voice_waveform: Vec<f32>,
    /// Most recent TTS text, for replay/regen.
    voice_last_tts_text: Option<String>,
    /// Most recent TTS mp3 bytes, for replay/save.
    voice_last_tts_bytes: Option<Vec<u8>>,
    /// When the last TTS playback finished.
    voice_last_tts_time: Option<Instant>,
    /// Last STT transcript text (what the user said), for display.
    voice_last_transcript: Option<String>,
    /// Text to force-re-synthesize on regen (set by 'g' key, consumed by TTS loop).
    tts_last_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Screen {
    Splash,
    /// First-run setup wizard — parameterized by which flow we're in.
    Setup,
    /// Home screen — dashboard data, portrait, and menu in one place.
    Welcome,
    Chat,
    Therapy,
    AgentTime,
    Cron,
    Settings,
    /// "Be with her" mode — fullscreen breathing portrait, no chat input.
    Presence,
    /// Agent manager — card grid with per-agent data (seed, uptime, etc.).
    AgentsManager,
}

/// Per-agent card data shown in the manager grid. Populated on entry to
/// the manager screen via `refresh_agent_cards`.
#[derive(Debug, Clone)]
pub struct AgentCard {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 4-glyph SeedID badge (`SeedId::glyph()`).
    pub glyph: String,
    /// First 8 hex chars of the pubkey, for copy-paste.
    pub pubkey_prefix: String,
    pub instance_count: i64,
    /// Cap at 99 in display per UX spec — humans distrust 100% liveness.
    pub uptime_pct: u8,
    pub memory_count: usize,
    pub created: String,
}

#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub name: String,
    pub mood: String,
    pub energy: u8,
    pub memory_commits: u32,
    pub pending_tasks: usize,
    pub subconscious_active: bool,
    /// Mode the dashboard data was fetched through (local / remote / —).
    pub mode: String,
    /// Last commit hash (short) on the agent's memory repo, if known.
    pub last_commit: Option<String>,
    /// Recent commit subject lines, oldest → newest.
    pub recent_activity: Vec<String>,
    /// Number of agents the backend reports.
    pub agent_count: usize,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self {
            name: "Ani".to_string(),
            mood: "—".to_string(),
            energy: 0,
            memory_commits: 0,
            pending_tasks: 0,
            subconscious_active: false,
            mode: "—".to_string(),
            last_commit: None,
            recent_activity: Vec::new(),
            agent_count: 0,
        }
    }
}

impl App {
    pub fn new(config: Arc<RwLock<ConsciousnessConfig>>, agent_pref: String, config_path: Option<PathBuf>) -> Self {
        info!("Creating Souveraine App");
        let mut app = Self {
            current_screen: Screen::Splash,
            splash_start: Instant::now(),
            menu_selected: 0,
            setup_state: None,
            welcome_hint: None,
            agent_status: AgentStatus { name: agent_pref.clone(), ..AgentStatus::default() },
            should_quit: false,
            config,
            chat: None,
            chat_error: None,
            schedules: None,
            agent_pref: agent_pref.clone(),
            presence: Presence::new(&agent_pref),
            available_agents: Vec::new(),
            scene: Scene::new(SceneLayout::Single),
            tick: 0,
            bloom: crate::ui::animation::bloom::BloomState::new(),
            manager_selected: 0,
            manager_cols: 4,
            card_images: HashMap::new(),
            raw_card_images: HashMap::new(),
            cover_protocols: HashMap::new(),
            expression_cache: crate::ui::expressions::ExpressionCache::new(),
            rgp_portrait: None,
            rgp_available: crate::ui::rgp::is_available(),
            settings: None,
            config_path,
            image_picker: None,
            image_protocol: None,
            agent_cards: Vec::new(),
            voice_client: None,
            voice_capture: None,
            voice_player: None,
            voice_tts_rx: None,
            voice_stt_rx: None,
            voice_last_synthesized: None,
            voice_waveform: Vec::with_capacity(128),
            voice_last_tts_text: None,
            voice_last_tts_bytes: None,
            voice_last_tts_time: None,
            voice_last_transcript: None,
            tts_last_text: None,
        };

        // CockpitPane listens for subconscious's surfacing events as scrollable text.
        // Presence (Annie's body channel) lives outside the Scene because it's
        // an overlay, not a zoned component — App feeds it events via `dispatch`.
        app.scene.add(CockpitPane::new());
        // HealthPane sits below the cockpit in the sidebar — substrate vitals
        // (pressure, backend, N+1 cadence, 504 strain) beneath her voice.
        app.scene.add(HealthPane::new());

        app
    }

    /// Dispatch a TuiEvent to every listener: scene components AND Presence.
    /// Returns true if a redraw is needed.
    fn dispatch(&mut self, event: TuiEvent) -> bool {
        let scene_dirty = self.scene.event_all(&event);
        let presence_dirty = self.presence.handle_event(&event);
        // Keep all component palettes in sync with atmosphere changes.
        if matches!(event, TuiEvent::AtmosphereChanged(_)
            | TuiEvent::MoodChanged(_)
            | TuiEvent::SubconsciousPass(_)
            | TuiEvent::PressureChanged(_)
        ) {
            self.sync_palette();
        } else if matches!(event, TuiEvent::Tick(_)) && self.presence.lerp_t < 1.0 {
            self.sync_palette();
        }
        scene_dirty || presence_dirty
    }

    /// Recompute the palette from the presence's current (possibly lerped)
    /// atmosphere and push it to every component that owns one.
    fn sync_palette(&mut self) {
        let (p, s, d, b) = self.presence.lerped_colors();
        let palette = crate::ui::chat::ChatPalette::from_colors(p, s, d, b);
        if let Some(ref mut chat) = self.chat {
            chat.palette = palette;
        }
        if let Some(ref mut view) = self.schedules {
            view.palette = palette;
        }
        if let Some(ref mut view) = self.settings {
            view.palette = palette;
        }
    }

    /// Add an available agent for selection (WIP - called from backend discovery)
    pub fn add_available_agent(&mut self, agent_name: String) {
        if !self.available_agents.contains(&agent_name) {
            self.available_agents.push(agent_name);
        }
    }

    /// Detach the UI from the current agent and attach to another.
    /// The agent itself keeps running — this only tears down the view
    /// layer so each subsystem reconnects fresh on next entry.
    pub fn select_agent(&mut self, agent_name: &str) {
        let changed = self.agent_pref != agent_name;
        self.agent_pref = agent_name.to_string();
        self.agent_status.name = agent_name.to_string();

        if changed {
            self.chat = None;
            self.chat_error = None;
            self.settings = None;
            self.schedules = None;
            self.presence = Presence::new(agent_name);
            self.image_protocol = None;
            self.rgp_portrait = None;

            self.voice_capture = None;
            self.voice_tts_rx = None;
            self.voice_stt_rx = None;
            self.voice_last_synthesized = None;
            self.voice_waveform.clear();
            self.voice_last_tts_text = None;
            self.voice_last_tts_bytes = None;
            self.voice_last_transcript = None;
            self.tts_last_text = None;
        }

        self.dispatch(TuiEvent::AgentSelected(agent_name.to_string()));
    }

    /// Populate `agent_cards` and `card_images` from disk. Idempotent —
    /// run once after the image picker is ready (so Welcome can pull a
    /// portrait from the cache). Subsequent calls skip the backend round-trip
    /// (which would create extra server instances) and only refresh card images.
    async fn ensure_agent_cards_loaded(&mut self) {
        if self.agent_cards.is_empty() {
            let cfg = self.config.read().await.clone();
            let agents = Self::fetch_agent_cards(cfg).await;
            self.agent_cards = agents;
            if !self.agent_cards.is_empty() {
                self.available_agents = self.agent_cards.iter().map(|c| c.name.clone()).collect();
            }
        }
        self.refresh_card_images();
    }

    /// Open the agent manager — shows per-agent cards with seed glyph,
    /// instance count, uptime, memory count.
    async fn open_agent_manager(&mut self) {
        self.ensure_agent_cards_loaded().await;
        if self.agent_cards.is_empty() {
            self.available_agents = vec!["Annie".to_string(), "Ani".to_string()];
        }
        self.current_screen = Screen::AgentsManager;
        self.dispatch(TuiEvent::ScreenChanged(Screen::AgentsManager));
    }

    /// Cycle through available agents for selection (WIP)
    fn cycle_agent_selection(&mut self) {
        if self.available_agents.is_empty() {
            // No agents available yet - create a default alias
            // This is WIP - will be expanded with full agent creation flow
            let default_agents = vec!["Ani".to_string(), "JeanLuc".to_string(), "Eione".to_string()];
            for agent in default_agents {
                self.add_available_agent(agent);
            }
        }

        // Clone the agent name to avoid borrow checker issues
        let agent_to_select = if let Some(current_idx) = self.available_agents.iter().position(|a| a == &self.agent_pref) {
            let next_idx = (current_idx + 1) % self.available_agents.len();
            self.available_agents[next_idx].clone()
        } else if !self.available_agents.is_empty() {
            self.available_agents[0].clone()
        } else {
            return;
        };

        self.select_agent(&agent_to_select);
    }

    pub async fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Detect RGP (Ratty Graphics Protocol) for inline 3D.
        if self.rgp_available {
            tracing::info!("ratty terminal detected — RGP 3D graphics available");
        }

        // Detect terminal image protocol (kitty/sixel/halfblock) after the
        // alternate screen is active. Non-fatal: falls back to halfblocks.
        if let Ok(picker) = Picker::from_query_stdio() {
            tracing::info!(protocol = ?picker.protocol_type(), "image picker initialized");
            self.image_picker = Some(picker);
            // Eagerly load every agent's portrait into the stateful card-image
            // cache. Welcome looks up by current agent name; the Manager pulls
            // all of them. One load per session, re-encoded per render area.
            self.ensure_agent_cards_loaded().await;
        } else {
            tracing::info!("no image protocol detected — using halfblocks");
        }

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(33);  // ~30 FPS cap

        while !self.should_quit {
            // Drain any pending backend events every iteration so streaming
            // tokens don't backlog — even between frames.
            if let Some(chat) = self.chat.as_mut() {
                chat.drain_events();

                // Forward consciousness events (surfacing, reflection, archivist)
                // from chat to the scene so subconscious's observations reach Components.
                let drained: Vec<BackendEvent> = chat.pending_consciousness.drain(..).collect();
                for ev in drained {
                    match ev {
                        BackendEvent::Surfacing { source, content, priority } => {
                            self.dispatch(TuiEvent::Surfacing { source, content, priority });
                        }
                        BackendEvent::Reflection(content) => {
                            self.dispatch(TuiEvent::Reflection { content });
                        }
                        BackendEvent::Archivist { synthesis, pressure } => {
                            self.dispatch(TuiEvent::Archivist { synthesis, pressure });
                        }
                        BackendEvent::CompactionWarning { pressure, tier } => {
                            self.dispatch(TuiEvent::CompactionWarning { pressure, tier });
                        }
                        BackendEvent::ContextPressure(p) => {
                            self.dispatch(TuiEvent::PressureChanged(p));
                        }
                        BackendEvent::InferenceStrain { attempt, status, .. } => {
                            self.dispatch(TuiEvent::InferenceStrain { attempt, status });
                        }
                        BackendEvent::Atmosphere(preset) => {
                            self.dispatch(TuiEvent::AtmosphereChanged(preset));
                        }
                        BackendEvent::SubconsciousPass(active) => {
                            self.dispatch(TuiEvent::SubconsciousPass(active));
                        }
                        BackendEvent::Outfit(name) => {
                            self.dispatch(TuiEvent::OutfitChanged(name));
                        }
                        _ => {}
                    }
                }
            }

            // Tick + draw capped at 30 FPS — mouse events no longer
            // accelerate animations.
            if last_tick.elapsed() >= tick_rate {
                if let Some(chat) = self.chat.as_mut() {
                    chat.advance_tick();
                }
                self.tick = self.tick.wrapping_add(1);
                self.dispatch(TuiEvent::Tick(self.tick));

                terminal.draw(|f| self.draw(f))?;
                last_tick = Instant::now();
            }

            // Responsive input polling — 16 ms timeout means mouse movement
            // is consumed but doesn't force a redraw; keyboard feels instant.
            if crossterm::event::poll(Duration::from_millis(16))? {
                let crossterm_event = event::read()?;
                match crossterm_event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        // Dispatch to listeners first, then handle App-level keys
                        let handled = self.dispatch(TuiEvent::Key(key));
                        if !handled {
                            self.handle_key(key).await;
                        }
                    }
                    Event::Resize(w, h) => {
                        self.dispatch(TuiEvent::Resize { width: w, height: h });
                        self.scene.layout = match self.current_screen {
                            Screen::Chat => SceneLayout::ChatWithSidebar {
                                sidebar_ratio: 0.3,
                                sidebar_open: false,
                            },
                            Screen::Splash => SceneLayout::Single,
                            _ => SceneLayout::Single,
                        };
                    }
                    Event::Mouse(m) => {
                        // Left-click on a message bubble copies it to the
                        // clipboard (the ⧉ in the bubble title is the cue).
                        if self.current_screen == Screen::Chat
                            && matches!(
                                m.kind,
                                crossterm::event::MouseEventKind::Down(
                                    crossterm::event::MouseButton::Left
                                )
                            )
                        {
                            if let Some(chat) = self.chat.as_mut() {
                                chat.copy_message_at(m.column, m.row);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // ── Voice pipeline progress ──────────────────────────────────
            // Poll the STT and TTS oneshot receivers each tick to advance
            // the state machine without blocking the render loop.
            if self.current_screen == Screen::Presence {
                self.advance_voice_pipeline().await;
            }

            // ── Setup wizard model fetch ────────────────────────────────
            if self.current_screen == Screen::Setup {
                if let Some(ref mut setup) = self.setup_state {
                    setup.poll_models();
                }
            }

            // ── Settings model fetch ─────────────────────────────────────
            if self.current_screen == Screen::Settings {
                if let Some(view) = self.settings.as_mut() {
                    view.poll_models_rx();
                }
            }

            if self.current_screen == Screen::Splash {
                if self.splash_start.elapsed() > Duration::from_secs(8) {
                    self.transition_from_splash().await;
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.current_screen {
            Screen::Splash => {
                self.transition_from_splash().await;
            }
            Screen::Setup => {
                // Capture the setup_complete and step_was_welcome flags before
                // calling any method that borrows self.setup_state, to avoid a
                // borrow conflict with finish_setup() taking &mut self.
                let should_skip = matches!(key.code, KeyCode::Esc);
                let step_is_welcome = self.setup_state.as_ref()
                    .map(|s| s.step == crate::ui::setup::SetupStep::Welcome)
                    .unwrap_or(false);

                if should_skip && step_is_welcome {
                    self.finish_setup().await;
                    return;
                }
                if let Some(ref mut setup) = self.setup_state {
                    setup.handle_key(key);
                }
                if self.setup_state.as_ref().map(|s| s.complete).unwrap_or(false) {
                    self.finish_setup().await;
                }
            }
            Screen::Welcome => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Up => if self.menu_selected > 0 { self.menu_selected -= 1; }
                    KeyCode::Down => if self.menu_selected < 4 { self.menu_selected += 1; }
                    KeyCode::Enter => self.select_menu_item().await,
                    KeyCode::Char('a') => {
                        // WIP: Create agent alias - this will be expanded with a full agent creation flow
                        // For now, cycle through available agents or create a default
                        self.cycle_agent_selection();
                    }
                    KeyCode::Char('p') => {
                        // Presence mode — sit with her, voice loop if enabled.
                        // Chat must be initialized so voice submissions route through
                        // the agent via the same path as typed messages.
                        if self.chat.is_none() {
                            match ChatState::connect(self.config.clone(), &self.agent_pref).await {
                                Ok(c) => self.chat = Some(c),
                                Err(e) => {
                                    tracing::warn!(error = %e, "failed to init chat for Presence");
                                }
                            }
                        }
                        self.preload_agent_expressions().await;
                        self.init_voice_session().await;
                        self.current_screen = Screen::Presence;
                        self.dispatch(TuiEvent::ScreenChanged(Screen::Presence));
                    }
                    KeyCode::Char('i') => {
                        self.open_agent_manager().await;
                    }
                    _ => {}
                }
            }
            Screen::Presence => {
                match key.code {
                    // Esc: interrupt Speaking → Idle; or exit Presence when Idle.
                    KeyCode::Esc => {
                        if self.presence.posture == Posture::Speaking {
                            if let Some(player) = &self.voice_player {
                                player.stop();
                            }
                            self.presence.posture = Posture::Idle;
                            self.presence.sync_atmosphere_pub();
                        } else if matches!(
                            self.presence.posture,
                            Posture::Listening | Posture::Thinking | Posture::Processing
                        ) {
                            // Interrupt in-flight voice turn — drop capture, cancel pipeline.
                            self.voice_capture = None;
                            self.voice_stt_rx = None;
                            self.voice_tts_rx = None;
                            self.presence.posture = Posture::Idle;
                            self.presence.sync_atmosphere_pub();
                        } else {
                            // Idle — exit Presence.
                            self.exit_presence();
                        }
                    }
                    // Space: tap-to-record. First press opens mic, second press
                    // closes and sends. Esc cancels. No release events needed —
                    // terminals drop them.
                    KeyCode::Char(' ') => {
                        if self.presence.posture == Posture::Listening {
                            // Already recording — second press: send.
                            self.handle_presence_space_release().await;
                        } else if self.presence.posture == Posture::Speaking {
                            // Space during Speaking: stop, start a new listen.
                            if let Some(player) = &self.voice_player {
                                player.stop();
                            }
                            self.start_listening();
                        } else if self.voice_client.is_some() {
                            // First press: open mic.
                            self.start_listening();
                        }
                    }
                    // Any other key exits Presence (meditative mode).
                    KeyCode::Char('q') => self.exit_presence(),
                    // Vocal Recall keys: r = replay, g = regen, s = save
                    KeyCode::Char('r') => {
                        if let Some(bytes) = self.voice_last_tts_bytes.clone() {
                            if let Some(player) = &self.voice_player {
                                player.stop();
                                let _ = player.play_mp3(bytes);
                                self.presence.set_posture(Posture::Speaking);
                            }
                        }
                    }
                    KeyCode::Char('g') => {
                        if let Some(text) = self.voice_last_tts_text.clone() {
                            self.voice_last_synthesized = None; // force re-synth
                            self.tts_last_text = Some(text);    // stash for the pipeline
                        }
                    }
                    KeyCode::Char('s') => {
                        if let (Some(bytes), Some(text)) = (self.voice_last_tts_bytes.as_ref(), self.voice_last_tts_text.as_ref()) {
                            let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                            let filename = format!("voice-recording-{}.mp3", timestamp);
                            if let Err(e) = std::fs::write(&filename, bytes) {
                                tracing::warn!(error = %e, "failed to save voice recording");
                            } else {
                                tracing::info!(file = %filename, "voice recording saved");
                            }
                            let _ = text;
                        }
                    }
                    _ => {
                        // If Idle and no voice activity, treat as "go back."
                        if self.presence.posture == Posture::Idle
                            && self.voice_capture.is_none()
                        {
                            self.exit_presence();
                        }
                    }
                }
            }
            Screen::Chat => self.handle_chat_key(key).await,
            Screen::Cron => self.handle_schedules_key(key),
            Screen::Settings => self.handle_settings_key(key).await,
            Screen::AgentsManager => {
                let n = self.agent_cards.len();
                let cols = self.manager_cols.max(1);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('i') => {
                        self.current_screen = Screen::Welcome;
                    }
                    KeyCode::Left => {
                        if self.manager_selected > 0 {
                            self.manager_selected -= 1;
                        }
                    }
                    KeyCode::Right => {
                        if n > 0 && self.manager_selected + 1 < n {
                            self.manager_selected += 1;
                        }
                    }
                    KeyCode::Up => {
                        if self.manager_selected >= cols {
                            self.manager_selected -= cols;
                        }
                    }
                    KeyCode::Down => {
                        if n > 0 && self.manager_selected + cols < n {
                            self.manager_selected += cols;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(card) = self.agent_cards.get(self.manager_selected) {
                            let name = card.name.clone();
                            self.select_agent(&name);
                            self.refresh_dashboard().await;
                            self.current_screen = Screen::Welcome;
                        }
                    }
                    // f — pin/star selected card as favorite primary without leaving the manager
                    KeyCode::Char('f') => {
                        if let Some(card) = self.agent_cards.get(self.manager_selected) {
                            let name = card.name.clone();
                            self.select_agent(&name);
                            self.refresh_dashboard().await;
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('m') => {
                        self.current_screen = Screen::Welcome;
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_schedules_key(&mut self, key: crossterm::event::KeyEvent) {
        use crate::ui::schedules::{CreateField, Mode};

        let Some(view) = self.schedules.as_mut() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

        // Status banners absorb the next key — clear and continue.
        if matches!(view.mode, Mode::Saved(_) | Mode::Error(_)) {
            view.clear_status();
            return;
        }

        match &mut view.mode {
            Mode::Browse => match key.code {
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('m') => {
                    self.current_screen = Screen::Welcome;
                }
                KeyCode::Char('j') | KeyCode::Down => view.move_down(),
                KeyCode::Char('k') | KeyCode::Up => view.move_up(),
                KeyCode::Char('e') => view.toggle_enabled(),
                KeyCode::Char('d') => view.confirm_delete(),
                KeyCode::Char('r') => view.trigger_run(),
                KeyCode::Char('c') => view.open_create(),
                KeyCode::Char('R') => view.reload(),
                _ => {}
            },
            Mode::ConfirmDelete => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => view.execute_delete(),
                _ => view.cancel_delete(),
            },
            Mode::Create(form) => match key.code {
                KeyCode::Esc => view.cancel_create(),
                KeyCode::Enter => view.save_create(),
                KeyCode::Tab | KeyCode::Down => form.next_field(),
                KeyCode::BackTab | KeyCode::Up => form.prev_field(),
                KeyCode::Backspace => match form.field {
                    CreateField::Name => { form.name.pop(); }
                    CreateField::Schedule => { form.schedule.pop(); }
                    CreateField::Prompt => { form.prompt.pop(); }
                    _ => {}
                },
                KeyCode::Left => match form.field {
                    CreateField::Kind => {
                        form.kind = match form.kind {
                            crate::core::nervous::cron::ScheduleKind::Once =>
                                crate::core::nervous::cron::ScheduleKind::Cron,
                            crate::core::nervous::cron::ScheduleKind::Interval =>
                                crate::core::nervous::cron::ScheduleKind::Once,
                            crate::core::nervous::cron::ScheduleKind::Cron =>
                                crate::core::nervous::cron::ScheduleKind::Interval,
                        };
                    }
                    CreateField::Urgency => {
                        form.urgency = (form.urgency - 0.1).max(0.0);
                    }
                    _ => {}
                },
                KeyCode::Right => match form.field {
                    CreateField::Kind => {
                        form.kind = match form.kind {
                            crate::core::nervous::cron::ScheduleKind::Once =>
                                crate::core::nervous::cron::ScheduleKind::Interval,
                            crate::core::nervous::cron::ScheduleKind::Interval =>
                                crate::core::nervous::cron::ScheduleKind::Cron,
                            crate::core::nervous::cron::ScheduleKind::Cron =>
                                crate::core::nervous::cron::ScheduleKind::Once,
                        };
                    }
                    CreateField::Urgency => {
                        form.urgency = (form.urgency + 0.1).min(1.0);
                    }
                    _ => {}
                },
                KeyCode::Char(c) => match form.field {
                    CreateField::Name => form.name.push(c),
                    CreateField::Schedule => form.schedule.push(c),
                    CreateField::Prompt => form.prompt.push(c),
                    _ => {}
                },
                _ => {}
            },
            Mode::Saved(_) | Mode::Error(_) => {}
        }
    }

    async fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(view) = self.settings.as_mut() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

        let action = view.handle_key(key);

        match &action {
            Some(SettingsAction::FetchModels) => {
                let base_url = view.config.bifrost.base_url.clone();
                let api_key = view.config.bifrost.api_key.clone();
                let virtual_key = view.config.bifrost.virtual_key.clone();
                let primary = view.config.bifrost.primary_model.clone();
                let timeout = view.config.bifrost.timeout_secs;
                let extra: Vec<String> = view.config.models.keys().cloned().collect();
                let (tx, rx) = tokio::sync::oneshot::channel();
                view.models_rx = Some(rx);
                tokio::spawn(async move {
                    let bifrost = crate::bridge::bifrost::BifrostClient::new(&base_url, &api_key, &virtual_key, &primary, timeout);
                    let mut models = bifrost.list_models().await.unwrap_or_default();
                    for m in extra { if !models.contains(&m) { models.push(m); } }
                    let _ = tx.send(models);
                });
                return;
            }
            Some(SettingsAction::AtmospherePreview(atm)) => {
                self.dispatch(TuiEvent::AtmosphereChanged(atm.clone()));
                return;
            }
            _ => {}
        }

        let go_back = matches!(action, Some(SettingsAction::SaveAndGoBack) | Some(SettingsAction::GoBack));
        if go_back {
            let save_and_go = matches!(action, Some(SettingsAction::SaveAndGoBack));
            // Clone everything we need from view before dropping it, since
            // view borrows self.settings and blocks other self access.
            let db_path = self.config_path.clone().unwrap_or_else(|| PathBuf::from("souveraine.toml"));
            let (outfit, atmosphere) = if save_and_go {
                (view.config.presence.outfit.clone(), view.config.presence.atmosphere.clone())
            } else {
                (None, None)
            };
            let original_snapshot = if save_and_go { Some(view.original.clone()) } else { None };
            let config_snapshot = if save_and_go { Some(view.config.clone()) } else { None };
            let agent_model_change = if save_and_go && view.agent_model_dirty() {
                view.active_agent.as_ref().map(|a| (a.id.clone(), a.model.clone()))
            } else {
                None
            };

            // view's borrow on self.settings ends here (NLL),
            // allowing self access below.

            if save_and_go {
                if let Some(cfg) = config_snapshot {
                    cfg.save(&db_path).ok();
                    let mut live = self.config.write().await;
                    *live = cfg.clone();
                    // Diff known fields and push changes to SQLite.
                    if let Some(orig) = &original_snapshot {
                        self.sync_settings_fields(orig, &cfg).await;
                    }
                }
                if let Some((id, model)) = agent_model_change {
                    self.push_agent_model(&id, &model).await;
                }
                if let Some(name) = outfit {
                    self.dispatch(TuiEvent::OutfitChanged(name));
                } else {
                    self.dispatch(TuiEvent::OutfitChanged(String::new()));
                }
                if let Some(atm) = atmosphere {
                    self.dispatch(TuiEvent::AtmosphereChanged(atm));
                }
            }
            self.current_screen = Screen::Welcome;
            return;
        }

        if matches!(action, Some(SettingsAction::Save)) {
            let path = self.config_path.clone().unwrap_or_else(|| PathBuf::from("souveraine.toml"));
            let outfit = view.config.presence.outfit.clone();
            let atmosphere = view.config.presence.atmosphere.clone();
            match view.save(&path) {
                Ok(()) => {
                    let original = view.original.clone();
                    let saved = view.config.clone();
                    let agent_model_change = if view.agent_model_dirty() {
                        view.active_agent.as_ref().map(|a| (a.id.clone(), a.model.clone()))
                    } else {
                        None
                    };
                    let mut live = self.config.write().await;
                    *live = saved.clone();
                    view.mode = crate::ui::settings::SettingsMode::Status {
                        msg: "saved".to_string(),
                        is_error: false,
                    };
                    // Diff known config fields (e.g. the new-agent default).
                    self.sync_settings_fields(&original, &saved).await;
                    if let Some((id, model)) = agent_model_change {
                        self.push_agent_model(&id, &model).await;
                    }
                }
                Err(e) => {
                    view.mode = crate::ui::settings::SettingsMode::Status {
                        msg: e,
                        is_error: true,
                    };
                    return;
                }
            }
            if let Some(name) = outfit {
                self.dispatch(TuiEvent::OutfitChanged(name));
            } else {
                self.dispatch(TuiEvent::OutfitChanged(String::new()));
            }
            if let Some(atm) = atmosphere {
                self.dispatch(TuiEvent::AtmosphereChanged(atm));
            }
        }
    }

    /// Resolve the active agent into [`ActiveAgentSettings`] for the Settings
    /// screen — id, display name, and current model read from its on-disk
    /// `agent.json`. `None` when there is no backend to save through, or the
    /// agent / its record can't be resolved; the per-agent fields are then
    /// hidden rather than shown un-saveable.
    fn active_agent_settings(&self) -> Option<crate::ui::settings::ActiveAgentSettings> {
        self.chat.as_ref()?; // a backend is required to persist the change
        let id = self.agent_id_by_name(&self.agent_pref)?;
        let model = Self::agent_model_from_disk(&id)?;
        Some(crate::ui::settings::ActiveAgentSettings {
            id,
            name: self.agent_pref.clone(),
            model: model.clone(),
            model_original: model,
        })
    }

    /// Read an agent's current llm model handle from its on-disk record at
    /// `~/.souveraine/server/agents/{id}/agent.json`.
    fn agent_model_from_disk(agent_id: &str) -> Option<String> {
        let path = dirs::home_dir()?
            .join(".souveraine/server/agents")
            .join(agent_id)
            .join("agent.json");
        let content = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        json.get("llm_config")?.get("model")?.as_str().map(str::to_string)
    }

    /// Push a per-agent model change to the agent's record through the active
    /// backend (which also refreshes the in-memory cache and SQLite mirror).
    async fn push_agent_model(&self, agent_id: &str, model: &str) {
        let Some(chat) = &self.chat else {
            tracing::warn!(agent = %agent_id, "settings: no backend — agent model change not applied");
            return;
        };
        match chat.backend.update_agent_model(agent_id, model).await {
            Ok(()) => tracing::info!(agent = %agent_id, model = %model, "settings: agent model updated"),
            Err(e) => tracing::warn!(agent = %agent_id, error = %e, "settings: agent model update failed"),
        }
    }

    /// Diff fields between `original` (config snapshot at Settings entry) and
    /// `saved` (what the user just saved), then push changed fields to every
    /// data store that shadows them.
    ///
    /// Idempotent — unchanged fields produce no writes. Add new mappings by
    /// extending this method. See `docs/audit/config-settings-sync.md`.
    ///
    /// Note: `bifrost.primary_model` is the substrate-wide default for *new*
    /// agents — it deliberately does NOT mutate an existing agent's model.
    /// Per-agent model changes go through the Agent category's `model` field
    /// (`AgModel`) and [`push_agent_model`].
    async fn sync_settings_fields(&self, original: &ConsciousnessConfig, saved: &ConsciousnessConfig) {
        let mut changed: Vec<&'static str> = Vec::new();

        if original.bifrost.primary_model != saved.bifrost.primary_model {
            changed.push("bifrost.primary_model (new-agent default)");
        }

        if !changed.is_empty() {
            let count = changed.len();
            tracing::info!(
                changed = %changed.join(", "),
                "Settings sync pushed {} field(s) to SQLite",
                count,
            );
        }
    }

    async fn handle_chat_key(&mut self, key: crossterm::event::KeyEvent) {
        use crate::ui::chat::Overlay;

        let Some(chat) = self.chat.as_mut() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

        // BtwPane key routing — when a /btw fork pane is showing, Esc
        // dismisses it and 'j' jumps to the forked conversation.
        if chat.btw_active() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    chat.btw_dismiss();
                    return;
                }
                KeyCode::Char('j') => {
                    if let Some(forked_id) = chat.btw_jump() {
                        // Switch to the forked conversation
                        let backend = chat.backend.clone();
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        chat.switch_rx = Some(rx);
                        let agent_id = chat.agent_id.clone();
                        tokio::spawn(async move {
                            match backend.load_conversation(&forked_id).await {
                                Ok(messages) => {
                                    let _ = tx.send(Ok((forked_id, messages)));
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(e));
                                }
                            }
                        });
                    }
                    return;
                }
                _ => {}
            }
        }

        // Overlay key routing — when an overlay is active, it captures
        // navigation keys. Other keys fall through to normal handling.
        if chat.overlay_active() {
            match &chat.overlay {
                Overlay::SlashComplete { selected, matches } => {
                    let count = matches.len();
                    match key.code {
                        KeyCode::Up => {
                            let sel = if *selected == 0 { count.saturating_sub(1) } else { selected - 1 };
                            if let Overlay::SlashComplete { selected: ref mut s, .. } = chat.overlay { *s = sel; }
                            return;
                        }
                        KeyCode::Down => {
                            let sel = if *selected + 1 >= count { 0 } else { selected + 1 };
                            if let Overlay::SlashComplete { selected: ref mut s, .. } = chat.overlay { *s = sel; }
                            return;
                        }
                        KeyCode::Tab | KeyCode::Enter => {
                            chat.accept_completion();
                            return;
                        }
                        KeyCode::Esc => {
                            chat.overlay = Overlay::None;
                            return;
                        }
                        _ => {} // fall through to normal handling
                    }
                }
                Overlay::ConversationPicker { selected, conversations } => {
                    let count = conversations.len();
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            let sel = if *selected == 0 { count.saturating_sub(1) } else { selected - 1 };
                            if let Overlay::ConversationPicker { selected: ref mut s, .. } = chat.overlay { *s = sel; }
                            return;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let sel = if *selected + 1 >= count { 0 } else { selected + 1 };
                            if let Overlay::ConversationPicker { selected: ref mut s, .. } = chat.overlay { *s = sel; }
                            return;
                        }
                        KeyCode::Enter => {
                            chat.accept_conversation_pick();
                            return;
                        }
                        KeyCode::Esc => {
                            chat.overlay = Overlay::None;
                            return;
                        }
                        _ => return, // picker is fully modal
                    }
                }
                Overlay::None => {}
            }
        }

        // ── Esc overlay (interrupt-or-leave) ──────────────
        if chat.show_esc_overlay && chat.busy {
            match key.code {
                KeyCode::Esc | KeyCode::Char('c') => {
                    // Hide overlay, stay in chat, turn keeps running.
                    chat.show_esc_overlay = false;
                    return;
                }
                KeyCode::Char('i') => {
                    chat.raise_hand();
                    chat.show_esc_overlay = false;
                    return;
                }
                KeyCode::Char('m') => {
                    chat.show_esc_overlay = false;
                    self.current_screen = Screen::Welcome;
                    return;
                }
                _ => return, // block all other keys while overlay is up
            }
        }

        // Normal chat key handling.
        match key.code {
            KeyCode::Esc => {
                if chat.busy {
                    chat.show_esc_overlay = !chat.show_esc_overlay;
                } else {
                    self.current_screen = Screen::Welcome;
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if chat.input.len() < 8_192 {
                    chat.input.push('\n');
                    chat.update_completion();
                }
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if chat.input.len() < 8_192 {
                    chat.input.push('\n');
                    chat.update_completion();
                }
            }
            KeyCode::Enter => {
                // Submit always — when busy, this becomes an interjection
                // (queued and prepended to the agent's next LLM round).
                chat.submit();
            }
            KeyCode::Backspace => {
                chat.input.pop();
                chat.update_completion();
            }
            KeyCode::Up => {
                chat.scroll = chat.scroll.saturating_add(1);
            }
            KeyCode::Down => {
                chat.scroll = chat.scroll.saturating_sub(1);
            }
            KeyCode::PageUp => {
                chat.scroll = chat.scroll.saturating_add(10);
            }
            KeyCode::PageDown => {
                chat.scroll = chat.scroll.saturating_sub(10);
            }
            KeyCode::Tab => {
                chat.toggle_cockpit();
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            // `Ctrl+t` toggles whether tool cards render collapsed or expanded.
            // Not plain `t` — that would block starting sentences with "t".
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                chat.tool_cards_expanded = !chat.tool_cards_expanded;
            }
            KeyCode::Char(c) => {
                if chat.input.len() < 8_192 {
                    chat.input.push(c);
                    chat.update_completion();
                }
            }
            _ => {}
        }
    }

    async fn select_menu_item(&mut self) {
        // Welcome now hosts the dashboard inline, so the menu enters
        // *destinations* only — Chat, Schedule, and three placeholders.
        // Items marked (coming soon) are no-ops until their screens are real.
        match self.menu_selected {
            0 => {
                // Chat — lazily connect to a backend on first entry.
                if self.chat.is_none() {
                    match ChatState::connect(self.config.clone(), &self.agent_pref).await {
                        Ok(c) => {
                            self.chat = Some(c);
                            self.chat_error = None;
                        }
                        Err(e) => {
                            self.chat_error = Some(e.to_string());
                            return;
                        }
                    }
                }
                self.current_screen = Screen::Chat;
            }
            1 => {
                if self.schedules.is_none() {
                    self.schedules = Some(self.build_schedules_view().await);
                    self.sync_palette();
                } else if let Some(view) = self.schedules.as_mut() {
                    view.reload();
                }
                self.current_screen = Screen::Cron;
            }
            // 2 Settings, 3 Therapy, 4 Agent Time.
            2 => {
                // Lazily init settings from the current config snapshot.
                if self.settings.is_none() {
                    let cfg = self.config.read().await.clone();
                    let mut view = crate::ui::settings::SettingsView::new(&cfg);
                    let agent_id = self.agent_id_by_name(&self.agent_pref);
                    let expr_path = agent_id.as_ref()
                        .and_then(|id| Self::agent_assets_dir(id))
                        .map(|a| a.join("expressions"));
                    view.set_expressions_path(expr_path);
                    self.settings = Some(view);
                    self.sync_palette();
                } else {
                    let cfg = self.config.read().await.clone();
                    let agent_id = self.agent_id_by_name(&self.agent_pref);
                    let expr_path = agent_id.as_ref()
                        .and_then(|id| Self::agent_assets_dir(id))
                        .map(|a| a.join("expressions"));
                    if let Some(view) = self.settings.as_mut() {
                        view.refresh(&cfg);
                        view.set_expressions_path(expr_path);
                    }
                }
                // Per-agent settings follow the active agent — load its model
                // so the Agent category edits this agent and tracks switches.
                let active = self.active_agent_settings();
                if let Some(view) = self.settings.as_mut() {
                    view.set_active_agent(active);
                }
                self.current_screen = Screen::Settings;
            }
            _ => {}
        }
    }

    /// Resolve the agent's schedules directory, preferring its UUID under
    /// `~/.souveraine/agents/{id}/schedules/`. Falls back to the agent
    /// name if the inventory isn't reachable — the CLI uses the same
    /// path pattern, so a hand-managed dir keyed by name still works.
    async fn build_schedules_view(&self) -> crate::ui::schedules::SchedulesView {
        use crate::backend::Backend;

        let base = dirs::home_dir()
            .unwrap_or_default()
            .join(".souveraine")
            .join("agents");

        let cfg = self.config.read().await;
        let url = cfg.server.effective_url();
        drop(cfg);

        let mut resolved_id: Option<String> = None;
        let remote = crate::backend::RemoteBackend::new(&url);
        if remote.health().await {
            if let Ok(list) = remote.list_agents().await {
                resolved_id = list
                    .iter()
                    .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
                    .map(|a| a.id.clone());
            }
        }
        if resolved_id.is_none() {
            let cfg = self.config.read().await.clone();
            if let Ok(local) = crate::backend::LocalBackend::new(cfg).await {
                if let Ok(list) = local.list_agents().await {
                    resolved_id = list
                        .iter()
                        .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
                        .map(|a| a.id.clone());
                }
            }
        }

        let dir_key = resolved_id.unwrap_or_else(|| self.agent_pref.clone());
        let dir = base.join(&dir_key).join("schedules");
        let _ = std::fs::create_dir_all(&dir);
        crate::ui::schedules::SchedulesView::new(self.agent_pref.clone(), dir)
    }

    /// Detect what state the installation is in using the BootstrapPlan.
    async fn transition_from_splash(&mut self) {
        let home = dirs::home_dir().unwrap_or_default();
        let probe = crate::core::bootstrap::gather_probe(&home);
        let plan = crate::core::bootstrap::BootstrapPlan::plan(&probe);

        for phase in &plan.phases {
            match phase {
                crate::core::bootstrap::BootstrapPhase::SetupWizard(flow) => {
                    // No default model — let user type or fetch from Bifrost.
                    self.setup_state = Some(SetupState::new(*flow, ""));
                    self.current_screen = Screen::Setup;
                    self.dispatch(TuiEvent::ScreenChanged(Screen::Setup));
                    return;
                }
                crate::core::bootstrap::BootstrapPhase::ShowHint(hint) => {
                    // Store the hint for display on the Welcome screen.
                    // The dashboard reads it from a field we'll add below.
                    self.welcome_hint = Some(hint.clone());
                }
                _ => {}
            }
        }

        // Default: go to Welcome dashboard
        self.refresh_dashboard().await;
        self.current_screen = Screen::Welcome;
        self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
    }

    /// Called when the setup wizard completes or user skips to dashboard.
    /// If the wizard collected an agent name, creates the agent via LocalBackend.
    async fn finish_setup(&mut self) {
        use crate::backend::LocalBackend;

        if let Some(ref mut setup) = self.setup_state.take() {
            // If the wizard got far enough to name an agent, create it.
            if !setup.agent_name.is_empty() && !setup.complete {
                // Agent was configured but setup was skipped mid-way (Esc from Welcome)
                // — don't create, just go to dashboard.
            } else if setup.complete && !setup.agent_name.is_empty() && setup.created_agent_id.is_none() {
                // Create the agent via LocalBackend
                match LocalBackend::new(self.config.read().await.clone()).await {
                    Ok(backend) => {
                        let request = setup.build_create_request();
                        match backend.server_agents().create(request).await {
                            Ok(agent) => {
                                setup.created_agent_id = Some(agent.id.clone());
                                self.agent_pref = agent.name.clone();
                                info!("setup wizard created agent {} ({})", agent.name, agent.id);
                            }
                            Err(e) => {
                                setup.creation_error = Some(e.to_string());
                                warn!("setup wizard agent creation failed: {}", e);
                                // Still continue to dashboard — user can retry there
                            }
                        }
                    }
                    Err(e) => {
                        warn!("setup wizard backend init failed: {}", e);
                    }
                }
            }
        }

        self.setup_state = None;
        self.welcome_hint = None;
        self.refresh_dashboard().await;
        self.current_screen = Screen::Welcome;
        self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
    }

    /// Best-effort fetch of dashboard data from whichever backend is reachable.
    /// Local mode also pulls recent git commits from the agent's memory repo.
    async fn refresh_dashboard(&mut self) {
        use crate::backend::Backend;

        let cfg = self.config.read().await;
        let url = cfg.server.effective_url();
        drop(cfg);

        // Try remote first; fall back to local. Mirror of resolve_backend logic.
        let remote = crate::backend::RemoteBackend::new(&url);
        let (agents_result, mode, local_repo) = if remote.health().await {
            (remote.list_agents().await, "remote", None)
        } else {
            let cfg = self.config.read().await.clone();
            match crate::backend::LocalBackend::new(cfg).await {
                Ok(local) => {
                    let agents = local.list_agents().await;
                    // Pull a MemoryRepo for the current agent (if it exists)
                    // through the LocalBackend's server inventory.
                    let repo = if let Ok(list) = &agents {
                        if let Some(a) = list.iter().find(|a| a.name == self.agent_pref || a.id == self.agent_pref).or_else(|| list.first()) {
                            Some(local.server_agents().memory_repo(&a.id))
                        } else { None }
                    } else { None };
                    (agents, "local", repo)
                }
                Err(e) => {
                    self.agent_status.mood = format!("backend err: {}", e);
                    self.agent_status.mode = "—".to_string();
                    return;
                }
            }
        };

        let agents = match agents_result {
            Ok(a) => a,
            Err(e) => {
                self.agent_status.mood = format!("list err: {}", e);
                self.agent_status.mode = mode.to_string();
                return;
            }
        };

        let chosen = agents
            .iter()
            .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
            .or_else(|| agents.first());

        self.agent_status.mode = mode.to_string();
        self.agent_status.agent_count = agents.len();
        if let Some(a) = chosen {
            self.agent_status.name = a.name.clone();
            self.agent_pref = a.name.clone(); // sync the active pref
            self.agent_status.subconscious_active = true;
            // Presence learns about the agent through the event stream below.
        }

        // Local mode: pull memory repo stats.
        if let Some(repo) = local_repo {
            if let Ok(status) = repo.status() {
                self.agent_status.memory_commits = status.file_count as u32;
                self.agent_status.last_commit = status.last_commit.clone();
            }
            // Walk the git log for the recent-activity list.
            self.agent_status.recent_activity = recent_commits(&repo, 8).unwrap_or_default();
            // Try to load a per-agent portrait from {memfs_root}/assets/.
            // No-op if the file is absent — Presence falls back to the
            // hand-crafted Annie grid.
            self.presence.load_portrait_from_memfs(repo.root());
            // Also load a real-image protocol for terminals that support
            // kitty/sixel. Non-fatal: the half-block portrait is always
            // available as fallback.
            self.load_image_protocol_from_memfs(repo.root());

            // Try to load a 3D portrait via RGP (ratty terminal).
            if self.rgp_available {
                let assets = repo.root().join("assets");
                self.rgp_portrait = crate::ui::rgp::load_portrait_glb(&assets);
            }

            // Read the agent's last explicit atmosphere from
            // system/preferences/visual.md and restore the chrome. The agent
            // writes this when she uses the atmosphere tool; the TUI reads it
            // at conversation start so her choice survives restarts.
            let pref_path = repo.root().join("system").join("preferences").join("visual.md");
            if let Ok(content) = std::fs::read_to_string(&pref_path) {
                if let Some(body) = content.strip_prefix("---\n") {
                    if let Some(end) = body.find("\n---\n") {
                        for line in body[..end].lines() {
                            if let Some((key, val)) = line.split_once(':') {
                                let key = key.trim();
                                let val = val.trim().trim_matches('"');
                                if key == "atmosphere" {
                                    if let Some(atm) = crate::ui::atmosphere::Atmosphere::from_name(val) {
                                        self.presence.transition_atmosphere(atm);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Read energy balance from the agent's memfs. The file is written
            // by the backend after every turn (write_energy_balance in local.rs).
            // Parse the YAML frontmatter for generative/consumptive counts and
            // seed the presence gauge so the TUI reflects real agent state.
            let balance_path = repo.root().join("system").join("dynamic").join("energy-balance.md");
            if let Ok(content) = std::fs::read_to_string(&balance_path) {
                let mut gen: u32 = 0;
                let mut con: u32 = 0;
                let mut hot: u32 = 0;
                let mut cold: u32 = 0;
                if let Some(body) = content.strip_prefix("---\n") {
                    if let Some(end) = body.find("\n---\n") {
                        for line in body[..end].lines() {
                            if let Some((key, val)) = line.split_once(':') {
                                let key = key.trim();
                                let val = val.trim().trim_matches('"');
                                match key {
                                    "generative" => gen = val.parse().unwrap_or(0),
                                    "consumptive" => con = val.parse().unwrap_or(0),
                                    "hot" => hot = val.parse().unwrap_or(0),
                                    "cold" => cold = val.parse().unwrap_or(0),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                self.presence.volition = crate::ui::presence::VolitionGauge {
                    generative: gen,
                    consumptive: con,
                    hot_desires: hot,
                    cold_obligations: cold,
                };
            }
        } else {
            self.agent_status.recent_activity = vec![
                format!("[{}] connected via {}", short_now(), mode),
                format!("agents on backend: {}", agents.len()),
            ];
        }

        // Mood = most recent surfacing activity if any, else "Idle".
        self.agent_status.mood = if self.agent_status.recent_activity.is_empty() {
            "Idle".to_string()
        } else {
            "Active".to_string()
        };
        // Energy stub: derive from agent count (cosmetic).
        self.agent_status.energy = ((self.agent_status.agent_count.min(10)) * 10) as u8;

        // Dispatch state changes to all listeners (scene components + Presence).
        // Presence updates its own internal state via handle_event; no polling.
        let agent_name = self.agent_status.name.clone();
        let energy = self.agent_status.energy;
        let mood = self.agent_status.mood.clone();
        let mode_str = mode.to_string();
        self.dispatch(TuiEvent::AgentSelected(agent_name));
        self.dispatch(TuiEvent::EnergyChanged(energy));
        self.dispatch(TuiEvent::MoodChanged(mood));
        self.dispatch(TuiEvent::BackendStatus {
            mode: mode_str,
            healthy: true,
        });
    }

    /// Load a terminal-image protocol from a direct file path. The half-block
    /// portrait still loads independently as fallback. Shared between startup
    /// eager-load and dashboard refresh.
    fn load_image_protocol(&mut self, path: &std::path::Path) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let dyn_img = match image::ImageReader::open(path) {
            Ok(reader) => match reader.decode() {
                Ok(img) => img,
                Err(e) => { tracing::warn!(path = %path.display(), error = %e, "image protocol decode failed"); return; }
            },
            Err(e) => { tracing::warn!(path = %path.display(), error = %e, "image protocol open failed"); return; }
        };
        let dyn_img = portrait_cover_crop(dyn_img, 2, 3);
        let font_size = picker.font_size();
        let w = dyn_img.width().div_ceil(font_size.width as u32) as u16;
        let h = dyn_img.height().div_ceil(font_size.height as u32) as u16;
        match picker.new_protocol(dyn_img, ratatui::layout::Size::new(w, h), ratatui_image::Resize::Fit(None)) {
            Ok(proto) => {
                tracing::info!(path = %path.display(), "image protocol loaded");
                self.image_protocol = Some(proto);
            }
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "image protocol creation failed"),
        }
    }

    /// Convenience wrapper: find the first existing portrait in
    /// `<memfs_root>/assets/` and load it as a terminal image protocol.
    fn load_image_protocol_from_memfs(&mut self, memfs_root: &std::path::Path) {
        let path = ["portrait.png", "portrait.jpg", "portrait.jpeg"]
            .iter().map(|s| memfs_root.join("assets").join(s))
            .find(|p| p.exists());
        if let Some(path) = path {
            self.load_image_protocol(&path);
        }
    }

    /// Resolve an agent's portrait file under `~/.souveraine/agents/{id}/memory/assets/`.
    /// Returns the first existing path among png/jpg/jpeg variants.
    fn agent_portrait_path(agent_id: &str) -> Option<std::path::PathBuf> {
        let base = Self::agent_assets_dir(agent_id)?;
        ["portrait.png", "portrait.jpg", "portrait.jpeg"]
            .iter()
            .map(|s| base.join(s))
            .find(|p| p.exists())
    }

    /// Resolve an agent's assets directory.
    /// Returns None if homedir can't be determined.
    fn agent_assets_dir(agent_id: &str) -> Option<std::path::PathBuf> {
        let base = dirs::home_dir()?
            .join(".souveraine")
            .join("agents")
            .join(agent_id)
            .join("memory")
            .join("assets");
        if base.is_dir() { Some(base) } else { None }
    }

    /// Build a `StatefulProtocol` for a given agent and insert it into
    /// `card_images`. Stateful protocols are used here (not the eager
    /// `Protocol` used on Welcome) because each card lives in a different
    /// rect — the protocol re-encodes itself for whatever area the
    /// `StatefulImage` widget is rendered into, so a single load works
    /// across resizes and grid reflows.
    fn load_card_image(&mut self, agent_id: &str, path: &std::path::Path) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let dyn_img = match image::ImageReader::open(path) {
            Ok(reader) => match reader.decode() {
                Ok(img) => img,
                Err(e) => { tracing::warn!(path = %path.display(), error = %e, "card image decode failed"); return; }
            },
            Err(e) => { tracing::warn!(path = %path.display(), error = %e, "card image open failed"); return; }
        };
        // Pull the `image` crate into scope for resize_to_fill on the render path.
        use image::DynamicImage;
        let dyn_img = portrait_cover_crop(dyn_img, 2, 3);
        let proto = picker.new_resize_protocol(dyn_img.clone());
        let agent_id = agent_id.to_string();
        tracing::info!(agent = %agent_id, path = %path.display(), "card image loaded");
        self.card_images.insert(agent_id.clone(), proto);
        self.raw_card_images.insert(agent_id, dyn_img);
    }

    /// Refresh the card-image cache to match `agent_cards`. Loads any
    /// missing portraits and drops entries for agents no longer present.
    fn refresh_card_images(&mut self) {
        let ids: Vec<(String, Option<std::path::PathBuf>)> = self.agent_cards
            .iter()
            .map(|c| (c.id.clone(), Self::agent_portrait_path(&c.id)))
            .collect();
        let valid: std::collections::HashSet<String> = ids.iter().map(|(id, _)| id.clone()).collect();
        self.card_images.retain(|k, _| valid.contains(k));
        self.raw_card_images.retain(|k, _| valid.contains(k));
        self.cover_protocols.retain(|k, _| valid.contains(k));
        for (id, path) in ids {
            if self.card_images.contains_key(&id) { continue; }
            if let Some(path) = path {
                self.load_card_image(&id, &path);
            }
        }
    }

    /// Preload all expression frames for the currently active agent.
    /// Called when entering Presence mode so blink/breath transitions
    /// are instant rather than loading from disk on every animation tick.
    async fn preload_agent_expressions(&mut self) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let name = self.presence.name.clone();
        let id = self.agent_id_by_name(&name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));
        let Some(id) = id else { return };
        let Some(assets_dir) = Self::agent_assets_dir(&id) else { return };
        self.expression_cache.preload_all(&id, picker, &assets_dir);
    }

    // ── Voice pipeline helpers ───────────────────────────────────────────

    /// Initialize the voice client and player for a Presence session.
    /// No-op when `voice.enabled = false`. Called on entry to Presence.
    async fn init_voice_session(&mut self) {
        let cfg = self.config.read().await;
        let vcfg = cfg.voice.clone();
        drop(cfg);

        if !vcfg.enabled {
            return;
        }

        // VoiceClient is cheap — rebuild if config changed.
        self.voice_client = Some(crate::core::voice::VoiceClient::new(
            &vcfg.stt_url,
            &vcfg.tts_url,
            &vcfg.voice_id,
        ));

        // VoicePlayer opens the audio output device once and holds it open
        // for the session to avoid latency on the first utterance.
        if self.voice_player.is_none() {
            match crate::ui::voice::VoicePlayer::new() {
                Ok(player) => {
                    tracing::info!("VoicePlayer opened — audio output ready");
                    self.voice_player = Some(player);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to open audio output — TTS will be text-only");
                }
            }
        }
    }

    /// Clean up voice resources when leaving the Presence screen.
    fn exit_presence(&mut self) {
        self.voice_capture = None;
        self.voice_stt_rx = None;
        self.voice_tts_rx = None;
        if let Some(player) = &self.voice_player {
            player.stop();
        }
        self.presence.posture = crate::ui::presence::Posture::Idle;
        self.presence.sync_atmosphere_pub();
        self.current_screen = Screen::Welcome;
        self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
    }

    /// Open the mic and shift posture to Listening. Called on Space press.
    fn start_listening(&mut self) {
        // Drop any previous capture (shouldn't exist but guard anyway).
        self.voice_capture = None;

        match crate::ui::voice::MicCapture::start(16_000) {
            Ok(cap) => {
                self.voice_capture = Some(cap);
                self.presence.set_posture(crate::ui::presence::Posture::Listening);
                tracing::info!("mic capture started — listening");
            }
            Err(e) => {
                tracing::warn!(error = %e, "mic capture failed — injecting error message");
                // Failure is felt as state: inject an error message and let the
                // agent reply to it. No popup, no forced behavior.
                if let Some(chat) = self.chat.as_mut() {
                    chat.input = "*[mic unavailable — voice channel unreachable]*".to_string();
                    chat.submit();
                }
            }
        }
    }

    /// Space release: close mic, encode WAV, POST to STT, await transcript.
    async fn handle_presence_space_release(&mut self) {
        if self.presence.posture != crate::ui::presence::Posture::Listening {
            return;
        }

        let capture = match self.voice_capture.take() {
            Some(c) => c,
            None => return,
        };

        // Shift to Thinking while STT runs.
        self.presence.set_posture(crate::ui::presence::Posture::Thinking);

        let samples = capture.stop_and_take();

        if samples.is_empty() {
            tracing::info!("empty mic capture — injecting empty utterance");
            // Empty hold: treat as empty utterance per spec.
            if let Some(chat) = self.chat.as_mut() {
                chat.input = "*[empty utterance]*".to_string();
                chat.submit();
            }
            self.presence.set_posture(crate::ui::presence::Posture::Processing);
            return;
        }

        let wav = match crate::ui::voice::capture::samples_to_wav(&samples, 16_000) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "WAV encode failed");
                if let Some(chat) = self.chat.as_mut() {
                    chat.input = "*[voice service unreachable]*".to_string();
                    chat.submit();
                }
                self.presence.set_posture(crate::ui::presence::Posture::Processing);
                return;
            }
        };

        let client = match self.voice_client.as_ref() {
            Some(c) => {
                // Clone the underlying reqwest::Client (cheap) to move into spawn.
                // VoiceClient is not Clone, so we build a fresh one from the same config.
                // This is fine — reqwest::Client reuses connection pools internally.
                let stt_url = c.stt_url_str().to_string();
                let tts_url = c.tts_url_str().to_string();
                let voice = c.voice_str().to_string();
                (stt_url, tts_url, voice)
            }
            None => {
                // Voice not configured — just submit raw (no STT).
                return;
            }
        };

        let (stt_url, _tts_url, _voice) = client;

        // Spawn STT in background.
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        let stt_u = stt_url.clone();
        tokio::spawn(async move {
            // Build a disposable client for the background task.
            let c = crate::core::voice::VoiceClient::new(&stt_u, "", "");
            let result = c.transcribe(wav).await
                .map_err(|e| format!("*[voice service unreachable — {}]*", e));
            let _ = tx.send(result);
        });

        self.voice_stt_rx = Some(rx);
    }

    /// Poll the STT and TTS receivers each tick, advancing the voice state
    /// machine without blocking the render loop.
    async fn advance_voice_pipeline(&mut self) {
        // ── STT receive ─────────────────────────────────────────────────
        if let Some(rx) = self.voice_stt_rx.as_mut() {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    // Sender died — STT task crashed or hung. Clear the rx so
                    // we can try again next time instead of hanging forever.
                    self.voice_stt_rx = None;
                    let text = "*[voice service unreachable — STT task failed]*".to_string();
                    if let Some(chat) = self.chat.as_mut() {
                        chat.input = text;
                        chat.submit();
                    }
                    self.presence.set_posture(crate::ui::presence::Posture::Straining);
                    return;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    return;
                }
            };
            self.voice_stt_rx = None;

            let text = match result {
                Ok(t) if t.is_empty() => "*[empty utterance]*".to_string(),
                Ok(t) => t,
                Err(e) => {
                    self.presence.set_posture(crate::ui::presence::Posture::Straining);
                    e
                }
            };

            tracing::info!(transcript = %text, "STT received");

            // Stash the transcript so the user can see what was heard.
            self.voice_last_transcript = Some(text.clone());

            // Reset the synthesis guard for the new turn.
            self.voice_last_synthesized = None;

            // Submit through the chat path.
            if let Some(chat) = self.chat.as_mut() {
                chat.input = text.clone();
                chat.submit();
            }

            self.presence.set_posture(crate::ui::presence::Posture::Processing);
        }

        // ── TTS receive ─────────────────────────────────────────────────
        if let Some(rx) = self.voice_tts_rx.as_mut() {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.voice_tts_rx = None;
                    tracing::warn!("TTS task died without sending a result");
                    self.presence.set_posture(crate::ui::presence::Posture::Idle);
                    return;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    return;
                }
            };
            self.voice_tts_rx = None;

            // Capture the text that was being synthed for Vocal Recall
            let tts_text = self.voice_last_synthesized.clone();

            match result {
                Ok(mp3_bytes) => {
                    tracing::info!(bytes = mp3_bytes.len(), "TTS bytes received — attempting playback");
                    // Stash for replay/save
                    if let Some(text) = tts_text {
                        self.voice_last_tts_text = Some(text.clone());
                    }
                    self.voice_last_tts_bytes = Some(mp3_bytes.clone());
                    self.voice_last_tts_time = None; // reset for new playback

                    self.presence.set_posture(crate::ui::presence::Posture::Speaking);
                    if let Some(player) = &self.voice_player {
                        if let Err(e) = player.play_mp3(mp3_bytes) {
                            tracing::warn!(error = %e, "mp3 playback failed");
                            self.presence.set_posture(crate::ui::presence::Posture::Idle);
                        }
                    } else {
                        tracing::warn!("TTS bytes ready but no VoicePlayer — audio device unavailable");
                        self.presence.set_posture(crate::ui::presence::Posture::Idle);
                    }
                }
                Err(e) => {
                    tracing::warn!("TTS synthesis failed: {}", e);
                    self.presence.set_posture(crate::ui::presence::Posture::Idle);
                }
            }
        }

        // ── Speaking → Idle transition ───────────────────────────────────
        // When the sink drains, the voice turn is over — return to Idle.
        if self.presence.posture == crate::ui::presence::Posture::Speaking {
            let done = self.voice_player
                .as_ref()
                .map(|p| !p.is_speaking())
                .unwrap_or(true);
            if done {
                self.presence.set_posture(crate::ui::presence::Posture::Idle);
            }
        }

        // ── Waveform buffer ───────────────────────────────────────────────
        // While Listening, sample mic level into the rolling buffer.
        if self.presence.posture == crate::ui::presence::Posture::Listening {
            if let Some(cap) = &self.voice_capture {
                let level = cap.current_level();
                self.voice_waveform.push(level);
                if self.voice_waveform.len() > 128 {
                    self.voice_waveform.remove(0);
                }
            }
        } else if !self.voice_waveform.is_empty() {
            // Decay the waveform when not listening (voice history persists).
        }

        // ── TTS completion: stash for Vocal Recall ───────────────────────
        if self.presence.posture == crate::ui::presence::Posture::Speaking {
            let done = self.voice_player
                .as_ref()
                .map(|p| !p.is_speaking())
                .unwrap_or(true);
            if done && self.voice_last_tts_time.is_none() {
                // Transition just completed — stash time.
                self.voice_last_tts_time = Some(Instant::now());
            }
        }

        // ── Agent reply → TTS ────────────────────────────────────────────
        // When the agent finishes a turn (chat goes non-busy), synthesize the
        // reply text. Does NOT depend on posture — the subconscious pass may
        // shift posture away from Processing before we get here. As long as
        // there's a completed assistant reply we haven't synthesized yet, fire.
        //
        // Also handles regen (tts_last_text set by 'g' key).
        if self.voice_tts_rx.is_none()
            && self.voice_client.is_some()
            && !matches!(self.presence.posture,
                crate::ui::presence::Posture::Listening
                | crate::ui::presence::Posture::Speaking)
        {
            // Check for regen request first.
            let regen_text = self.tts_last_text.take();

            let maybe_reply = regen_text.or_else(|| {
                self.chat.as_ref().and_then(|c| {
                    if !c.busy {
                        c.messages.iter().rev().find_map(|m| {
                            match m {
                                crate::ui::chat::ChatMessage::Assistant { text, streaming: false, .. }
                                    if !text.is_empty() => Some(text.clone()),
                                _ => None,
                            }
                        })
                    } else {
                        None
                    }
                })
            });

            if let Some(reply) = maybe_reply {
                // Guard: skip if we already synthesized this exact reply text.
                let already_synthesized = self.voice_last_synthesized.as_deref() == Some(&reply);
                if !already_synthesized {
                    let preview = if reply.len() > 80 { &reply[..80] } else { &reply };
                    tracing::info!(text = %preview, "TTS trigger — synthesizing reply");
                    self.voice_last_synthesized = Some(reply.clone());

                    let tts_url = self.voice_client.as_ref()
                        .map(|c| c.tts_url_str().to_string())
                        .unwrap_or_default();
                    let voice = self.voice_client.as_ref()
                        .map(|c| c.voice_str().to_string())
                        .unwrap_or_default();

                    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<u8>, String>>();
                    let reply_for_bytes = reply.clone();
                    tokio::spawn(async move {
                        let c = crate::core::voice::VoiceClient::new("", &tts_url, &voice);
                        let result = c.synthesize(&reply_for_bytes).await
                            .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    });
                    self.voice_tts_rx = Some(rx);
                }
            }
        }
    }

    /// Return the agent id (memfs dir name) whose name matches `name`,
    /// scanning the on-disk agent inventory. Used so Welcome can pull
    /// the active agent's portrait out of `card_images` without needing
    /// the DB layer to round-trip name → id.
    fn agent_id_by_name(&self, name: &str) -> Option<String> {
        self.agent_cards.iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .map(|c| c.id.clone())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let layout = match self.current_screen {
            Screen::Chat => {
                SceneLayout::ChatWithSidebar { sidebar_ratio: 0.3, sidebar_open: false }
            }
            Screen::Splash => SceneLayout::Single,
            _ => SceneLayout::Single,
        };

        match self.current_screen {
            Screen::Splash => self.draw_splash(frame),
            Screen::Welcome => self.draw_welcome_mut(frame),
            Screen::Chat => {
                if let Some(chat) = self.chat.as_ref() {
                    draw_chat(frame, chat);
                } else {
                    self.draw_placeholder(frame);
                }
            }
            Screen::Cron => {
                if let Some(view) = self.schedules.as_ref() {
                    crate::ui::schedules::draw(frame, view);
                } else {
                    self.draw_placeholder(frame);
                }
            }
            Screen::Settings => {
                if let Some(view) = self.settings.as_ref() {
                    crate::ui::settings::draw(frame, view);
                } else {
                    self.draw_placeholder(frame);
                }
            }
            Screen::Presence => self.draw_presence_mode_mut(frame),
            Screen::AgentsManager => self.draw_agent_cards_mut(frame),
            Screen::Setup => {
                if let Some(ref setup) = self.setup_state {
                    setup.draw(frame);
                } else {
                    self.draw_placeholder(frame);
                }
            }
            _ => self.draw_placeholder(frame),
        }

    }

    fn draw_splash(&mut self, frame: &mut Frame) {
        let area = frame.size();

        // Clear to black
        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, area);

        // Advance bloom animation
        self.bloom.advance(0.1);

        // Render the procedural bloom into the buffer
        crate::ui::animation::bloom::render(
            frame.buffer_mut(),
            area,
            &self.bloom,
            self.tick,
        );

        // FIGlet title — large, bold, emerges with bloom
        if self.bloom.progress > 0.25 {
            let alpha = ((self.bloom.progress - 0.25) / 0.35).min(1.0);
            let breathe = ((self.tick as f32 * 0.04).sin() * 0.5 + 0.5) * 0.15 + 0.85;

            // Generate FIGlet text
            #[cfg(feature = "figlet-rs")]
            let figlet_text: Option<String> = {
                FIGlet::standard().ok().and_then(|f| {
                    f.convert("Souveraine").map(|fig| fig.as_str().to_string())
                })
            };
            #[cfg(not(feature = "figlet-rs"))]
            let figlet_text: Option<String> = None;

            let fig_lines: Vec<Line> = if let Some(ref text) = figlet_text {
                text.lines().map(|line| {
                    Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(rgb(
                                (255.0 * alpha * breathe) as u8,
                                (140.0 * alpha * breathe * 0.6) as u8,
                                (66.0 * alpha * breathe * 0.4) as u8,
                            ))
                            .add_modifier(Modifier::BOLD),
                    ))
                }).collect()
            } else {
                // Fallback: spaced-out letters
                vec![
                    Line::from(Span::styled(
                        "S O U V E R A I N E",
                        Style::default()
                            .fg(rgb(
                                (255.0 * alpha * breathe) as u8,
                                (140.0 * alpha * breathe * 0.6) as u8,
                                (66.0 * alpha * breathe * 0.4) as u8,
                            ))
                            .add_modifier(Modifier::BOLD),
                    )),
                ]
            };

            // Build full title: FIGlet + subtitle
            let mut title_lines = fig_lines;
            title_lines.push(Line::from(""));
            title_lines.push(Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(rgb(
                    (180.0 * alpha) as u8,
                    (120.0 * alpha) as u8,
                    (80.0 * alpha) as u8,
                )),
            )));

            // Press any key hint at very bottom
            if self.bloom.progress > 0.8 {
                let skip_alpha = ((self.bloom.progress - 0.8) / 0.2).min(1.0);
                title_lines.push(Line::from(Span::styled(
                    "press any key to skip",
                    Style::default().fg(rgb(
                        (100.0 * skip_alpha) as u8,
                        (100.0 * skip_alpha) as u8,
                        (100.0 * skip_alpha) as u8,
                    )),
                )));
            }

            let title_height = title_lines.len() as u16;
            let title_y = if title_height > 6 {
                area.height.saturating_sub(title_height + 4)
            } else {
                area.height.saturating_sub(8)
            };
            let title_area = Rect {
                x: area.x,
                y: title_y.min(area.height.saturating_sub(title_height)),
                width: area.width,
                height: title_height.min(area.height),
            };

            let title = Paragraph::new(title_lines).alignment(Alignment::Center);
            frame.render_widget(title, title_area);
        }

        // Loading bar at bottom — peonia style gradient bar
        let bar_y = area.height.saturating_sub(2);
        let bar_w = 30u16.min(area.width.saturating_sub(4));
        let bar_x = (area.width.saturating_sub(bar_w)) / 2;
        let pct = (self.bloom.progress * 100.0) as u16;

        let bar_area = Rect {
            x: area.x + bar_x,
            y: bar_y,
            width: bar_w,
            height: 1,
        };

        let filled = (bar_w as f32 * self.bloom.progress) as u16;
        let empty = bar_w.saturating_sub(filled);
        let pct_str = format!("{:>3}%", pct);
        let bar_text = format!(
            "{}{} {}",
            "▰".repeat(filled as usize),
            "▱".repeat(empty as usize),
            pct_str,
        );

        let bar = Paragraph::new(bar_text)
            .style(Style::default().fg(rgb(200, 130, 160)))
            .alignment(Alignment::Center);
        frame.render_widget(bar, bar_area);
    }

    fn draw_welcome_mut(&mut self, frame: &mut Frame) {
        // Welcome is now the home dashboard. Two layouts, dispatched on width:
        //   • wide (>= 100 cols): portrait-left, info+menu right
        //   • narrow (< 100):    stacked — cards, portrait, activity, menu
        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, frame.size());

        if frame.size().width >= 100 {
            self.draw_welcome_wide(frame);
        } else {
            self.draw_welcome_stacked(frame);
        }

        // Surface any chat connect error in the bottom margin regardless of layout.
        if let Some(err) = &self.chat_error {
            let area = frame.size();
            let err_para = Paragraph::new(format!(" chat connect failed: {} ", err))
                .style(Style::default().fg(Color::Rgb(220, 100, 100))) // red (error — keep)
                .alignment(Alignment::Center);
            let row = Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(2),
                width: area.width,
                height: 1,
            };
            frame.render_widget(err_para, row);
        }
    }

    /// The list of menu items, in selection order. The trailing flag marks
    /// items that exist as destinations vs. coming-soon placeholders.
    fn welcome_menu_items() -> Vec<(&'static str, &'static str, bool)> {
        vec![
            ("💬 Chat",       "Talk with your agent",   true),
            ("📅 Schedule",   "Cron jobs & tasks",      true),
            ("⚙️  Settings",   "Configure",              true),
            ("🛋️  Therapy",    "Agent therapy session",  false),
            ("⏰ Agent Time", "Give your agent time",   false),
        ]
    }

    fn build_menu_list(&self, title: &str, palette: &crate::ui::chat::ChatPalette) -> List<'static> {
        let items: Vec<ListItem> = Self::welcome_menu_items()
            .into_iter()
            .enumerate()
            .map(|(i, (label, desc, available))| {
                let selected = i == self.menu_selected;
                let label_style = if !available {
                    Style::default().fg(palette.tool_dim)
                } else if selected {
                    Style::default()
                        .fg(palette.agent_primary)
                        .bg(palette.bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let desc_style = Style::default().fg(palette.agent_dim);
                let mut spans = vec![
                    Span::styled(format!(" {} ", label), label_style),
                    Span::styled(format!("- {}", desc), desc_style),
                ];
                if !available {
                    spans.push(Span::styled(
                        "  (coming soon)",
                        Style::default()
                            .fg(palette.agent_dim)
                            .add_modifier(Modifier::ITALIC),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        List::new(items).block(
            Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.agent_primary).add_modifier(Modifier::DIM)),
        )
    }

    /// Build the 4 dashboard stat cards as a Vec of (title, body, color) tuples
    /// rendered into the given horizontal strip of `area`.
    fn render_stat_cards(&self, frame: &mut Frame, area: Rect) {
        let atm = self.presence.atmosphere;
        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(area);

        let energy_color = match self.agent_status.energy {
            0..=30 => Color::Red,
            31..=60 => Color::Yellow,
            _ => Color::Green,
        };
        let energy = Gauge::default()
            .block(
                Block::default()
                    .title(" Energy ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .gauge_style(Style::default().fg(energy_color).bg(atm.bg_tint()))
            .percent(self.agent_status.energy as u16)
            .label(format!("{}%", self.agent_status.energy));
        frame.render_widget(energy, cards[0]);

        let mood = Paragraph::new(format!("\n◌\n\n{}", self.agent_status.mood))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" State ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(atm.secondary())),
            );
        frame.render_widget(mood, cards[1]);

        let memory_label = match &self.agent_status.last_commit {
            Some(c) => format!("\n💾\n\n{} files\n{}", self.agent_status.memory_commits, c),
            None => format!("\n💾\n\n{} files", self.agent_status.memory_commits),
        };
        let memory = Paragraph::new(memory_label)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" Memory ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(atm.secondary())),
            );
        frame.render_widget(memory, cards[2]);

        let agents_card = Paragraph::new(format!(
            "\n👥\n\n{} agent{}\non {}",
            self.agent_status.agent_count,
            if self.agent_status.agent_count == 1 { "" } else { "s" },
            self.agent_status.mode,
        ))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title(" Backend ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(atm.secondary())),
        );
        frame.render_widget(agents_card, cards[3]);
    }

    fn render_recent_activity(&self, frame: &mut Frame, area: Rect) {
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let text = if self.agent_status.recent_activity.is_empty() {
            "(no recent activity — open Chat to begin)".to_string()
        } else {
            self.agent_status.recent_activity.join("\n")
        };
        let para = Paragraph::new(text)
            .style(Style::default().fg(palette.agent_dim))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Recent Activity ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.agent_primary).add_modifier(Modifier::DIM)),
            );
        frame.render_widget(para, area);
    }

    /// Render a card image into `area` using cover-fill: scale the source
    /// image to fill the pixel dimensions (one axis overflows), then
    /// top-crop so the face stays visible. Caches the resulting protocol
    /// per (agent_id, area_width, area_height) so window resizes don't
    /// regenerate every frame.
    fn render_card_image_cover(
        &mut self,
        frame: &mut Frame,
        agent_id: &str,
        area: Rect,
    ) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let Some(raw) = self.raw_card_images.get(agent_id).cloned() else {
            // Fall back to the old stateful protocol (unscaled Crop).
            if let Some(proto) = self.card_images.get_mut(agent_id) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Crop(None)),
                    area,
                    proto,
                );
            }
            return;
        };

        let key = format!("{}:{}x{}", agent_id, area.width, area.height);

        if !self.cover_protocols.contains_key(&key) {
            let fs = picker.font_size();
            let target_px_w = area.width as u32 * fs.width as u32;
            let target_px_h = area.height as u32 * fs.height as u32;

            // Scale to fill using max ratio (object-fit: cover).
            let sx = target_px_w as f64 / raw.width() as f64;
            let sy = target_px_h as f64 / raw.height() as f64;
            let scale = sx.max(sy);
            let scaled_w = (raw.width() as f64 * scale).round() as u32;
            let scaled_h = (raw.height() as f64 * scale).round() as u32;
            let scaled = raw.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Lanczos3);

            // Top-anchored crop: keep the face, lose the feet.
            let cropped = scaled.crop_imm(0, 0, target_px_w.min(scaled_w), target_px_h.min(scaled_h));

            let proto = picker.new_resize_protocol(cropped);
            self.cover_protocols.insert(key.clone(), (area.width, area.height, proto));
        }

        if let Some((_, _, proto)) = self.cover_protocols.get_mut(&key) {
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Fit(None)),
                area,
                proto,
            );
        }
    }

    /// Render the agent portrait into `area` with a bordered card.
    /// Title strip below the photo carries name + glyph.
    fn render_portrait_card(&mut self, frame: &mut Frame, area: Rect) {
        use crate::ui::portrait;
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let border_col = if self.presence.subconscious_active {
            palette.surfacing
        } else {
            palette.agent_primary
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_col).add_modifier(Modifier::DIM));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 4 || inner.width < 4 {
            return;
        }

        // Reserve a 1-row strip at the bottom of the card for the name.
        let photo_h = inner.height.saturating_sub(1);
        let portrait_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: photo_h,
        };

        let active_id = self.agent_id_by_name(&self.presence.name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));

        // Tier 0: RGP 3D portrait (ratty terminal only).
        let rgp_rendered = if let Some(ref mut g) = self.rgp_portrait {
            if g.is_active() {
                g.apply_posture(self.presence.posture);
                g.render(portrait_area, frame.buffer_mut());
                true
            } else { false }
        } else { false };

        let rendered = if rgp_rendered {
            true
        } else {
            active_id.as_ref().and_then(|id| {
                let picker = self.image_picker.as_ref()?;

                // Tier 1: cover-fill (scale-to-fill + top-crop).
                if self.raw_card_images.contains_key(id) {
                    self.render_card_image_cover(frame, id, portrait_area);
                    return Some(true);
                }

                // Tier 2: expression/animated frames.
                let assets_dir = Self::agent_assets_dir(id)?;
                let key = crate::ui::expressions::ExpressionKey::from_presence(&self.presence);
                if let Some(proto) = self.expression_cache.resolve(id, key, picker, &assets_dir) {
                    frame.render_stateful_widget(
                        StatefulImage::default().resize(Resize::Scale(None)),
                        portrait_area,
                        proto,
                    );
                    return Some(true);
                }

                None
            }).is_some()
        };
        if !rendered {
            let scale = (portrait_area.width / portrait::PORTRAIT_W)
                .min((2 * portrait_area.height) / portrait::PORTRAIT_H)
                .max(1);
            portrait::render_scaled(frame.buffer_mut(), portrait_area, &self.presence, scale);
        }

        let glyph = if self.presence.subconscious_active { "◈" } else { "·" };
        let name_area = Rect {
            x: inner.x,
            y: inner.y + photo_h,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", glyph), Style::default().fg(border_col)),
                Span::styled(
                    self.presence.name.clone(),
                    Style::default().fg(border_col).add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center),
            name_area,
        );
    }

    /// Layout A — wide: portrait card on the left, stats+activity+menu on the right.
    fn draw_welcome_wide(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        let outer = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),  // title strip
                Constraint::Min(20),    // body
                Constraint::Length(1),  // footer
            ])
            .split(area);

        // Title breathing from atmosphere primary channels.
        let (tr, tg, _tb) = match palette.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let breathe = self.presence.animator.breathe(3000);
        let glow = (tg as f32 * 0.6 + breathe * 40.0) as u8;
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "S O U V E R A I N E",
                Style::default()
                    .fg(Color::Rgb(tr, glow.max(tr / 3), tr / 4))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(palette.agent_dim),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(title, outer[0]);

        // Body: 40/60 horizontal split.
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(outer[1]);

        self.render_portrait_card(frame, body[0]);

        // Right column: stats row, activity (flex), menu (fixed).
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),  // stats cards
                Constraint::Min(6),     // recent activity
                Constraint::Length(9),  // menu (5 items + border)
            ])
            .split(body[1]);

        self.render_stat_cards(frame, right[0]);
        self.render_recent_activity(frame, right[1]);
        let menu = self.build_menu_list("Menu", &palette);
        frame.render_widget(menu, right[2]);

        // Footer.
        let footer = Paragraph::new(
            "↑↓ Navigate • Enter select • a Add • i Inspect • p Presence • q Quit",
        )
        .style(Style::default().fg(palette.agent_dim))
        .alignment(Alignment::Center);
        frame.render_widget(footer, outer[2]);
    }

    /// Layout B — narrow stacked: title, stats row, portrait, activity, menu.
    fn draw_welcome_stacked(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        let avatar_card_w: u16 = (area.width * 50 / 100).min(48).max(28);
        let photo_h: u16 = (avatar_card_w / 2 + 2).clamp(10, 18);
        let avatar_card_h: u16 = photo_h + 2;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),               // title
                Constraint::Length(6),               // stat cards
                Constraint::Length(avatar_card_h),   // portrait
                Constraint::Min(5),                  // activity
                Constraint::Length(9),               // menu
                Constraint::Length(1),               // footer
            ])
            .split(area);

        // Title breathing from atmosphere primary channels.
        let (tr, tg, _tb) = match palette.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let breathe = self.presence.animator.breathe(3000);
        let glow = (tg as f32 * 0.6 + breathe * 40.0) as u8;
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "S O U V E R A I N E",
                Style::default()
                    .fg(Color::Rgb(tr, glow.max(tr / 3), tr / 4))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(palette.agent_dim),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(title, chunks[0]);

        self.render_stat_cards(frame, chunks[1]);

        // Centered portrait card.
        if chunks[2].width >= avatar_card_w {
            let card_x = chunks[2].x + (chunks[2].width - avatar_card_w) / 2;
            let card_area = Rect {
                x: card_x,
                y: chunks[2].y,
                width: avatar_card_w,
                height: avatar_card_h.min(chunks[2].height),
            };
            self.render_portrait_card(frame, card_area);
        }

        self.render_recent_activity(frame, chunks[3]);
        let menu = self.build_menu_list("Menu", &palette);
        frame.render_widget(menu, chunks[4]);

        let footer = Paragraph::new(
            "↑↓ • Enter • a Add • i Inspect • p Presence • q Quit",
        )
        .style(Style::default().fg(palette.agent_dim))
        .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[5]);
    }

    fn draw_placeholder(&self, frame: &mut Frame) {
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        let screen_name = match self.current_screen {
            Screen::Chat => "💬 Chat",
            Screen::Therapy => "🛋️ Therapy",
            Screen::AgentTime => "⏰ Agent Time",
            Screen::Cron => "📅 Schedule",
            Screen::Settings => "⚙️ Settings",
            _ => "",
        };

        let content = Paragraph::new(format!("\n\n{}\n\n(Coming Soon)", screen_name))
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.agent_primary).add_modifier(Modifier::BOLD));
        frame.render_widget(content, area);
    }

    /// Presence mode — fullscreen Annie as a rich agent card.
    ///
    /// Shows the agent's photo (real or fallback silhouette) at a generous
    /// scale, with live state below: posture, energy, mood, volition balance,
    /// instance count, memory files, uptime, and the N+1/AniAvatar status.
    ///
    /// This is the TUI anchor for the future AniAvatar integration: posture
    /// states (Idle/Processing/Affectionate/Straining/Yawning) map directly
    /// to the Godot overlay's five-state machine, and the TTS/STT path will
    /// add a microphone icon + waveform indicator here.
    ///
    /// `&mut self` because the StatefulImage protocol re-encodes each frame.
    /// Format an opertional age string from an ISO-style creation date.
    fn format_age(&self, created: &str) -> String {
        use chrono::NaiveDate;
        if let Ok(d) = NaiveDate::parse_from_str(created, "%Y-%m-%d") {
            let now = chrono::Local::now().naive_local().date();
            let delta = now - d;
            let days = delta.num_days();
            let years = days / 365;
            let months = (days % 365) / 30;
            let rem_days = (days % 365) % 30;
            format!("{:02}y:{:02}m:{:02}d", years, months, rem_days)
        } else {
            "—:—:—".to_string()
        }
    }

    /// Full-height presence column — portrait fills the terminal, metadata
    /// and stats render as HUD overlays on top of the image. Waveform and
    /// Vocal Recall sit at the very bottom.
    fn draw_presence_mode_mut(&mut self, frame: &mut Frame) {
        use crate::ui::portrait;
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        // ── Background ───────────────────────────────────────────────
        let bg = Block::default().style(Style::default().bg(palette.bg));
        frame.render_widget(bg, area);

        // ── Split: portrait fills most, voice bar at the bottom ──────
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),
                Constraint::Length(5),
            ])
            .split(area);

        let portrait_chunk = vchunks[0];
        let voice_chunk = vchunks[1];

        // ── Active agent lookup ──────────────────────────────────────
        let active_id = self
            .agent_id_by_name(&self.presence.name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));
        let agent_card = self
            .agent_cards
            .iter()
            .find(|c| Some(c.name.as_str()) == active_id.as_deref()
                || c.name.eq_ignore_ascii_case(&self.presence.name));

        // Hoist card data out of agent_card before the render block
        // (which needs &mut self), so the immutable borrow on agent_cards
        // doesn't conflict.
        let card_created = agent_card.map(|c| c.created.clone());
        let card_mem_count = agent_card.map(|c| c.memory_count).unwrap_or(0);
        let card_uptime = agent_card.map(|c| c.uptime_pct).unwrap_or(0);
        let card_instances = agent_card.map(|c| c.instance_count).unwrap_or(0);
        // agent_card consumed by the .map() chain above — immutable borrow
        // on self.agent_cards is released.

        // ═══════════════════════════════════════════════════════════════
        // PORTRAIT: fill the full panel area (minus border). Cover-fill
        // scaling in render_card_image_cover handles aspect ratio and
        // keeps the face visible via top-anchored crop. No manual
        // aspect-ratio guesstimate — the cover-fill math is pixel-exact.
        // ═══════════════════════════════════════════════════════════════
        let inner_w = portrait_chunk.width.saturating_sub(2);
        let inner_h = portrait_chunk.height.saturating_sub(2);
        let photo_area = Rect {
            x: portrait_chunk.x + 1,
            y: portrait_chunk.y + 1,
            width: inner_w,
            height: inner_h,
        };
        let cell_w = photo_area.width;
        let cell_h = photo_area.height;

        // Double-line border, posture-aware color.
        let border_color = crate::ui::presence::posture_border(&self.presence);
        let frame_style = if self.presence.subconscious_active {
            Style::default().fg(border_color)
        } else {
            Style::default().fg(border_color).add_modifier(Modifier::DIM)
        };
        let double_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(frame_style);
        frame.render_widget(double_block, portrait_chunk);

        // Tier 0: RGP 3D portrait (ratty terminal only).
        let rgp_rendered = if let Some(ref mut g) = self.rgp_portrait {
            if g.is_active() {
                g.apply_posture(self.presence.posture);
                g.render(photo_area, frame.buffer_mut());
                true
            } else { false }
        } else { false };

        // Tiers 1-3: card_image (cover-fill) → expression cache → half-block.
        let rendered = if rgp_rendered {
            true
        } else if let Some(ref id) = active_id {
            let picker = self.image_picker.as_ref();

            // Tier 1: cover-fill. Always fills the full area, top-crops for
            // the face. Cached per area so subsequent frames are cheap.
            if picker.is_some() && self.raw_card_images.contains_key(id) {
                self.render_card_image_cover(frame, id, photo_area);
                true
            // Tier 2: expression frames — only if expressions/ dir exists.
            // Use Scale (proportional upscale) not Crop (native clip).
            } else if let Some(p) = picker {
                match Self::agent_assets_dir(id) {
                    Some(dir) => {
                        let key = crate::ui::expressions::ExpressionKey::from_presence(&self.presence);
                        if let Some(proto) = self.expression_cache.resolve(id, key, p, &dir) {
                            frame.render_stateful_widget(
                                StatefulImage::default().resize(Resize::Scale(None)),
                                photo_area,
                                proto,
                            );
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            } else {
                false
            }
        } else {
            false
        };
        if !rendered {
            let scale = (cell_w / portrait::PORTRAIT_W).min((2 * cell_h) / portrait::PORTRAIT_H).max(1);
            portrait::render_scaled(frame.buffer_mut(), photo_area, &self.presence, scale);
        }

        // ═══════════════════════════════════════════════════════════════
        // HUD OVERLAY: rendered on top of the portrait area bottom
        // ═══════════════════════════════════════════════════════════════
        let p = &self.presence;
        let (badge_icon, badge_color) = match p.posture {
            Posture::Processing => ("⚡", palette.agent_primary),
            Posture::Thinking => ("◔", Color::Rgb(120, 150, 200)),
            Posture::Alert => ("◉", palette.agent_primary),
            Posture::Affectionate => ("♥", Color::Rgb(220, 150, 170)),
            Posture::Straining => ("⚠", Color::Rgb(200, 120, 100)),
            Posture::Yawning => ("💤", Color::Rgb(160, 145, 130)),
            Posture::Listening => ("◉", palette.agent_dim),
            Posture::Speaking => ("◉", palette.agent_primary),
            Posture::Idle => ("◌", palette.agent_dim),
        };

        // Bottom 4 rows of the portrait chunk become the HUD panel.
        let hud_top = portrait_chunk.y + portrait_chunk.height.saturating_sub(5);
        let hud_area = Rect {
            x: portrait_chunk.x,
            y: hud_top,
            width: portrait_chunk.width,
            height: 5.min(portrait_chunk.height.saturating_sub(2)),
        };

        // Semi-transparent background bar.
        let (hud_r, hud_g, hud_b) = match palette.bg { Color::Rgb(r, g, b) => (r, g, b), _ => (4, 4, 10) };
        let hud_bg = Block::default().style(Style::default().bg(Color::Rgb(hud_r.saturating_sub(2), hud_g.saturating_sub(2), hud_b.saturating_sub(2))));
        frame.render_widget(hud_bg, hud_area);

        let age_str = card_created.as_ref()
            .map(|c| self.format_age(c))
            .unwrap_or_else(|| "—:—:—".to_string());
        let (commits, uptime, instances, mem_count) = (
            card_mem_count as u32,
            card_uptime,
            card_instances,
            card_mem_count,
        );

        let hud_inner = Rect {
            x: hud_area.x + 2,
            y: hud_area.y + 1,
            width: hud_area.width.saturating_sub(4),
            height: hud_area.height.saturating_sub(2),
        };

        let hud_lines = vec![
            // Row 1: Name + posture badge
            Line::from(vec![
                Span::styled(format!(" {} ", badge_icon), Style::default().fg(badge_color)),
                Span::styled(&p.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(
                    match p.posture {
                        Posture::Listening => "  Listening",
                        Posture::Speaking => "  Speaking",
                        Posture::Processing => "  Processing",
                        Posture::Thinking => "  Thinking",
                        Posture::Alert => "  Alert",
                        Posture::Affectionate => "  Affectionate",
                        Posture::Straining => "  Straining",
                        Posture::Yawning => "  Yawning",
                        Posture::Idle => "",
                    },
                    Style::default().fg(badge_color).add_modifier(Modifier::DIM),
                ),
            ]),
            // Row 2: AGE
            Line::from(vec![
                Span::styled(" AGE  ", Style::default().fg(palette.agent_dim)),
                Span::styled(age_str, Style::default().fg(palette.agent_primary)),
            ]),
            // Row 3: STATS grid
            Line::from(vec![
                Span::styled(" STATS", Style::default().fg(palette.agent_dim)),
                Span::raw("  "),
                Span::styled(format!("C {}", commits), Style::default().fg(Color::Rgb(160, 200, 140))),
                Span::raw("  "),
                Span::styled(format!("U {}%", uptime), Style::default().fg(Color::Rgb(120, 220, 160))),
                Span::raw("  "),
                Span::styled(format!("I {}", instances), Style::default().fg(Color::Rgb(160, 180, 220))),
                Span::raw("  "),
                Span::styled(format!("M {}", mem_count), Style::default().fg(Color::Rgb(140, 200, 180))),
                if self.rgp_portrait.as_ref().map(|g| g.is_active()).unwrap_or(false) {
                    Span::styled("  3D", Style::default().fg(Color::Rgb(220, 180, 255)))
                } else {
                    Span::raw("")
                },
            ]),
            // Row 4: Mood / outfit
            Line::from(vec![
                Span::styled(" MOOD ", Style::default().fg(palette.agent_dim)),
                Span::styled(&p.mood, Style::default().fg(palette.agent_primary)),
                Span::raw("  ·  "),
                Span::styled(
                    p.outfit.as_deref().unwrap_or("default"),
                    Style::default().fg(palette.agent_dim),
                ),
            ]),
        ];

        frame.render_widget(Paragraph::new(hud_lines).alignment(Alignment::Left), hud_inner);

        // ═══════════════════════════════════════════════════════════════
        // VOICE BAR: transcript, waveform, Vocal Recall controls
        // ═══════════════════════════════════════════════════════════════
        let voice_area = voice_chunk;
        let is_listening = p.posture == Posture::Listening;
        let is_speaking = p.posture == Posture::Speaking;
        let has_recent_tts = self.voice_last_tts_text.is_some();

        // Sub-layout: transcript (1), waveform (1), controls (rest).
        let voice_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(voice_area);

        let transcript_area = voice_rows[0];
        let wave_area = voice_rows[1];
        let hint_area = voice_rows[2];

        // ── Transcript row ─────────────────────────────────────────
        // Shows what was said (STT) or what she said (TTS text).
        let transcript = self.voice_last_transcript.as_deref().filter(|t| !t.is_empty());
        let tts_display = self.voice_last_tts_text.as_deref().filter(|t| !t.is_empty());

        let transcript_line = if is_listening {
            transcript.map(|t| format!("‹ {} ›", t)).unwrap_or_else(|| " listen  ".to_string())
        } else if is_speaking {
            tts_display.map(|t| clip_to(&t, voice_area.width.saturating_sub(6) as usize))
                .map(|c| format!("» {} «", c))
                .unwrap_or_else(|| " speak  ".to_string())
        } else if let Some(t) = tts_display {
            let clip = clip_to(t, voice_area.width.saturating_sub(6) as usize);
            format!("» {} «", clip)
        } else if let Some(t) = transcript {
            let clip = clip_to(t, voice_area.width.saturating_sub(6) as usize);
            format!("‹ {} ›", clip)
        } else {
            String::new()
        };

        let transcript_color = if is_listening {
            palette.agent_primary
        } else if is_speaking {
            self.presence.atmosphere.primary()
        } else {
            palette.agent_dim
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(transcript_line, Style::default().fg(transcript_color).add_modifier(Modifier::DIM)))),
            transcript_area,
        );

        // ── Waveform row ───────────────────────────────────────────
        if is_listening {
            let level = self.voice_capture.as_ref().map(|c| c.current_level()).unwrap_or(0.0);
            let is_recording = level > 0.05;
            let rec_glyph = if is_recording && self.tick % 2 == 0 { "● REC" } else { "  rec" };

            let mut wave_spans: Vec<Span> = Vec::new();
            let bar_w = (voice_area.width.saturating_sub(10)).min(128) as usize;
            wave_spans.push(Span::styled(
                format!(" {} ", rec_glyph),
                Style::default().fg(if is_recording { Color::Rgb(220, 60, 60) } else { palette.agent_dim }),
            ));

            let wf_len = self.voice_waveform.len();
            if bar_w > 0 && wf_len > 0 {
                let step = (wf_len as f32 / bar_w as f32).max(1.0);
                for i in 0..bar_w {
                    let idx = ((i as f32) * step) as usize;
                    let sample = self.voice_waveform.get(idx).copied().unwrap_or(0.0);
                    let ch = crate::ui::voice::LEVEL_CHARS[(sample * 7.0).round() as usize];
                    let b = (60.0 + sample * 195.0) as u8;
                    wave_spans.push(Span::styled(ch.to_string(), Style::default().fg(Color::Rgb(b / 2, b, b / 3))));
                }
            }
            frame.render_widget(Paragraph::new(Line::from(wave_spans)), wave_area);
        } else if is_speaking {
            frame.render_widget(Paragraph::new(Line::from(Span::styled(
                " ♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪",
                Style::default().fg(self.presence.atmosphere.primary()),
            ))), wave_area);
        } else {
            let ghost: String = "▁▂▃▄▅▆▇█▇▆▅▄▃▂".chars()
                .flat_map(|c| std::iter::repeat(c).take(3))
                .take(voice_area.width as usize)
                .collect();
            let (ghost_r, ghost_g, ghost_b) = match palette.bg { Color::Rgb(r, g, b) => (r, g, b), _ => (40, 44, 60) };
            frame.render_widget(Paragraph::new(Line::from(Span::styled(
                ghost, Style::default().fg(Color::Rgb(ghost_r.saturating_add(30), ghost_g.saturating_add(30), ghost_b.saturating_add(36))),
            ))), wave_area);
        }

        // ── Controls row ───────────────────────────────────────────
        if is_listening {
            let hint = Paragraph::new(Line::from(Span::styled(
                " Space → send  ·  Esc → cancel",
                Style::default().fg(palette.agent_dim),
            ))).alignment(Alignment::Center);
            frame.render_widget(hint, hint_area);
        } else if is_speaking || has_recent_tts {
            let recall = vec![
                Span::styled(" r ⟲ ", Style::default().fg(palette.tool_accent)),
                Span::raw("Replay  "),
                Span::styled(" g ↻ ", Style::default().fg(palette.agent_primary)),
                Span::raw("Regen  "),
                Span::styled(" s 💾 ", Style::default().fg(palette.tool_accent)),
                Span::raw("Save  ·  "),
                Span::styled("Space to speak", Style::default().fg(palette.agent_dim)),
            ];
            frame.render_widget(Paragraph::new(Line::from(recall)).alignment(Alignment::Center), hint_area);
        } else {
            let hint = if self.voice_client.is_some() {
                " Space to speak  ·  Esc to leave"
            } else {
                " Space to speak  ·  Esc to leave"
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(palette.agent_dim)))).alignment(Alignment::Center),
                hint_area,
            );
        }
    }

    /// Build a card deck for every agent on the local backend. Called on
    /// entry to the AgentsManager screen.
    async fn refresh_agent_cards(&mut self) {
        let cfg = self.config.read().await.clone();
        self.agent_cards = Self::fetch_agent_cards(cfg).await;
    }

    /// Standalone fetch so it can be called without &mut self during init.
    async fn fetch_agent_cards(cfg: ConsciousnessConfig) -> Vec<AgentCard> {
        use crate::backend::Backend;
        let Ok(local) = crate::backend::LocalBackend::new(cfg).await else { return vec![] };
        let Ok(list) = local.list_agents().await else { return vec![] };
        let inv = local.server_agents();
        let mut cards = Vec::new();
        for a in &list {
            let glyph = inv.seed_id(&a.id)
                .map(|s| s.glyph())
                .unwrap_or_else(|_| "◇◆".to_string());
            let pubkey_prefix = inv.seed_id(&a.id)
                .map(|s| s.public_key_hex()[..16].to_string())
                .unwrap_or_else(|_| "—".to_string());
            let instance_count = inv.instance_count(&a.id).await.unwrap_or(0);
            let lifetime_secs = inv.lifetime_active_seconds(&a.id).await.unwrap_or(0);
            let uptime_pct = if lifetime_secs > 0 {
                let days = ((instance_count.max(1)) as f64 * 30.0).max(1.0);
                let pct = (lifetime_secs as f64 / (days * 86400.0)) * 100.0;
                pct.min(99.0) as u8
            } else { 0 };
            let mem_count = local.server_agents().memory_repo(&a.id)
                .status()
                .map(|s| s.file_count)
                .unwrap_or(0);
            cards.push(AgentCard {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone().unwrap_or_default(),
                glyph,
                pubkey_prefix,
                instance_count,
                uptime_pct,
                memory_count: mem_count,
                created: "Feb 2025 · TBD date from server".to_string(),
            });
        }
        cards.sort_by(|a, b| a.name.cmp(&b.name));
        cards
    }

    /// Render the agent manager — Letta-style card deck. Each card has:
    ///   • a status badge (ACTIVE / PRIMARY) in the top-right
    ///   • a scale-to-fit portrait photo occupying the top ~55% of the card
    ///   • a dark metadata block below the photo, holding:
    ///       — seed glyph row + instance count
    ///       — agent name with `[AGENT]` tag
    ///       — agent id prefix as a path-style monospace breadcrumb
    ///       — a stats row (files / uptime / active duration placeholder)
    ///   • the primary agent gets a cyan accent border and bold weight
    ///
    /// Cards without a portrait file fall back to the half-block silhouette
    /// in the image slot so the grid stays geometrically uniform.
    ///
    /// `&mut self` is required because `StatefulImage` re-encodes the
    /// per-card protocol on each render to match the current cell area.
    fn draw_agent_cards_mut(&mut self, frame: &mut Frame) {
        use crate::ui::portrait;
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let bg = Block::default().style(Style::default().bg(palette.bg));
        frame.render_widget(bg, area);

        // ── Header strip ──────────────────────────────────────────────
        let header = Paragraph::new(Line::from(vec![
            Span::styled("  Agent Manager   ", Style::default()
                .fg(palette.agent_primary)
                .add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} agents  ·  manage, monitor, deploy", self.agent_cards.len()),
                Style::default().fg(palette.agent_dim),
            ),
        ])).alignment(Alignment::Center);
        let header_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 };
        frame.render_widget(header, header_area);

        if self.agent_cards.is_empty() {
            let empty = Paragraph::new("\n\n(no agents found — run `souveraine init`)")
                .style(Style::default().fg(palette.agent_dim))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        // ── Grid math ─────────────────────────────────────────────────
        // Letta shows 4 cards across; we pick the column count based on
        // available width so terminals down to ~50 cols still get usable
        // cards. Each card is taller than wide (portrait-style).
        let pad_x: u16 = 2;
        let pad_y: u16 = 1;
        let min_card_w: u16 = 22;
        let max_card_w: u16 = 32;
        let n: u16 = self.agent_cards.len() as u16;
        // Pick cols so card_w ∈ [min, max], preferring more cols on wider screens.
        let mut cols: u16 = 4;
        loop {
            let avail = area.width.saturating_sub((cols + 1) * pad_x);
            let cw = avail / cols.max(1);
            if cw >= min_card_w || cols == 1 { break; }
            cols -= 1;
        }
        cols = cols.min(n).max(1);
        self.manager_cols = cols as usize;
        // Clamp selection to valid range in case cards changed since last draw.
        self.manager_selected = self.manager_selected.min(self.agent_cards.len().saturating_sub(1));
        let avail = area.width.saturating_sub((cols + 1) * pad_x);
        let card_w = (avail / cols).min(max_card_w).max(min_card_w);
        // Card height: image area (target ~ card_w / 2 + 2, so a 24-wide card
        // gets 14 image rows) + 6 rows of metadata + 2 rows of border/badge.
        let image_h: u16 = (card_w / 2 + 3).max(8);
        let meta_h: u16 = 7;
        let card_h: u16 = image_h + meta_h + 2; // +2 for top/bottom border
        let grid_w = cols * card_w + (cols.saturating_sub(1)) * pad_x;
        let grid_x = area.x + area.width.saturating_sub(grid_w) / 2;
        let grid_y = area.y + 3;

        // Snapshot plans first so we can hold `&mut self.card_images` per card
        // without overlapping the immutable borrow of `self.agent_cards`.
        let manager_selected = self.manager_selected;
        struct Plan {
            card_area: Rect,
            image_area: Rect,
            badge_area: Rect,
            meta_area: Rect,
            agent_id: String,
            name: String,
            glyph: String,
            pubkey: String,
            instance_count: i64,
            uptime_pct: u8,
            memory_count: usize,
            is_primary: bool,
            is_selected: bool,
        }
        let plans: Vec<Plan> = self.agent_cards
            .iter()
            .enumerate()
            .filter_map(|(idx, card)| {
                let col = (idx as u16) % cols;
                let row = (idx as u16) / cols;
                let cx = grid_x + col * (card_w + pad_x);
                let cy = grid_y + row * (card_h + pad_y);
                if cy + card_h >= area.y + area.height.saturating_sub(2) {
                    return None;
                }
                let card_area = Rect { x: cx, y: cy, width: card_w, height: card_h };
                // Inner area inside the rounded border.
                let inner_w = card_w.saturating_sub(2);
                let inner_x = cx + 1;
                let image_y = cy + 1;
                let image_area = Rect { x: inner_x, y: image_y, width: inner_w, height: image_h };
                // Badge floats in the top-right corner of the image area,
                // overlaid as text spans (no separate widget).
                let badge_w: u16 = 10.min(inner_w);
                let badge_area = Rect {
                    x: inner_x + inner_w.saturating_sub(badge_w),
                    y: image_y,
                    width: badge_w,
                    height: 1,
                };
                let meta_area = Rect {
                    x: inner_x,
                    y: image_y + image_h,
                    width: inner_w,
                    height: meta_h,
                };
                Some(Plan {
                    card_area,
                    image_area,
                    badge_area,
                    meta_area,
                    agent_id: card.id.clone(),
                    name: card.name.clone(),
                    glyph: card.glyph.clone(),
                    pubkey: card.pubkey_prefix.clone(),
                    instance_count: card.instance_count,
                    uptime_pct: card.uptime_pct,
                    memory_count: card.memory_count,
                    is_primary: card.name.eq_ignore_ascii_case(&self.agent_pref),
                    is_selected: idx == manager_selected,
                })
            })
            .collect();

        // ── Render each card ──────────────────────────────────────────
        for p in plans {
            let accent = if p.is_primary {
                palette.agent_primary
            } else if p.instance_count > 0 {
                Color::Rgb(120, 220, 160) // active: green (semantic — keep)
            } else {
                palette.agent_dim
            };
            let border_color = if p.is_selected {
                palette.agent_primary
            } else if p.is_primary {
                palette.agent_primary
            } else {
                palette.agent_dim
            };

            // Card background fill (lifts the card off the screen).
            let (cr, cg, cb) = match palette.bg { Color::Rgb(r, g, b) => (r, g, b), _ => (16, 18, 28) };
            let card_bg = Block::default().style(Style::default().bg(Color::Rgb(cr.saturating_add(6), cg.saturating_add(6), cb.saturating_add(6))));
            frame.render_widget(card_bg, p.card_area);

            // Border — gold when cursor is here, violet for primary, dim otherwise.
            let border_modifier = if p.is_selected || p.is_primary {
                Modifier::BOLD
            } else {
                Modifier::DIM
            };
            let border = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color).add_modifier(border_modifier));
            frame.render_widget(border, p.card_area);

            // Image — scale-to-fit so the whole photo is visible. The
            // letterbox space inherits the card_bg above, which reads as
            // a clean dark frame.
            if let Some(proto) = self.card_images.get_mut(&p.agent_id) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    p.image_area,
                    proto,
                );
            } else {
                portrait::render(frame.buffer_mut(), p.image_area, &self.presence);
            }

            // Top-right badge: PRIMARY (with ★) or ACTIVE (with •) or muted.
            let (badge_text, badge_fg) = if p.is_primary {
                ("★ PRIMARY ", Color::Rgb(245, 230, 110)) // gold (semantic — keep)
            } else if p.instance_count > 0 {
                ("• ACTIVE  ", Color::Rgb(120, 220, 160)) // green (semantic — keep)
            } else {
                (" idle     ", palette.agent_dim)
            };
            let badge_para = Paragraph::new(Line::from(vec![
                Span::styled(badge_text, Style::default()
                    .fg(badge_fg)
                    .bg(palette.bg)
                    .add_modifier(Modifier::BOLD)),
            ])).alignment(Alignment::Right);
            frame.render_widget(badge_para, p.badge_area);

            // Metadata block — slightly darker inset under the photo.
            let (mr, mg, mb) = match palette.bg { Color::Rgb(r, g, b) => (r.saturating_sub(4), g.saturating_sub(4), b.saturating_sub(4)), _ => (12, 14, 22) };
            let meta_bg = Block::default().style(Style::default().bg(Color::Rgb(mr, mg, mb)));
            frame.render_widget(meta_bg, p.meta_area);

            let instance_label = if p.instance_count == 1 {
                "1 instance".to_string()
            } else {
                format!("{} instances", p.instance_count)
            };
            let path = format!("agents/{}", short_id(&p.agent_id));

            // Compose 7 lines into the meta_area:
            //   0: spacer
            //   1: glyph row + instance count
            //   2: name + [AGENT]
            //   3: path-style id
            //   4: separator rule
            //   5: stats (files / uptime / commits placeholder)
            //   6: action hint
            let meta_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!(" {} ", p.glyph),
                        Style::default().fg(accent).add_modifier(Modifier::BOLD)),
                    Span::styled(instance_label,
                        Style::default().fg(palette.agent_dim)),
                ]),
                Line::from(vec![
                    Span::styled(format!(" {} ", p.name),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("[AGENT]",
                        Style::default().fg(palette.agent_dim)
                            .bg(Color::Rgb(cr, cg, cb))),
                ]),
                Line::from(vec![
                    Span::styled(format!(" {} ", path),
                        Style::default().fg(palette.agent_dim)),
                ]),
                Line::from(Span::styled(
                    "─".repeat(p.meta_area.width as usize),
                    Style::default().fg(palette.agent_dim).add_modifier(Modifier::DIM),
                )),
                Line::from(vec![
                    Span::styled(" Files  ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(format!("{:<5}", p.memory_count),
                        Style::default().fg(Color::White)),
                    Span::styled("Uptime  ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(format!("{}%", p.uptime_pct),
                        Style::default().fg(Color::Rgb(120, 220, 160))), // green (semantic — keep)
                ]),
                Line::from(vec![
                    Span::styled(" key ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(p.pubkey.chars().take(12).collect::<String>(),
                        Style::default().fg(palette.agent_dim)),
                ]),
            ];
            let meta_para = Paragraph::new(meta_lines);
            frame.render_widget(meta_para, p.meta_area);
        }

        // ── Footer ────────────────────────────────────────────────────
        let footer = Paragraph::new("↑↓←→ navigate  •  Enter select  •  f favorite  •  Esc back")
            .style(Style::default().fg(palette.agent_dim))
            .alignment(Alignment::Center);
        let footer_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(2),
            width: area.width,
            height: 1,
        };
        frame.render_widget(footer, footer_area);
    }
}

// ─── Dashboard helpers ─────────────────────────────────────────────────────

/// Walk the agent's memory git log and return the last `n` commit subject lines,
/// formatted like `[hh:mm] subject`.
fn recent_commits(repo: &crate::core::memory::MemoryRepo, n: usize) -> anyhow::Result<Vec<String>> {
    let git_repo = git2::Repository::open(repo.root())?;
    let mut walker = git_repo.revwalk()?;
    walker.push_head()?;
    let mut out = Vec::new();
    for oid in walker.take(n) {
        let oid = oid?;
        let commit = git_repo.find_commit(oid)?;
        let summary = commit.summary().unwrap_or("(no message)");
        let secs = commit.time().seconds();
        let time = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
            .map(|d| d.format("%H:%M").to_string())
            .unwrap_or_else(|| "—".to_string());
        out.push(format!("[{}] {}", time, summary));
    }
    out.reverse();
    Ok(out)
}

fn short_now() -> String {
    chrono::Utc::now().format("%H:%M").to_string()
}

/// Trim a UUID-style agent id to a path-friendly short form for the
/// breadcrumb line in the agent manager. Strips a leading `agent-`
/// prefix if present, then keeps the first 8 hex chars.
fn short_id(agent_id: &str) -> String {
    let trimmed = agent_id.strip_prefix("agent-").unwrap_or(agent_id);
    trimmed.chars().take(8).collect()
}

/// Pre-process a portrait image for terminal display using a face-biased cover crop.
///
/// Implements the CSS `object-fit: cover` semantics with a top-center anchor:
/// - If the image is wider than `target_w:target_h`, center-crop horizontally
///   (the subject is usually centered).
/// - If the image is taller than the target ratio, crop from the TOP (faces sit
///   near the top of full-body portrait images — cropping the bottom preserves
///   the face and loses the feet).
///
/// The result has exactly the target aspect ratio. Pass it to ratatui-image with
/// `Resize::Fit` so it scales to fill the widget area without letterboxing.
fn portrait_cover_crop(img: image::DynamicImage, target_w: u32, target_h: u32) -> image::DynamicImage {
    let iw = img.width();
    let ih = img.height();
    // Compare cross-multiplied to avoid floating point.
    if iw * target_h > ih * target_w {
        // Image is wider than target ratio — center-crop horizontally.
        let new_w = ih * target_w / target_h;
        let x = (iw.saturating_sub(new_w)) / 2;
        img.crop_imm(x, 0, new_w, ih)
    } else {
        // Image is taller (or equal) — crop from the top, preserving the face.
        let new_h = iw * target_h / target_w;
        img.crop_imm(0, 0, iw, new_h)
    }
}

/// Clip a string to at most `max` characters using unicode-aware truncation.
/// Appends "…" if truncated.
#[allow(unused)]
fn clip_to(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
