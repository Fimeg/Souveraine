//! tuie-based App root widget.
//!
//! This is the root widget passed to `tuie::start_tui()`. It manages screen
//! transitions by swapping the inner widget. The first screen shown is the
//! SplashScreen; after the bloom animation completes (or the user presses a
//! key) and dashboard data has loaded, it transitions to the WelcomeScreen.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tuie::prelude::*;

use crate::core::config::ConsciousnessConfig;
use crate::ui::atmosphere::Atmosphere;
use crate::ui::chat::{ChatPalette, ChatState};
use crate::ui::presence::Posture;
use crate::ui::presence::Presence;
use crate::ui::screens::chat::ChatScreen;
use crate::ui::screens::cron::CronScreen;
use crate::ui::screens::presence::PresenceScreen;
use crate::ui::screens::settings::SettingsScreen;
use crate::ui::screens::splash::SplashScreen;
use crate::ui::screens::welcome::WelcomeScreen;
use crate::ui::theme;

// ── Agent types ─────────────────────────────────────────────────────────────────

/// A single agent known to the backend, with all the detail the unified agent
/// screen needs to render a rich master–detail view.
///
/// Supersedes both the lossy `(String, String, String)` tuples that
/// `AgentStatus` used to carry and `manager::AgentProcessInfo`.
#[derive(Debug, Clone, Default)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 4-glyph SeedID badge (e.g. "◇◆◇◆").
    pub glyph: String,
    /// Running instance count (≈ process count).
    pub instance_count: i64,
    /// Lifetime uptime percentage, capped at 99.
    pub uptime_pct: u8,
    /// Number of files in the agent's memory repo.
    pub memory_count: usize,
    /// First 16 hex chars of the pubkey.
    pub pubkey_prefix: String,
    /// Whether this agent is marked as the primary.
    pub is_primary: bool,
    /// Atmosphere name from the agent's preferences, if set.
    pub atmosphere: Option<String>,
    /// Recent commit subject lines or activity entries, newest first.
    pub recent_activity: Vec<String>,
}

// ── Agent status ───────────────────────────────────────────────────────────────

/// Live agent data populated by dashboard refresh.
///
/// This is a simplified clone of `app::AgentStatus` for the tuie path.
/// Once ratatui is removed, this becomes the canonical status struct.
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub name: String,
    /// The agent's backend ID — used to resolve portrait images on disk.
    pub agent_id: Option<String>,
    pub mood: String,
    pub energy: u8,
    pub memory_commits: u32,
    pub pending_tasks: usize,
    pub subconscious_active: bool,
    /// How the dashboard data was fetched ("local", "remote", or "—").
    pub mode: String,
    /// Last commit hash (short) on the agent's memory repo, if known.
    pub last_commit: Option<String>,
    /// Recent commit subject lines or activity entries, oldest → newest.
    pub recent_activity: Vec<String>,
    /// Number of agents the backend reports.
    pub agent_count: usize,
    /// All agents the backend knows about — used for agent selection.
    pub available_agents: Vec<AgentSummary>,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self {
            name: "Ani".to_string(),
            agent_id: None,
            mood: "—".to_string(),
            energy: 0,
            memory_commits: 0,
            pending_tasks: 0,
            subconscious_active: false,
            mode: "—".to_string(),
            last_commit: None,
            recent_activity: Vec::new(),
            agent_count: 0,
            available_agents: Vec::new(),
        }
    }
}

impl AgentSummary {
    /// Build a bare summary from backend `AgentInfo` — rich fields stay at default.
    pub fn from_agent_info(a: &crate::backend::AgentInfo) -> Self {
        Self {
            id: a.id.clone(),
            name: a.name.clone(),
            description: a.description.clone().unwrap_or_default(),
            ..Default::default()
        }
    }
}

// ── Screen enum ────────────────────────────────────────────────────────────────

/// Which screen is currently active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Splash,
    Welcome,
    Chat,
    Settings,
    Presence,
    Cron,
    Agents,
}

// ── TuieApp ────────────────────────────────────────────────────────────────────

