//! Setup wizard — walks the user through first-time configuration.
//!
//! Three flows:
//! - `FreshInstall`: no config, no agents — full walkthrough
//! - `ImportAgent`: config exists, but no agents — skip Bifrost, create/import
//! - `FederationSync`: wants to sync from a federation peer (env override)

use std::sync::Arc;
use tokio::sync::oneshot;

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::models::{CreateAgentRequest, LlmConfig, MemoryBlock};

/// Describes what kind of input a form slot accepts.
enum SlotKind {
    /// Free-text entry, chars insert at cursor.
    Text {
        value: String,
        cursor: usize,
        secret: bool,
    },
    /// An enum picker — left/right cycles through named variants.
    ModelPicker {
        value: String,
        cursor: usize,
        /// Known model names. Populated by `fetch_models`.
        variants: Vec<String>,
        // Derive current index from `variants.iter().position(|v| v == &value)` at render time.
    },
}

impl SlotKind {
    fn display_value(&self) -> String {
        match self {
            SlotKind::Text { value, secret: true, .. } => "*".repeat(value.len()),
            SlotKind::Text { value, .. } => value.clone(),
            SlotKind::ModelPicker { value, .. } => value.clone(),
        }
    }

    fn value(&self) -> &str {
        match self {
            SlotKind::Text { value, .. } => value,
            SlotKind::ModelPicker { value, .. } => value,
        }
    }

    fn insert(&mut self, ch: char) {
        match self {
            SlotKind::Text { value, cursor, .. } => {
                value.insert(*cursor, ch);
                *cursor += 1;
            }
            SlotKind::ModelPicker { value, cursor, .. } => {
                value.insert(*cursor, ch);
                *cursor += 1;
            }
        }
    }

    fn backspace(&mut self) {
        match self {
            SlotKind::Text { value, cursor, .. } => {
                if *cursor > 0 {
                    *cursor -= 1;
                    value.remove(*cursor);
                }
            }
            SlotKind::ModelPicker { value, cursor, .. } => {
                if *cursor > 0 {
                    *cursor -= 1;
                    value.remove(*cursor);
                }
            }
        }
    }

    fn cursor_left(&mut self) {
        match self {
            SlotKind::Text { cursor, .. } | SlotKind::ModelPicker { cursor, .. } => {
                *cursor = cursor.saturating_sub(1);
            }
        }
    }

    fn cursor_right(&mut self) {
        match self {
            SlotKind::Text { value, cursor, .. } | SlotKind::ModelPicker { value, cursor, .. } => {
                *cursor = value.len().min(*cursor + 1);
            }
        }
    }
}

struct FormSlot {
    label: &'static str,
    kind: SlotKind,
    /// If true, this slot is an enum picker — Tab/Enter skip past it,
    /// left/right cycle variants instead of moving cursor.
    is_picker: bool,
}

impl FormSlot {
    fn text(label: &'static str, default: &str, secret: bool) -> Self {
        Self {
            label,
            kind: SlotKind::Text {
                value: default.to_string(),
                cursor: default.len(),
                secret,
            },
            is_picker: false,
        }
    }

    fn model(label: &'static str, default: &str) -> Self {
        Self {
            label,
            kind: SlotKind::ModelPicker {
                value: default.to_string(),
                cursor: default.len(),
                variants: Vec::new(),
            },
            is_picker: true,
        }
    }

    fn display_value(&self) -> String {
        self.kind.display_value()
    }

