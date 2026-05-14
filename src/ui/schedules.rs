//! Schedules editor — the Cron screen in the TUI.
//!
//! Reads the agent's schedules directory (`~/.souveraine/agents/{id}/schedules/`)
//! and lets the user list, create, delete, enable/disable, and run-now.
//! Each schedule is a markdown file with YAML frontmatter — the same format
//! `CronSensor` and the `souveraine schedule` CLI use, so what we write here
//! is picked up by the next cron tick.

use std::path::{Path, PathBuf};

use chrono::Utc;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::core::nervous::cron::{parse_schedule_file, ScheduleEntry, ScheduleKind};
use crate::ui::chat::ChatPalette;

pub enum Mode {
    Browse,
    ConfirmDelete,
    Create(CreateForm),
    Saved(String),
    Error(String),
}

pub struct CreateForm {
    pub field: CreateField,
    pub name: String,
    pub kind: ScheduleKind,
    pub schedule: String,
    pub prompt: String,
    pub urgency: f32,
}

#[derive(Clone, Copy)]
pub enum CreateField {
    Name,
    Kind,
    Schedule,
    Prompt,
    Urgency,
}

impl CreateForm {
    pub fn new() -> Self {
        Self {
            field: CreateField::Name,
            name: String::new(),
            kind: ScheduleKind::Interval,
            schedule: "3600".to_string(),
            prompt: String::new(),
            urgency: 0.3,
        }
    }

    pub fn next_field(&mut self) {
        self.field = match self.field {
            CreateField::Name => CreateField::Kind,
            CreateField::Kind => CreateField::Schedule,
            CreateField::Schedule => CreateField::Prompt,
            CreateField::Prompt => CreateField::Urgency,
            CreateField::Urgency => CreateField::Name,
        };
    }

    pub fn prev_field(&mut self) {
        self.field = match self.field {
            CreateField::Name => CreateField::Urgency,
            CreateField::Kind => CreateField::Name,
            CreateField::Schedule => CreateField::Kind,
            CreateField::Prompt => CreateField::Schedule,
            CreateField::Urgency => CreateField::Prompt,
        };
    }
}

pub struct SchedulesView {
    pub agent_label: String,
    pub schedules_dir: PathBuf,
    pub entries: Vec<ScheduleEntry>,
    pub selected: usize,
    pub mode: Mode,

    /// Atmosphere-derived colour palette. Updated when the agent changes mood
    /// or the user triggers an AtmospherePreview from settings.
    pub palette: ChatPalette,
}

impl SchedulesView {
    pub fn new(agent_label: String, schedules_dir: PathBuf) -> Self {
        let mut s = Self {
            agent_label,
            schedules_dir,
            entries: Vec::new(),
            selected: 0,
            mode: Mode::Browse,
            palette: ChatPalette::default(),
        };
        s.reload();
        s
    }

    pub fn reload(&mut self) {
        self.entries = scan(&self.schedules_dir);
        if self.selected >= self.entries.len() && !self.entries.is_empty() {
            self.selected = self.entries.len() - 1;
        }
    }

    pub fn current(&self) -> Option<&ScheduleEntry> {
        self.entries.get(self.selected)
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    pub fn toggle_enabled(&mut self) {
        let Some(entry) = self.entries.get_mut(self.selected) else { return };
        entry.enabled = !entry.enabled;
        let path = self.schedules_dir.join(format!("{}.md", entry.name));
        match write_schedule_file(&path, entry) {
            Ok(()) => self.mode = Mode::Saved(format!("{}: {}", entry.name, if entry.enabled { "enabled" } else { "disabled" })),
            Err(e) => self.mode = Mode::Error(format!("save failed: {e}")),
        }
    }

    pub fn confirm_delete(&mut self) {
        if self.entries.get(self.selected).is_some() {
            self.mode = Mode::ConfirmDelete;
        }
    }

    pub fn cancel_delete(&mut self) {
        self.mode = Mode::Browse;
    }

    pub fn execute_delete(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            self.mode = Mode::Browse;
            return;
        };
        let path = self.schedules_dir.join(format!("{}.md", entry.name));
        match std::fs::remove_file(&path) {
            Ok(()) => {
                let name = entry.name.clone();
                self.reload();
                self.mode = Mode::Saved(format!("deleted '{name}'"));
            }
            Err(e) => self.mode = Mode::Error(format!("delete failed: {e}")),
        }
    }

