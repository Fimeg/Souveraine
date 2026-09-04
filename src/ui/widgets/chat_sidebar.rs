//! Chat sidebar — agent vitals displayed as a toggleable panel.
//!
//! Shows agent name, mood, energy bar, memory pressure bar, N+1 cycles,
//! recent memory commits, pending tasks, backend status, N+25/N+100 timestamps,
//! compaction warnings, inference strain, and uptime. Each section is a Text
//! widget row inside a vertical Pane, updated from AgentStatus snapshots.

use std::time::Instant;

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::tuie_app::AgentStatus;
use crate::ui::widgets::stats::push_bar;

/// Additional health vitals not in AgentStatus.
pub struct HealthData {
    pub backend_mode: String,
    pub backend_healthy: bool,
    pub last_reflection: Option<String>,
    pub last_archivist: Option<String>,
    pub last_compaction: Option<(u8, String)>,
    pub strain_504: u32,
    pub strain_429: u32,
    pub strain_other: u32,
}

impl Default for HealthData {
    fn default() -> Self {
        Self {
            backend_mode: "—".into(),
            backend_healthy: false,
            last_reflection: None,
            last_archivist: None,
            last_compaction: None,
            strain_504: 0,
            strain_429: 0,
            strain_other: 0,
        }
    }
}

/// Toggleable sidebar widget for the chat screen.
///
/// Delegates to a bordered vertical `Pane`. Each stat row is a `Text`
/// widget addressed by a typed `WidgetId`. Call `update()` to refresh
/// all vitals from the latest `AgentStatus` and pressure values.
pub struct ChatSidebar {
    pane: Box<Pane>,
    name_id: WidgetId<Text>,
    mood_id: WidgetId<Text>,
    energy_id: WidgetId<Text>,
    pressure_id: WidgetId<Text>,
    n1_id: WidgetId<Text>,
    commits_id: WidgetId<Text>,
    tasks_id: WidgetId<Text>,
    backend_id: WidgetId<Text>,
    n25_id: WidgetId<Text>,
    n100_id: WidgetId<Text>,
    compaction_id: WidgetId<Text>,
    strain_id: WidgetId<Text>,
    uptime_id: WidgetId<Text>,
    started: Instant,
}

impl DelegateWidget for ChatSidebar {
    tuie::delegate_widget!(pane);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl ChatSidebar {
    /// Build a new sidebar with placeholder text.
    ///
    /// All rows start blank — call `update()` to populate them from an
    /// `AgentStatus` snapshot.
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut name_id = WidgetId::EMPTY;
        let mut mood_id = WidgetId::EMPTY;
        let mut energy_id = WidgetId::EMPTY;
        let mut pressure_id = WidgetId::EMPTY;
        let mut n1_id = WidgetId::EMPTY;
        let mut commits_id = WidgetId::EMPTY;
        let mut tasks_id = WidgetId::EMPTY;
        let mut backend_id = WidgetId::EMPTY;
        let mut n25_id = WidgetId::EMPTY;
        let mut n100_id = WidgetId::EMPTY;
        let mut compaction_id = WidgetId::EMPTY;
        let mut strain_id = WidgetId::EMPTY;
        let mut uptime_id = WidgetId::EMPTY;

        let title = {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new(" Vitals ").fg(primary).bold());
            s
        };

        let dim_text = StyledStr::new("—").fg(dim);

        let pane = Pane::new()
            .vertical()
            .bordered()
            .border_style(Style::new().fg(primary).dim())
            .children([
                Text::new().content(title) as Box<dyn Widget>,
                Text::new().content(dim_text).id(&mut name_id),
                Text::new().content(dim_text).id(&mut mood_id),
                Text::new().content(dim_text).id(&mut energy_id),
                Text::new().content(dim_text).id(&mut pressure_id),
                Text::new().content(dim_text).id(&mut n1_id),
                Text::new().content(dim_text).id(&mut commits_id),
                Text::new().content(dim_text).id(&mut tasks_id),
                Text::new().content(dim_text).id(&mut backend_id),
                Text::new().content(dim_text).id(&mut n25_id),
                Text::new().content(dim_text).id(&mut n100_id),
                Text::new().content(dim_text).id(&mut compaction_id),
                Text::new().content(dim_text).id(&mut strain_id),
                Text::new().content(dim_text).id(&mut uptime_id),
            ]);

