//! Presence — Annie Composite, made felt in the TUI.
//!
//! This module is the *body channel* for the running agent. Cockpit shows what
//! Aster *says* (surfaced lines, ledger pings); Presence shows what Annie *looks
//! like* while saying it. Same underlying state (volition + InferenceStrain +
//! context tier + ledger drift) routed through two different sensory channels.
//!
//! ## Subject
//!
//! Annie Composite. One face. Aster is a *mode* of Annie, not a separate being —
//! there is no "subconscious avatar" and no swap-to-subconscious affordance.
//! When the N+1 pass runs, the same face shifts posture (gaze fixed, eyes narrow);
//! the character does not change.
//!
//! ## State model (orthogonal axes)
//!
//! - [`Posture`] — what's happening: `Idle | Processing | Affectionate | Straining | Yawning`.
//! - [`Eye`] — transient overlay: `Open | Blinking` (driven by a jittered timer).
//! - `breath_phase` — continuous, always on, drives subtle bob and color pulse.
//!
//! Plus name, last surfacing, and a [`VolitionGauge`] that mirrors the energy
//! topology (generative vs consumptive, hot desires vs cold obligations) Casey
//! brought over from production Letta. The gauge is *displayed* here as embodied
//! state (posture, warmth); the numbers themselves live in the agent's memfs.
//!
//! ## Event subscriptions
//!
//! `AgentSelected` → swap subject.
//! `MoodChanged` → coarse posture hint (placeholder until C2 wires real signals).
//! `Surfacing` → record + brief affectionate flash, subconscious_active = true.
//! `PressureChanged` / `CompactionWarning` → `Yawning` at tier 2/3.
//! `InferenceStrain` *(future)* → `Straining`.
//! `Tick` → drive breath phase, blink timer.
//!
//! ## Visual tiers
//!
//! - **C1 (this commit)** — placeholder card, parity with the previous buddy
//!   visuals so the diff is pure shape. No portrait yet.
//! - **C2** — Tier 1 hand-crafted half-block portrait of Annie, seven visible
//!   states driven by `Posture` + `Eye` + `breath_phase`.
//! - **C4** — Tier 2 image protocol (kitty/sixel) via `ratatui-image`, with
//!   Tier 1 as automatic fallback. Per-agent portraits loaded from agent memfs
//!   `assets/` (outside `system/`, which is pinned into context).
//!
//! ## What this module is *not*
//!
//! - Not a "buddy." That name is reserved for a future agent-pet system (a
//!   lower-cognition long-running secondary entity an agent can care for — a
//!   real `buddy`, in the sense the word usually means).
//! - Not a dashboard. Numbers belong in the cockpit. Sensation belongs here.
//! - Not a separate avatar for Aster. Same face, different state.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::ui::animation::{Animator, colors};
use crate::ui::component::TuiEvent;

// ── State ───────────────────────────────────────────────────────

/// What Annie's posture is doing right now. Mutually exclusive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Posture {
    /// At rest. Slack, soft breath, default eye behavior.
    Idle,
    /// Tool call or inference active. Gaze fixed, no blinking, brighter border.
    Processing,
    /// Warm tag from conversation context. Tint shifts, soft glow.
    Affectionate,
    /// Inference strain — retry, backoff, hoarse model voice. Droop, desaturate.
    Straining,
    /// Context pressure tier 2/3. The embodied "three warnings" — yawn, lean.
    Yawning,
}

/// Transient overlay on top of [`Posture`]. A blink lasts ~6 ticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Eye {
    Open,
    Blinking,
}

/// Where the overlay sits on screen when rendered as a corner card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Mirror of the agent's energy topology (volition system).
///
/// Numbers live in the agent's memfs (`system/dynamic/energy-balance.md`);
/// this struct is just a snapshot the Presence consults to color the portrait.
/// All-consumptive Annie looks subtly slack; all-generative Annie looks subtly wired.
#[derive(Debug, Clone, Default)]
pub struct VolitionGauge {
    pub generative: u32,
    pub consumptive: u32,
    pub hot_desires: u32,
    pub cold_obligations: u32,
}

impl VolitionGauge {
    /// −1.0 (all consumptive) → +1.0 (all generative). 0.0 when empty.
    pub fn balance(&self) -> f32 {
        let total = self.generative + self.consumptive;
        if total == 0 {
            0.0
        } else {
            (self.generative as f32 - self.consumptive as f32) / total as f32
        }
    }
}

