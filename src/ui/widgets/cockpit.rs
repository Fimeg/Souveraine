//! Cockpit widget — thinking / subconscious / inner-voice stream.
//!
//! A single bordered column split into three sections by thin rules:
//!   • thinking (top)       — the agent's reasoning log
//!   • subconscious (mid)   — N+1 surfacings, reflections, archivist notes
//!   • inner voice (bottom) — tail of `subconscious.md` from the agent's memory
//! A pressure bar sits at the foot. The section that is currently *active*
//! glows in the accent colour and is given more vertical room; an idle
//! section collapses to a one-line header with a chevron. Older entries
//! fade so the newest line is always brightest.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use tuie::prelude::*;

use super::progress_bar::ProgressBar;
use crate::ui::chat::{ChatPalette, CockpitEntry as ChatCockpitEntry, CockpitKind};
use crate::ui::theme;

/// Which section is currently doing work — drives the glow + the room split.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    None,
    Thinking,
    Subconscious,
}

/// Three-section cockpit — thinking (top), subconscious (mid), inner voice (bottom).
pub struct Cockpit {
    root: Box<Pane>,
    thinking_title_id: WidgetId<Text>,
    thinking_body_id: WidgetId<Pane>,
    thinking_text_id: WidgetId<Text>,
    sub_title_id: WidgetId<Text>,
    sub_body_id: WidgetId<Pane>,
    sub_text_id: WidgetId<Text>,
    voice_title_id: WidgetId<Text>,
    voice_body_id: WidgetId<Pane>,
    voice_text_id: WidgetId<Text>,
    pressure_bar_id: WidgetId<ProgressBar>,
    pressure_label_id: WidgetId<Text>,

    palette: ChatPalette,
    thinking_count: usize,
    sub_count: usize,
    voice_count: usize,
    active: ActivePane,

    // Inner voice file tracking
    inner_voice_path: Option<PathBuf>,
    inner_voice_mtime: Option<SystemTime>,
    inner_voice_lines: VecDeque<String>,
    last_scan: Option<Instant>,
}

impl DelegateWidget for Cockpit {
    tuie::delegate_widget!(root);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl Cockpit {
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut thinking_title_id = WidgetId::EMPTY;
        let mut thinking_body_id = WidgetId::EMPTY;
        let mut thinking_text_id = WidgetId::EMPTY;
        let mut sub_title_id = WidgetId::EMPTY;
        let mut sub_body_id = WidgetId::EMPTY;
        let mut sub_text_id = WidgetId::EMPTY;
        let mut voice_title_id = WidgetId::EMPTY;
        let mut voice_body_id = WidgetId::EMPTY;
        let mut voice_text_id = WidgetId::EMPTY;
        let mut pressure_bar_id = WidgetId::EMPTY;
        let mut pressure_label_id = WidgetId::EMPTY;

        let thinking_title = Text::new()
            .content(section_title("thinking", false, 0, dim))
            .id(&mut thinking_title_id);
        let thinking_body = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .max_height(0) // starts collapsed (no content yet)
            .y_scroll(Scrollbar::AutoHide)
            .children([Text::new().content("").id(&mut thinking_text_id) as Box<dyn Widget>])
            .id(&mut thinking_body_id);

        let sub_title = Text::new()
            .content(section_title("subconscious", false, 0, dim))
            .id(&mut sub_title_id);
        let sub_body = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .max_height(0)
            .y_scroll(Scrollbar::AutoHide)
            .children([Text::new().content("").id(&mut sub_text_id) as Box<dyn Widget>])
            .id(&mut sub_body_id);

        let voice_title = Text::new()
            .content(section_title("inner voice", false, 0, dim))
            .id(&mut voice_title_id);
        let voice_body = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .max_height(0)
            .y_scroll(Scrollbar::AutoHide)
            .children([Text::new().content("").id(&mut voice_text_id) as Box<dyn Widget>])
            .id(&mut voice_body_id);

        // ── Pressure footer ──────────────────────────────────────────────
        let pressure_label = Text::new()
            .content(StyledStr::new(" ctx ").fg(dim))
            .id(&mut pressure_label_id);
        let pressure_bar = {
            let mut b = ProgressBar::new();
            b.get_layout_mut().style = Style::new().fg(dim);
            b
        };
        let pbar_holder = pressure_bar.flex(1).id(&mut pressure_bar_id);
        let footer = Pane::new()
            .horizontal()
            .height(1)
            .y_place(Place::Center)
            .gap(1)
            .horizontal_padding(1)
            .children([pressure_label as Box<dyn Widget>, pbar_holder]);

        let root = Pane::new()
            .vertical()
            .flex(1)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .children([
                thinking_title as Box<dyn Widget>,
                thinking_body,
                Rule::new(dim) as Box<dyn Widget>,
                sub_title as Box<dyn Widget>,
                sub_body,
                Rule::new(dim) as Box<dyn Widget>,
                voice_title as Box<dyn Widget>,
                voice_body,
                Rule::new(dim) as Box<dyn Widget>,
                footer,
            ]);

        Box::new(Self {
            root,
            thinking_title_id,
            thinking_body_id,
            thinking_text_id,
            sub_title_id,
            sub_body_id,
            sub_text_id,
            voice_title_id,
            voice_body_id,
            voice_text_id,
            pressure_bar_id,
            pressure_label_id,
            palette: *palette,
            thinking_count: 0,
            sub_count: 0,
            voice_count: 0,
            active: ActivePane::None,
            inner_voice_path: None,
            inner_voice_mtime: None,
            inner_voice_lines: VecDeque::new(),
            last_scan: None,
        })
    }