    pub fn trigger_run(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else { return };
        let trigger = self.schedules_dir.join(format!(".trigger-{}", entry.name));
        match std::fs::write(&trigger, "") {
            Ok(()) => self.mode = Mode::Saved(format!("'{}' will fire on next tick", entry.name)),
            Err(e) => self.mode = Mode::Error(format!("trigger failed: {e}")),
        }
    }

    pub fn open_create(&mut self) {
        self.mode = Mode::Create(CreateForm::new());
    }

    pub fn cancel_create(&mut self) {
        self.mode = Mode::Browse;
    }

    pub fn save_create(&mut self) {
        let Mode::Create(form) = &self.mode else { return };
        if form.name.trim().is_empty() {
            self.mode = Mode::Error("name is required".to_string());
            return;
        }
        if form.schedule.trim().is_empty() {
            self.mode = Mode::Error("schedule expression is required".to_string());
            return;
        }
        if form.prompt.trim().is_empty() {
            self.mode = Mode::Error("prompt is required".to_string());
            return;
        }
        let entry = ScheduleEntry {
            name: form.name.trim().to_string(),
            kind: form.kind.clone(),
            schedule: form.schedule.trim().to_string(),
            source: "user".to_string(),
            enabled: true,
            urgency: form.urgency,
            created_at: Utc::now(),
            prompt: form.prompt.trim().to_string(),
        };
        let path = self.schedules_dir.join(format!("{}.md", entry.name));
        if path.exists() {
            self.mode = Mode::Error(format!("'{}' already exists", entry.name));
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&self.schedules_dir) {
            self.mode = Mode::Error(format!("mkdir failed: {e}"));
            return;
        }
        match write_schedule_file(&path, &entry) {
            Ok(()) => {
                let name = entry.name.clone();
                self.reload();
                self.mode = Mode::Saved(format!("created '{name}'"));
            }
            Err(e) => self.mode = Mode::Error(format!("save failed: {e}")),
        }
    }

    pub fn clear_status(&mut self) {
        if matches!(self.mode, Mode::Saved(_) | Mode::Error(_)) {
            self.mode = Mode::Browse;
        }
    }
}

fn scan(dir: &Path) -> Vec<ScheduleEntry> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(e) = parse_schedule_file(&path) {
                    entries.push(e);
                }
            }
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn write_schedule_file(path: &Path, entry: &ScheduleEntry) -> anyhow::Result<()> {
    let kind = match entry.kind {
        ScheduleKind::Once => "once",
        ScheduleKind::Interval => "interval",
        ScheduleKind::Cron => "cron",
    };
    let content = format!(
        "---\nname: {name}\nkind: {kind}\nschedule: \"{schedule}\"\nsource: {source}\nenabled: {enabled}\nurgency: {urgency}\ncreated_at: {created}\n---\n\n{prompt}\n",
        name = entry.name,
        kind = kind,
        schedule = entry.schedule,
        source = entry.source,
        enabled = entry.enabled,
        urgency = entry.urgency,
        created = entry.created_at.to_rfc3339(),
        prompt = entry.prompt,
    );
    std::fs::write(path, content)?;
    Ok(())
}

