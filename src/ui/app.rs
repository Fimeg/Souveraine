//! Souveraine - Full Terminal UI
//! Splash → Welcome → Dashboard / Chat / etc.
//!
//! The `App` holds a `Scene` which dispatches `TuiEvent` variants to all
//! registered `Component`s. Components are extracted here incrementally.
//! Existing draw methods remain until their panels become proper Components.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tokio::sync::RwLock;
use tracing::info;

use crate::core::config::ConsciousnessConfig;
use crate::ui::chat::{ChatState, draw as draw_chat};
use crate::ui::cockpit_panel::CockpitPane;
use crate::ui::presence::{Presence, draw_overlay as draw_presence_overlay};
use crate::ui::color_support::rgb;
use crate::ui::component::{Component, Scene, SceneLayout, TuiEvent};
use crate::backend::BackendEvent;

#[cfg(feature = "figlet-rs")]
use figlet_rs::FIGlet;

pub struct App {
    current_screen: Screen,
    splash_start: Instant,
    menu_selected: usize,
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
    /// Available agents for selection.
    available_agents: Vec<String>,
    /// Cursor index when the gallery is open.
    gallery_selected: usize,
    /// The component scene — owns event dispatch and layout.
    scene: Scene,
    /// Monotonic tick counter, incremented each frame.
    tick: u64,
    /// Splash bloom animation state.
    bloom: crate::ui::animation::bloom::BloomState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Screen {
    Splash,
    Welcome,
    Dashboard,
    Chat,
    Code,
    Therapy,
    AgentTime,
    Cron,
    Settings,
    /// "Be with her" mode — fullscreen breathing portrait, no chat input.
    Presence,
    /// Agent gallery — portrait grid of all available agents, choose one.
    Gallery,
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
    pub fn new(config: Arc<RwLock<ConsciousnessConfig>>, agent_pref: String) -> Self {
        info!("Creating Souveraine App");
        let mut app = Self {
            current_screen: Screen::Splash,
            splash_start: Instant::now(),
            menu_selected: 0,
            agent_status: AgentStatus { name: agent_pref.clone(), ..AgentStatus::default() },
            should_quit: false,
            config,
            chat: None,
            chat_error: None,
            schedules: None,
            agent_pref: agent_pref.clone(),
            presence: Presence::new(&agent_pref),
            available_agents: Vec::new(),
            gallery_selected: 0,
            scene: Scene::new(SceneLayout::Single),
            tick: 0,
            bloom: crate::ui::animation::bloom::BloomState::new(),
        };

        // CockpitPane listens for Aster's surfacing events as scrollable text.
        // Presence (Annie's body channel) lives outside the Scene because it's
        // an overlay, not a zoned component — App feeds it events via `dispatch`.
        app.scene.add(CockpitPane::new());

        app
    }

    /// Dispatch a TuiEvent to every listener: scene components AND Presence.
    /// Returns true if a redraw is needed.
    fn dispatch(&mut self, event: TuiEvent) -> bool {
        let scene_dirty = self.scene.event_all(&event);
        let presence_dirty = self.presence.handle_event(&event);
        scene_dirty || presence_dirty
    }

    /// Add an available agent for selection (WIP - called from backend discovery)
    pub fn add_available_agent(&mut self, agent_name: String) {
        if !self.available_agents.contains(&agent_name) {
            self.available_agents.push(agent_name);
        }
    }

    /// Select an agent as the primary companion
    pub fn select_agent(&mut self, agent_name: &str) {
        self.agent_pref = agent_name.to_string();
        self.agent_status.name = agent_name.to_string();
        self.dispatch(TuiEvent::AgentSelected(agent_name.to_string()));
    }

    /// Open the agent gallery — the portrait grid is the way to swap agents.
    fn open_gallery(&mut self) {
        if self.available_agents.is_empty() {
            // Seed defaults so the gallery is never empty on first open.
            let defaults = ["Annie", "Ani", "JeanLuc", "Eione"];
            for name in defaults {
                self.add_available_agent(name.to_string());
            }
        }
        self.gallery_selected = self
            .available_agents
            .iter()
            .position(|a| a == &self.agent_pref)
            .unwrap_or(0);
        self.current_screen = Screen::Gallery;
        self.dispatch(TuiEvent::ScreenChanged(Screen::Gallery));
    }

    fn handle_gallery_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.available_agents.is_empty() {
            self.current_screen = Screen::Welcome;
            return;
        }
        let cols = self.gallery_cols();
        let n = self.available_agents.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('g') => {
                self.current_screen = Screen::Welcome;
                self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.gallery_selected > 0 {
                    self.gallery_selected -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.gallery_selected + 1 < n {
                    self.gallery_selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.gallery_selected >= cols {
                    self.gallery_selected -= cols;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.gallery_selected + cols < n {
                    self.gallery_selected += cols;
                }
            }
            KeyCode::Enter => {
                let chosen = self.available_agents[self.gallery_selected].clone();
                self.select_agent(&chosen);
                self.current_screen = Screen::Welcome;
                self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
            }
            _ => {}
        }
    }

    fn gallery_cols(&self) -> usize {
        // Match draw_gallery's column count; safe default of 4 when terminal
        // dimensions aren't relevant for keyboard navigation correctness.
        4
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

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(100);

        while !self.should_quit {
            // Drain any pending backend events into the chat state before
            // rendering so streaming tokens land each tick.
            if let Some(chat) = self.chat.as_mut() {
                chat.drain_events();
                chat.advance_tick();

                // Forward consciousness events (surfacing, reflection, archivist)
                // from chat to the scene so Aster's observations reach Components.
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
                        _ => {}
                    }
                }
            }

            // Tick dispatch
            self.tick = self.tick.wrapping_add(1);
            self.dispatch(TuiEvent::Tick(self.tick));

            terminal.draw(|f| self.draw(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
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
                            Screen::Chat | Screen::Code => SceneLayout::ChatWithSidebar {
                                sidebar_ratio: 0.3,
                                sidebar_open: false,
                            },
                            Screen::Dashboard => SceneLayout::Dashboard,
                            Screen::Splash => SceneLayout::Single,
                            _ => SceneLayout::Single,
                        };
                    }
                    _ => {}
                }
            }