    /// Replace the thinking log content.
    pub fn set_thinking(&mut self, lines: &[String], palette: &ChatPalette) {
        self.palette = *palette;
        self.thinking_count = lines.len();
        let content = build_thinking_text(lines, palette);
        if let Some(t) = self.root.get_widget_mut(self.thinking_text_id) {
            t.set_content(content);
        }
        self.relayout();
    }

    /// Replace the subconscious log content.
    pub fn set_subconscious(&mut self, entries: &[ChatCockpitEntry], palette: &ChatPalette) {
        self.palette = *palette;
        self.sub_count = entries.len();
        let content = build_subconscious_text(entries, palette);
        if let Some(t) = self.root.get_widget_mut(self.sub_text_id) {
            t.set_content(content);
        }
        self.relayout();
    }

    /// Mark which section is currently working — glows it and gives it room.
    pub fn set_active(&mut self, active: ActivePane) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.relayout();
    }

    /// Update the context-pressure bar (0.0–1.0).
    pub fn set_pressure(&mut self, pressure: f32, palette: &ChatPalette) {
        self.palette = *palette;
        let pct = (pressure.clamp(0.0, 1.0) * 100.0).round() as u8;
        let color = pressure_color(pressure, palette);
        if let Some(b) = self.root.get_widget_mut(self.pressure_bar_id) {
            b.set_progress(pressure.clamp(0.0, 1.0));
            b.get_layout_mut().style = Style::new().fg(color);
        }
        if let Some(l) = self.root.get_widget_mut(self.pressure_label_id) {
            l.set_content(StyledStr::new(&format!(" ctx {pct:>3}% ")).fg(color));
        }
    }

    /// Refresh the inner voice file — scans agent dirs, reads tail if changed.
    /// Call once per poll tick.
    pub fn refresh_inner_voice(&mut self) {
        self.refresh_inner_voice_path();
        self.refresh_inner_voice_content();
        let content = build_voice_text(&self.inner_voice_lines, &self.palette);
        self.voice_count = self.inner_voice_lines.len();
        if let Some(t) = self.root.get_widget_mut(self.voice_text_id) {
            t.set_content(content);
        }
        self.relayout();
    }

    /// Find the agent dir whose subconscious.md was most recently written.
    /// Scans at most once per 5s.
    fn refresh_inner_voice_path(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_scan {
            if now.duration_since(last).as_secs() < 5 && self.inner_voice_path.is_some() {
                return;
            }
        }
        self.last_scan = Some(now);

        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let agents_dir = home.join(".souveraine").join("agents");
        let Ok(entries) = std::fs::read_dir(&agents_dir) else {
            return;
        };

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
            let Ok(meta) = std::fs::metadata(&inner) else {
                continue;
            };
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
                self.inner_voice_mtime = None;
            }
        }
    }

    /// Read the tail of the inner-voice file if it has changed.
    fn refresh_inner_voice_content(&mut self) {
        let Some(path) = self.inner_voice_path.clone() else {
            return;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        let Ok(mtime) = meta.modified() else { return };
        if self.inner_voice_mtime == Some(mtime) {
            return;
        }
        self.inner_voice_mtime = Some(mtime);

        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        self.inner_voice_lines.clear();
        for line in content.lines().rev().take(64) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            self.inner_voice_lines.push_front(trimmed.to_string());
        }
    }

    /// Recompute titles (glow + chevron) and the section room split.
    fn relayout(&mut self) {
        let primary = theme::to_tuie_color(self.palette.agent_primary);
        let surfacing = theme::to_tuie_color(self.palette.surfacing);
        let dim = theme::to_tuie_color(self.palette.agent_dim);

        let think_active = self.active == ActivePane::Thinking;
        let sub_active = self.active == ActivePane::Subconscious;
        let think_open = self.thinking_count > 0;
        let sub_open = self.sub_count > 0;
        let voice_open = self.voice_count > 0;

        // Titles — accent + filled chevron when active, dim otherwise.
        if let Some(t) = self.root.get_widget_mut(self.thinking_title_id) {
            let c = if think_active { primary } else { dim };
            t.set_content(section_title(
                "thinking",
                think_open,
                self.thinking_count,
                c,
            ));
        }
        if let Some(t) = self.root.get_widget_mut(self.sub_title_id) {
            let c = if sub_active { surfacing } else { dim };
            t.set_content(section_title("subconscious", sub_open, self.sub_count, c));
        }
        if let Some(t) = self.root.get_widget_mut(self.voice_title_id) {
            t.set_content(section_title(
                "inner voice",
                voice_open,
                self.voice_count,
                dim,
            ));
        }

        // Room split: a collapsed (empty) section hides its body; otherwise the
        // active section gets double the weight so the live stream has room.
        let think_flex = section_flex(think_open, think_active);
        let sub_flex = section_flex(sub_open, sub_active);
        let voice_flex = section_flex(voice_open, false);
        if let Some(b) = self.root.get_widget_mut(self.thinking_body_id) {
            apply_section_room(b, think_open, think_flex);
        }
        if let Some(b) = self.root.get_widget_mut(self.sub_body_id) {
            apply_section_room(b, sub_open, sub_flex);
        }
        if let Some(b) = self.root.get_widget_mut(self.voice_body_id) {
            apply_section_room(b, voice_open, voice_flex);
        }
        self.root.dirty_layout();
    }
}

