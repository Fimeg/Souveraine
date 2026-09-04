use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::types::{Category, EditableValue, FieldLoc, PanelFocus, SettingsMode};
use super::view::SettingsView;

/// Actions the settings handler can request from the parent App.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    GoBack,
    Save,
    SaveAndGoBack,
    FetchModels,
    /// Live atmosphere preview — dispatch before saving so the UI responds immediately.
    AtmospherePreview(String),
}

// ── Key handling ────────────────────────────────────────────────────────────

impl SettingsView {
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<SettingsAction> {
        // Status messages absorb any key press.
        if self.is_status() {
            self.clear_status();
            return None;
        }

        // Confirm discard dialog.
        if self.is_confirm_discard() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.discard();
                    return Some(SettingsAction::GoBack);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.mode = SettingsMode::Browse;
                }
                _ => {}
            }
            return None;
        }

        match &self.mode.clone() {
            SettingsMode::Browse => self.handle_browse_key(key),
            SettingsMode::Editing {
                loc,
                buffer,
                cursor,
            } => self.handle_edit_key(key, *loc, buffer.clone(), *cursor),
            _ => None,
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Option<SettingsAction> {
        let categories = Category::all();
        let fields = self.fields_for_category(self.selected_category());

        // Global keys work regardless of focus.
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.dirty {
                    self.mode = SettingsMode::ConfirmDiscard;
                } else {
                    return Some(SettingsAction::GoBack);
                }
                return None;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                return Some(SettingsAction::Save);
            }
            KeyCode::Char('S') | KeyCode::Char('s')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                return Some(SettingsAction::SaveAndGoBack);
            }
            _ => {}
        }

        // r: fetch models (only in Bifrost category, any focus).
        if key.code == KeyCode::Char('r')
            && matches!(self.selected_category(), Category::Providers)
            && !self.models_fetching
        {
            self.models_fetching = true;
            return Some(SettingsAction::FetchModels);
        }

        match self.focus {
            PanelFocus::Categories => match key.code {
                KeyCode::Up | KeyCode::BackTab => {
                    if self.category_idx > 0 {
                        self.category_idx -= 1;
                        self.field_idx = 0;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if self.category_idx + 1 < categories.len() {
                        self.category_idx += 1;
                        self.field_idx = 0;
                    }
                }
                KeyCode::Right | KeyCode::Enter => {
                    self.focus = PanelFocus::Fields;
                }
                _ => {}
            },
            PanelFocus::Fields => match key.code {
                KeyCode::Up | KeyCode::BackTab => {
                    if self.field_idx > 0 {
                        self.field_idx -= 1;
                    } else if !fields.is_empty() {
                        self.field_idx = fields.len() - 1;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if self.field_idx + 1 < fields.len() {
                        self.field_idx += 1;
                    } else {
                        self.field_idx = 0;
                    }
                }
                KeyCode::Left => {
                    if let Some((loc, EditableValue::EnumVariant { index, variants })) =
                        fields.get(self.field_idx)
                    {
                        let prev = if *index == 0 {
                            variants.len() - 1
                        } else {
                            index - 1
                        };
                        self.apply_field(
                            *loc,
                            EditableValue::EnumVariant {
                                index: prev,
                                variants: variants.clone(),
                            },
                        );
                        return self.maybe_atmosphere_preview(*loc);
                    }
                    self.focus = PanelFocus::Categories;
                }
                KeyCode::Right => {
                    if let Some((loc, EditableValue::EnumVariant { index, variants })) =
                        fields.get(self.field_idx)
                    {
                        let next = (index + 1) % variants.len();
                        self.apply_field(
                            *loc,
                            EditableValue::EnumVariant {
                                index: next,
                                variants: variants.clone(),
                            },
                        );
                        return self.maybe_atmosphere_preview(*loc);
                    }
                }
                KeyCode::Enter => {
                    if let Some((loc, value)) = fields.get(self.field_idx) {
                        match value {
                            EditableValue::Bool(v) => {
                                self.apply_field(*loc, EditableValue::Bool(!v));
                            }
                            EditableValue::EnumVariant { index, variants } => {
                                let next = (index + 1) % variants.len();
                                self.apply_field(
                                    *loc,
                                    EditableValue::EnumVariant {
                                        index: next,
                                        variants: variants.clone(),
                                    },
                                );
                                return self.maybe_atmosphere_preview(*loc);
                            }
                            EditableValue::Secret(v) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: v.clone(),
                                    cursor: v.len(),
                                };
                            }
                            EditableValue::Text(v) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: v.clone(),
                                    cursor: v.len(),
                                };
                            }
                            EditableValue::OptionalText(Some(s)) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: s.clone(),
                                    cursor: s.len(),
                                };
                            }
                            EditableValue::OptionalText(None) => {
                                self.apply_field(
                                    *loc,
                                    EditableValue::OptionalText(Some(String::new())),
                                );
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: String::new(),
                                    cursor: 0,
                                };
                            }
                            EditableValue::Float(v) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: format!("{v}"),
                                    cursor: 0,
                                };
                            }
                            EditableValue::Uint(v) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: format!("{v}"),
                                    cursor: 0,
                                };
                            }
                            EditableValue::Int(v) => {
                                self.mode = SettingsMode::Editing {
                                    loc: *loc,
                                    buffer: format!("{v}"),
                                    cursor: 0,
                                };
                            }
                            EditableValue::Point2D { .. } => {
                                // Point picker handles its own input; no text editing mode.
                            }
                        }
                    }
                }
                _ => {}
            },
        }
        None
    }

    /// If `loc` is the atmosphere field, return a live-preview action.
    fn maybe_atmosphere_preview(&self, loc: FieldLoc) -> Option<SettingsAction> {
        if loc == FieldLoc::PrAtmosphere {
            let atm = self.config.presence.atmosphere.clone().unwrap_or_default();
            Some(SettingsAction::AtmospherePreview(atm))
        } else {
            None
        }
    }

    fn handle_edit_key(
        &mut self,
        key: KeyEvent,
        loc: FieldLoc,
        mut buffer: String,
        mut cursor: usize,
    ) -> Option<SettingsAction> {
        match key.code {
            KeyCode::Esc => {
                self.mode = SettingsMode::Browse;
            }
            KeyCode::Enter => {
                self.commit_edit(loc, &buffer);
            }
            KeyCode::Tab => {
                self.commit_edit(loc, &buffer);
                let fields = self.fields_for_category(self.selected_category());
                if self.field_idx + 1 < fields.len() {
                    self.field_idx += 1;
                }
            }
            KeyCode::Left => {
                cursor = buffer[..cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::Right => {
                if let Some(c) = buffer[cursor..].chars().next() {
                    cursor += c.len_utf8();
                }
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::Home => {
                cursor = 0;
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::End => {
                cursor = buffer.len();
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::Backspace => {
                if cursor > 0 {
                    let prev = buffer[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    buffer.remove(prev);
                    cursor = prev;
                }
                if buffer.is_empty()
                    && matches!(
                        loc,
                        FieldLoc::ScModel
                            | FieldLoc::RfModel
                            | FieldLoc::CpModel
                            | FieldLoc::ScMaxTokens
                            | FieldLoc::MeBasePath
                            | FieldLoc::FdInstanceLabel
                    )
                {
                    self.apply_field(loc, EditableValue::OptionalText(None));
                    self.mode = SettingsMode::Browse;
                    return None;
                }
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::Delete => {
                if cursor < buffer.len() {
                    buffer.remove(cursor);
                }
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            KeyCode::Char(c) => {
                let is_numeric = matches!(
                    loc,
                    FieldLoc::ScMaxTokens
                        | FieldLoc::RfMessageInterval
                        | FieldLoc::ArInterval
                        | FieldLoc::SaMaxConcurrent
                        | FieldLoc::SaTimeout
                        | FieldLoc::SaMaxDepth
                        | FieldLoc::SaMaxToolRounds
                        | FieldLoc::SaInterRoundDelayMs
                        | FieldLoc::WsPort
                        | FieldLoc::SvPort
                        | FieldLoc::PvTimeoutSecs
                        | FieldLoc::ScN1Every
                        | FieldLoc::ScN1Secs
                        | FieldLoc::TuCennoThreshold
                        | FieldLoc::EvRetainDays
                        | FieldLoc::PrPulseIntervalSecs
                );
                let is_float = matches!(
                    loc,
                    FieldLoc::ArThreshold
                        | FieldLoc::SaWarning1Threshold
                        | FieldLoc::SaWarning2Threshold
                        | FieldLoc::CpWarnPressure
                        | FieldLoc::CpUrgentPressure
                        | FieldLoc::CpCriticalPressure
                );

                if is_numeric {
                    if c.is_ascii_digit() {
                        buffer.insert(cursor, c);
                        cursor += 1;
                    }
                } else if is_float {
                    if c.is_ascii_digit() || c == '.' {
                        buffer.insert(cursor, c);
                        cursor += 1;
                    }
                } else {
                    buffer.insert(cursor, c);
                    cursor += c.len_utf8();
                }
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
            _ => {
                self.mode = SettingsMode::Editing {
                    loc,
                    buffer,
                    cursor,
                };
            }
        }
        None
    }

    fn commit_edit(&mut self, loc: FieldLoc, buffer: &str) {
        match loc {
            FieldLoc::RfMessageInterval
            | FieldLoc::ArInterval
            | FieldLoc::SaMaxConcurrent
            | FieldLoc::SaTimeout
            | FieldLoc::SaMaxDepth
            | FieldLoc::SaMaxToolRounds
            | FieldLoc::SaInterRoundDelayMs
            | FieldLoc::WsPort
            | FieldLoc::SvPort
            | FieldLoc::PvTimeoutSecs
            | FieldLoc::ScN1Every
            | FieldLoc::ScN1Secs
            | FieldLoc::TuCennoThreshold
            | FieldLoc::PrPulseIntervalSecs => {
                if let Ok(v) = buffer.parse::<u64>() {
                    self.apply_field(loc, EditableValue::Uint(v));
                }
            }
            FieldLoc::EvRetainDays => {
                if let Ok(v) = buffer.parse::<i64>() {
                    self.apply_field(loc, EditableValue::Int(v));
                }
            }
            FieldLoc::ArThreshold
            | FieldLoc::SaWarning1Threshold
            | FieldLoc::SaWarning2Threshold
            | FieldLoc::CpWarnPressure
            | FieldLoc::CpUrgentPressure
            | FieldLoc::CpCriticalPressure => {
                if let Ok(v) = buffer.parse::<f32>() {
                    self.apply_field(loc, EditableValue::Float(v));
                }
            }
            FieldLoc::ScModel
            | FieldLoc::RfModel
            | FieldLoc::CpModel
            | FieldLoc::MeBasePath
            | FieldLoc::ScMaxTokens
            | FieldLoc::FdInstanceLabel => {
                let val = if buffer.is_empty() {
                    None
                } else {
                    Some(buffer.to_string())
                };
                self.apply_field(loc, EditableValue::OptionalText(val));
            }
            _ => {
                self.apply_field(loc, EditableValue::Text(buffer.to_string()));
            }
        }
        self.mode = SettingsMode::Browse;
    }
}