/// Root widget for the tuie-based TUI.
///
/// Uses `DelegateWidget` — all rendering and layout is forwarded to the
/// currently active screen widget. Screen transitions replace the delegate.
pub struct TuieApp {
    current: Box<dyn Widget>,
    // Shared state
    config: Arc<RwLock<ConsciousnessConfig>>,
    _presence: Presence,
    palette: ChatPalette,
    // Active screen
    current_screen: Screen,
    // Agent info
    agent_name: String,
    _human_name: String,
    // Splash → Welcome transition
    splash_complete: Rc<Cell<bool>>,
    agent_status: Option<AgentStatus>,
    // Welcome menu selection signal (set by WelcomeScreen, consumed here)
    menu_action: Rc<Cell<Option<usize>>>,
    // Chat screen — stored so we can activate it after widget registration
    chat_screen_id: Option<WidgetId<ChatScreen>>,
    // Sub-screen exit signals
    presence_exit_signal: Option<Rc<Cell<bool>>>,
    manager_back_signal: Option<Rc<Cell<bool>>>,
    manager_kill_signal: Option<Rc<Cell<Option<usize>>>>,
    manager_restart_signal: Option<Rc<Cell<Option<usize>>>>,
    settings_go_back_signal: Option<Rc<Cell<bool>>>,
    settings_save_signal: Option<Rc<Cell<bool>>>,
    settings_fetch_models_signal: Option<Rc<Cell<bool>>>,
    /// Shared buffer for fetched models. Written by async task, read by SettingsScreen.
    settings_models_buffer: Rc<std::cell::RefCell<Option<Vec<String>>>>,
    /// Signal from SettingsScreen when atmosphere field is edited.
    settings_atmosphere_changed: Option<Rc<Cell<Option<String>>>>,
    /// Signal from SettingsScreen when outfit field is edited.
    settings_outfit_changed: Option<Rc<Cell<Option<String>>>>,
    // Cached config snapshot — avoids blocking_read() inside the runtime.
    config_snapshot: ConsciousnessConfig,
}

impl DelegateWidget for TuieApp {
    tuie::delegate_widget!(current);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        // Check for global keybinds first
        if let Some(event) = queue.peek() {
            if self.handle_global_key(&event.chord) {
                queue.next();
                return InputResult::Handled;
            }
        }

        // Forward to the active screen
        let result = self.get_delegate_mut().on_input(queue);

        // After any input, check for menu selection on Welcome screen.
        if self.current_screen == Screen::Welcome {
            if let Some(idx) = self.menu_action.take() {
                self.handle_menu_select(idx);
            }
        }

        // After any input, check for agent selection on Agents screen.
        if self.current_screen == Screen::Agents {
            if let Some(idx) = self.menu_action.take() {
                self.handle_agent_select(idx);
            }
        }

        // After any input, check for Presence screen exit signal.
        if self.current_screen == Screen::Presence {
            if let Some(ref signal) = self.presence_exit_signal {
                if signal.get() {
                    self.go_to_welcome();
                }
            }
        }

        // After any input, check for Agents screen signals (back / kill / restart).
        if self.current_screen == Screen::Agents {
            if let Some(ref signal) = self.manager_back_signal {
                if signal.get() {
                    self.go_to_welcome();
                }
            }
            if let Some(ref signal) = self.manager_kill_signal {
                if let Some(idx) = signal.take() {
                    tracing::info!("Kill requested for agent at index {}", idx);
                    // TODO: wire to backend kill_agent when available
                }
            }
            if let Some(ref signal) = self.manager_restart_signal {
                if let Some(idx) = signal.take() {
                    tracing::info!("Restart requested for agent at index {}", idx);
                    // TODO: wire to backend restart_agent when available
                }
            }
        }