        Box::new(Self {
            pane,
            name_id,
            mood_id,
            energy_id,
            pressure_id,
            n1_id,
            commits_id,
            tasks_id,
            backend_id,
            n25_id,
            n100_id,
            compaction_id,
            strain_id,
            uptime_id,
            started: Instant::now(),
        })
    }

    /// Additional health vitals not in AgentStatus.
    pub fn update_health(&mut self, health: &HealthData, palette: &ChatPalette) {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);
        let surfacing = theme::to_tuie_color(palette.surfacing);
        let compaction = theme::to_tuie_color(palette.compaction);

        // Backend status
        if let Some(t) = self.pane.get_widget_mut(self.backend_id) {
            let (dot, dot_c) = if health.backend_healthy {
                ("●", Color::GREEN)
            } else {
                ("○", Color::RED)
            };
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Backend  ").fg(dim));
            s.push_span(StyledStr::new(&format!("{dot} ")).fg(dot_c));
            s.push_span(StyledStr::new(&health.backend_mode).fg(primary));
            t.set_content(s);
        }

        // N+25 reflection
        if let Some(t) = self.pane.get_widget_mut(self.n25_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("◎ N+25   ").fg(dim));
            match &health.last_reflection {
                Some(t2) => s.push_span(StyledStr::new(t2).fg(surfacing)),
                None => s.push_span(StyledStr::new("—").fg(dim)),
            }
            t.set_content(s);
        }

        // N+100 archivist
        if let Some(t) = self.pane.get_widget_mut(self.n100_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("◉ N+100  ").fg(dim));
            match &health.last_archivist {
                Some(t2) => {
                    s.push_span(StyledStr::new(t2).fg(theme::to_tuie_color(palette.archivist)))
                }
                None => s.push_span(StyledStr::new("—").fg(dim)),
            }
            t.set_content(s);
        }

        // Compaction
        if let Some(t) = self.pane.get_widget_mut(self.compaction_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("⚠ Comp   ").fg(dim));
            match &health.last_compaction {
                Some((tier, t2)) => {
                    let label = match tier {
                        3 => "critical",
                        2 => "urgent",
                        _ => "warn",
                    };
                    s.push_span(StyledStr::new(&format!("{label} · {t2}")).fg(compaction));
                }
                None => s.push_span(StyledStr::new("none").fg(dim)),
            }
            t.set_content(s);
        }

        // Inference strain
        if let Some(t) = self.pane.get_widget_mut(self.strain_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Strain   ").fg(dim));
            if health.strain_504 + health.strain_429 + health.strain_other == 0 {
                s.push_span(StyledStr::new("clear").fg(Color::GREEN));
            } else {
                let mut parts = Vec::new();
                if health.strain_504 > 0 {
                    parts.push(format!("504×{}", health.strain_504));
                }
                if health.strain_429 > 0 {
                    parts.push(format!("429×{}", health.strain_429));
                }
                if health.strain_other > 0 {
                    parts.push(format!("err×{}", health.strain_other));
                }
                s.push_span(StyledStr::new(&parts.join(" · ")).fg(compaction));
            }
            t.set_content(s);
        }

        // Uptime
        if let Some(t) = self.pane.get_widget_mut(self.uptime_id) {
            let secs = self.started.elapsed().as_secs();
            let (h, m) = (secs / 3600, (secs % 3600) / 60);
            let uptime = if h > 0 {
                format!("{h}:{m:02} up")
            } else {
                format!("{m}m up")
            };
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Uptime   ").fg(dim));
            s.push_span(StyledStr::new(&uptime).fg(primary));
            t.set_content(s);
        }
    }

    /// Update all vitals from an AgentStatus snapshot + pressure value.
    pub fn update(
        &mut self,
        status: &AgentStatus,
        pressure: f32,
        n1_count: usize,
        palette: &ChatPalette,
    ) {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);
        let surfacing = theme::to_tuie_color(palette.surfacing);
        let tool = theme::to_tuie_color(palette.tool_accent);

        // Name
        if let Some(t) = self.pane.get_widget_mut(self.name_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Name    ").fg(dim));
            s.push_span(StyledStr::new(&status.name).fg(primary).bold());
            t.set_content(s);
        }

        // Mood
        if let Some(t) = self.pane.get_widget_mut(self.mood_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Mood    ").fg(dim));
            s.push_span(StyledStr::new(&status.mood).fg(surfacing));
            t.set_content(s);
        }

        // Energy bar [████████░░] 80%
        if let Some(t) = self.pane.get_widget_mut(self.energy_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Energy  ").fg(dim));
            push_bar(&mut s, status.energy as f32 / 100.0, 10, primary);
            s.push_span(StyledStr::new(&format!(" {}%", status.energy)).fg(primary));
            t.set_content(s);
        }

        // Memory pressure bar — coloured by threshold
        if let Some(t) = self.pane.get_widget_mut(self.pressure_id) {
            let color = if pressure > 0.8 {
                Color::RED
            } else if pressure > 0.5 {
                Color::YELLOW
            } else {
                Color::GREEN
            };
            let pct = (pressure * 100.0).round() as u8;
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("MemPres ").fg(dim));
            push_bar(&mut s, pressure.clamp(0.0, 1.0), 10, color);
            s.push_span(StyledStr::new(&format!(" {}%", pct)).fg(color));
            t.set_content(s);
        }

        // N+1 cycles
        if let Some(t) = self.pane.get_widget_mut(self.n1_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("N+1     ").fg(dim));
            if n1_count > 0 {
                s.push_span(StyledStr::new(&format!("{} active", n1_count)).fg(surfacing));
            } else {
                s.push_span(StyledStr::new("idle").fg(dim));
            }
            t.set_content(s);
        }

        // Recent memory commits
        if let Some(t) = self.pane.get_widget_mut(self.commits_id) {
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Commits ").fg(dim));
            s.push_span(StyledStr::new(&status.memory_commits.to_string()).fg(tool));
            t.set_content(s);
        }

        // Pending tasks
        if let Some(t) = self.pane.get_widget_mut(self.tasks_id) {
            let color = if status.pending_tasks > 0 {
                Color::YELLOW
            } else {
                dim
            };
            let mut s = StyledString::new();
            s.push_span(StyledStr::new("Tasks   ").fg(dim));
            s.push_span(StyledStr::new(&status.pending_tasks.to_string()).fg(color));
            t.set_content(s);
        }
    }
}