fn section_flex(open: bool, active: bool) -> u8 {
    if !open {
        0
    } else if active {
        2
    } else {
        1
    }
}

fn apply_section_room(body: &mut dyn Widget, open: bool, flex: u8) {
    body.set_flex(flex);
    if open {
        body.set_max_height(None);
    } else {
        body.set_max_height(Some(0));
    }
}

/// Section header: chevron + title + (count). Chevron is filled when the
/// section holds entries, hollow when collapsed.
fn section_title(name: &str, open: bool, count: usize, color: Color) -> StyledString {
    let chevron = if open { "▾" } else { "▸" };
    let label = if open && count > 0 {
        format!(" {chevron} {name} · {count} ")
    } else {
        format!(" {chevron} {name} ")
    };
    let mut s = StyledString::new();
    s.push_span(StyledStr::new(&label).fg(color).bold());
    s
}

/// Pressure colour: dim → primary → compaction-red as the context fills.
fn pressure_color(pressure: f32, palette: &ChatPalette) -> Color {
    if pressure >= 0.85 {
        theme::to_tuie_color(palette.compaction)
    } else if pressure >= 0.6 {
        theme::to_tuie_color(palette.agent_primary)
    } else {
        theme::to_tuie_color(palette.agent_dim)
    }
}

/// Build thinking-lines text — newest at bottom, fading upward.
fn build_thinking_text(lines: &[String], palette: &ChatPalette) -> StyledString {
    let dim_color = theme::to_tuie_color(palette.agent_dim);
    let mut content = StyledString::new();
    let n = lines.len();

    for (i, t) in lines.iter().enumerate() {
        let fade = entry_fade(i, n);
        let color = dim_color_faded(dim_color, fade);
        if t.starts_with("────") {
            content.push_span(StyledStr::new(&format!("{}\n", t)).fg(color));
        } else {
            content.push_span(StyledStr::new(&format!("· {}\n", t)).fg(color));
        }
    }

    content
}

