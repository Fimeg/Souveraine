use tracing::{info, warn};

use super::{recent_commits, short_now, App, Screen};
use crate::ui::chat::ChatState;
use crate::ui::component::TuiEvent;
use crate::ui::setup::SetupState;

impl App {
    pub(super) async fn select_menu_item(&mut self) {
        // Welcome now hosts the dashboard inline, so the menu enters
        // *destinations* only — Chat, Schedule, and three placeholders.
        // Items marked (coming soon) are no-ops until their screens are real.
        match self.menu_selected {
            0 => {
                // Chat — lazily connect to a backend on first entry.
                if self.chat.is_none() {
                    match ChatState::connect(self.config.clone(), &self.agent_pref).await {
                        Ok(mut c) => {
                            // Fresh chat surface (first entry, or after an
                            // agent switch): offer resume-or-new.
                            c.offer_resume_or_new();
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
                    let expr_path = agent_id
                        .as_ref()
                        .and_then(|id| Self::agent_assets_dir(id))
                        .map(|a| a.join("expressions"));
                    view.set_expressions_path(expr_path);
                    self.settings = Some(view);
                    self.sync_palette();
                } else {
                    let cfg = self.config.read().await.clone();
                    let agent_id = self.agent_id_by_name(&self.agent_pref);
                    let expr_path = agent_id
                        .as_ref()
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
    pub(super) async fn build_schedules_view(&self) -> crate::ui::schedules::SchedulesView {
        use crate::backend::Backend;

        let base = dirs::home_dir()
            .unwrap_or_default()
            .join(".souveraine")
            .join("agents");

        let cfg = self.config.read().await;
        let url = cfg.server.effective_url();
        drop(cfg);

        let mut resolved_id: Option<String> = None;
        let remote = crate::backend::RemoteBackend::new(&url).ok();
        if let Some(ref remote) = remote {
            if remote.health().await {
                if let Ok(list) = remote.list_agents().await {
                    resolved_id = list
                        .iter()
                        .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
                        .map(|a| a.id.clone());
                }
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
    pub(super) async fn transition_from_splash(&mut self) {
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
    pub(super) async fn finish_setup(&mut self) {
        use crate::backend::LocalBackend;

        if let Some(ref mut setup) = self.setup_state.take() {
            // If the wizard got far enough to name an agent, create it.
            if !setup.agent_name.is_empty() && !setup.complete {
                // Agent was configured but setup was skipped mid-way (Esc from Welcome)
                // — don't create, just go to dashboard.
            } else if setup.complete
                && !setup.agent_name.is_empty()
                && setup.created_agent_id.is_none()
            {
                // Persist the Bifrost settings the wizard collected, otherwise
                // they are lost and the next launch has no config.
                {
                    let mut cfg = self.config.write().await;
                    cfg.bifrost.base_url = setup.bifrost_url.clone();
                    cfg.bifrost.primary_model = setup.model_handle.clone();
                }
                let key = setup.bifrost_key.trim();
                if !key.is_empty() {
                    if let Err(e) = crate::core::credentials::store_bifrost_key(key) {
                        warn!("setup wizard could not store Bifrost key in keyring: {}", e);
                    }
                }
                {
                    let path = self.config_path.clone().unwrap_or_else(|| {
                        let home = dirs::home_dir().unwrap_or_default();
                        let dir = home.join(".souveraine");
                        let _ = std::fs::create_dir_all(&dir);
                        dir.join("config.toml")
                    });
                    let cfg = self.config.read().await;
                    if let Err(e) = cfg.save(&path) {
                        warn!(
                            "setup wizard could not save config to {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
                // Create the agent via LocalBackend against the updated config.
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
    pub(super) async fn refresh_dashboard(&mut self) {
        use crate::backend::Backend;

        let cfg = self.config.read().await;
        let url = cfg.server.effective_url();
        drop(cfg);

        // Try remote first; fall back to local. Mirror of resolve_backend logic.
        let remote = crate::backend::RemoteBackend::new(&url).ok();
        let (agents_result, mode, local_repo) = if let Some(ref remote) = remote {
            if remote.health().await {
                (remote.list_agents().await, "remote", None)
            } else {
                // Fall through to local
                let cfg = self.config.read().await.clone();
                match crate::backend::LocalBackend::new(cfg).await {
                    Ok(local) => {
                        let agents = local.list_agents().await;
                        let repo = if let Ok(list) = &agents {
                            list.iter()
                                .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
                                .or_else(|| list.first())
                                .map(|a| local.server_agents().memory_repo(&a.id))
                        } else {
                            None
                        };
                        (agents, "local", repo)
                    }
                    Err(e) => {
                        self.agent_status.mood = format!("backend err: {}", e);
                        self.agent_status.mode = "—".to_string();
                        return;
                    }
                }
            }
        } else {
            let cfg = self.config.read().await.clone();
            match crate::backend::LocalBackend::new(cfg).await {
                Ok(local) => {
                    let agents = local.list_agents().await;
                    // Pull a MemoryRepo for the current agent (if it exists)
                    // through the LocalBackend's server inventory.
                    let repo = if let Ok(list) = &agents {
                        list.iter()
                            .find(|a| a.name == self.agent_pref || a.id == self.agent_pref)
                            .or_else(|| list.first())
                            .map(|a| local.server_agents().memory_repo(&a.id))
                    } else {
                        None
                    };
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
            let pref_path = repo
                .root()
                .join("system")
                .join("preferences")
                .join("visual.md");
            if let Ok(content) = std::fs::read_to_string(&pref_path) {
                if let Some(body) = content.strip_prefix("---\n") {
                    if let Some(end) = body.find("\n---\n") {
                        for line in body[..end].lines() {
                            if let Some((key, val)) = line.split_once(':') {
                                let key = key.trim();
                                let val = val.trim().trim_matches('"');
                                if key == "atmosphere" {
                                    if let Some(atm) =
                                        crate::ui::atmosphere::Atmosphere::from_name(val)
                                    {
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
            let balance_path = repo
                .root()
                .join("system")
                .join("dynamic")
                .join("energy-balance.md");
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
}