        // After any input, check for Settings screen signals.
        if self.current_screen == Screen::Settings {
            // Check for model fetch request.
            if let Some(ref signal) = self.settings_fetch_models_signal {
                if signal.get() {
                    signal.set(false);
                    let config = self.config_snapshot.clone();
                    let extra: Vec<String> = self.config_snapshot.models.keys().cloned().collect();
                    let buffer = self.settings_models_buffer.clone();
                    let provider_id = self.config_snapshot.inference.provider.clone();
                    tuie::spawn(
                        self.get_id(),
                        async move {
                            let models = match crate::bridge::build_provider(&config) {
                                Ok(provider) => {
                                    let mut models =
                                        provider.list_models().await.unwrap_or_default();
                                    if models.is_empty() {
                                        tracing::warn!(
                                            "provider returned 0 models — endpoint may be down"
                                        );
                                    }
                                    for m in extra {
                                        if !models.contains(&m) {
                                            models.push(m);
                                        }
                                    }
                                    models
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "build_provider failed — using config models only");
                                    extra
                                }
                            };
                            (models, provider_id)
                        },
                        move |_app: &mut TuieApp, (models, provider_id): (Vec<String>, String)| {
                            // Cache to disk so the next session starts with data.
                            if !models.is_empty() {
                                let cache = crate::core::model_cache::ModelCache {
                                    fetched_at: chrono::Utc::now().to_rfc3339(),
                                    provider: provider_id,
                                    models: models.clone(),
                                };
                                if let Err(e) = cache.save() {
                                    tracing::warn!(error = %e, "failed to save model cache");
                                }
                            }
                            *buffer.borrow_mut() = Some(models);
                        },
                    );
                }
            }

            if let Some(ref signal) = self.settings_save_signal {
                if signal.get() {
                    // Config was already saved by SettingsScreen.
                    // Update the live config and snapshot.
                    // We can't downcast self.current to SettingsScreen, but the
                    // config was written to disk. Reload it.
                    if let Ok(saved) = crate::core::config::ConsciousnessConfig::load(
                        &crate::core::config::ConsciousnessConfig::discover_path()
                            .unwrap_or_else(|| PathBuf::from("souveraine.toml")),
                    ) {
                        self.config_snapshot = saved;
                    }
                    signal.set(false);
                }
            }
            if let Some(ref signal) = self.settings_go_back_signal {
                if signal.get() {
                    // Reload config snapshot from disk (was saved by SettingsScreen).
                    if let Ok(saved) = crate::core::config::ConsciousnessConfig::load(
                        &crate::core::config::ConsciousnessConfig::discover_path()
                            .unwrap_or_else(|| PathBuf::from("souveraine.toml")),
                    ) {
                        self.config_snapshot = saved;
                    }
                    signal.set(false);
                    self.go_to_welcome();
                }
            }

            // Live-apply atmosphere changes from settings.
            if let Some(ref signal) = self.settings_atmosphere_changed {
                if let Some(name) = signal.take() {
                    // Unknown falls back to Default here, unlike the presence
                    // path which leaves her choice alone: this arrives from the
                    // settings view's own enum, so a name it cannot parse is a
                    // bug in this binary rather than a typo from outside.
                    theme::apply_atmosphere(Atmosphere::from_name(&name).unwrap_or_default());
                }
            }

            // Live-apply outfit changes from settings.
            if let Some(ref signal) = self.settings_outfit_changed {
                if let Some(name) = signal.take() {
                    if name == "default" {
                        self._presence.outfit = None;
                    } else {
                        self._presence.outfit = Some(name);
                    }
                }
            }
        }

        // After any input, check if splash should transition
        if self.current_screen == Screen::Splash {
            self.try_transition_from_splash();
        }

        result
    }
}

