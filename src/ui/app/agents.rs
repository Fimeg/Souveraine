use super::{App, Screen};
use crate::ui::component::TuiEvent;
use crate::ui::presence::Presence;

impl App {
    pub(super) fn add_available_agent(&mut self, agent_name: String) {
        if !self.available_agents.contains(&agent_name) {
            self.available_agents.push(agent_name);
        }
    }

    /// Detach the UI from the current agent and attach to another.
    /// The agent itself keeps running — this only tears down the view
    /// layer so each subsystem reconnects fresh on next entry.
    pub(super) fn select_agent(&mut self, agent_name: &str) {
        let changed = self.agent_pref != agent_name;
        self.agent_pref = agent_name.to_string();
        self.agent_status.name = agent_name.to_string();

        if changed {
            // Cancel any in-flight turn before dropping the chat surface —
            // otherwise the turn keeps running orphaned and resurfaces as
            // "she's doing the same message again" on a later /resume.
            if let Some(chat) = &self.chat {
                chat.cancel_active_turn();
            }
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
    pub(super) async fn ensure_agent_cards_loaded(&mut self) {
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
    pub(super) async fn open_agent_manager(&mut self) {
        self.ensure_agent_cards_loaded().await;
        if self.agent_cards.is_empty() {
            self.available_agents = vec!["Annie".to_string(), "Ani".to_string()];
        }
        self.current_screen = Screen::AgentsManager;
        self.dispatch(TuiEvent::ScreenChanged(Screen::AgentsManager));
    }

    /// Cycle through available agents for selection (WIP)
    pub(super) fn cycle_agent_selection(&mut self) {
        if self.available_agents.is_empty() {
            // No agents available yet - create a default alias
            // This is WIP - will be expanded with full agent creation flow
            let default_agents = vec![
                "Ani".to_string(),
                "JeanLuc".to_string(),
                "Eione".to_string(),
            ];
            for agent in default_agents {
                self.add_available_agent(agent);
            }
        }

        // Clone the agent name to avoid borrow checker issues
        let agent_to_select = if let Some(current_idx) = self
            .available_agents
            .iter()
            .position(|a| a == &self.agent_pref)
        {
            let next_idx = (current_idx + 1) % self.available_agents.len();
            self.available_agents[next_idx].clone()
        } else if !self.available_agents.is_empty() {
            self.available_agents[0].clone()
        } else {
            return;
        };

        self.select_agent(&agent_to_select);
    }

    pub(super) fn agent_id_by_name(&self, name: &str) -> Option<String> {
        self.agent_cards
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .map(|c| c.id.clone())
    }
}
