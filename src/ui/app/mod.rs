#![allow(dead_code, deprecated)] // legacy ratatui render path + WIP scaffolding, pending tuie parity
//! Souveraine - Full Terminal UI
//! Splash → Welcome → Dashboard / Chat / etc.
//!
//! The `App` holds a `Scene` which dispatches `TuiEvent` variants to all
//! registered `Component`s. Components are extracted here incrementally.
//! Existing draw methods remain until their panels become proper Components.

mod agents;
mod dashboard;
mod images;
mod keymap;
mod manager_screen;
mod presence_screen;
mod settings_handler;
mod splash;
mod voice;
mod welcome;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Alignment,
    style::{Modifier, Style},
    widgets::Paragraph,
    Frame, Terminal,
};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

use crate::backend::BackendEvent;
use crate::core::config::ConsciousnessConfig;
use crate::ui::chat::{draw as draw_chat, ChatState};
use crate::ui::cockpit_panel::CockpitPane;
use crate::ui::component::{Scene, SceneLayout, TuiEvent};
use crate::ui::health_panel::HealthPane;
use crate::ui::presence::{Posture, Presence};
use crate::ui::setup::SetupState;

use ratatui_image::{
    picker::Picker,
    protocol::{Protocol, StatefulProtocol},
};

pub struct App {
    current_screen: Screen,
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
    pub fn new(
        config: Arc<RwLock<ConsciousnessConfig>>,
        agent_pref: String,
        config_path: Option<PathBuf>,
    ) -> Self {
        info!("Creating Souveraine App");
        let mut app = Self {
            current_screen: Screen::Splash,
            menu_selected: 0,
            setup_state: None,
            welcome_hint: None,
            agent_status: AgentStatus {
                name: agent_pref.clone(),
                ..AgentStatus::default()
            },
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
        if matches!(
            event,
            TuiEvent::AtmosphereChanged(_)
                | TuiEvent::MoodChanged(_)
                | TuiEvent::SubconsciousPass(_)
                | TuiEvent::PressureChanged(..)
        ) || (matches!(event, TuiEvent::Tick(_)) && self.presence.lerp_t < 1.0)
        {
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

    // Add an available agent for selection (WIP - called from backend discovery)

    pub async fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            event::EnableBracketedPaste
        )?;
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
        let tick_rate = Duration::from_millis(33); // ~30 FPS cap

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
                        BackendEvent::Surfacing {
                            source,
                            content,
                            priority,
                        } => {
                            self.dispatch(TuiEvent::Surfacing {
                                source,
                                content,
                                priority,
                            });
                        }
                        BackendEvent::Reflection(content) => {
                            self.dispatch(TuiEvent::Reflection { content });
                        }
                        BackendEvent::Archivist {
                            synthesis,
                            pressure,
                        } => {
                            self.dispatch(TuiEvent::Archivist {
                                synthesis,
                                pressure,
                            });
                        }
                        BackendEvent::CompactionWarning { pressure, tier } => {
                            self.dispatch(TuiEvent::CompactionWarning { pressure, tier });
                        }
                        BackendEvent::ContextPressure {
                            pressure,
                            context_limit,
                            ..
                        } => {
                            self.dispatch(TuiEvent::PressureChanged(pressure, context_limit));
                        }
                        BackendEvent::InferenceStrain {
                            attempt, status, ..
                        } => {
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
                        BackendEvent::Itinerary(line) => {
                            self.dispatch(TuiEvent::ItineraryChanged(line));
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
                        self.dispatch(TuiEvent::Resize {
                            width: w,
                            height: h,
                        });
                        self.scene.layout = match self.current_screen {
                            Screen::Chat => SceneLayout::ChatWithSidebar {
                                sidebar_ratio: 0.3,
                                sidebar_open: false,
                            },
                            Screen::Splash => SceneLayout::Single,
                            _ => SceneLayout::Single,
                        };
                    }
                    Event::Paste(data) => {
                        if self.current_screen == Screen::Chat {
                            if let Some(chat) = self.chat.as_mut() {
                                // Only count lines of what will actually fit
                                // so the tag is accurate.
                                chat.insert_at_cursor(&data);
                                let inserted = data.len();
                                let lines = data.lines().count();
                                if lines > 3 {
                                    chat.system_message(format!(
                                        "*[pasted {} lines — {} chars]*",
                                        lines, inserted
                                    ));
                                }
                            }
                        }
                    }
                    Event::Mouse(m) => {
                        use crossterm::event::{MouseButton, MouseEventKind};
                        if self.current_screen == Screen::Chat {
                            if let Some(chat) = self.chat.as_mut() {
                                match m.kind {
                                    // Left-click on a message bubble copies it
                                    // to the clipboard (the cue in the title).
                                    MouseEventKind::Down(MouseButton::Left) => {
                                        chat.copy_message_at(m.column, m.row);
                                    }
                                    // Wheel scrolls whichever pane the cursor
                                    // is over — thinking, subconscious, or the
                                    // message history. A notch is three lines.
                                    MouseEventKind::ScrollUp => {
                                        chat.wheel_scroll(m.column, m.row, true);
                                    }
                                    MouseEventKind::ScrollDown => {
                                        chat.wheel_scroll(m.column, m.row, false);
                                    }
                                    _ => {}
                                }
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
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            event::DisableBracketedPaste
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use keymap::{Action, KeyContext};

        // Screens with their own bespoke handlers (form text editing, async
        // transitions) keep their routing; everything else flows through the
        // central keymap (see `keymap.rs`).
        match self.current_screen {
            Screen::Splash => {
                self.transition_from_splash().await;
                return;
            }
            Screen::Setup => {
                // Capture the setup_complete and step_was_welcome flags before
                // calling any method that borrows self.setup_state, to avoid a
                // borrow conflict with finish_setup() taking &mut self.
                let should_skip = matches!(key.code, KeyCode::Esc);
                let step_is_welcome = self
                    .setup_state
                    .as_ref()
                    .map(|s| s.step == crate::ui::setup::SetupStep::Welcome)
                    .unwrap_or(false);

                if should_skip && step_is_welcome {
                    self.finish_setup().await;
                    return;
                }
                if let Some(ref mut setup) = self.setup_state {
                    setup.handle_key(key);
                }
                if self
                    .setup_state
                    .as_ref()
                    .map(|s| s.complete)
                    .unwrap_or(false)
                {
                    self.finish_setup().await;
                }
                return;
            }
            Screen::Chat => {
                self.handle_chat_key(key).await;
                return;
            }
            Screen::Cron => {
                self.handle_schedules_key(key);
                return;
            }
            Screen::Settings => {
                self.handle_settings_key(key).await;
                return;
            }
            _ => {}
        }

        // Keymap-driven screens.
        let ctx = match self.current_screen {
            Screen::Welcome => KeyContext::Welcome,
            Screen::Presence => KeyContext::Presence,
            Screen::AgentsManager => KeyContext::AgentsManager,
            _ => KeyContext::GenericBack,
        };
        let action = keymap::resolve(ctx, key.code, key.modifiers);
        match (ctx, action) {
            (KeyContext::Welcome, Some(a)) => self.welcome_action(a).await,
            (KeyContext::Presence, Some(a)) => self.presence_action(a).await,
            (KeyContext::Presence, None) => {
                // Unbound key while Idle and silent: treat as "go back."
                if self.presence.posture == Posture::Idle && self.voice_capture.is_none() {
                    self.exit_presence();
                }
            }
            (KeyContext::AgentsManager, Some(a)) => self.manager_action(a).await,
            (KeyContext::GenericBack, Some(Action::Back)) => {
                self.current_screen = Screen::Welcome;
            }
            _ => {}
        }
    }

    async fn welcome_action(&mut self, action: keymap::Action) {
        use keymap::Action;
        match action {
            Action::Quit => self.should_quit = true,
            Action::MenuUp => {
                if self.menu_selected > 0 {
                    self.menu_selected -= 1;
                }
            }
            Action::MenuDown => {
                if self.menu_selected < 4 {
                    self.menu_selected += 1;
                }
            }
            Action::MenuSelect => self.select_menu_item().await,
            Action::CycleAgent => {
                // WIP: Create agent alias - this will be expanded with a full agent creation flow
                // For now, cycle through available agents or create a default
                self.cycle_agent_selection();
            }
            Action::OpenPresence => {
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
            Action::OpenManager => self.open_agent_manager().await,
            _ => {}
        }
    }

    async fn presence_action(&mut self, action: keymap::Action) {
        use keymap::Action;
        match action {
            // Esc: interrupt Speaking → Idle; or exit Presence when Idle.
            Action::PresenceEsc => {
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
            Action::PresenceRecordToggle => {
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
            Action::PresenceExit => self.exit_presence(),
            // Vocal Recall keys: r = replay, g = regen, s = save
            Action::PresenceReplay => {
                if let Some(bytes) = self.voice_last_tts_bytes.clone() {
                    if let Some(player) = &self.voice_player {
                        player.stop();
                        let _ = player.play_mp3(bytes);
                        self.presence.set_posture(Posture::Speaking);
                    }
                }
            }
            Action::PresenceRegen => {
                if let Some(text) = self.voice_last_tts_text.clone() {
                    self.voice_last_synthesized = None; // force re-synth
                    self.tts_last_text = Some(text); // stash for the pipeline
                }
            }
            Action::PresenceSave => {
                if let (Some(bytes), Some(text)) = (
                    self.voice_last_tts_bytes.as_ref(),
                    self.voice_last_tts_text.as_ref(),
                ) {
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
            _ => {}
        }
    }

    async fn manager_action(&mut self, action: keymap::Action) {
        use keymap::Action;
        let n = self.agent_cards.len();
        let cols = self.manager_cols.max(1);
        match action {
            Action::Back => self.current_screen = Screen::Welcome,
            Action::ManagerLeft => {
                if self.manager_selected > 0 {
                    self.manager_selected -= 1;
                }
            }
            Action::ManagerRight => {
                if n > 0 && self.manager_selected + 1 < n {
                    self.manager_selected += 1;
                }
            }
            Action::ManagerUp => {
                if self.manager_selected >= cols {
                    self.manager_selected -= cols;
                }
            }
            Action::ManagerDown => {
                if n > 0 && self.manager_selected + cols < n {
                    self.manager_selected += cols;
                }
            }
            Action::ManagerSelect => {
                if let Some(card) = self.agent_cards.get(self.manager_selected) {
                    let name = card.name.clone();
                    self.select_agent(&name);
                    self.refresh_dashboard().await;
                    self.current_screen = Screen::Welcome;
                }
            }
            // f — pin/star selected card as favorite primary without leaving the manager
            Action::ManagerPin => {
                if let Some(card) = self.agent_cards.get(self.manager_selected) {
                    let name = card.name.clone();
                    self.select_agent(&name);
                    self.refresh_dashboard().await;
                }
            }
            _ => {}
        }
    }

    async fn handle_chat_key(&mut self, key: crossterm::event::KeyEvent) {
        use crate::ui::chat::Overlay;
        use keymap::{Action, KeyContext};

        // Chat selected but no session yet — only "go back" is bound.
        if self.chat.is_none() {
            if let Some(Action::Back) =
                keymap::resolve(KeyContext::ChatDisconnected, key.code, key.modifiers)
            {
                self.current_screen = Screen::Welcome;
            }
            return;
        }

        // Resolve the active modal sub-context. Precedence matches the old
        // nested guards: /btw pane > overlay > esc-overlay > normal input.
        let ctx = {
            let chat = self.chat.as_ref().unwrap();
            if chat.btw_active() {
                KeyContext::ChatBtw
            } else {
                match &chat.overlay {
                    Overlay::SlashComplete { .. } => KeyContext::ChatSlashComplete,
                    Overlay::ConversationPicker { .. } => KeyContext::ChatConvPicker,
                    Overlay::None => {
                        if chat.show_esc_overlay && chat.busy {
                            KeyContext::ChatEscOverlay
                        } else {
                            KeyContext::Chat
                        }
                    }
                }
            }
        };

        let action = keymap::resolve(ctx, key.code, key.modifiers);

        match ctx {
            // ── /btw fork pane ───────────────────────────────
            KeyContext::ChatBtw => {
                let chat = self.chat.as_mut().unwrap();
                match action {
                    Some(Action::BtwDismiss) => chat.btw_dismiss(),
                    Some(Action::BtwJump) => {
                        if let Some(forked_id) = chat.btw_jump() {
                            // Switch to the forked conversation.
                            let backend = chat.backend.clone();
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            chat.switch_rx = Some(rx);
                            let _agent_id = chat.agent_id.clone();
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
                    }
                    _ => {}
                }
            }
            // ── Slash-command completion (misses type a char) ─
            KeyContext::ChatSlashComplete => match action {
                Some(Action::OverlayUp) => {
                    let chat = self.chat.as_mut().unwrap();
                    if let Overlay::SlashComplete { selected, matches } = &chat.overlay {
                        let count = matches.len();
                        let sel = if *selected == 0 {
                            count.saturating_sub(1)
                        } else {
                            selected - 1
                        };
                        if let Overlay::SlashComplete {
                            selected: ref mut s,
                            ..
                        } = chat.overlay
                        {
                            *s = sel;
                        }
                    }
                }
                Some(Action::OverlayDown) => {
                    let chat = self.chat.as_mut().unwrap();
                    if let Overlay::SlashComplete { selected, matches } = &chat.overlay {
                        let count = matches.len();
                        let sel = if *selected + 1 >= count {
                            0
                        } else {
                            selected + 1
                        };
                        if let Overlay::SlashComplete {
                            selected: ref mut s,
                            ..
                        } = chat.overlay
                        {
                            *s = sel;
                        }
                    }
                }
                Some(Action::OverlayAccept) => self.chat.as_mut().unwrap().accept_completion(),
                Some(Action::OverlayCancel) => self.chat.as_mut().unwrap().overlay = Overlay::None,
                // Anything else falls through to normal input handling.
                _ => self.chat_input_action(key).await,
            },
            // ── Conversation picker (fully modal) ────────────
            KeyContext::ChatConvPicker => {
                let chat = self.chat.as_mut().unwrap();
                match action {
                    Some(Action::OverlayUp) => {
                        if let Overlay::ConversationPicker {
                            selected,
                            conversations,
                        } = &chat.overlay
                        {
                            let count = conversations.len();
                            let sel = if *selected == 0 {
                                count.saturating_sub(1)
                            } else {
                                selected - 1
                            };
                            if let Overlay::ConversationPicker {
                                selected: ref mut s,
                                ..
                            } = chat.overlay
                            {
                                *s = sel;
                            }
                        }
                    }
                    Some(Action::OverlayDown) => {
                        if let Overlay::ConversationPicker {
                            selected,
                            conversations,
                        } = &chat.overlay
                        {
                            let count = conversations.len();
                            let sel = if *selected + 1 >= count {
                                0
                            } else {
                                selected + 1
                            };
                            if let Overlay::ConversationPicker {
                                selected: ref mut s,
                                ..
                            } = chat.overlay
                            {
                                *s = sel;
                            }
                        }
                    }
                    Some(Action::OverlayAccept) => chat.accept_conversation_pick(),
                    Some(Action::PickerNewConversation) => {
                        // Keep the fresh conversation connect() already made.
                        chat.overlay = Overlay::None;
                        chat.system_message("New conversation.".to_string());
                    }
                    Some(Action::OverlayCancel) => chat.overlay = Overlay::None,
                    _ => {} // modal — swallow everything else
                }
            }
            // ── Esc interrupt-or-leave overlay ───────────────
            KeyContext::ChatEscOverlay => match action {
                Some(Action::EscOverlayResume) => {
                    // Hide overlay, stay in chat, turn keeps running.
                    self.chat.as_mut().unwrap().show_esc_overlay = false;
                }
                Some(Action::EscOverlayInterject) => {
                    let chat = self.chat.as_mut().unwrap();
                    chat.raise_hand();
                    chat.show_esc_overlay = false;
                }
                Some(Action::EscOverlayLeave) => {
                    self.chat.as_mut().unwrap().show_esc_overlay = false;
                    self.current_screen = Screen::Welcome;
                }
                _ => {} // block all other keys while overlay is up
            },
            // ── Normal text input ────────────────────────────
            KeyContext::Chat => self.chat_input_action(key).await,
            _ => {}
        }
    }

    /// Normal chat-input handling. Unbound keys fall through to inserting the
    /// typed character — that's why printable chars have no rows in the keymap.
    async fn chat_input_action(&mut self, key: crossterm::event::KeyEvent) {
        use keymap::{Action, KeyContext};

        let action = keymap::resolve(KeyContext::Chat, key.code, key.modifiers);
        let Some(chat) = self.chat.as_mut() else {
            return;
        };
        match action {
            Some(Action::ChatEsc) => {
                if chat.busy {
                    chat.show_esc_overlay = !chat.show_esc_overlay;
                } else {
                    self.current_screen = Screen::Welcome;
                }
            }
            Some(Action::InsertNewline) => {
                if chat.input.len() < 8_192 {
                    chat.input.insert(chat.input_cursor, '\n');
                    chat.input_cursor += '\n'.len_utf8();
                    chat.update_completion();
                }
            }
            Some(Action::Submit) => {
                // Submit always — when busy, this becomes an interjection
                // (queued and prepended to the agent's next LLM round).
                chat.submit();
            }
            Some(Action::Backspace) => {
                if chat.input_cursor > 0 {
                    let prev = chat.input[..chat.input_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    chat.input.remove(prev);
                    chat.input_cursor = prev;
                    chat.update_completion();
                }
            }
            Some(Action::CharLeft) => {
                if chat.input_cursor > 0 {
                    chat.input_cursor = chat.input[..chat.input_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            Some(Action::CharRight) => {
                if chat.input_cursor < chat.input.len() {
                    if let Some(c) = chat.input[chat.input_cursor..].chars().next() {
                        chat.input_cursor += c.len_utf8();
                    }
                }
            }
            Some(Action::WordLeft) => {
                // Step back to start of current or previous word.
                let before = &chat.input[..chat.input_cursor];
                let prev_word = before
                    .char_indices()
                    .rev()
                    .skip_while(|(_, c)| c.is_whitespace())
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| {
                        i + before[i..]
                            .chars()
                            .next()
                            .map(|c| c.len_utf8())
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                chat.input_cursor = prev_word;
            }
            Some(Action::WordRight) => {
                // Step forward to start of next word.
                let from = &chat.input[chat.input_cursor..];
                let first_non_space = from.find(|c: char| !c.is_whitespace());
                let next_word = match first_non_space {
                    Some(i) => {
                        let after_ws = &from[i..];
                        match after_ws.find(char::is_whitespace) {
                            Some(ws_end) => chat.input_cursor + i + ws_end,
                            None => chat.input.len(),
                        }
                    }
                    None => chat.input.len(),
                };
                chat.input_cursor = next_word;
            }
            Some(Action::LineHome) => chat.input_cursor = 0,
            Some(Action::LineEnd) => chat.input_cursor = chat.input.len(),
            Some(Action::ToggleCockpit) => chat.toggle_cockpit(),
            Some(Action::Quit) => self.should_quit = true,
            Some(Action::ToggleToolCards) => chat.tool_cards_expanded = !chat.tool_cards_expanded,
            Some(Action::PasteImage) => chat.paste_clipboard_image(),
            // Unbound key → type it.
            None => {
                if let KeyCode::Char(c) = key.code {
                    if chat.input.len() < 8_192 {
                        chat.input.insert(chat.input_cursor, c);
                        chat.input_cursor += c.len_utf8();
                        chat.update_completion();
                    }
                }
            }
            _ => {}
        }
    }

    /// eager-load and dashboard refresh.
    /// the DB layer to round-trip name → id.
    fn draw(&mut self, frame: &mut Frame) {
        let _area = frame.size();
        let _layout = match self.current_screen {
            Screen::Chat => SceneLayout::ChatWithSidebar {
                sidebar_ratio: 0.3,
                sidebar_open: false,
            },
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

    // draw_splash moved to splash.rs

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
            .style(
                Style::default()
                    .fg(palette.agent_primary)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(content, area);
    }
}

// ─── Dashboard helpers ─────────────────────────────────────────────────────

/// Walk the agent's memory git log and return the last `n` commit subject lines,
/// formatted like `[hh:mm] subject`.
pub(crate) fn recent_commits(
    repo: &crate::core::memory::MemoryRepo,
    n: usize,
) -> anyhow::Result<Vec<String>> {
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
fn portrait_cover_crop(
    img: image::DynamicImage,
    target_w: u32,
    target_h: u32,
) -> image::DynamicImage {
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