impl TuieApp {
    /// Create the app root widget with shared state.
    ///
    /// Starts on the splash screen. A scheduled task periodically checks
    /// whether the splash is complete and dashboard data has loaded, then
    /// auto-transitions to the Welcome screen.
    pub fn new(
        config: Arc<RwLock<ConsciousnessConfig>>,
        presence: Presence,
        palette: ChatPalette,
        agent_name: String,
        human_name: String,
    ) -> Box<Self> {
        let splash_complete = Rc::new(Cell::new(false));
        let splash: Box<dyn Widget> = SplashScreen::new(&palette, splash_complete.clone());

        Box::new(Self {
            current: splash,
            config,
            _presence: presence,
            palette,
            current_screen: Screen::Splash,
            agent_name,
            _human_name: human_name,
            splash_complete,
            agent_status: None,
            menu_action: Rc::new(Cell::new(None)),
            chat_screen_id: None,
            presence_exit_signal: None,
            manager_back_signal: None,
            manager_kill_signal: None,
            manager_restart_signal: None,
            settings_go_back_signal: None,
            settings_save_signal: None,
            settings_fetch_models_signal: None,
            settings_models_buffer: Rc::new(std::cell::RefCell::new(None)),
            settings_atmosphere_changed: None,
            settings_outfit_changed: None,
            config_snapshot: ConsciousnessConfig::default(),
        })
    }

    /// Register the tokio runtime spawner with tuie.
    pub fn setup_spawner() {
        tuie::set_spawner(|fut| {
            tokio::spawn(fut);
        });
    }

    /// Start async dashboard refresh and schedule splash→welcome polling.
    ///
    /// Must be called AFTER `start_tui()` has begun (the widget must be
    /// registered in the widget tree for WidgetId lookups to work).
    pub fn begin(&mut self) {
        // 1. Start async dashboard loading.
        let app_id = self.get_id();
        let config = self.config.clone();
        let agent_name = self.agent_name.clone();

        tuie::spawn(
            app_id,
            async move { load_dashboard_data(config, agent_name).await },
            |app: &mut TuieApp, status: AgentStatus| {
                app.agent_status = Some(status);
                app.try_transition_from_splash();
                app.dirty_layout();
            },
        );

        // 1b. Cache config snapshot for screens that need it synchronously.
        let cfg = self.config.clone();
        let app_id2 = self.get_id();
        tuie::spawn(
            app_id2,
            async move { cfg.read().await.clone() },
            |app: &mut TuieApp, snapshot: ConsciousnessConfig| {
                app.config_snapshot = snapshot;
            },
        );

        // 2. Schedule periodic splash-completion checks so we auto-advance
        //    even without user input.
        self.schedule_splash_check();
    }

    /// Schedule a one-shot check for splash completion.
    ///
    /// Each tick marks the screen dirty so tuie re-renders the bloom animation.
    /// Re-schedules itself until the splash screen is no longer active.
    fn schedule_splash_check(&self) {
        let app_id = self.get_id();
        tuie::schedule(
            app_id,
            Duration::from_millis(33), // ~30fps for smooth bloom animation
            |app: &mut TuieApp| {
                if app.current_screen == Screen::Splash {
                    // Force re-render so the bloom advances.
                    tuie::dirty_paint();
                    app.try_transition_from_splash();
                    // If still on splash, schedule another check.
                    if app.current_screen == Screen::Splash {
                        app.schedule_splash_check();
                    }
                }
            },
        );
    }

    // ── Screen switching ───────────────────────────────────────────────────────

