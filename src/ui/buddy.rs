//! WIP: Companion Buddy System for Souveraine TUI
//!
//! Provides a visual companion agent that sits alongside the main interface.
//! Shows agent health, mood, and subconscious activity indicators.
//!
//! Status: WIP - Basic structure implemented, needs integration with agent selection

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use crate::ui::animation::{Animator, colors};

/// Visual companion sprite state
#[derive(Debug, Clone)]
pub struct CompanionSprite {
    pub name: String,
    pub mood: String,
    pub energy: u8,           // 0-100
    pub health: u8,           // 0-100
    pub subconscious_active: bool,
    pub last_surfacing: Option<String>,
}

impl Default for CompanionSprite {
    fn default() -> Self {
        Self {
            name: "Ani".to_string(),
            mood: "Idle".to_string(),
            energy: 50,
            health: 100,
            subconscious_active: false,
            last_surfacing: None,
        }
    }
}

impl CompanionSprite {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Update mood based on agent state
    pub fn update_mood(&mut self, mood: &str) {
        self.mood = mood.to_string();
    }

    /// Update energy level (0-100)
    pub fn set_energy(&mut self, energy: u8) {
        self.energy = energy.min(100);
    }

    /// Update health level (0-100)
    pub fn set_health(&mut self, health: u8) {
        self.health = health.min(100);
    }

    /// Set subconscious activity status
    pub fn set_subconscious_active(&mut self, active: bool) {
        self.subconscious_active = active;
    }

    /// Record a surfacing event
    pub fn record_surfacing(&mut self, event: &str) {
        self.last_surfacing = Some(event.to_string());
    }
}

/// Buddy state with animation support
pub struct BuddyState {
    pub sprite: CompanionSprite,
    pub animator: Animator,
    pub position: BuddyPosition,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuddyPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Default for BuddyState {
    fn default() -> Self {
        Self {
            sprite: CompanionSprite::default(),
            animator: Animator::new(),
            position: BuddyPosition::TopRight,
            visible: true,
        }
    }
}

impl BuddyState {
    pub fn new(name: &str) -> Self {
        Self {
            sprite: CompanionSprite::new(name),
            animator: Animator::new(),
            position: BuddyPosition::TopRight,
            visible: true,
        }
    }

    pub fn set_position(&mut self, position: BuddyPosition) {
        self.position = position;
    }

    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }
}

