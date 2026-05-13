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
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use std::path::Path;

use crate::ui::animation::{Animator, colors};
use crate::ui::component::TuiEvent;
use crate::ui::portrait;

// ── State ───────────────────────────────────────────────────────

/// What Annie's posture is doing right now. Mutually exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
/// Rendered via [`draw_overlay`] as a corner card on Dashboard; the Welcome
/// screen draws its centered portrait inline in `App::draw_welcome` so it can
/// participate in the welcome vertical layout (title → avatar → menu → footer).
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
    /// Optional per-agent portrait loaded from `assets/portrait.{png,jpg}`
    /// in the agent's memfs. When `None`, the renderer falls back to the
    /// hand-crafted Annie palette grid (Tier 1).
    pub portrait_source: Option<()>,
    /// Current atmospheric colour preset. Drives border, title, and accent
    /// colours across all screens. Defaults to a posture-linked preset when
    /// none is explicitly set via `BackendEvent::Atmosphere`.
    pub atmosphere: crate::ui::atmosphere::Atmosphere,
    /// Most recent tick observed. Drives blink/breath timing.
    tick: u64,
    /// Tick at which the next blink should begin.
    next_blink_at: u64,
    /// Tick at which the current blink should end (Eye::Open resumes).
    blink_until: u64,
    /// `true` while the breath interim frame is showing (~2 s every 8-15 s).
    pub is_breathing: bool,
    /// Tick at which the next breath should begin.
    next_breath_at: u64,
    /// Tick at which the current breath frame should end.
    breath_until: u64,
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
            position: Position::BottomRight,
            visible: true,
            animator: Animator::new(),
            portrait_source: None,
            atmosphere: crate::ui::atmosphere::Atmosphere::default(),
            tick: 0,
            next_blink_at: 180, // ~3 s at 60 Hz
            blink_until: 0,
            is_breathing: false,
            next_breath_at: 480, // ~8 s at 60 Hz
            breath_until: 0,
        }
    }

    /// Try to load a portrait from a path (stub — future use).
    pub fn load_portrait<P: AsRef<Path>>(&mut self, _path: P) {
    }

    /// Sync the active atmosphere from the current posture. Called whenever
    /// `posture` changes — keeps the UI chrome in step with Annie's state.
    fn sync_atmosphere(&mut self) {
        self.atmosphere = crate::ui::atmosphere::Atmosphere::from_posture(self.posture);
    }

    /// Attempt to load a portrait from an agent's memfs root (stub — future use).
    pub fn load_portrait_from_memfs<P: AsRef<Path>>(&mut self, _memfs_root: P) {
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
                self.sync_atmosphere();
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
                    self.sync_atmosphere();
                }
                true
            }
            TuiEvent::CompactionWarning { pressure, .. } => {
                if *pressure >= 0.85 {
                    self.posture = Posture::Yawning;
                    self.sync_atmosphere();
                }
                true
            }
            TuiEvent::InferenceStrain { .. } => {
                // The voice is hoarse — drop into strain posture immediately.
                // Posture will tick back to Idle once a Mood/EnergyChanged
                // event arrives from the next successful round.
                self.posture = Posture::Straining;
                self.sync_atmosphere();
                true
            }
            TuiEvent::AtmosphereChanged(name) => {
                // Parse the preset name from the agent's structured event.
                // Falls back to posture-linked default on unrecognised names.
                use crate::ui::atmosphere::Atmosphere;
                match name.to_lowercase().replace(' ', "_").as_str() {
                    "mint_tea" => self.atmosphere = Atmosphere::MintTea,
                    "therapeutic_blue" => self.atmosphere = Atmosphere::TherapeuticBlue,
                    "lavender_calm" => self.atmosphere = Atmosphere::LavenderCalm,
                    "warm_amber" => self.atmosphere = Atmosphere::WarmAmber,
                    "peach_sunset" => self.atmosphere = Atmosphere::PeachSunset,
                    "autumn_browns" => self.atmosphere = Atmosphere::AutumnBrowns,
                    "neon_glow" => self.atmosphere = Atmosphere::NeonGlow,
                    "aurora_borealis" => self.atmosphere = Atmosphere::AuroraBorealis,
                    "cherry_blossom" => self.atmosphere = Atmosphere::CherryBlossom,
                    "ocean_depths" => self.atmosphere = Atmosphere::OceanDepths,
                    "midnight_galaxy" => self.atmosphere = Atmosphere::MidnightGalaxy,
                    "twilight_mist" => self.atmosphere = Atmosphere::TwilightMist,
                    "forest_greens" => self.atmosphere = Atmosphere::ForestGreens,
                    _ => self.sync_atmosphere(), // fall back to posture-linked
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
                // Breath: 8-15 s jittered interval, 2 s hold (Godot parity).
                if self.is_breathing && *t >= self.breath_until {
                    self.is_breathing = false;
                    let jitter = (*t % 120) as u64;
                    self.next_breath_at = *t + 480 + jitter; // 8-15 s
                } else if !self.is_breathing && *t >= self.next_breath_at {
                    self.is_breathing = true;
                    self.breath_until = *t + 120; // 2 s hold
                }
                false
            }
            _ => false,
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────
//
// Tier 1: hand-crafted half-block portrait of Annie via `portrait::render`.
// The card layout is portrait (top) + name strip (bottom), with a rounded
// border whose color reflects the current Posture. State changes are
// expressed through the portrait itself — eyes blink/close, mouth opens on
// yawn, palette desaturates on strain, colors warm on affection — not
// through gauges.

const CARD_W: u16 = portrait::RENDER_W + 2; // portrait + border
const CARD_H: u16 = portrait::RENDER_H + 3; // portrait + name row + border

/// Border color derived from current posture. Subtle, not loud.
/// Falls back to the atmosphere primary colour when the posture doesn't
/// specify an override — keeps the chrome in sync with the ambient preset.
fn posture_border(p: &Presence) -> Color {
    let atm = p.atmosphere;
    match p.posture {
        Posture::Processing => atm.primary(),
        Posture::Affectionate => Color::Rgb(220, 150, 170),
        Posture::Straining => Color::Rgb(140, 100, 100),
        Posture::Yawning => Color::Rgb(160, 145, 130),
        Posture::Idle => atm.dim(),
    }
}

/// Draw the presence as a portrait card in a corner of `area`.
pub fn draw_overlay(frame: &mut Frame, p: &Presence, area: Rect) {
    if !p.visible || area.width < CARD_W || area.height < CARD_H {
        return;
    }

    let (x, y) = match p.position {
        Position::TopLeft => (area.x + 1, area.y + 1),
        Position::TopRight => (area.x + area.width.saturating_sub(CARD_W + 1), area.y + 1),
        Position::BottomLeft => (
            area.x + 1,
            area.y + area.height.saturating_sub(CARD_H + 1),
        ),
        Position::BottomRight => (
            area.x + area.width.saturating_sub(CARD_W + 1),
            area.y + area.height.saturating_sub(CARD_H + 1),
        ),
    };

    let card_area = Rect {
        x,
        y,
        width: CARD_W,
        height: CARD_H,
    };

    let border = posture_border(p);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).add_modifier(Modifier::DIM));
    frame.render_widget(block, card_area);

    let portrait_area = Rect {
        x: card_area.x + 1,
        y: card_area.y + 1,
        width: portrait::RENDER_W,
        height: portrait::RENDER_H,
    };
    portrait::render(frame.buffer_mut(), portrait_area, p);

    let name_area = Rect {
        x: card_area.x + 1,
        y: card_area.y + 1 + portrait::RENDER_H,
        width: card_area.width.saturating_sub(2),
        height: 1,
    };
    let glyph = if p.subconscious_active { "◈" } else { "·" };
    let name_line = Line::from(vec![
        Span::styled(format!(" {} ", glyph), Style::default().fg(border)),
        Span::styled(
            p.name.clone(),
            Style::default().fg(border).add_modifier(Modifier::BOLD),
        ),
    ]);
    let name_para = Paragraph::new(name_line).alignment(Alignment::Center);
    frame.render_widget(name_para, name_area);
}

// Welcome rendering moved inline into `App::draw_welcome` (src/ui/app.rs)
// so the avatar can be centered within the welcome layout instead of pinned
// to a corner.

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
    fn posture_border_changes_with_state() {
        // Just confirms posture maps to distinct colors for the four
        // "interesting" states; idle stays as the dim default.
        let idle = posture_border(Posture::Idle);
        let processing = posture_border(Posture::Processing);
        let affectionate = posture_border(Posture::Affectionate);
        let straining = posture_border(Posture::Straining);
        let yawning = posture_border(Posture::Yawning);
        assert_ne!(idle, processing);
        assert_ne!(idle, affectionate);
        assert_ne!(idle, straining);
        assert_ne!(idle, yawning);
    }
}
