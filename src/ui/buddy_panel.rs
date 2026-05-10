//! BuddyPanel — visual companion Component for the TUI.
//!
//! Receives real Aster surfacing data, mood, and energy events from the
//! scene and renders a visual companion card. Unlike the overlay-based
//! `draw_buddy()` in buddy.rs, this is a proper Component that renders
//! into its allocated zone with real data.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph},
    Frame,
};

use super::component::{Component, TuiEvent};
use super::animation::colors;

const SURFACING_YELLOW: Color = Color::Rgb(220, 190, 100);

/// The buddy panel — shows agent name, energy, mood, and the last surfacing.
pub struct BuddyPanel {
    /// Agent name to display.
    pub name: String,
    /// Current mood string.
    pub mood: String,
    /// Energy level 0–100.
    pub energy: u8,
    /// Whether Aster (subconscious) is active.
    pub subconscious_active: bool,
    /// The most recent surfacing content.
    pub last_surfacing: Option<String>,
}

impl Default for BuddyPanel {
    fn default() -> Self {
        Self {
            name: "Ani".to_string(),
            mood: "Idle".to_string(),
            energy: 50,
            subconscious_active: false,
            last_surfacing: None,
        }
    }
}

impl BuddyPanel {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }
}

impl Component for BuddyPanel {
    fn name(&self) -> &str {
        "buddy"
    }

    fn handle_event(&mut self, event: &TuiEvent) -> bool {
        match event {
            TuiEvent::AgentSelected(name) => {
                self.name = name.clone();
                true
            }
            TuiEvent::MoodChanged(mood) => {
                self.mood = mood.clone();
                true
            }
            TuiEvent::EnergyChanged(energy) => {
                self.energy = *energy;
                true
            }
            TuiEvent::Surfacing { content, .. } => {
                self.last_surfacing = Some(content.clone());
                self.subconscious_active = true;
                true
            }
            TuiEvent::BackendStatus { .. } => {
                self.subconscious_active = true;
                true
            }
            _ => false,
        }
    }

    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < 16 || area.height < 5 {
            return;
        }

        let energy_color = if self.energy > 70 {
            colors::ANI_PRIMARY
        } else if self.energy > 40 {
            colors::ANI_SECONDARY
        } else {
            colors::ANI_DIM
        };

        let mut content = vec![
            Line::from(vec![
                Span::styled("  ◈ ", Style::default().fg(energy_color).add_modifier(Modifier::BOLD)),
                Span::styled(&self.name, Style::default().fg(energy_color).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
        ];

        // Energy bar
        let bar_w = (area.width as usize).saturating_sub(6).min(14);
        let filled = ((self.energy as usize) * bar_w / 100).min(bar_w);
        let empty = bar_w.saturating_sub(filled);
        content.push(Line::from(vec![
            Span::styled("  ⚡ ", Style::default().fg(Color::DarkGray)),
            Span::styled("▰".repeat(filled), Style::default().fg(energy_color)),
            Span::styled("▱".repeat(empty), Style::default().fg(Color::DarkGray)),
        ]));

        // Mood
        if !self.mood.is_empty() {
            content.push(Line::from(vec![
                Span::styled("  ◌ ", Style::default().fg(Color::DarkGray)),
                Span::styled(&self.mood, Style::default().fg(Color::Gray)),
            ]));
        }

        // Last surfacing
        if let Some(ref s) = self.last_surfacing {
            let max = (area.width as usize).saturating_sub(6).max(10);
            let truncated = if s.chars().count() > max {
                let mut t: String = s.chars().take(max).collect();
                t.push('…');
                t
            } else {
                s.clone()
            };
            content.push(Line::from(""));
            content.push(Line::from(vec![
                Span::styled("  ⤷ ", Style::default().fg(Color::DarkGray)),
                Span::styled(truncated, Style::default().fg(SURFACING_YELLOW).add_modifier(Modifier::ITALIC)),
            ]));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(energy_color).add_modifier(Modifier::DIM));

        let para = Paragraph::new(content).block(block);
        frame.render_widget(para, area);
    }
}