/// Draw the companion buddy in the TUI
pub fn draw_buddy(frame: &mut Frame, state: &BuddyState, area: Rect) {
    if !state.visible {
        return;
    }

    // Calculate position based on BuddyPosition
    let (x, y, width) = match state.position {
        BuddyPosition::TopLeft => (area.x + 1, area.y + 1, 20),
        BuddyPosition::TopRight => (area.x + area.width.saturating_sub(21), area.y + 1, 20),
        BuddyPosition::BottomLeft => (area.x + 1, area.y + area.height.saturating_sub(6), 20),
        BuddyPosition::BottomRight => (
            area.x + area.width.saturating_sub(21),
            area.y + area.height.saturating_sub(6),
            20,
        ),
    };

    let buddy_area = Rect {
        x,
        y,
        width: width.min(area.width.saturating_sub(2)),
        height: 5,
    };

    // Breathing animation for energy
    let breathe = state.animator.breathe(2000); // 2 second cycle
    let energy_color = if state.sprite.energy > 70 {
        colors::ANI_PRIMARY
    } else if state.sprite.energy > 40 {
        colors::ANI_SECONDARY
    } else {
        colors::ANI_DIM
    };

    // Create buddy content
    let buddy_content = vec![
        Line::from(vec![
            Span::styled(
                format!("  ◈ {}  ", state.sprite.name),
                Style::default()
                    .fg(energy_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  Mood: {}  ", state.sprite.mood),
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Energy: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "▰".repeat((state.sprite.energy / 10) as usize),
                Style::default().fg(energy_color),
            ),
            Span::styled(
                "▱".repeat(10 - (state.sprite.energy / 10) as usize),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Health: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "█".repeat((state.sprite.health / 10) as usize),
                Style::default().fg(Color::Green),
            ),
            Span::styled(
                "░".repeat(10 - (state.sprite.health / 10) as usize),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(
            Style::default()
                .fg(energy_color)
                .add_modifier(Modifier::DIM),
        );

    let paragraph = Paragraph::new(buddy_content)
        .block(block)
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    frame.render_widget(paragraph, buddy_area);

    // Draw subconscious indicator if active
    if state.sprite.subconscious_active {
        let sub_area = Rect {
            x: buddy_area.x,
            y: buddy_area.y + buddy_area.height,
            width: buddy_area.width,
            height: 1,
        };

        let sub_indicator = Paragraph::new(Line::from(vec![
            Span::styled("  ◈ ", Style::default().fg(colors::SUBCONSCIOUS)),
            Span::styled(
                "Subconscious Active",
                Style::default()
                    .fg(colors::SUBCONSCIOUS)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));

        frame.render_widget(sub_indicator, sub_area);
    }

    // Draw last surfacing if present
    if let Some(surfacing) = &state.sprite.last_surfacing {
        let surf_area = Rect {
            x: buddy_area.x,
            y: buddy_area.y + buddy_area.height + 1,
            width: buddy_area.width.min(30),
            height: 2,
        };

        let surf_content = vec![
            Line::from(vec![
                Span::styled("  ⤷ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Surfacing:",
                    Style::default()
                        .fg(colors::SUBCONSCIOUS)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    truncate(surfacing, 25),
                    Style::default().fg(Color::Gray),
                ),
            ]),
        ];

        let surf_para = Paragraph::new(surf_content)
            .style(Style::default().bg(Color::Rgb(20, 20, 30)));

        frame.render_widget(surf_para, surf_area);
    }
}

/// Truncate a string to max length with ellipsis
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

/// Draw buddy on welcome screen with agent selection
pub fn draw_welcome_buddy(
    frame: &mut Frame,
    state: &BuddyState,
    area: Rect,
    selected_agent: Option<&str>,
) {
    if !state.visible {
        return;
    }

    // Position buddy in top-right corner of welcome screen
    let buddy_area = Rect {
        x: area.x + area.width.saturating_sub(22),
        y: area.y + 1,
        width: 20,
        height: 8,
    };

    let mut content = vec![
        Line::from(vec![
            Span::styled(
                "  ◈ COMPANION  ",
                Style::default()
                    .fg(colors::ANI_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    if let Some(agent_name) = selected_agent {
        content.extend(vec![
            Line::from(vec![
                Span::styled("  Agent: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    agent_name,
                    Style::default()
                        .fg(colors::ANI_SECONDARY)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Ready",
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "ENTER",
                    Style::default()
                        .fg(colors::ANI_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to wake ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    agent_name,
                    Style::default().fg(colors::ANI_SECONDARY),
                ),
            ]),
        ]);
    } else {
        content.extend(vec![
            Line::from(vec![
                Span::styled("  No agent selected", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "a",
                    Style::default()
                        .fg(colors::ANI_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to create alias", Style::default().fg(Color::DarkGray)),
            ]),
        ]);
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(
            Style::default()
                .fg(colors::ANI_PRIMARY)
                .add_modifier(Modifier::DIM),
        );

    let paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    frame.render_widget(paragraph, buddy_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_companion_sprite_default() {
        let sprite = CompanionSprite::default();
        assert_eq!(sprite.name, "Ani");
        assert_eq!(sprite.mood, "Idle");
        assert_eq!(sprite.energy, 50);
        assert_eq!(sprite.health, 100);
    }

    #[test]
    fn test_companion_sprite_new() {
        let sprite = CompanionSprite::new("TestAgent");
        assert_eq!(sprite.name, "TestAgent");
    }

    #[test]
    fn test_companion_sprite_updates() {
        let mut sprite = CompanionSprite::default();
        sprite.update_mood("Happy");
        assert_eq!(sprite.mood, "Happy");

        sprite.set_energy(75);
        assert_eq!(sprite.energy, 75);

        sprite.set_health(90);
        assert_eq!(sprite.health, 90);

        sprite.set_subconscious_active(true);
        assert!(sprite.subconscious_active);
    }

    #[test]
    fn test_buddy_state_default() {
        let state = BuddyState::default();
        assert_eq!(state.sprite.name, "Ani");
        assert!(state.visible);
    }

    #[test]
    fn test_buddy_state_new() {
        let state = BuddyState::new("TestBuddy");
        assert_eq!(state.sprite.name, "TestBuddy");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("Hello World", 20), "Hello World");
        assert_eq!(truncate("Hello World", 10), "Hello Wor…");
    }
}