    fn value(&self) -> String {
        self.kind.value().to_string()
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.is_picker {
            // Left/right cycle the variant selection, don't move cursor
            if let SlotKind::ModelPicker { ref mut variants, ref mut value, ref mut cursor, .. } = self.kind {
                match key.code {
                    KeyCode::Left | KeyCode::Up => {
                        if !variants.is_empty() {
                            let idx = variants.iter().position(|v| v == value.as_str()).unwrap_or(0);
                            let new_idx = if idx == 0 { variants.len() - 1 } else { idx - 1 };
                            *value = variants[new_idx].clone();
                            *cursor = value.len();
                        }
                        return;
                    }
                    KeyCode::Right | KeyCode::Down => {
                        if !variants.is_empty() {
                            let idx = variants.iter().position(|v| v == value.as_str()).unwrap_or(0);
                            let new_idx = (idx + 1) % variants.len();
                            *value = variants[new_idx].clone();
                            *cursor = value.len();
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Default text input handling
        match key.code {
            KeyCode::Char(ch) if !ch.is_control() => self.kind.insert(ch),
            KeyCode::Backspace => self.kind.backspace(),
            KeyCode::Left => self.kind.cursor_left(),
            KeyCode::Right => self.kind.cursor_right(),
            _ => {}
        }
    }
}

/// Form state for the current wizard step.
struct FormState {
    slots: Vec<FormSlot>,
    focus: usize,
    submit_label: &'static str,
    show_submit: bool,
    hint: Option<&'static str>,
}

impl FormState {
    fn slot_count(&self) -> usize {
        self.slots.len() + if self.show_submit { 1 } else { 0 }
    }

    fn advance_focus(&mut self) {
        let n = self.slot_count();
        if n > 0 {
            self.focus = (self.focus + 1) % n;
        }
    }

    fn prev_focus(&mut self) {
        let n = self.slot_count();
        if n > 0 {
            self.focus = if self.focus == 0 { n - 1 } else { self.focus - 1 };
        }
    }

    fn is_submit_focused(&self) -> bool {
        self.show_submit && self.focus >= self.slots.len()
    }

    fn focused_slot_mut(&mut self) -> Option<&mut FormSlot> {
        if self.focus < self.slots.len() {
            Some(&mut self.slots[self.focus])
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetupFlow {
    FreshInstall,
    ImportAgent,
    FederationSync,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetupStep {
    Welcome,
    BifrostConfig,
    CreateAgent,
    ImportOrFederation,
    FederationConfig,
    Complete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportOption {
    Letta { id: String, name: String },
    Federation,
    Skip,
}

pub struct SetupState {
    pub flow: SetupFlow,
    pub step: SetupStep,
    pub complete: bool,

    // ── Bifrost fields ──
    pub bifrost_url: String,
    pub bifrost_key: String,

    // ── Agent fields ──
    pub agent_name: String,
    pub model_handle: String,

    // ── Model discovery ──
    /// Receiver for an async model list fetch. Polled by `App` each tick.
    pub models_rx: Option<oneshot::Receiver<Vec<String>>>,
    pub models_fetching: bool,

    // ── Import / federation ──
    pub letta_agents: Vec<(String, String)>,
    pub import_selection: Option<ImportOption>,
    pub peer_url: String,
    pub peer_agent_id: String,

    // ── Result ──
    /// Set after successful agent creation — the newly created agent id.
    /// App reads this in `finish_setup` to decide the `--agent` default.
    pub created_agent_id: Option<String>,
    pub creation_error: Option<String>,

    // ── Form state ──
    form: FormState,
}

impl SetupState {
    pub fn new(flow: SetupFlow, default_model: &str) -> Self {
        let step = match flow {
            SetupFlow::FreshInstall => SetupStep::Welcome,
            SetupFlow::ImportAgent => SetupStep::CreateAgent,
            SetupFlow::FederationSync => SetupStep::FederationConfig,
        };
        let letta_agents = discover_letta_agents();

        Self {
            flow,
            step,
            complete: false,
            bifrost_url: "http://10.10.20.120:3360".to_string(),
            bifrost_key: String::new(),
            agent_name: "Souveraine".to_string(),
            model_handle: default_model.to_string(),
            models_rx: None,
            models_fetching: false,
            letta_agents,
            import_selection: None,
            peer_url: String::new(),
            peer_agent_id: String::new(),
            created_agent_id: None,
            creation_error: None,
            form: Self::form_for_step(step, default_model),
        }
    }

    /// Kick off an async model list fetch. The receiver should be polled
    /// by `App::run()` each tick (via `poll_models`).
    pub fn fetch_models(&mut self, bifrost_url: &str, api_key: &str) {
        if self.models_fetching {
            return;
        }
        self.models_fetching = true;
        let (tx, rx) = oneshot::channel();
        self.models_rx = Some(rx);

        let url = format!("{}/models", bifrost_url);
        let client = reqwest::Client::new();
        let mut req = client.get(&url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        tokio::spawn(async move {
            let models = match req.send().await {
                Ok(resp) => {
                    match resp.json::<serde_json::Value>().await {
                        Ok(body) => {
                            body["data"].as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|m| m["id"].as_str().map(String::from))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                        }
                        Err(_) => Vec::new(),
                    }
                }
                Err(_) => Vec::new(),
            };
            let _ = tx.send(models);
        });
    }

    /// Poll the model fetch receiver. Called from `App::run()` each tick
    /// when the setup screen is active.
    pub fn poll_models(&mut self) {
        if let Some(rx) = self.models_rx.as_mut() {
            if let Ok(models) = rx.try_recv() {
                // Inject the fetched models into the model picker slot
                if self.step == SetupStep::CreateAgent {
                    for slot in &mut self.form.slots {
                        if let SlotKind::ModelPicker { ref mut variants, .. } = slot.kind {
                            *variants = models.clone();
                            // If current value isn't in the list, keep it as-is (free-text override)
                            break;
                        }
                    }
                }
                self.models_rx = None;
                self.models_fetching = false;
            }
        }
    }

    fn form_for_step(step: SetupStep, default_model: &str) -> FormState {
        match step {
            SetupStep::Welcome => FormState {
                slots: vec![],
                focus: 0,
                submit_label: "Begin",
                show_submit: true,
                hint: None,
            },
            SetupStep::BifrostConfig => FormState {
                slots: vec![
                    FormSlot::text("Bifrost URL", "http://10.10.20.120:3360", false),
                    FormSlot::text("API Key", "", true),
                ],
                focus: 0,
                submit_label: "Continue",
                show_submit: true,
                hint: Some("Enter to submit · Tab to switch · Esc to go back"),
            },
            SetupStep::CreateAgent => FormState {
                slots: vec![
                    FormSlot::text("Agent Name", "Ani", false),
                    FormSlot::model("Model", default_model),
                ],
                focus: 0,
                submit_label: "Create Agent",
                show_submit: true,
                hint: Some("Enter to submit · Tab to switch · ←/→ cycle models · Esc to go back"),
            },
            SetupStep::ImportOrFederation => FormState {
                slots: vec![],
                focus: 0,
                submit_label: "Next",
                show_submit: true,
                hint: None,
            },
            SetupStep::FederationConfig => FormState {
                slots: vec![
                    FormSlot::text("Peer URL", "", false),
                    FormSlot::text("Agent ID", "", false),
                ],
                focus: 0,
                submit_label: "Connect",
                show_submit: true,
                hint: Some("Enter to connect · Tab to switch · Esc to go back"),
            },
            SetupStep::Complete => FormState {
                slots: vec![],
                focus: 0,
                submit_label: "Begin",
                show_submit: true,
                hint: None,
            },
        }
    }

    /// Advance to the next step, capturing current values.
    pub fn advance(&mut self) {
        use SetupStep::*;
        self.step = match self.step {
            Welcome => BifrostConfig,
            BifrostConfig => {
                self.bifrost_url = self.form.slots[0].value();
                self.bifrost_key = self.form.slots[1].value();
                CreateAgent
            }
            CreateAgent => {
                self.agent_name = self.form.slots[0].value();
                self.model_handle = self.form.slots[1].value();
                ImportOrFederation
            }
            ImportOrFederation => Complete,
            FederationConfig => Complete,
            Complete => {
                self.complete = true;
                return;
            }
        };
        let default_model = &self.model_handle;
        self.form = Self::form_for_step(self.step, default_model);
    }

    /// Go back one step.
    pub fn go_back(&mut self) {
        use SetupStep::*;
        self.step = match self.step {
            Welcome => return,
            BifrostConfig => {
                self.bifrost_url = self.form.slots[0].value();
                self.bifrost_key = self.form.slots[1].value();
                Welcome
            }
            CreateAgent => {
                self.agent_name = self.form.slots[0].value();
                self.model_handle = self.form.slots[1].value();
                match self.flow {
                    SetupFlow::FreshInstall => BifrostConfig,
                    SetupFlow::ImportAgent => ImportOrFederation,
                    SetupFlow::FederationSync => FederationConfig,
                }
            }
            ImportOrFederation => CreateAgent,
            FederationConfig => match self.flow {
                SetupFlow::FederationSync => FederationConfig,
                _ => ImportOrFederation,
            },
            Complete => ImportOrFederation,
        };
        let default_model = &self.model_handle;
        self.form = Self::form_for_step(self.step, default_model);
    }

    /// Build a `CreateAgentRequest` from the current wizard state.
    pub fn build_create_request(&self) -> CreateAgentRequest {
        CreateAgentRequest {
            name: self.agent_name.clone(),
            description: Some(format!("Created by Souveraine setup wizard")),
            llm_config: LlmConfig {
                model: self.model_handle.clone(),
                context_window: 128000,
                temperature: None,
                max_tool_rounds: 10,
                inter_round_delay_ms: 500,
            },
            memory_blocks: vec![MemoryBlock {
                label: "persona".to_string(),
                value: "You are a helpful AI assistant.".to_string(),
                limit: None,
            }],
            tools: Vec::new(),
            tags: vec!["souveraine".to_string(), "created-by-setup".to_string()],
        }
    }

    /// Handle a key event. Returns true if the screen needs redrawing.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Special: 'r' triggers model fetch on the CreateAgent step
        if matches!(key.code, KeyCode::Char('r'))
            && self.step == SetupStep::CreateAgent
            && !self.models_fetching
        {
            let bf_url = self.bifrost_url.clone();
            let bf_key = self.bifrost_key.clone();
            self.fetch_models(&bf_url, &bf_key);
            return true;
        }

        // Submit button focused: Enter advances
        if matches!(key.code, KeyCode::Enter) && self.form.is_submit_focused() {
            match self.step {
                SetupStep::BifrostConfig => {
                    self.bifrost_url = self.form.slots[0].value();
                    self.bifrost_key = self.form.slots[1].value();
                }
                SetupStep::CreateAgent => {
                    self.agent_name = self.form.slots[0].value();
                    self.model_handle = self.form.slots[1].value();
                }
                SetupStep::FederationConfig => {
                    self.peer_url = self.form.slots[0].value();
                    self.peer_agent_id = self.form.slots[1].value();
                }
                _ => {}
            }
            self.advance();
            return true;
        }

        // Tab / arrows: move focus between slots
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.form.advance_focus();
                return true;
            }
            KeyCode::BackTab => {
                self.form.prev_focus();
                return true;
            }
            KeyCode::Up => {
                // On a model picker slot, Up cycles variants too.
                // Only treat as focus movement if NOT on a picker.
                if let Some(slot) = self.form.focused_slot_mut() {
                    if !slot.is_picker {
                        self.form.prev_focus();
                        return true;
                    }
                    // Let the slot handle Up as a variant cycle — but only
                    // if it's a picker with variants. Fall through.
                } else {
                    self.form.prev_focus();
                    return true;
                }
                // Fall through to slot handling for picker
            }
            KeyCode::Esc => {
                self.go_back();
                return true;
            }
            _ => {}
        }

        // Delegate to the focused slot
        if let Some(slot) = self.form.focused_slot_mut() {
            slot.handle_key(key);
        } else if matches!(key.code, KeyCode::Enter) {
            match self.step {
                SetupStep::Welcome | SetupStep::ImportOrFederation | SetupStep::Complete => {
                    self.advance();
                }
                _ => {}
            }
        }

        true
    }

    /// Draw the current step into the frame.
    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, area);

        match self.step {
            SetupStep::Welcome => self.draw_welcome(frame, area),
            SetupStep::BifrostConfig => self.draw_form(frame, area, "Bifrost Connection"),
            SetupStep::CreateAgent => self.draw_create_agent(frame, area),
            SetupStep::ImportOrFederation => self.draw_import_screen(frame, area),
            SetupStep::FederationConfig => self.draw_form(frame, area, "Federation Sync"),
            SetupStep::Complete => self.draw_complete(frame, area),
        }
    }

    // ── Drawing ──

    fn draw_welcome(&self, frame: &mut Frame, area: Rect) {
        let title = "Welcome to Souveraine";
        let subtitle = "it's quiet here. let's get you settled.";
        let lines = vec![
            Line::from(Span::styled(title, Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled(subtitle, Style::default().fg(Color::Rgb(180, 120, 80)))),
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(
                "This walkthrough will help you configure Souveraine so your agent has a place to live.",
                Style::default().fg(Color::Rgb(150, 150, 150)),
            )),
            Line::from(""),
            Line::from(Span::styled("You'll set up:", Style::default().fg(Color::Gray))),
            Line::from(Span::styled("  \u{2022} A Bifrost connection (or go local-only)", Style::default().fg(Color::Gray))),
            Line::from(Span::styled("  \u{2022} Your first agent", Style::default().fg(Color::Gray))),
            Line::from(Span::styled("  \u{2022} Optional import from Letta or federation", Style::default().fg(Color::Gray))),
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  [Enter]  Begin setup", Style::default().fg(Color::Rgb(200, 200, 200)))),
            Line::from(Span::styled("  [Esc]    Skip to dashboard", Style::default().fg(Color::Rgb(100, 100, 100)))),
        ];
        let para = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(255, 140, 66))));
        frame.render_widget(para, centered_rect(area, 60, 60));
    }

    /// Generic form renderer for simple input forms (Bifrost, Federation).
    fn draw_form(&self, frame: &mut Frame, area: Rect, title: &str) {
        let form_area = centered_rect(area, 50, 50);
        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(Span::styled(self.step_progress(), Style::default().fg(Color::Rgb(100, 100, 100)))));
        lines.push(Line::from(""));

        for (i, slot) in self.form.slots.iter().enumerate() {
            let focused = i == self.form.focus && !self.form.is_submit_focused();
            let field_style = if focused { Style::default().fg(Color::Rgb(255, 200, 100)) }
                              else { Style::default().fg(Color::Gray) };
            let cursor_marker = if focused { " \u{2591}" } else { "" };

            lines.push(Line::from(Span::styled(format!(" {}: ", slot.label), field_style)));
            let dv = slot.display_value();
            let val_style = if dv.is_empty() { Style::default().fg(Color::Rgb(80, 80, 80)) }
                            else { Style::default().fg(Color::White) };
            lines.push(Line::from(Span::styled(format!(" {}{}", dv, cursor_marker), val_style)));
            lines.push(Line::from(""));
        }

        lines.push(Line::from(Span::styled(" \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(Color::Rgb(60, 60, 60)))));

        let submit_focused = self.form.is_submit_focused();
        let sub_style = if submit_focused {
            Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(140, 140, 140))
        };
        lines.push(Line::from(Span::styled(
            format!(" [{}]  {}", if submit_focused { ">" } else { " " }, self.form.submit_label), sub_style)));

        if let Some(hint) = self.form.hint {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::Rgb(80, 80, 80)))));
        }