    /// Switch to a different screen.
    pub fn switch_screen(&mut self, screen: Screen) {
        self.current_screen = screen;

        let widget: Box<dyn Widget> = match screen {
            Screen::Splash => SplashScreen::new(&self.palette, self.splash_complete.clone()),
            Screen::Welcome => {
                let status = self.agent_status.clone().unwrap_or_default();
                let (welcome, menu_action) = WelcomeScreen::new(&self.palette, &status);
                self.menu_action = menu_action;
                welcome.schedule_breathe();
                welcome
            }
            Screen::Chat => {
                let mut chat =
                    ChatScreen::new(self.config.clone(), self.palette, self.agent_name.clone());
                let chat_id = chat.get_id();
                self.chat_screen_id = Some(chat_id);

                // Mark connecting immediately so the interstitial shows. The
                // async connect itself is kicked off AFTER `self.current` is
                // assigned below — see the `Screen::Chat` block after the match.
                chat.start_connecting();

                chat
            }
            Screen::Settings => {
                let config_path = crate::core::config::ConsciousnessConfig::discover_path()
                    .unwrap_or_else(|| PathBuf::from("souveraine.toml"));
                // Pre-populate the models buffer from the local cache so the
                // picker has data immediately, without waiting for a network fetch.
                let provider = &self.config_snapshot.inference.provider;
                if let Some(cache) = crate::core::model_cache::ModelCache::load(provider) {
                    *self.settings_models_buffer.borrow_mut() = Some(cache.models);
                } else {
                    *self.settings_models_buffer.borrow_mut() = None;
                }
                let mut settings = SettingsScreen::new(
                    &self.config_snapshot,
                    config_path,
                    self.settings_models_buffer.clone(),
                );
                // Pull cached models from the buffer so the field grid renders
                // with picker widgets from the first frame.
                settings.poll_models_from_buffer();
                settings.set_palette(self.palette);

                // Resolve the active agent into per-agent settings for the
                // Agent category — follows the same pattern as
                // settings_handler::active_agent_settings().
                if let Some(status) = &self.agent_status {
                    if let Some(id) = &status.agent_id {
                        let base = dirs::home_dir().unwrap_or_default();
                        let agent_json_path = base
                            .join(".souveraine/server/agents")
                            .join(id)
                            .join("agent.json");
                        let model = crate::ui::app::App::agent_model_from_disk(id);
                        let memory_root = base.join(".souveraine/agents").join(id).join("memory");
                        let subconscious_root = base
                            .join(".souveraine/subconscious-agents")
                            .join(format!("{id}-sub"))
                            .join("memory.git");
                        let has_subconscious = subconscious_root.join("HEAD").exists();
                        let agent_json_exists = agent_json_path.exists();

                        if let Some(model) = model {
                            let agent_settings = crate::ui::settings::ActiveAgentSettings {
                                id: id.clone(),
                                name: self.agent_name.clone(),
                                model: model.clone(),
                                model_original: model,
                                provider: None,
                                subconscious_id: format!("{id}-sub"),
                                memory_root,
                                subconscious_root,
                                has_subconscious,
                                agent_json_exists,
                            };
                            settings.set_active_agent(Some(agent_settings));
                        }
                    }
                }

                self.settings_go_back_signal = Some(settings.go_back_signal());
                self.settings_save_signal = Some(settings.save_signal());
                self.settings_fetch_models_signal = Some(settings.fetch_models_signal());
                self.settings_atmosphere_changed = Some(settings.atmosphere_changed_signal());
                self.settings_outfit_changed = Some(settings.outfit_changed_signal());
                settings
            }
            Screen::Presence => {
                let (presence, should_exit) =
                    PresenceScreen::new(&self.palette, Atmosphere::Default, Posture::Idle);
                self.presence_exit_signal = Some(should_exit);
                presence
            }
            Screen::Cron => {
                let schedules_dir = self
                    .config_snapshot
                    .schedules
                    .schedules_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("schedules"));
                CronScreen::new(schedules_dir, &self.palette)
            }
            Screen::Agents => {
                let agents: Vec<AgentSummary> = self
                    .agent_status
                    .as_ref()
                    .map(|s| s.available_agents.clone())
                    .unwrap_or_default();
                let (
                    agents_screen,
                    selection_signal,
                    back_signal,
                    kill_signal,
                    restart_signal,
                    pin_signal,
                ) = crate::ui::screens::agents::AgentsScreen::new(agents, &self.palette);
                self.menu_action = selection_signal;
                self.manager_back_signal = Some(back_signal);
                self.manager_kill_signal = Some(kill_signal);
                self.manager_restart_signal = Some(restart_signal);
                // pin_signal stored for future use (agent pin-to-primary).
                let _ = pin_signal;
                agents_screen
            }
        };

        self.current = widget;
        self.dirty_layout();