/// Append a bar like `[████████░░] ` to `out` using `width` blocks.
///
/// `fraction` is 0.0–1.0; filled blocks use `color`, empty blocks and
/// brackets use dim gray.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tuie_app::AgentStatus;
    use tuie::emulator::Emulator;

    #[test]
    fn sidebar_renders_agent_name() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("TestBot"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_mood() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("Happy"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_energy_bar() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("80%"), "got: {rendered:?}");
        assert!(rendered.contains('['), "got: {rendered:?}");
        assert!(rendered.contains(']'), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_pressure_green() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.3, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(!rendered.is_empty(), "output was empty");
        assert!(rendered.contains("MemPres"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_pressure_red() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.9, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(!rendered.is_empty(), "output was empty");
    }

    #[test]
    fn sidebar_renders_n1_active() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 3, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("3 active"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_n1_idle() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 0, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("idle"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_commits() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("42"), "got: {rendered:?}");
    }

    #[test]
    fn sidebar_renders_tasks() {
        let palette = ChatPalette::default();
        let mut widget = ChatSidebar::new(&palette);
        let status = AgentStatus {
            name: "TestBot".into(),
            mood: "Happy".into(),
            energy: 80,
            memory_commits: 42,
            pending_tasks: 3,
            ..Default::default()
        };
        widget.update(&status, 0.45, 2, &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(30, 12));
        let rendered = term.get_snapshot_text();
        assert!(rendered.contains("3"), "got: {rendered:?}");
    }
}