/// The Presence — Annie's felt-state in the TUI.
///
/// Updated via [`Presence::handle_event`] in response to `TuiEvent`s; never polled.
/// Rendered via [`draw_overlay`] / [`draw_welcome`] as a corner card today, with
/// half-block portraits arriving in C2.
pub struct Presence {
    pub name: String,
    pub posture: Posture,
    pub eye: Eye,
    pub mood: String,
    pub energy: u8,
    pub subconscious_active: bool,
    pub last_surfacing: Option<String>,
    pub volition: VolitionGauge,
    pub position: Position,
    pub visible: bool,
    pub animator: Animator,
    /// Most recent tick observed. Drives blink/breath timing.
    tick: u64,
    /// Tick at which the next blink should begin.
    next_blink_at: u64,
    /// Tick at which the current blink should end (Eye::Open resumes).
    blink_until: u64,
}

impl Presence {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            posture: Posture::Idle,
            eye: Eye::Open,
            mood: "Idle".to_string(),
            energy: 50,
            subconscious_active: false,
            last_surfacing: None,
            volition: VolitionGauge::default(),
            position: Position::TopRight,
            visible: true,
            animator: Animator::new(),
            tick: 0,
            next_blink_at: 180, // ~3 s at 60 Hz
            blink_until: 0,
        }
    }

    pub fn set_position(&mut self, p: Position) {
        self.position = p;
    }

    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    /// Dispatch a single `TuiEvent`. Returns true if a redraw is needed.
    /// Ticks return false on purpose so we don't force-redraw every frame.
    pub fn handle_event(&mut self, event: &TuiEvent) -> bool {
        match event {
            TuiEvent::AgentSelected(name) => {
                self.name = name.clone();
                true
            }
            TuiEvent::MoodChanged(mood) => {
                self.mood = mood.clone();
                self.posture = match mood.as_str() {
                    "Straining" => Posture::Straining,
                    "Yawning" => Posture::Yawning,
                    "Processing" => Posture::Processing,
                    "Affectionate" => Posture::Affectionate,
                    _ => Posture::Idle,
                };
                true
            }
            TuiEvent::EnergyChanged(e) => {
                self.energy = *e;
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
            TuiEvent::PressureChanged(p) => {
                if *p >= 0.85 {
                    self.posture = Posture::Yawning;
                }
                true
            }
            TuiEvent::CompactionWarning { pressure, .. } => {
                if *pressure >= 0.85 {
                    self.posture = Posture::Yawning;
                }
                true
            }
            TuiEvent::Tick(t) => {
                self.tick = *t;
                if self.eye == Eye::Blinking && *t >= self.blink_until {
                    self.eye = Eye::Open;
                    let jitter = (*t % 60) as u64;
                    self.next_blink_at = *t + 180 + jitter;
                } else if self.eye == Eye::Open && *t >= self.next_blink_at {
                    self.eye = Eye::Blinking;
                    self.blink_until = *t + 6;
                }
                false
            }
            _ => false,
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────
// Placeholder card for C1 — parity with the previous buddy visuals so the
// diff is shape-only. C2 replaces this with a hand-crafted half-block portrait.

/// Draw the presence as a small overlay card in a corner of `area`.
pub fn draw_overlay(frame: &mut Frame, p: &Presence, area: Rect) {
    if !p.visible {
        return;
    }

    let (x, y, width) = match p.position {
        Position::TopLeft => (area.x + 1, area.y + 1, 20),
        Position::TopRight => (area.x + area.width.saturating_sub(21), area.y + 1, 20),
        Position::BottomLeft => (area.x + 1, area.y + area.height.saturating_sub(6), 20),
        Position::BottomRight => (
            area.x + area.width.saturating_sub(21),
            area.y + area.height.saturating_sub(6),
            20,
        ),
    };

    let card_area = Rect {
        x,
        y,
        width: width.min(area.width.saturating_sub(2)),
        height: 5,
    };

    let energy_color = if p.energy > 70 {
        colors::ANI_PRIMARY
    } else if p.energy > 40 {
        colors::ANI_SECONDARY
    } else {
        colors::ANI_DIM
    };

    let content = vec![
        Line::from(Span::styled(
            format!("  ◈ {}  ", p.name),
            Style::default()
                .fg(energy_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  Mood: {}  ", p.mood),
            Style::default().fg(Color::Gray),
        )),
        Line::from(vec![
            Span::styled("  Energy: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "▰".repeat((p.energy / 10) as usize),
                Style::default().fg(energy_color),
            ),
            Span::styled(
                "▱".repeat(10 - (p.energy / 10) as usize),
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

    let para = Paragraph::new(content)
        .block(block)
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    frame.render_widget(para, card_area);

    if p.subconscious_active {
        let sub_area = Rect {
            x: card_area.x,
            y: card_area.y + card_area.height,
            width: card_area.width,
            height: 1,
        };
        let sub = Paragraph::new(Line::from(vec![
            Span::styled("  ◈ ", Style::default().fg(colors::SUBCONSCIOUS)),
            Span::styled(
                "Subconscious Active",
                Style::default()
                    .fg(colors::SUBCONSCIOUS)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        frame.render_widget(sub, sub_area);
    }

    if let Some(s) = &p.last_surfacing {
        let surf_area = Rect {
            x: card_area.x,
            y: card_area.y + card_area.height + 1,
            width: card_area.width.min(30),
            height: 2,
        };
        let truncated = truncate(s, 25);
        let surf = Paragraph::new(vec![
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
                Span::styled(truncated, Style::default().fg(Color::Gray)),
            ]),
        ])
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));
        frame.render_widget(surf, surf_area);
    }
}

/// Draw the presence on the welcome screen with agent selection state.
pub fn draw_welcome(
    frame: &mut Frame,
    p: &Presence,
    area: Rect,
    selected_agent: Option<&str>,
) {
    if !p.visible {
        return;
    }

    let card_area = Rect {
        x: area.x + area.width.saturating_sub(22),
        y: area.y + 1,
        width: 20,
        height: 8,
    };

    let mut content = vec![
        Line::from(Span::styled(
            "  ◈ COMPANION  ",
            Style::default()
                .fg(colors::ANI_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )),
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
                Span::styled("Ready", Style::default().fg(Color::Green)),
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
                Span::styled(agent_name, Style::default().fg(colors::ANI_SECONDARY)),
            ]),
        ]);
    } else {
        content.extend(vec![
            Line::from(Span::styled(
                "  No agent selected",
                Style::default().fg(Color::DarkGray),
            )),
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

    let para = Paragraph::new(content)
        .block(block)
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    frame.render_widget(para, card_area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let p = Presence::new("Annie");
        assert_eq!(p.name, "Annie");
        assert_eq!(p.posture, Posture::Idle);
        assert_eq!(p.eye, Eye::Open);
        assert!(p.visible);
        assert!(!p.subconscious_active);
    }

    #[test]
    fn agent_selected_renames() {
        let mut p = Presence::new("Annie");
        let dirty = p.handle_event(&TuiEvent::AgentSelected("HAL".to_string()));
        assert!(dirty);
        assert_eq!(p.name, "HAL");
    }

    #[test]
    fn mood_maps_to_posture() {
        let mut p = Presence::new("Annie");
        p.handle_event(&TuiEvent::MoodChanged("Straining".to_string()));
        assert_eq!(p.posture, Posture::Straining);
        p.handle_event(&TuiEvent::MoodChanged("Processing".to_string()));
        assert_eq!(p.posture, Posture::Processing);
        p.handle_event(&TuiEvent::MoodChanged("Idle".to_string()));
        assert_eq!(p.posture, Posture::Idle);
    }

    #[test]
    fn surfacing_records_and_marks_subconscious() {
        let mut p = Presence::new("Annie");
        let dirty = p.handle_event(&TuiEvent::Surfacing {
            source: "aster".into(),
            content: "hello from beneath".into(),
            priority: "low".into(),
        });
        assert!(dirty);
        assert_eq!(p.last_surfacing.as_deref(), Some("hello from beneath"));
        assert!(p.subconscious_active);
    }

    #[test]
    fn pressure_triggers_yawn_at_threshold() {
        let mut p = Presence::new("Annie");
        p.handle_event(&TuiEvent::PressureChanged(0.5));
        assert_eq!(p.posture, Posture::Idle);
        p.handle_event(&TuiEvent::PressureChanged(0.9));
        assert_eq!(p.posture, Posture::Yawning);
    }

    #[test]
    fn blink_cycle() {
        let mut p = Presence::new("Annie");
        assert_eq!(p.eye, Eye::Open);
        // Drive past first scheduled blink.
        p.handle_event(&TuiEvent::Tick(200));
        assert_eq!(p.eye, Eye::Blinking);
        // Drive past blink end.
        p.handle_event(&TuiEvent::Tick(220));
        assert_eq!(p.eye, Eye::Open);
    }

    #[test]
    fn volition_balance() {
        let mut g = VolitionGauge::default();
        assert_eq!(g.balance(), 0.0);
        g.generative = 3;
        g.consumptive = 1;
        assert!((g.balance() - 0.5).abs() < 1e-6);
        g.generative = 0;
        g.consumptive = 4;
        assert!((g.balance() - -1.0).abs() < 1e-6);
    }

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("hi", 10), "hi");
        assert_eq!(truncate("hello world", 8), "hello w…");
    }
}