        // Kick off the async chat connect now that `self.current` IS the
        // ChatScreen. `TuieApp` is a DelegateWidget, so `self.get_id()` forwards
        // to the current screen's id — it must be read AFTER the swap, or it
        // resolves to the previous screen and the completion callback is
        // silently dropped (the id no longer exists in the tree). That stale-id
        // capture was the bug behind "waiting for connection" forever.
        if screen == Screen::Chat {
            let config = self.config.clone();
            let agent_name = self.agent_name.clone();
            tuie::spawn(
                self.get_id(),
                async move { ChatState::connect(config, &agent_name).await },
                |app: &mut TuieApp, result| {
                    if let Some(chat) = app.current.downcast_mut::<ChatScreen>() {
                        chat.finish_connect(result);
                    }
                    app.schedule_chat_poll();
                },
            );
        }
    }

    // ── Splash → Welcome transition ────────────────────────────────────────────

    /// If the splash is done and data is ready, transition to Welcome.
    fn try_transition_from_splash(&mut self) {
        if self.current_screen != Screen::Splash {
            return;
        }
        if !self.splash_complete.get() {
            return;
        }
        if self.agent_status.is_none() {
            return;
        }
        self.go_to_welcome();
    }

    fn go_to_welcome(&mut self) {
        self.current_screen = Screen::Welcome;
        let status = self.agent_status.clone().unwrap_or_default();
        let (welcome, menu_action) = WelcomeScreen::new(&self.palette, &status);
        self.menu_action = menu_action;
        welcome.schedule_breathe();
        self.current = welcome;
        self.dirty_layout();
    }

    // ── Welcome menu handling ──────────────────────────────────────────────────

    /// Handle a menu selection from the Welcome screen.
    fn handle_menu_select(&mut self, idx: usize) {
        match idx {
            0 => self.switch_screen(Screen::Chat),
            1 => self.switch_screen(Screen::Agents),
            2 => self.switch_screen(Screen::Cron),
            3 => self.switch_screen(Screen::Settings),
            _ => {}
        }
    }

    /// Handle an agent selection from the Agents screen.
    fn handle_agent_select(&mut self, idx: usize) {
        if let Some(ref mut status) = self.agent_status {
            if let Some(agent) = status.available_agents.get(idx) {
                status.name = agent.name.clone();
                status.agent_id = Some(agent.id.clone());
                self.agent_name = agent.name.clone();
                self.go_to_welcome();
            }
        }
    }

    // ── Chat polling ──────────────────────────────────────────────────────────

    /// Drive `ChatScreen::poll_tick` from the TuieApp level.
    ///
    /// ChatScreen is TuieApp's direct delegate, so they share a widget id.
    /// A `tuie::schedule` targeting ChatScreen's id would resolve to TuieApp
    /// and fail the downcast. Scheduling on TuieApp's own id works because
    /// `get_widget_mut::<TuieApp>(app_id)` resolves the root correctly.
    fn schedule_chat_poll(&self) {
        tuie::schedule(
            self.get_id(),
            Duration::from_millis(50),
            |app: &mut TuieApp| {
                if app.current_screen != Screen::Chat {
                    return;
                }
                if let Some(chat) = app.current.downcast_mut::<ChatScreen>() {
                    chat.poll_tick();
                }
                app.schedule_chat_poll();
            },
        );
    }

    // ── Global key handling ────────────────────────────────────────────────────

    /// Handle a global key press (not handled by child widgets).
    fn handle_global_key(&mut self, chord: &Chord) -> bool {
        use tuie::input::key::Key;
        use tuie::input::modifiers::Modifier;
        use tuie::input::trigger::Trigger;

        let Trigger::Key(key) = &chord.trigger else {
            return false;
        };

        // Ctrl+C quits from anywhere
        if *key == Key::Char('c') && chord.modifiers.has(Modifier::Ctrl) {
            self.quit();
            return true;
        }

        // Esc goes back to Welcome (unless already on Splash or Welcome)
        if *key == Key::Esc {
            if self.current_screen == Screen::Splash {
                // Esc on splash = skip (handled by SplashScreen's on_input)
                return false;
            }
            if self.current_screen != Screen::Welcome {
                self.go_to_welcome();
                return true;
            }
            return false;
        }

        // Screen-specific keys
        match self.current_screen {
            Screen::Splash => {
                // SplashScreen handles any key to skip; we don't intercept.
                false
            }
            Screen::Welcome => match key {
                Key::Char('q') => {
                    self.quit();
                    true
                }
                Key::Char('c') => {
                    self.switch_screen(Screen::Chat);
                    true
                }
                Key::Char('s') => {
                    self.switch_screen(Screen::Settings);
                    true
                }
                Key::Char('p') => {
                    self.switch_screen(Screen::Presence);
                    true
                }
                Key::Char('a') => {
                    self.switch_screen(Screen::Agents);
                    true
                }
                Key::Char('j') => {
                    self.switch_screen(Screen::Cron);
                    true
                }
                _ => false,
            },
            // Chat is a text-entry screen — never steal printable keys (e.g. 'q'),
            // or the user can't type words containing them.
            Screen::Chat => false,
            _ => {
                // 'q' on any non-text sub-screen goes back to Welcome.
                if *key == Key::Char('q') {
                    self.go_to_welcome();
                    true
                } else {
                    false
                }
            }
        }
    }

    // ── State access ───────────────────────────────────────────────────────────

    #[allow(dead_code)]
    pub fn set_atmosphere(&mut self, palette: ChatPalette, atm: crate::ui::atmosphere::Atmosphere) {
        self.palette = palette;
        theme::apply_atmosphere(atm);
    }

    pub fn quit(&self) {
        tuie::quit(0);
    }

    #[allow(dead_code)]
    pub fn current_screen(&self) -> Screen {
        self.current_screen
    }
}