            if self.current_screen == Screen::Splash {
                if self.splash_start.elapsed() > Duration::from_secs(8) {
                    self.current_screen = Screen::Welcome;
                    self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
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
                self.current_screen = Screen::Welcome;
                self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
            }
            Screen::Welcome => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Up => if self.menu_selected > 0 { self.menu_selected -= 1; }
                    KeyCode::Down => if self.menu_selected < 6 { self.menu_selected += 1; }
                    KeyCode::Enter => self.select_menu_item().await,
                    KeyCode::Char('a') => {
                        // WIP: Create agent alias - this will be expanded with a full agent creation flow
                        // For now, cycle through available agents or create a default
                        self.cycle_agent_selection();
                    }
                    KeyCode::Char('p') => {
                        // Presence mode — sit with her, no chat input.
                        self.current_screen = Screen::Presence;
                        self.dispatch(TuiEvent::ScreenChanged(Screen::Presence));
                    }
                    KeyCode::Char('g') => {
                        // Gallery — the avatar IS the doorway to "who am I talking to."
                        self.open_gallery();
                    }
                    _ => {}
                }
            }
            Screen::Presence => {
                // Any key exits the meditative view.
                self.current_screen = Screen::Welcome;
                self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
            }
            Screen::Gallery => self.handle_gallery_key(key),
            Screen::Chat => self.handle_chat_key(key).await,
            Screen::Cron => self.handle_schedules_key(key),
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

    async fn handle_chat_key(&mut self, key: crossterm::event::KeyEvent) {
        use crate::ui::chat::Overlay;

        let Some(chat) = self.chat.as_mut() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

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

        // Normal chat key handling.
        match key.code {
            KeyCode::Esc => {
                self.current_screen = Screen::Welcome;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if !chat.busy && chat.input.len() < 8_192 {
                    chat.input.push('\n');
                    chat.update_completion();
                }
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !chat.busy && chat.input.len() < 8_192 {
                    chat.input.push('\n');
                    chat.update_completion();
                }
            }
            KeyCode::Enter => {
                if !chat.busy {
                    chat.submit();
                }
            }
            KeyCode::Backspace => {
                if !chat.busy {
                    chat.input.pop();
                    chat.update_completion();
                }
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
            KeyCode::Char(c) => {
                if !chat.busy && chat.input.len() < 8_192 {
                    chat.input.push(c);
                    chat.update_completion();
                }
            }
            _ => {}
        }
    }

    async fn select_menu_item(&mut self) {
        match self.menu_selected {
            0 => {
                // Best-effort live refresh of the dashboard data on entry.
                self.refresh_dashboard().await;
                self.current_screen = Screen::Dashboard;
            }
            // Both "Chat" and "Code" enter the chat screen — they're the same
            // endpoint today; specialized coding mode is future work.
            1 | 2 => {
                // Lazily connect to a backend the first time chat is opened.
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
            3 => self.current_screen = Screen::Therapy,
            4 => self.current_screen = Screen::AgentTime,
            5 => {
                if self.schedules.is_none() {
                    self.schedules = Some(self.build_schedules_view().await);
                } else if let Some(view) = self.schedules.as_mut() {
                    view.reload();
                }
                self.current_screen = Screen::Cron;
            }
            6 => self.current_screen = Screen::Settings,
            _ => self.current_screen = Screen::Welcome,
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

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let layout = match self.current_screen {
            Screen::Chat | Screen::Code => {
                SceneLayout::ChatWithSidebar { sidebar_ratio: 0.3, sidebar_open: false }
            }
            Screen::Dashboard => SceneLayout::Dashboard,
            Screen::Splash => SceneLayout::Single,
            _ => SceneLayout::Single,
        };

        match self.current_screen {
            Screen::Splash => self.draw_splash(frame),
            Screen::Welcome => self.draw_welcome(frame),
            Screen::Dashboard => self.draw_dashboard(frame),
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
            Screen::Presence => self.draw_presence_mode(frame),
            Screen::Gallery => self.draw_gallery(frame),
            _ => self.draw_placeholder(frame),
        }

        // Presence overlay on Welcome and Dashboard only — in Chat mode,
        // the cockpit panel shows agent state in its own register (words).
        if matches!(self.current_screen, Screen::Welcome | Screen::Dashboard) {
            draw_presence_overlay(frame, &self.presence, area);
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

    fn draw_welcome(&self, frame: &mut Frame) {
        use crate::ui::portrait;

        let area = frame.size();

        // Background
        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, area);

        // Avatar card occupies its own row in the welcome stack — centered,
        // framed, modest. It IS the "your agent is loaded" indicator. The
        // menu beneath then reads as actions on that agent.
        let avatar_card_w: u16 = portrait::RENDER_W + 2;
        let avatar_card_h: u16 = portrait::RENDER_H + 3;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(2),            // top breathing room
                Constraint::Length(4),            // title + subtitle
                Constraint::Length(avatar_card_h), // centered avatar
                Constraint::Min(8),               // menu
                Constraint::Length(3),            // footer
            ])
            .split(area);

        let breathe = self.presence.animator.breathe(3000);
        let glow = (140.0 + breathe * 60.0) as u8;

        let title = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "S O U V E R A I N E",
                Style::default()
                    .fg(Color::Rgb(255, glow, 66))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(Color::Rgb(180, 120, 80)),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(title, chunks[1]);

        // Centered avatar card under the title.
        if chunks[2].width >= avatar_card_w {
            let card_x = chunks[2].x + (chunks[2].width - avatar_card_w) / 2;
            let card_y = chunks[2].y;
            let card_area = Rect {
                x: card_x,
                y: card_y,
                width: avatar_card_w,
                height: avatar_card_h.min(chunks[2].height),
            };
            let border_col = if self.presence.subconscious_active {
                Color::Rgb(120, 200, 220)
            } else {
                Color::Rgb(120, 130, 150)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_col).add_modifier(Modifier::DIM));
            frame.render_widget(block, card_area);

            let portrait_area = Rect {
                x: card_area.x + 1,
                y: card_area.y + 1,
                width: portrait::RENDER_W,
                height: portrait::RENDER_H,
            };
            portrait::render(frame.buffer_mut(), portrait_area, &self.presence);

            let name_area = Rect {
                x: card_area.x + 1,
                y: card_area.y + 1 + portrait::RENDER_H,
                width: card_area.width.saturating_sub(2),
                height: 1,
            };
            let glyph = if self.presence.subconscious_active { "◈" } else { "·" };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {} ", glyph), Style::default().fg(border_col)),
                    Span::styled(
                        self.presence.name.clone(),
                        Style::default()
                            .fg(border_col)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]))
                .alignment(Alignment::Center),
                name_area,
            );
        }

        let menu_items = vec![
            ("📊 Dashboard", "See how your agent is doing"),
            ("💬 Chat", "Talk with your agent"),
            ("💻 Code", "Get right to coding"),
            ("🛋️ Therapy", "Agent therapy session"),
            ("⏰ Agent Time", "Give your agent time"),
            ("📅 Schedule", "Cron jobs & tasks"),
            ("⚙️ Settings", "Configure"),
        ];

        let menu: Vec<ListItem> = menu_items
            .iter()
            .enumerate()
            .map(|(i, (t, d))| {
                let style = if i == self.menu_selected {
                    Style::default()
                        .fg(Color::Rgb(255, 200, 100))
                        .bg(Color::Rgb(60, 40, 20))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", t), style),
                    Span::styled(format!("- {}", d), Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();

        let menu_widget = List::new(menu)
            .block(
                Block::default()
                    .title(" Main Menu ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(255, 140, 66)))
            );
        frame.render_widget(menu_widget, chunks[3]);

        // Surface any chat connect error so the user knows why Chat didn't open.
        if let Some(err) = &self.chat_error {
            let err_para = Paragraph::new(format!(" chat connect failed: {} ", err))
                .style(Style::default().fg(Color::Rgb(220, 100, 100)))
                .alignment(Alignment::Center);
            let row = Rect {
                x: chunks[3].x,
                y: chunks[3].y + chunks[3].height.saturating_sub(2),
                width: chunks[3].width,
                height: 1,
            };
            frame.render_widget(err_para, row);
        }

        let footer = Paragraph::new("↑↓ Navigate • Enter • a Add • g Gallery • p Presence • q Quit")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[4]);
    }

    fn draw_dashboard(&self, frame: &mut Frame) {
        let area = frame.size();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(10),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(area);

        let title = Paragraph::new(format!("✦ {} ✦", self.agent_status.name))
            .style(Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::Rgb(255, 140, 66)))
            );
        frame.render_widget(title, chunks[0]);

        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(chunks[1]);

        let energy_color = match self.agent_status.energy {
            0..=30 => Color::Red,
            31..=60 => Color::Yellow,
            _ => Color::Green,
        };

        let energy = Gauge::default()
            .block(Block::default().title(" Energy ").borders(Borders::ALL).border_type(BorderType::Rounded))
            .gauge_style(Style::default().fg(energy_color).bg(Color::Black))
            .percent(self.agent_status.energy as u16)
            .label(format!("{}%", self.agent_status.energy));
        frame.render_widget(energy, cards[0]);

        let mood = Paragraph::new(format!("\n◌\n\n{}", self.agent_status.mood))
            .alignment(Alignment::Center)
            .block(Block::default().title(" State ").borders(Borders::ALL).border_type(BorderType::Rounded));
        frame.render_widget(mood, cards[1]);

        let memory_label = match &self.agent_status.last_commit {
            Some(c) => format!("\n💾\n\n{} files\n{}", self.agent_status.memory_commits, c),
            None => format!("\n💾\n\n{} files", self.agent_status.memory_commits),
        };
        let memory = Paragraph::new(memory_label)
            .alignment(Alignment::Center)
            .block(Block::default().title(" Memory ").borders(Borders::ALL).border_type(BorderType::Rounded));
        frame.render_widget(memory, cards[2]);

        let agents_card = Paragraph::new(format!(
            "\n👥\n\n{} agent{}\non {}",
            self.agent_status.agent_count,
            if self.agent_status.agent_count == 1 { "" } else { "s" },
            self.agent_status.mode,
        ))
            .alignment(Alignment::Center)
            .block(Block::default().title(" Backend ").borders(Borders::ALL).border_type(BorderType::Rounded));
        frame.render_widget(agents_card, cards[3]);

        let activity_text = if self.agent_status.recent_activity.is_empty() {
            "(no recent activity — open Chat to begin)".to_string()
        } else {
            self.agent_status.recent_activity.join("\n")
        };
        let activity = Paragraph::new(activity_text)
            .block(
                Block::default()
                    .title(" Recent Activity ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
            );
        frame.render_widget(activity, chunks[2]);

        let footer = Paragraph::new("m Menu • q Quit")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[3]);
    }

    fn draw_placeholder(&self, frame: &mut Frame) {
        let area = frame.size();

        let screen_name = match self.current_screen {
            Screen::Chat => "💬 Chat",
            Screen::Code => "💻 Code",
            Screen::Therapy => "🛋️ Therapy",
            Screen::AgentTime => "⏰ Agent Time",
            Screen::Cron => "📅 Schedule",
            Screen::Settings => "⚙️ Settings",
            _ => "",
        };

        let content = Paragraph::new(format!("\n\n{}\n\n(Coming Soon)", screen_name))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD));
        frame.render_widget(content, area);
    }

    /// Gallery — the portrait grid is the way to swap agents.
    /// 4-column grid of portrait cards; cursor highlights with a bright border.
    /// For C3 every card shows Annie's portrait (per-agent portraits land in C4
    /// when image-protocol PNGs arrive from `assets/`).
    fn draw_gallery(&self, frame: &mut Frame) {
        use crate::ui::portrait;

        let area = frame.size();
        let bg = Block::default().style(Style::default().bg(Color::Rgb(10, 10, 16)));
        frame.render_widget(bg, area);

        // Header
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                "  Annie Composite — Agent Gallery  ",
                Style::default()
                    .fg(Color::Rgb(220, 215, 215))
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .alignment(Alignment::Center);
        let header_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 };
        frame.render_widget(header, header_area);

        if self.available_agents.is_empty() {
            let empty = Paragraph::new("\n\n(no agents available)")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        // Grid math: 4 cols, card = portrait card (CARD_W × CARD_H) + 2 row pad.
        let cols: u16 = self.gallery_cols() as u16;
        let card_w = portrait::RENDER_W + 2;
        let card_h = portrait::RENDER_H + 3;
        let pad_x: u16 = 2;
        let pad_y: u16 = 1;
        let total_grid_w = cols * card_w + (cols - 1) * pad_x;
        let grid_x = area.x + area.width.saturating_sub(total_grid_w) / 2;
        let grid_y = area.y + 3;

        for (idx, name) in self.available_agents.iter().enumerate() {
            let row = (idx as u16) / cols;
            let col = (idx as u16) % cols;
            let cx = grid_x + col * (card_w + pad_x);
            let cy = grid_y + row * (card_h + pad_y + 1);
            if cy + card_h + 1 >= area.y + area.height {
                break;
            }
            let selected = idx == self.gallery_selected;
            let border_col = if selected {
                Color::Rgb(120, 220, 230)
            } else {
                Color::Rgb(70, 70, 90)
            };

            let card_area = Rect { x: cx, y: cy, width: card_w, height: card_h };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(
                    Style::default()
                        .fg(border_col)
                        .add_modifier(if selected { Modifier::BOLD } else { Modifier::DIM }),
                );
            frame.render_widget(block, card_area);

            let portrait_area = Rect {
                x: cx + 1,
                y: cy + 1,
                width: portrait::RENDER_W,
                height: portrait::RENDER_H,
            };
            // For C3, all cards use the running Presence (Annie). C4 will load
            // per-agent portraits from assets/ in each agent's memfs.
            portrait::render(frame.buffer_mut(), portrait_area, &self.presence);

            let name_area = Rect {
                x: cx,
                y: cy + card_h,
                width: card_w,
                height: 1,
            };
            let primary_marker = if name == &self.agent_pref { "● " } else { "  " };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(primary_marker, Style::default().fg(Color::Rgb(120, 220, 230))),
                    Span::styled(
                        name.clone(),
                        Style::default()
                            .fg(if selected { Color::Rgb(220, 215, 215) } else { Color::Gray })
                            .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                ]))
                .alignment(Alignment::Center),
                name_area,
            );
        }

        // Footer
        let footer = Paragraph::new("← → ↑ ↓ navigate • Enter select • Esc cancel")
            .style(Style::default().fg(Color::Rgb(70, 70, 90)))
            .alignment(Alignment::Center);
        let footer_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(2),
            width: area.width,
            height: 1,
        };
        frame.render_widget(footer, footer_area);
    }

    /// Presence mode — fullscreen Annie. Centered, breathing, no chat input.
    /// Any keypress exits back to Welcome.
    fn draw_presence_mode(&self, frame: &mut Frame) {
        use crate::ui::portrait;

        let area = frame.size();
        let bg = Block::default().style(Style::default().bg(Color::Rgb(8, 8, 14)));
        frame.render_widget(bg, area);

        // Figure out the biggest scale that fits, centered. Use scale = min(area_w/W, 2*area_h/(H/2)).
        let max_scale_w = area.width / portrait::PORTRAIT_W;
        // Half the rows occupy 1 cell each before scaling; pixel→cell ratio is scale/2 vertical.
        let max_scale_h = (2 * area.height) / portrait::PORTRAIT_H;
        let scale = max_scale_w.min(max_scale_h).max(1);

        let cell_w = scale;
        let cell_h = (scale / 2).max(1);
        let portrait_w = portrait::PORTRAIT_W * cell_w;
        let portrait_h = (portrait::PORTRAIT_H / 2) * cell_h;

        let ox = area.x + area.width.saturating_sub(portrait_w) / 2;
        let oy = area.y + area.height.saturating_sub(portrait_h + 2) / 2;

        let portrait_area = Rect {
            x: ox,
            y: oy,
            width: portrait_w,
            height: portrait_h,
        };
        portrait::render_scaled(frame.buffer_mut(), portrait_area, &self.presence, scale);

        // Name line below.
        let name_line = Line::from(vec![
            Span::styled("◈ ", Style::default().fg(Color::Rgb(120, 200, 220))),
            Span::styled(
                self.presence.name.clone(),
                Style::default()
                    .fg(Color::Rgb(220, 215, 215))
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let name_area = Rect {
            x: area.x,
            y: portrait_area.y + portrait_h + 1,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(name_line).alignment(Alignment::Center),
            name_area,
        );

        // Quiet footer hint.
        let footer = Paragraph::new("press any key to return")
            .style(Style::default().fg(Color::Rgb(60, 60, 80)))
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