pub fn draw(frame: &mut Frame, view: &SchedulesView) {
    let area = frame.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(format!(" Schedules — {} ", view.agent_label))
        .style(Style::default().fg(view.palette.agent_primary).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
    frame.render_widget(header, chunks[0]);

    match &view.mode {
        Mode::Create(form) => draw_create_form(frame, chunks[1], form, &view.palette),
        Mode::ConfirmDelete => draw_confirm_delete(frame, chunks[1], view),
        _ => draw_list(frame, chunks[1], view),
    }

    let footer = footer_for(&view.mode);
    let footer = Paragraph::new(footer)
        .style(Style::default().fg(view.palette.agent_dim))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
    frame.render_widget(footer, chunks[2]);
}

fn draw_list(frame: &mut Frame, area: Rect, view: &SchedulesView) {
    let p = &view.palette;
    let mut lines: Vec<Line> = Vec::new();

    if view.entries.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No schedules yet. Press `c` to create one.",
            Style::default().fg(p.agent_dim),
        )));
    } else {
        for (idx, entry) in view.entries.iter().enumerate() {
            let selected = idx == view.selected;
            let marker = if selected { ">" } else { " " };
            let status_color = if entry.enabled { Color::Rgb(120, 200, 120) } else { p.agent_dim };
            let status_label = if entry.enabled { "●" } else { "○" };
            let kind = match entry.kind {
                ScheduleKind::Once => "once",
                ScheduleKind::Interval => "interval",
                ScheduleKind::Cron => "cron",
            };
            let style = if selected {
                Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(status_label.to_string(), Style::default().fg(status_color)),
                Span::raw("  "),
                Span::styled(format!("{:<20}", entry.name), style),
                Span::styled(format!("{:<10}", kind), Style::default().fg(p.agent_dim)),
                Span::styled(format!("{:<24}", entry.schedule), Style::default().fg(p.agent_primary)),
                Span::styled(format!("urg {:.1}", entry.urgency), Style::default().fg(p.agent_dim)),
            ]));
            if selected {
                let prompt_preview: String = entry.prompt.chars().take(120).collect();
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(prompt_preview, Style::default().fg(p.agent_dim).add_modifier(Modifier::ITALIC)),
                ]));
            }
        }
    }

    if let Mode::Saved(msg) = &view.mode {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("  ✓ {msg}"), Style::default().fg(Color::Rgb(120, 200, 120)))));
    } else if let Mode::Error(msg) = &view.mode {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("  ✗ {msg}"), Style::default().fg(Color::Rgb(220, 100, 100)))));
    }

    let list = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
    frame.render_widget(list, area);
}

fn draw_confirm_delete(frame: &mut Frame, area: Rect, view: &SchedulesView) {
    let p = &view.palette;
    let name = view.current().map(|e| e.name.as_str()).unwrap_or("?");
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Delete '{name}'?"),
            Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  y — yes, delete    n / Esc — cancel",
            Style::default().fg(p.agent_dim),
        )),
    ];
    let body = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
    frame.render_widget(body, area);
}

fn draw_create_form(frame: &mut Frame, area: Rect, form: &CreateForm, palette: &ChatPalette) {
    let kind_str = match form.kind {
        ScheduleKind::Once => "once",
        ScheduleKind::Interval => "interval",
        ScheduleKind::Cron => "cron",
    };
    let is_field = |f: CreateField| {
        std::mem::discriminant(&form.field) == std::mem::discriminant(&f)
    };
    let cursor = |f: CreateField| if is_field(f) { "▌" } else { " " };
    let field_style = |f: CreateField| {
        if is_field(f) {
            Style::default().fg(palette.agent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        }
    };

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  name:      ", Style::default().fg(palette.agent_dim)),
        Span::styled(form.name.clone(), field_style(CreateField::Name)),
        Span::raw(cursor(CreateField::Name)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  kind:      ", Style::default().fg(palette.agent_dim)),
        Span::styled(kind_str.to_string(), field_style(CreateField::Kind)),
        Span::styled("   (←/→ to cycle: interval, cron, once)", Style::default().fg(palette.agent_dim)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  schedule:  ", Style::default().fg(palette.agent_dim)),
        Span::styled(form.schedule.clone(), field_style(CreateField::Schedule)),
        Span::raw(cursor(CreateField::Schedule)),
        Span::styled("    (seconds for interval, cron expr for cron)", Style::default().fg(palette.agent_dim)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  prompt:    ", Style::default().fg(palette.agent_dim)),
        Span::styled(form.prompt.clone(), field_style(CreateField::Prompt)),
        Span::raw(cursor(CreateField::Prompt)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  urgency:   ", Style::default().fg(palette.agent_dim)),
        Span::styled(format!("{:.1}", form.urgency), field_style(CreateField::Urgency)),
        Span::styled("    (←/→ to adjust, 0.0–1.0)", Style::default().fg(palette.agent_dim)),
    ]));

    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(" New schedule ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.agent_primary)),
        );
    frame.render_widget(body, area);
}

fn footer_for(mode: &Mode) -> String {
    match mode {
        Mode::Browse => "j/k navigate • c create • e toggle • r run-now • d delete • q back".to_string(),
        Mode::ConfirmDelete => "y confirm • n/Esc cancel".to_string(),
        Mode::Create(_) => "Tab next field • Enter save • Esc cancel".to_string(),
        Mode::Saved(_) | Mode::Error(_) => "any key to continue".to_string(),
    }
}