/// Build subconscious-entries text — newest at bottom, fading upward.
fn build_subconscious_text(entries: &[ChatCockpitEntry], palette: &ChatPalette) -> StyledString {
    let mut content = StyledString::new();
    let n = entries.len();

    for (i, entry) in entries.iter().enumerate() {
        let base = entry_kind_color(entry.kind, palette);
        let fade = entry_fade(i, n);
        let color = dim_color_faded(base, fade);

        let prefix = entry_kind_prefix(entry.kind);
        let line = format!(" {} {}\n", prefix, entry.text);
        let start = content.as_ref().len();
        content.push_str(&line);
        content.style_range(start..content.as_ref().len(), |s| {
            s.set_fg(Some(color));
        });
    }

    content
}

/// Build inner-voice text from the tail of `subconscious.md`.
fn build_voice_text(lines: &VecDeque<String>, palette: &ChatPalette) -> StyledString {
    let dim_color = theme::to_tuie_color(palette.agent_dim);
    let mut content = StyledString::new();

    if lines.is_empty() {
        content.push_span(
            StyledStr::new("  no inner voice yet\n")
                .fg(dim_color)
                .italic(),
        );
        return content;
    }

    for line in lines.iter() {
        let pretty = format_inner_voice_line(line);
        content.push_span(StyledStr::new(&format!(" {}\n", pretty)).fg(dim_color));
    }

    content
}

/// Strip the date prefix and reformat inner voice lines.
/// `[2026-05-12 14:32] [URGENCY: low] — content` → `14:32  low  — content`
fn format_inner_voice_line(raw: &str) -> String {
    if let Some(close) = raw.find(']') {
        let rest = &raw[close + 1..];
        let header = &raw[..close + 1];
        let time = header.chars().rev().skip(1).take(5).collect::<String>();
        let time: String = time.chars().rev().collect();
        let rest = rest.trim_start();
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

fn entry_kind_color(kind: CockpitKind, p: &ChatPalette) -> Color {
    match kind {
        CockpitKind::Surfacing => theme::to_tuie_color(p.surfacing),
        CockpitKind::Reflection => theme::to_tuie_color(p.agent_primary),
        CockpitKind::Archivist => theme::to_tuie_color(p.archivist),
        CockpitKind::CompactionWarn => Color::YELLOW,
        CockpitKind::CompactionUrgent => Color::YELLOW,
        CockpitKind::CompactionCritical => theme::to_tuie_color(p.compaction),
        CockpitKind::InferenceStrain => theme::to_tuie_color(p.tool_dim),
    }
}

fn entry_kind_prefix(kind: CockpitKind) -> &'static str {
    match kind {
        CockpitKind::Surfacing => "~",
        CockpitKind::Reflection => "*",
        CockpitKind::Archivist => "A",
        CockpitKind::CompactionWarn => "!",
        CockpitKind::CompactionUrgent => "!!",
        CockpitKind::CompactionCritical => "!!!",
        CockpitKind::InferenceStrain => "%%",
    }
}

/// Fade factor for entry at index `i` of `n` (0 = newest/bottom).
fn entry_fade(i: usize, n: usize) -> f32 {
    if n <= 1 {
        return 1.0;
    }
    let age = 1.0 - (i as f32 / (n - 1) as f32);
    0.4 + 0.6 * (1.0 - age)
}

fn dim_color_faded(c: Color, factor: f32) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * factor) as u8,
            (g as f32 * factor) as u8,
            (b as f32 * factor) as u8,
        ),
        other => other,
    }
}

// ── Small leaf widgets ─────────────────────────────────────────────────────

/// One-cell-tall horizontal divider that fills its width.
struct Rule {
    layout: Layout,
}

impl Rule {
    fn new(color: Color) -> Box<Self> {
        let mut layout = Layout::new();
        layout.style = Style::new().fg(color).dim();
        Box::new(Self { layout })
    }
}

impl Widget for Rule {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }
    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }
    fn get_name(&self) -> &'static str {
        "CockpitRule"
    }

    fn measure_constraints(&mut self) -> Constraints {
        Constraints {
            min_size: Vec2::new(0, 1),
            max_size: Vec2::new(u16::MAX, 1),
            preferred_size: Vec2::new(0, 1),
        }
    }

    fn render(&self, mut ctx: RenderContext) {
        ctx.set_style(self.layout.style);
        ctx.fill("─");
    }
}
