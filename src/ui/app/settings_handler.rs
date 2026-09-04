use std::path::PathBuf;

use crossterm::event::KeyCode;

use super::{App, Screen};
use crate::core::config::ConsciousnessConfig;
use crate::ui::component::TuiEvent;
use crate::ui::settings::SettingsAction;

impl App {
    pub(super) fn handle_schedules_key(&mut self, key: crossterm::event::KeyEvent) {
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
                    CreateField::Name => {
                        form.name.pop();
                    }
                    CreateField::Schedule => {
                        form.schedule.pop();
                    }
                    CreateField::Prompt => {
                        form.prompt.pop();
                    }
                    _ => {}
                },
                KeyCode::Left => match form.field {
                    CreateField::Kind => {
                        form.kind = match form.kind {
                            crate::core::nervous::cron::ScheduleKind::Once => {
                                crate::core::nervous::cron::ScheduleKind::Cron
                            }
                            crate::core::nervous::cron::ScheduleKind::Interval => {
                                crate::core::nervous::cron::ScheduleKind::Once
                            }
                            crate::core::nervous::cron::ScheduleKind::Cron => {
                                crate::core::nervous::cron::ScheduleKind::Interval
                            }
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
                            crate::core::nervous::cron::ScheduleKind::Once => {
                                crate::core::nervous::cron::ScheduleKind::Interval
                            }
                            crate::core::nervous::cron::ScheduleKind::Interval => {
                                crate::core::nervous::cron::ScheduleKind::Cron
                            }
                            crate::core::nervous::cron::ScheduleKind::Cron => {
                                crate::core::nervous::cron::ScheduleKind::Once
                            }
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

    pub(super) async fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(view) = self.settings.as_mut() else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.current_screen = Screen::Welcome;
            }
            return;
        };

        let action = view.handle_key(key);

        match &action {
            Some(SettingsAction::FetchModels) => {
                let config = view.config.clone();
                let extra: Vec<String> = view.config.models.keys().cloned().collect();
                let (tx, rx) = tokio::sync::oneshot::channel();
                view.models_rx = Some(rx);
                tokio::spawn(async move {
                    let models = match crate::bridge::build_provider(&config) {
                        Ok(provider) => {
                            let mut models = provider.list_models().await.unwrap_or_default();
                            for m in extra {
                                if !models.contains(&m) {
                                    models.push(m);
                                }
                            }
                            models
                        }
                        Err(_) => extra,
                    };
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

        let go_back = matches!(
            action,
            Some(SettingsAction::SaveAndGoBack) | Some(SettingsAction::GoBack)
        );
        if go_back {
            let save_and_go = matches!(action, Some(SettingsAction::SaveAndGoBack));
            // Clone everything we need from view before dropping it, since
            // view borrows self.settings and blocks other self access.
            let db_path = self
                .config_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("souveraine.toml"));
            let (outfit, atmosphere) = if save_and_go {
                (
                    view.config.presence.outfit.clone(),
                    view.config.presence.atmosphere.clone(),
                )
            } else {
                (None, None)
            };
            let original_snapshot = if save_and_go {
                Some(view.original.clone())
            } else {
                None
            };
            let config_snapshot = if save_and_go {
                Some(view.config.clone())
            } else {
                None
            };
            let agent_model_change = if save_and_go && view.agent_model_dirty() {
                view.active_agent
                    .as_ref()
                    .map(|a| (a.id.clone(), a.model.clone()))
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
            let path = self
                .config_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("souveraine.toml"));
            let outfit = view.config.presence.outfit.clone();
            let atmosphere = view.config.presence.atmosphere.clone();
            match view.save(&path) {
                Ok(()) => {
                    let original = view.original.clone();
                    let saved = view.config.clone();
                    let agent_model_change = if view.agent_model_dirty() {
                        view.active_agent
                            .as_ref()
                            .map(|a| (a.id.clone(), a.model.clone()))
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
    pub(super) fn active_agent_settings(&self) -> Option<crate::ui::settings::ActiveAgentSettings> {
        self.chat.as_ref()?; // a backend is required to persist the change
        let id = self.agent_id_by_name(&self.agent_pref)?;
        let model = Self::agent_model_from_disk(&id)?;
        let base = dirs::home_dir()?;
        let memory_root = base.join(".souveraine/agents").join(&id).join("memory");
        let subconscious_root = base
            .join(".souveraine/subconscious-agents")
            .join(format!("{id}-sub"))
            .join("memory.git");
        let has_subconscious = subconscious_root.join("HEAD").exists();
        let agent_json_exists = base
            .join(".souveraine/server/agents")
            .join(&id)
            .join("agent.json")
            .exists();
        Some(crate::ui::settings::ActiveAgentSettings {
            id: id.clone(),
            name: self.agent_pref.clone(),
            model: model.clone(),
            model_original: model,
            provider: None, // populated from agent.json if present
            subconscious_id: format!("{id}-sub"),
            memory_root,
            subconscious_root,
            has_subconscious,
            agent_json_exists,
        })
    }

    /// Read an agent's current llm model handle from its on-disk record at
    /// `~/.souveraine/server/agents/{id}/agent.json`.
    pub(crate) fn agent_model_from_disk(agent_id: &str) -> Option<String> {
        let path = dirs::home_dir()?
            .join(".souveraine/server/agents")
            .join(agent_id)
            .join("agent.json");
        let content = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        json.get("llm_config")?
            .get("model")?
            .as_str()
            .map(str::to_string)
    }

    /// Push a per-agent model change to the agent's record through the active
    /// backend (which also refreshes the in-memory cache and SQLite mirror).
    pub(super) async fn push_agent_model(&self, agent_id: &str, model: &str) {
        let Some(chat) = &self.chat else {
            tracing::warn!(agent = %agent_id, "settings: no backend — agent model change not applied");
            return;
        };
        match chat.backend.update_agent_model(agent_id, model).await {
            Ok(()) => {
                tracing::info!(agent = %agent_id, model = %model, "settings: agent model updated")
            }
            Err(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "settings: agent model update failed")
            }
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
    pub(super) async fn sync_settings_fields(
        &self,
        original: &ConsciousnessConfig,
        saved: &ConsciousnessConfig,
    ) {
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
}
