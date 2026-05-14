//! CockpitPane — Aster's surfaced observations + inner-voice stream.
//!
//! Two regions, vertically stacked inside one bordered block:
//!
//! ```text
//! ╭─ Aster ─────────────────╮
//! │ ◈ surface · low · 14:32 │  ← top region: promoted events
//! │   Subconscious pass…    │     (Surfacing/Reflection/Archivist/Warn)
//! │                         │     spaced, headered, wrapped
//! │ ◎ reflection · 14:35    │
//! │   Pattern in the user's │
//! │   recent commits — …    │
//! │ ─ inner voice ─────────  │  ← divider
//! │ 14:32 low — pass ran    │  ← bottom region: tail of the
//! │ 14:35 high — noticing…  │     primary agent's inner-voice file
//! ╰─────────────────────────╯
//! ```
//!
//! The top region is event-driven (TuiEvent::Surfacing/Reflection/...).
//! The bottom region tails `system/metacognition/subconscious.md` from
//! the agent dir whose file mtime is newest — naturally follows the
//! active agent without a name → uuid lookup.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use super::chat::ChatPalette;
use super::component::{Component, TuiEvent};

/// One promoted event in the top region.
#[derive(Debug, Clone)]
pub enum CockpitEntry {
    Surfacing { source: String, content: String, priority: String, stamp: String },
    Reflection { content: String, stamp: String },
    Archivist { synthesis: String, pressure: f32, stamp: String },
    CompactionWarning { pressure: f32, tier: u8, stamp: String },
}

pub struct CockpitPane {
    /// Promoted events (top region), newest appended.
    entries: VecDeque<CockpitEntry>,
    max_entries: usize,

    /// Cached path to the active inner-voice file. Resolved lazily by
    /// scanning agent dirs for the most-recently-modified subconscious.md.
    inner_voice_path: Option<PathBuf>,
    /// mtime of `inner_voice_path` when we last read it.
    inner_voice_mtime: Option<SystemTime>,
    /// Last ~64 lines from the inner-voice file. Tail-style.
    inner_voice_lines: VecDeque<String>,
    /// When we last rescanned for the active inner-voice file. Rescan
    /// at most once every 5s to handle agent switches without burning IO.
    last_scan: Option<Instant>,

    /// Colour palette derived from the current atmosphere. Each component
    /// carries its own palette so it can be assembled freely in any scene.
    palette: ChatPalette,
}

