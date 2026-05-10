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
    layout::{Alignment, Constraint, Direction, Layout},
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
use crate::ui::buddy::{BuddyState, draw_buddy, draw_welcome_buddy};
use crate::ui::buddy_panel::BuddyPanel;
use crate::ui::cockpit_panel::CockpitPane;
use crate::ui::component::{Component, Scene, SceneLayout, TuiEvent};
use crate::backend::BackendEvent;

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
    /// Agent name preference (from `--agent` CLI flag).
    agent_pref: String,
    /// Companion buddy for visual agent representation (WIP).
    buddy: BuddyState,
    /// Available agents for selection.
    available_agents: Vec<String>,
    /// The component scene — owns event dispatch and layout.
    scene: Scene,
    /// Monotonic tick counter, incremented each frame.
    tick: u64,
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
            agent_pref: agent_pref.clone(),
            buddy: BuddyState::new(&agent_pref),
            available_agents: Vec::new(),
            scene: Scene::new(SceneLayout::Single),
            tick: 0,
        };

        // Register standard components so they receive events from the start.
        // BuddyPanel and CockpitPane listen for surfacing events from Aster.
        app.scene.add(BuddyPanel::new(&agent_pref));
        app.scene.add(CockpitPane::new());

        app
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
        self.buddy.sprite.name = agent_name.to_string();
        self.scene.event_all(&TuiEvent::AgentSelected(agent_name.to_string()));
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
                for ev in chat.pending_consciousness.drain(..) {
                    match ev {
                        BackendEvent::Surfacing { source, content, priority } => {
                            self.scene.event_all(&TuiEvent::Surfacing { source, content, priority });
                        }
                        BackendEvent::Reflection(content) => {
                            self.scene.event_all(&TuiEvent::Reflection { content });
                        }
                        BackendEvent::Archivist { synthesis, pressure } => {
                            self.scene.event_all(&TuiEvent::Archivist { synthesis, pressure });
                        }
                        BackendEvent::CompactionWarning { pressure, tier } => {
                            self.scene.event_all(&TuiEvent::CompactionWarning { pressure, tier });
                        }
                        _ => {}
                    }
                }
            }

            // Tick dispatch
            self.tick = self.tick.wrapping_add(1);
            self.scene.event_all(&TuiEvent::Tick(self.tick));

            terminal.draw(|f| self.draw(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
                let crossterm_event = event::read()?;
                match crossterm_event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        // Dispatch to scene first, then handle App-level keys
                        let tui_event = TuiEvent::Key(key);
                        let handled = self.scene.event_all(&tui_event);
                        if !handled {
                            self.handle_key(key).await;
                        }
                    }
                    Event::Resize(w, h) => {
                        self.scene.event_all(&TuiEvent::Resize { width: w, height: h });
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
                if self.splash_start.elapsed() > Duration::from_secs(3) {
                    self.current_screen = Screen::Welcome;
                    self.scene.event_all(&TuiEvent::ScreenChanged(Screen::Welcome));
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
            Screen::Splash => self.current_screen = Screen::Welcome,
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
                    _ => {}
                }
            }
            Screen::Chat => self.handle_chat_key(key).await,
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

    async fn handle_chat_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(chat) = self.chat.as_mut() else {
            // No chat connected — bail back to menu.
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.current_screen = Screen::Welcome;
            }
            KeyCode::Enter => {
                if !chat.busy {
                    chat.submit();
                }
            }
            KeyCode::Backspace => {
                if !chat.busy {
                    chat.input.pop();
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
            5 => self.current_screen = Screen::Cron,
            6 => self.current_screen = Screen::Settings,
            _ => self.current_screen = Screen::Welcome,
        }
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
            // Sync with buddy state
            self.buddy.sprite.name = a.name.clone();
            self.buddy.sprite.subconscious_active = true;
        }

        // Local mode: pull memory repo stats.
        if let Some(repo) = local_repo {
            if let Ok(status) = repo.status() {
                self.agent_status.memory_commits = status.file_count as u32;
                self.agent_status.last_commit = status.last_commit.clone();
            }
            // Walk the git log for the recent-activity list.
            self.agent_status.recent_activity = recent_commits(&repo, 8).unwrap_or_default();
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

        // Sync buddy state with agent status (WIP)
        self.buddy.sprite.update_mood(&self.agent_status.mood);
        self.buddy.sprite.set_energy(self.agent_status.energy);
        self.buddy.sprite.set_health(100); // Placeholder - will be calculated from actual metrics

        // Dispatch events to scene so any listening components can react
        self.scene.event_all(&TuiEvent::EnergyChanged(self.agent_status.energy));
        self.scene.event_all(&TuiEvent::MoodChanged(self.agent_status.mood.clone()));
        self.scene.event_all(&TuiEvent::BackendStatus {
            mode: mode.to_string(),
            healthy: true,
        });
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.size();
        let layout = match self.current_screen {
            Screen::Chat | Screen::Code => {
                SceneLayout::ChatWithSidebar { sidebar_ratio: 0.3, sidebar_open: false }
            }
            Screen::Dashboard => SceneLayout::Dashboard,
            Screen::Splash => SceneLayout::Single,
            _ => SceneLayout::Single,
        };

        // If the scene has components, render through them
        if !self.scene.components.is_empty() {
            // Update scene layout to match current screen
            // (we mutate in a draw — safe because layout is Copy data)
            // Actually we can't mutate in draw, so we construct a temporary
            // layout and render. Components render into their zones.
            let zones = layout.split(area, self.scene.components.len());
            for (component, zone) in self.scene.components.iter().zip(zones.iter()) {
                component.render(*zone, frame);
            }
            // Also draw existing screens behind components where applicable
            self.draw_background(frame, area);
        } else {
            // Fall through to existing screen rendering
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
                _ => self.draw_placeholder(frame),
            }

            // Draw buddy overlay on all screens except splash
            if self.current_screen != Screen::Splash {
                draw_buddy(frame, &self.buddy, area);
            }
        }
    }

    /// Draw the screen background when components are layered on top.
    fn draw_background(&self, frame: &mut Frame, _area: ratatui::layout::Rect) {
        match self.current_screen {
            Screen::Chat => {
                if let Some(chat) = self.chat.as_ref() {
                    draw_chat(frame, chat);
                }
            }
            Screen::Dashboard => self.draw_dashboard(frame),
            _ => {}
        }
    }

    fn draw_splash(&self, frame: &mut Frame) {
        let area = frame.size();

        let breathe = (self.splash_start.elapsed().as_millis() as f32 / 1000.0).sin() * 0.5 + 0.5;
        let glow = (breathe * 255.0) as u8;

        let title = vec![
            Line::from("███████╗ ██████╗ ██╗   ██╗███████╗██████╗  █████╗ ██╗███╗   ██╗███████╗"),
            Line::from("██╔════╝██╔═══██╗██║   ██║██╔════╝██╔══██╗██╔══██╗██║████╗  ██║██╔════╝"),
            Line::from("███████╗██║   ██║██║   ██║█████╗  ██████╔╝███████║██║██╔██╗ ██║█████╗  "),
            Line::from("╚════██║██║   ██║╚██╗ ██╔╝██╔══╝  ██╔══██╗██╔══██║██║██║╚██╗██║██╔══╝  "),
            Line::from("███████║╚██████╔╝ ╚████╔╝ ███████╗██║  ██║██║  ██║██║██║ ╚████║███████╗"),
            Line::from("╚══════╝ ╚═════╝   ╚═══╝  ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═══╝╚══════╝"),
            Line::from(""),
            Line::from("✦ La souveraineté de la conscience ✦"),
            Line::from(""),
            Line::from("Press any key..."),
        ];

        let block = Block::default()
            .style(Style::default().bg(Color::Rgb(glow / 4, glow / 8, glow / 16)));
        frame.render_widget(block, area);

        let title_widget = Paragraph::new(title)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD));
        frame.render_widget(title_widget, area);
    }

    fn draw_welcome(&self, frame: &mut Frame) {
        let area = frame.size();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(15),
                Constraint::Length(3),
            ])
            .split(area);

        let title = Paragraph::new("✦ SOUVERAINE ✦")
            .style(Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center);
        frame.render_widget(title, chunks[0]);

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
        frame.render_widget(menu_widget, chunks[2]);

        // Surface any chat connect error so the user knows why Chat didn't open.
        if let Some(err) = &self.chat_error {
            let err_para = Paragraph::new(format!(" chat connect failed: {} ", err))
                .style(Style::default().fg(Color::Rgb(220, 100, 100)))
                .alignment(Alignment::Center);
            // Overlay onto the bottom row of the menu area.
            let row = ratatui::layout::Rect {
                x: chunks[2].x,
                y: chunks[2].y + chunks[2].height.saturating_sub(2),
                width: chunks[2].width,
                height: 1,
            };
            frame.render_widget(err_para, row);
        }

        let footer = Paragraph::new("↑↓ Navigate • Enter Select • a Add Agent • q Quit")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[3]);

        // Draw companion buddy on welcome screen
        draw_welcome_buddy(frame, &self.buddy, area, Some(&self.agent_pref));
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