// ── Async dashboard loading ────────────────────────────────────────────────────

/// Fetch agent data from whichever backend is reachable.
///
/// Tries remote first, falling back to local. Returns an `AgentStatus` with
/// the current agent count, mode, and basic health. This is the tuie-portable
/// equivalent of `App::refresh_dashboard()`.
async fn load_dashboard_data(
    config: Arc<RwLock<ConsciousnessConfig>>,
    agent_pref: String,
) -> AgentStatus {
    use crate::backend::Backend;

    let cfg = config.read().await;
    let url = cfg.server.effective_url();
    drop(cfg);

    // Try remote backend first.
    let remote = match crate::backend::RemoteBackend::new(&url) {
        Ok(r) => r,
        Err(_) => {
            return AgentStatus {
                name: agent_pref,
                ..Default::default()
            }
        }
    };
    if remote.health().await {
        match remote.list_agents().await {
            Ok(agents) => {
                let chosen = agents
                    .iter()
                    .find(|a| a.name == agent_pref || a.id == agent_pref)
                    .or_else(|| agents.first());

                let available_agents: Vec<AgentSummary> =
                    agents.iter().map(AgentSummary::from_agent_info).collect();

                return AgentStatus {
                    name: chosen.map(|a| a.name.clone()).unwrap_or(agent_pref),
                    agent_id: chosen.map(|a| a.id.clone()),
                    mood: "Active".to_string(),
                    energy: ((agents.len().min(10)) * 10) as u8,
                    memory_commits: 0,
                    pending_tasks: 0,
                    subconscious_active: true,
                    mode: "remote".to_string(),
                    last_commit: None,
                    recent_activity: vec![format!(
                        "connected via remote · {} agent{}",
                        agents.len(),
                        if agents.len() == 1 { "" } else { "s" },
                    )],
                    agent_count: agents.len(),
                    available_agents,
                };
            }
            Err(e) => {
                return AgentStatus {
                    name: agent_pref,
                    mood: format!("remote err: {}", e),
                    mode: "remote".to_string(),
                    ..Default::default()
                };
            }
        }
    }

    // Fall back to local backend.
    let cfg = config.read().await.clone();
    match crate::backend::LocalBackend::new(cfg).await {
        Ok(local) => match local.list_agents().await {
            Ok(agents) => {
                let chosen = agents
                    .iter()
                    .find(|a| a.name == agent_pref || a.id == agent_pref)
                    .or_else(|| agents.first());

                // Try to get memory repo stats for the chosen agent.
                let (commits, recent) = if let Some(a) = chosen {
                    let repo = local.server_agents().memory_repo(&a.id);
                    match repo.status() {
                        Ok(status) => {
                            let activity =
                                crate::ui::app::recent_commits(&repo, 8).unwrap_or_default();
                            (status.file_count as u32, activity)
                        }
                        Err(_) => (0, Vec::new()),
                    }
                } else {
                    (0, Vec::new())
                };

                // Enrich every agent with per-agent stats (glyph, instances,
                // uptime, memory count, pubkey, atmosphere) following the
                // ratatui `fetch_agent_cards` recipe.
                let inv = local.server_agents();
                let mut available_agents: Vec<AgentSummary> = Vec::new();
                for a in &agents {
                    let mut summary = AgentSummary::from_agent_info(a);
                    summary.glyph = inv.seed_id(&a.id).map(|s| s.glyph()).unwrap_or_default();
                    summary.pubkey_prefix = inv
                        .seed_id(&a.id)
                        .map(|s| s.public_key_hex()[..16].to_string())
                        .unwrap_or_default();
                    summary.instance_count = inv.instance_count(&a.id).await.unwrap_or(0);
                    let lifetime_secs = inv.lifetime_active_seconds(&a.id).await.unwrap_or(0);
                    summary.uptime_pct = if lifetime_secs > 0 {
                        let days = ((summary.instance_count.max(1)) as f64 * 30.0).max(1.0);
                        let pct = (lifetime_secs as f64 / (days * 86400.0)) * 100.0;
                        pct.min(99.0) as u8
                    } else {
                        0
                    };
                    summary.memory_count = local
                        .server_agents()
                        .memory_repo(&a.id)
                        .status()
                        .map(|s| s.file_count)
                        .unwrap_or(0);
                    // Per-agent atmosphere from preferences.
                    if let Some(home) = dirs::home_dir() {
                        let visual_path = home
                            .join(".souveraine")
                            .join("agents")
                            .join(&a.id)
                            .join("memory")
                            .join("system")
                            .join("preferences")
                            .join("visual.md");
                        if let Ok(contents) = std::fs::read_to_string(&visual_path) {
                            if let Some(atmosphere) = contents
                                .lines()
                                .find(|l| l.starts_with("atmosphere:"))
                                .and_then(|l| l.split(':').nth(1))
                                .map(|s| s.trim().trim_matches('"').to_string())
                            {
                                summary.atmosphere = Some(atmosphere);
                            }
                        }
                    }
                    // Recent activity for each agent.
                    let repo = local.server_agents().memory_repo(&a.id);
                    summary.recent_activity =
                        crate::ui::app::recent_commits(&repo, 5).unwrap_or_default();
                    // Mark primary — the first agent in the list is primary by convention.
                    summary.is_primary =
                        chosen.as_ref().map(|c| c.id == summary.id).unwrap_or(false);
                    available_agents.push(summary);
                }
                available_agents.sort_by(|a, b| a.name.cmp(&b.name));

                AgentStatus {
                    name: chosen.map(|a| a.name.clone()).unwrap_or(agent_pref),
                    agent_id: chosen.map(|a| a.id.clone()),
                    mood: if recent.is_empty() {
                        "Idle".to_string()
                    } else {
                        "Active".to_string()
                    },
                    energy: ((agents.len().min(10)) * 10) as u8,
                    memory_commits: commits,
                    pending_tasks: 0,
                    subconscious_active: true,
                    mode: "local".to_string(),
                    last_commit: recent.first().cloned(),
                    recent_activity: if recent.is_empty() {
                        vec![format!("agents on backend: {} · mode: local", agents.len(),)]
                    } else {
                        recent
                    },
                    agent_count: agents.len(),
                    available_agents,
                }
            }
            Err(e) => AgentStatus {
                name: agent_pref,
                mood: format!("local err: {}", e),
                mode: "local".to_string(),
                ..Default::default()
            },
        },
        Err(e) => AgentStatus {
            name: agent_pref,
            mood: format!("backend err: {}", e),
            mode: "—".to_string(),
            ..Default::default()
        },
    }
}