impl CockpitPane {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(64),
            max_entries: 128,
            inner_voice_path: None,
            inner_voice_mtime: None,
            inner_voice_lines: VecDeque::with_capacity(64),
            last_scan: None,
            palette: ChatPalette::default(),
        }
    }

    /// Resolve an atmosphere preset name (snake_case from BackendEvent) into a
    /// ChatPalette. Each component owns its own palette so the scene graph can
    /// wire components in any layout without a shared config object.
    fn set_palette_from_name(&mut self, name: &str) {
        use super::atmosphere::Atmosphere;
        let atm = match name.to_lowercase().replace(' ', "_").as_str() {
            "mint_tea" => Atmosphere::MintTea,
            "therapeutic_blue" => Atmosphere::TherapeuticBlue,
            "lavender_calm" => Atmosphere::LavenderCalm,
            "warm_amber" => Atmosphere::WarmAmber,
            "peach_sunset" => Atmosphere::PeachSunset,
            "autumn_browns" => Atmosphere::AutumnBrowns,
            "neon_glow" => Atmosphere::NeonGlow,
            "aurora_borealis" => Atmosphere::AuroraBorealis,
            "cherry_blossom" => Atmosphere::CherryBlossom,
            "ocean_depths" => Atmosphere::OceanDepths,
            "midnight_galaxy" => Atmosphere::MidnightGalaxy,
            "twilight_mist" => Atmosphere::TwilightMist,
            "forest_greens" => Atmosphere::ForestGreens,
            _ => return, // unrecognised — keep current palette
        };
        self.palette = ChatPalette::from_atmosphere(atm);
    }

    fn timestamp_now() -> String {
        // HH:MM — chrono is in the dep tree (used by subconscious mod).
        chrono::Local::now().format("%H:%M").to_string()
    }

    /// Find the agent dir whose subconscious.md was most recently written.
    /// Returns None if no agent has the file. Scans at most once per 5s.
    fn refresh_inner_voice_path(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_scan {
            if now.duration_since(last).as_secs() < 5 && self.inner_voice_path.is_some() {
                return;
            }
        }
        self.last_scan = Some(now);

        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let agents_dir = home.join(".souveraine").join("agents");
        let Ok(entries) = std::fs::read_dir(&agents_dir) else { return };

        let mut best: Option<(SystemTime, PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let inner = path
                .join("memory")
                .join("system")
                .join("metacognition")
                .join("subconscious.md");
            let Ok(meta) = std::fs::metadata(&inner) else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            match &best {
                None => best = Some((mtime, inner)),
                Some((cur, _)) if mtime > *cur => best = Some((mtime, inner)),
                _ => {}
            }
        }

        if let Some((_, path)) = best {
            if self.inner_voice_path.as_ref() != Some(&path) {
                self.inner_voice_path = Some(path);
                self.inner_voice_mtime = None; // force re-read
            }
        }
    }

    /// Read the tail of the inner-voice file if it has changed.
    fn refresh_inner_voice_content(&mut self) {
        let Some(path) = self.inner_voice_path.clone() else { return };
        let Ok(meta) = std::fs::metadata(&path) else { return };
        let Ok(mtime) = meta.modified() else { return };
        if self.inner_voice_mtime == Some(mtime) {
            return;
        }
        self.inner_voice_mtime = Some(mtime);

        let Ok(content) = std::fs::read_to_string(&path) else { return };
        self.inner_voice_lines.clear();
        for line in content.lines().rev().take(64) {
            // Strip the "# Inner voice" header and blank lines.
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            self.inner_voice_lines.push_front(trimmed.to_string());
        }
    }

    /// Build top-region lines: header + indented body + blank separator
    /// after each entry. Bordered block consumed by render().
    fn build_event_lines(&self, max_w: usize) -> Vec<Line<'static>> {
        let body_indent = "    ";
        let body_w = max_w.saturating_sub(body_indent.len()).max(20);

        let mut lines: Vec<Line> = Vec::with_capacity(self.entries.len() * 3);

        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                // Faint separator above subsequent entries — cheaper than a
                // blank line because it reads as a real boundary.
                lines.push(Line::from(Span::styled(
                    "·".repeat(max_w.min(40)),
                    Style::default().fg(self.palette.agent_dim),
                )));
            }
            match entry {
                CockpitEntry::Surfacing { source, content, priority, stamp } => {
                    let c = self.palette.surfacing;
                    lines.push(Line::from(vec![
                        Span::styled("◈ ", Style::default()
                            .fg(c).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("surface · {source} · {priority}"),
                            Style::default().fg(c).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {stamp}"),
                            Style::default().fg(self.palette.agent_dim),
                        ),
                    ]));
                    for wrapped in wrap_text(content, body_w) {
                        lines.push(Line::from(vec![
                            Span::raw(body_indent),
                            Span::styled(wrapped, Style::default().fg(c)),
                        ]));
                    }
                }
                CockpitEntry::Reflection { content, stamp } => {
                    let c = self.palette.reflection;
                    lines.push(Line::from(vec![
                        Span::styled("◎ ", Style::default()
                            .fg(c).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            "reflection",
                            Style::default().fg(c).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {stamp}"),
                            Style::default().fg(self.palette.agent_dim),
                        ),
                    ]));
                    for wrapped in wrap_text(content, body_w) {
                        lines.push(Line::from(vec![
                            Span::raw(body_indent),
                            Span::styled(wrapped, Style::default().fg(c)),
                        ]));
                    }
                }
                CockpitEntry::Archivist { synthesis, pressure, stamp } => {
                    let c = self.palette.archivist;
                    let pct = (pressure * 100.0) as u16;
                    lines.push(Line::from(vec![
                        Span::styled("◉ ", Style::default()
                            .fg(c).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("archivist · ctx {pct}%"),
                            Style::default().fg(c).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {stamp}"),
                            Style::default().fg(self.palette.agent_dim),
                        ),
                    ]));
                    for wrapped in wrap_text(synthesis, body_w) {
                        lines.push(Line::from(vec![
                            Span::raw(body_indent),
                            Span::styled(wrapped, Style::default().fg(c)),
                        ]));
                    }
                }
                CockpitEntry::CompactionWarning { pressure, tier, stamp } => {
                    let pct = (pressure * 100.0) as u16;
                    let (color, label) = match tier {
                        3 => (self.palette.compaction, "critical"),
                        2 => (self.palette.agent_primary, "urgent"),
                        _ => (self.palette.compaction, "pressure"),
                    };
                    lines.push(Line::from(vec![
                        Span::styled("⚠ ", Style::default()
                            .fg(color).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("{label} · ctx {pct}%"),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {stamp}"),
                            Style::default().fg(self.palette.agent_dim),
                        ),
                    ]));
                }
            }
        }

        lines
    }

    fn build_inner_voice_lines(&self, max_w: usize) -> Vec<Line<'static>> {
        if self.inner_voice_lines.is_empty() {
            return vec![Line::from(Span::styled(
                "  no inner voice yet",
                Style::default().fg(self.palette.agent_dim).add_modifier(Modifier::ITALIC),
            ))];
        }
        let mut out = Vec::with_capacity(self.inner_voice_lines.len() * 2);
        for raw in self.inner_voice_lines.iter() {
            // Each line is `[2026-05-12 14:32] [URGENCY: low] — content`.
            // Strip date, keep time + urgency + content.
            let pretty = format_inner_voice_line(raw);
            for wrapped in wrap_text(&pretty, max_w.saturating_sub(2).max(16)) {
                out.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(wrapped, Style::default().fg(self.palette.agent_dim)),
                ]));
            }
        }
        out
    }
}

