//! CockpitPane — a component that surfaces Aster's observations.
//!
//! Receives Surfacing, Reflection, and Archivist events from the scene
//! and renders them as a scrollable log. Lives in the sidebar zone when
//! the layout is ChatWithSidebar, or in a dedicated area on Dashboard.
//!
//! This is the "see Aster working" panel — every observation, surfacing,
//! and context-pressure event appears here.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use std::collections::VecDeque;

use super::component::{Component, TuiEvent};

const SURFACING_YELLOW: Color = Color::Rgb(220, 190, 100);
const REFLECTION_CYAN: Color = Color::Rgb(100, 200, 220);
const ARCHIVIST_MAGENTA: Color = Color::Rgb(200, 140, 220);
const WARN_ORANGE: Color = Color::Rgb(255, 180, 80);
const CRITICAL_RED: Color = Color::Rgb(220, 80, 80);

/// A single entry in the cockpit log.
#[derive(Debug, Clone)]
pub enum CockpitEntry {
    Surfacing { source: String, content: String, priority: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
    CompactionWarning { pressure: f32, tier: u8 },
}

/// The cockpit panel — Aster's observations rendered to screen.
pub struct CockpitPane {
    /// Bounded log of observations, newest appended.
    entries: VecDeque<CockpitEntry>,
    /// Max entries before old ones are dropped.
    max_entries: usize,
}

impl CockpitPane {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(128),
            max_entries: 256,
        }
    }
}

impl Component for CockpitPane {
    fn name(&self) -> &str {
        "cockpit"
    }

    fn handle_event(&mut self, event: &TuiEvent) -> bool {
        match event {
            TuiEvent::Surfacing { source, content, priority } => {
                self.entries.push_back(CockpitEntry::Surfacing {
                    source: source.clone(),
                    content: content.clone(),
                    priority: priority.clone(),
                });
                if self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::Reflection { content } => {
                self.entries.push_back(CockpitEntry::Reflection {
                    content: content.clone(),
                });
                if self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::Archivist { synthesis, pressure } => {
                self.entries.push_back(CockpitEntry::Archivist {
                    synthesis: synthesis.clone(),
                    pressure: *pressure,
                });
                if self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            TuiEvent::CompactionWarning { pressure, tier } => {
                self.entries.push_back(CockpitEntry::CompactionWarning {
                    pressure: *pressure,
                    tier: *tier,
                });
                if self.entries.len() > self.max_entries {
                    self.entries.pop_front();
                }
                true
            }
            _ => false,
        }
    }

    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < 8 || area.height < 3 {
            return;
        }

        let mut lines: Vec<Line> = Vec::with_capacity(self.entries.len());

        for entry in &self.entries {
            match entry {
                CockpitEntry::Surfacing { source, content, priority } => {
                    // Truncate content to fit the panel width
                    let max_content = (area.width as usize).saturating_sub(12).max(20);
                    let content = truncate(content, max_content);
                    lines.push(Line::from(vec![
                        Span::styled("◈ ", Style::default().fg(SURFACING_YELLOW).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("{} · {} — {}", source, priority, content),
                            Style::default().fg(SURFACING_YELLOW),
                        ),
                    ]));
                }
                CockpitEntry::Reflection { content } => {
                    let max_content = (area.width as usize).saturating_sub(12).max(20);
                    let content = truncate(content, max_content);
                    lines.push(Line::from(vec![
                        Span::styled("◎ ", Style::default().fg(REFLECTION_CYAN).add_modifier(Modifier::BOLD)),
                        Span::styled(content, Style::default().fg(REFLECTION_CYAN)),
                    ]));
                }
                CockpitEntry::Archivist { synthesis, pressure } => {
                    let pct = (pressure * 100.0) as u16;
                    let max_content = (area.width as usize).saturating_sub(16).max(20);
                    let synthesis = truncate(synthesis, max_content);
                    lines.push(Line::from(vec![
                        Span::styled("◉ ", Style::default().fg(ARCHIVIST_MAGENTA).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("{} (ctx {}%)", synthesis, pct),
                            Style::default().fg(ARCHIVIST_MAGENTA),
                        ),
                    ]));
                }
                CockpitEntry::CompactionWarning { pressure, tier } => {
                    let pct = (pressure * 100.0) as u16;
                    let warn_color = match tier { 3 => CRITICAL_RED, 2 => Color::Rgb(255, 120, 50), _ => WARN_ORANGE };
                    let label = match tier { 3 => "critical", 2 => "urgent", _ => "warn" };
                    lines.push(Line::from(vec![
                        Span::styled("⚠ ", Style::default().fg(warn_color).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("ctx {}% ({})", pct, label), Style::default().fg(warn_color)),
                    ]));
                }
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "  Aster is listening…",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SURFACING_YELLOW).add_modifier(Modifier::DIM))
            .title(Span::styled(" Aster ", Style::default().fg(SURFACING_YELLOW).add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        let view_height = inner.height as usize;
        let scroll_offset = lines.len().saturating_sub(view_height);

        let visible_lines: Vec<Line> = if scroll_offset > 0 {
            lines.iter().skip(scroll_offset).take(view_height).cloned().collect()
        } else {
            lines
        };

        let para = Paragraph::new(visible_lines)
            .wrap(Wrap { trim: false })
            .block(block);

        frame.render_widget(para, area);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}