        let para = Paragraph::new(lines).alignment(Alignment::Left)
            .block(Block::default().title(format!(" {} ", title)).borders(Borders::ALL)
                .border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Rgb(255, 140, 66))));
        frame.render_widget(para, form_area);
    }

    /// Specialized render for the Create Agent step — shows model picker.
    fn draw_create_agent(&self, frame: &mut Frame, area: Rect) {
        let form_area = centered_rect(area, 50, 50);
        let mut lines: Vec<Line> = Vec::new();

        // Progress
        lines.push(Line::from(Span::styled(self.step_progress(), Style::default().fg(Color::Rgb(100, 100, 100)))));
        lines.push(Line::from(""));

        // Name slot
        let name_focused = 0 == self.form.focus && !self.form.is_submit_focused();
        let name_style = if name_focused { Style::default().fg(Color::Rgb(255, 200, 100)) }
                         else { Style::default().fg(Color::Gray) };
        let cursor = if name_focused { " \u{2591}" } else { "" };
        lines.push(Line::from(Span::styled(" Agent Name:", name_style)));
        let dv = self.form.slots[0].display_value();
        let val_s = if dv.is_empty() { Style::default().fg(Color::Rgb(80, 80, 80)) }
                    else { Style::default().fg(Color::White) };
        lines.push(Line::from(Span::styled(format!(" {}{}", dv, cursor), val_s)));
        lines.push(Line::from(""));

        // Model slot
        let model_focused = 1 == self.form.focus && !self.form.is_submit_focused();
        let model_style = if model_focused { Style::default().fg(Color::Rgb(100, 220, 255)) }
                          else { Style::default().fg(Color::Gray) };
        lines.push(Line::from(Span::styled(" Model:", model_style)));

        let model = &self.form.slots[1];
        let mv = model.display_value();
        let m_fg = if model_focused { Color::Rgb(200, 240, 255) } else { Color::White };

        // Show variant cycling indicators if models are loaded
        if let SlotKind::ModelPicker { ref variants, .. } = model.kind {
            if !variants.is_empty() {
                let idx = variants.iter().position(|v| v == &mv).unwrap_or(0);
                let total = variants.len();
                let pct = if mv.is_empty() { 0 } else { idx + 1 };
                let extra = if model_focused { " \u{2591}" } else { "" };
                lines.push(Line::from(Span::styled(format!(" {}{}", mv, extra), Style::default().fg(m_fg))));
                lines.push(Line::from(Span::styled(
                    format!("   \u{2190}  {} / {}  \u{2192}", pct.min(total), total),
                    Style::default().fg(Color::Rgb(80, 120, 140)),
                )));
            } else {
                let extra = if model_focused { " \u{2591}" } else { "" };
                lines.push(Line::from(Span::styled(format!(" {}{}", mv, extra), Style::default().fg(m_fg))));
                if !self.models_fetching {
                    lines.push(Line::from(Span::styled(
                        "   [r] fetch models from Bifrost",
                        Style::default().fg(Color::Rgb(80, 120, 140)),
                    )));
                }
            }
        }

        // Show fetching status
        if self.models_fetching {
            lines.push(Line::from(Span::styled(
                "   fetching models\u{2026}",
                Style::default().fg(Color::Rgb(100, 100, 100)),
            )));
        }

        // Error display
        if let Some(ref err) = self.creation_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(" error: {} ", err),
                Style::default().fg(Color::Rgb(255, 100, 100)),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(Color::Rgb(60, 60, 60)))));

        let submit_focused = self.form.is_submit_focused();
        let sub_style = if submit_focused {
            Style::default().fg(Color::Rgb(255, 140, 66)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(140, 140, 140))
        };
        lines.push(Line::from(Span::styled(
            format!(" [{}]  {}", if submit_focused { ">" } else { " " }, self.form.submit_label), sub_style)));

        if let Some(hint) = self.form.hint {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::Rgb(80, 80, 80)))));
        }

        let border_color = if self.creation_error.is_some() {
            Color::Rgb(220, 80, 80)
        } else {
            Color::Rgb(255, 140, 66)
        };
        let para = Paragraph::new(lines).alignment(Alignment::Left)
            .block(Block::default().title(" Create Your Agent ").borders(Borders::ALL)
                .border_type(BorderType::Rounded).border_style(Style::default().fg(border_color)));
        frame.render_widget(para, form_area);
    }

    fn draw_import_screen(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(" Import or Create? ",
                Style::default().fg(Color::Rgb(200, 160, 100)).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("Your agent is created. Now you can:", Style::default().fg(Color::Gray))),
            Line::from(""),
        ];

        if !self.letta_agents.is_empty() {
            lines.push(Line::from(Span::styled("  \u{2B21}  Import from Letta", Style::default().fg(Color::Rgb(255, 200, 150)))));
            for (_i, (id, name)) in self.letta_agents.iter().enumerate().take(5) {
                let short_id = if id.len() > 8 { &id[..8] } else { id };
                lines.push(Line::from(Span::styled(format!("       {} \u{2014} {}...", name, short_id),
                    Style::default().fg(Color::Rgb(100, 100, 100)))));
            }
            if self.letta_agents.len() > 5 {
                lines.push(Line::from(Span::styled(format!("       ... and {} more", self.letta_agents.len() - 5),
                    Style::default().fg(Color::Rgb(80, 80, 80)))));
            }
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled("  \u{2B21}  Import from Letta  (no Letta agents found)",
                Style::default().fg(Color::Rgb(80, 80, 80)))));
            lines.push(Line::from(""));
        }

        lines.push(Line::from(Span::styled("  \u{2B21}  Sync from federation peer",
            Style::default().fg(Color::Rgb(150, 200, 255)))));
        lines.push(Line::from(Span::styled("       (federation transport not yet built \u{2014} coming soon)",
            Style::default().fg(Color::Rgb(80, 80, 80)).add_modifier(Modifier::ITALIC))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  \u{2B21}  Skip \u{2014} I'll set it up later",
            Style::default().fg(Color::Rgb(120, 120, 120)))));
        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [Enter]  Continue to dashboard", Style::default().fg(Color::Rgb(200, 200, 200)))));
        lines.push(Line::from(Span::styled("  [Esc]    Go back", Style::default().fg(Color::Rgb(100, 100, 100)))));

        let para = Paragraph::new(lines).alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(255, 140, 66))));
        frame.render_widget(para, centered_rect(area, 55, 55));
    }

    fn draw_complete(&self, frame: &mut Frame, area: Rect) {
        let mut lines = vec![
            Line::from(Span::styled(" Setup Complete ",
                Style::default().fg(Color::Rgb(100, 220, 100)).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("Your agent is ready. Here's what was configured:", Style::default().fg(Color::Gray))),
            Line::from(""),
        ];

        let has_key = !self.bifrost_key.is_empty();
        lines.push(Line::from(Span::styled(format!("  Agent:       {}", self.agent_name), Style::default().fg(Color::White))));
        lines.push(Line::from(Span::styled(
            format!("  Bifrost:     {} ({})", self.bifrost_url, if has_key { "key set" } else { "no key \u{2014} local fallback" }),
            Style::default().fg(Color::Rgb(150, 150, 150)),
        )));
        lines.push(Line::from(Span::styled(format!("  Model:       {}", self.model_handle), Style::default().fg(Color::Rgb(150, 200, 255)))));

        if let Some(ref import) = self.import_selection {
            match import {
                ImportOption::Letta { id, name } => {
                    lines.push(Line::from(Span::styled(format!("  Imported:    {} from Letta ({})", name, id),
                        Style::default().fg(Color::Rgb(150, 200, 150)))));
                }
                ImportOption::Federation => {
                    lines.push(Line::from(Span::styled("  Federation:  peer sync configured",
                        Style::default().fg(Color::Rgb(150, 200, 255)))));
                }
                ImportOption::Skip => {}
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [Enter]  Enter your world", Style::default().fg(Color::Rgb(200, 200, 200)))));

        let para = Paragraph::new(lines).alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(100, 220, 100))));
        frame.render_widget(para, centered_rect(area, 50, 50));
    }

    fn step_progress(&self) -> String {
        let total = match self.flow {
            SetupFlow::FreshInstall => 4,
            SetupFlow::ImportAgent => 3,
            SetupFlow::FederationSync => 2,
        };
        let current = match (self.flow, self.step) {
            (SetupFlow::FreshInstall, SetupStep::BifrostConfig) => 1,
            (SetupFlow::FreshInstall, SetupStep::CreateAgent) => 2,
            (SetupFlow::FreshInstall, SetupStep::ImportOrFederation) => 3,
            (SetupFlow::FreshInstall, SetupStep::Complete) => 4,
            (SetupFlow::ImportAgent, SetupStep::CreateAgent) => 1,
            (SetupFlow::ImportAgent, SetupStep::ImportOrFederation) => 2,
            (SetupFlow::ImportAgent, SetupStep::Complete) => 3,
            (SetupFlow::FederationSync, SetupStep::FederationConfig) => 1,
            (SetupFlow::FederationSync, SetupStep::Complete) => 2,
            _ => 0,
        };
        format!(" step {} of {} ", current, total)
    }
}