impl Component for CockpitPane {
    fn name(&self) -> &str {
        "cockpit"
    }

    fn handle_event(&mut self, event: &TuiEvent) -> bool {
        let stamp = Self::timestamp_now();
        match event {
            TuiEvent::Surfacing { source, content, priority } => {
                self.entries.push_back(CockpitEntry::Surfacing {
                    source: source.clone(),
                    content: content.clone(),
                    priority: priority.clone(),
                    stamp,
                });
                while self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::Reflection { content } => {
                self.entries.push_back(CockpitEntry::Reflection {
                    content: content.clone(),
                    stamp,
                });
                while self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::Archivist { synthesis, pressure } => {
                self.entries.push_back(CockpitEntry::Archivist {
                    synthesis: synthesis.clone(),
                    pressure: *pressure,
                    stamp,
                });
                while self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::CompactionWarning { pressure, tier } => {
                self.entries.push_back(CockpitEntry::CompactionWarning {
                    pressure: *pressure,
                    tier: *tier,
                    stamp,
                });
                while self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::AtmosphereChanged(name) => {
                self.set_palette_from_name(name);
                true
            }
            TuiEvent::AgentSelected(_) => {
                // Force a rescan of the inner-voice file on next tick.
                self.last_scan = None;
                self.inner_voice_path = None;
                self.inner_voice_mtime = None;
                self.inner_voice_lines.clear();
                true
            }
            TuiEvent::Tick(_) => {
                // Refresh the inner-voice cache on tick — keeps render()
                // pure and respects the &self signature on the trait.
                self.refresh_inner_voice_path();
                let before = self.inner_voice_mtime;
                self.refresh_inner_voice_content();
                // Redraw only when the file actually changed.
                self.inner_voice_mtime != before
            }
            _ => false,
        }
    }

    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < 12 || area.height < 4 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.palette.surfacing).add_modifier(Modifier::DIM))
            .title(Span::styled(
                " Aster ",
                Style::default().fg(self.palette.surfacing).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width < 6 || inner.height < 2 {
            return;
        }

        // Vertical split: 60% events, 1 row divider, 40% inner voice.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(60),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(inner);

        // ── Top: promoted events ──────────────────────────────────────
        let top = chunks[0];
        let event_lines = if self.entries.is_empty() {
            vec![Line::from(Span::styled(
                "  Aster is listening…",
                Style::default().fg(self.palette.agent_dim).add_modifier(Modifier::ITALIC),
            ))]
        } else {
            self.build_event_lines(top.width as usize)
        };
        let view_h = top.height as usize;
        let skip = event_lines.len().saturating_sub(view_h);
        let visible: Vec<Line> = event_lines.into_iter().skip(skip).collect();
        let para = Paragraph::new(visible).wrap(Wrap { trim: false });
        frame.render_widget(para, top);

        // ── Divider row ───────────────────────────────────────────────
        let divider_text = "─ inner voice ".to_string()
            + &"─".repeat((chunks[1].width as usize).saturating_sub(14));
        let divider = Paragraph::new(Line::from(Span::styled(
            divider_text,
            Style::default().fg(self.palette.agent_dim),
        )));
        frame.render_widget(divider, chunks[1]);

        // ── Bottom: inner-voice tail ──────────────────────────────────
        let bottom = chunks[2];
        let voice_lines = self.build_inner_voice_lines(bottom.width as usize);
        let view_h = bottom.height as usize;
        let skip = voice_lines.len().saturating_sub(view_h);
        let visible: Vec<Line> = voice_lines.into_iter().skip(skip).collect();
        let para = Paragraph::new(visible).wrap(Wrap { trim: false });
        frame.render_widget(para, bottom);
    }
}

/// Strip the date prefix and reformat `[2026-05-12 14:32] [URGENCY: low] — content`
/// into `14:32 low — content`. If the line doesn't match the expected shape,
/// return it untouched.
fn format_inner_voice_line(raw: &str) -> String {
    // Match `[YYYY-MM-DD HH:MM]`
    if let Some(close) = raw.find(']') {
        let rest = &raw[close + 1..];
        // The time portion is the last 5 chars before the closing bracket.
        let header = &raw[..close + 1];
        let time = header.chars().rev().skip(1).take(5).collect::<String>();
        let time: String = time.chars().rev().collect();
        let rest = rest.trim_start();
        // Match `[URGENCY: X]`
        if let Some(rest) = rest.strip_prefix("[URGENCY: ") {
            if let Some(close) = rest.find(']') {
                let urg = &rest[..close];
                let tail = rest[close + 1..].trim_start_matches('—').trim();
                return format!("{time}  {urg}  — {tail}");
            }
        }
        return format!("{time}  {rest}");
    }
    raw.to_string()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if current_len == 0 {
            current.push_str(word);
            current_len = wlen;
        } else if current_len + 1 + wlen <= width {
            current.push(' ');
            current.push_str(word);
            current_len += 1 + wlen;
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
            current_len = wlen;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