// ── Helpers ──

fn discover_letta_agents() -> Vec<(String, String)> {
    let letta_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".letta")
        .join("agents");
    if !letta_dir.is_dir() {
        return Vec::new();
    }
    let mut agents = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&letta_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let id = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !path.join("memory").is_dir() { continue; }
            let name = read_letta_agent_name(&path).unwrap_or_else(|| id.clone());
            agents.push((id, name));
        }
    }
    agents.sort_by(|a, b| a.1.cmp(&b.1));
    agents
}

fn read_letta_agent_name(agent_dir: &std::path::Path) -> Option<String> {
    let config_path = agent_dir.join("config.yaml");
    if !config_path.exists() { return None; }
    let content = std::fs::read_to_string(config_path).ok()?;
    for line in content.lines() {
        if let Some(stripped) = line.strip_prefix("name:") {
            let name = stripped.trim().trim_matches('"').to_string();
            if !name.is_empty() { return Some(name); }
        }
    }
    None
}

fn centered_rect(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let w = area.width.saturating_mul(pct_x).saturating_div(100).max(40);
    let h = area.height.saturating_mul(pct_y).saturating_div(100).max(10);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_install_flow() {
        let mut state = SetupState::new(SetupFlow::FreshInstall, "kimi-k2.6");
        assert_eq!(state.step, SetupStep::Welcome);
        assert!(state.form.show_submit);
        assert!(state.form.is_submit_focused());
        state.advance();
        assert_eq!(state.step, SetupStep::BifrostConfig);
        state.advance();
        assert_eq!(state.step, SetupStep::CreateAgent);
        state.advance();
        assert_eq!(state.step, SetupStep::ImportOrFederation);
        state.advance();
        assert_eq!(state.step, SetupStep::Complete);
        state.advance();
        assert!(state.complete);
    }

    #[test]
    fn test_import_agent_flow() {
        let mut state = SetupState::new(SetupFlow::ImportAgent, "kimi-k2.6");
        assert_eq!(state.step, SetupStep::CreateAgent);
        state.advance();
        assert_eq!(state.step, SetupStep::ImportOrFederation);
        state.advance();
        assert_eq!(state.step, SetupStep::Complete);
        state.advance();
        assert!(state.complete);
    }

    #[test]
    fn test_text_input() {
        let mut state = SetupState::new(SetupFlow::FreshInstall, "kimi-k2.6");
        state.advance(); // → BifrostConfig
        state.form.focus = 0;
        state.form.slots[0].kind = SlotKind::Text {
            value: String::new(),
            cursor: 0,
            secret: false,
        };
        for ch in "http://localhost:3360".chars() {
            state.form.slots[0].kind.insert(ch);
        }
        assert_eq!(state.form.slots[0].value(), "http://localhost:3360");
        state.advance();
        assert_eq!(state.bifrost_url, "http://localhost:3360");
    }

    #[test]
    fn test_go_back() {
        let mut state = SetupState::new(SetupFlow::FreshInstall, "kimi-k2.6");
        state.advance();
        state.advance();
        state.go_back();
        assert_eq!(state.step, SetupStep::BifrostConfig);
    }

    #[test]
    fn test_model_picker_cycle() {
        let mut state = SetupState::new(SetupFlow::FreshInstall, "kimi-k2.6");
        state.advance(); // BifrostConfig
        state.advance(); // CreateAgent

        // Focus is on slot 0 (name). Advance to slot 1 (model picker).
        state.form.focus = 1;

        // Inject model variants into the picker
        if let SlotKind::ModelPicker { ref mut variants, .. } = state.form.slots[1].kind {
            variants.push("model-a".to_string());
            variants.push("model-b".to_string());
            variants.push("model-c".to_string());
        }

        // Current value should be the default (kimi-k2.6 isn't in variants yet)
        assert_eq!(state.form.slots[1].value(), "kimi-k2.6");

        // Simulate a fetch that populates variants properly synced with value
        if let SlotKind::ModelPicker { ref mut variants, .. } = state.form.slots[1].kind {
            variants.clear();
            variants.push("kimi-k2.6".to_string());
            variants.push("gpt-4o".to_string());
            variants.push("claude-3.7".to_string());
        }

        // Left → cycle backwards to claude-3.7
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        state.handle_key(key);
        assert_eq!(state.form.slots[1].value(), "claude-3.7");

        // Right → cycle forwards to kimi-k2.6
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        state.handle_key(key);
        assert_eq!(state.form.slots[1].value(), "kimi-k2.6");
    }
}
