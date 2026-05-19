//! HealthPane — a live vitals readout for the agent's substrate.
//!
//! Where the CockpitPane shows what the subconscious *says*, the HealthPane
//! shows how the substrate *is*: context pressure, backend connectivity, the
//! cadence of the N+1/N+25/N+100 passes, inference strain (the 504s Casey
//! kept seeing), and the agent's own energy and mood.
//!
//! ```text
//! ╭─ Health · 02:14 up ─────╮
//! │ context ███████░░░  73% │
//! │ backend ● local         │
//! │ N+1     48 · 12 surfaced│
//! │ N+25    refl 14:35      │
//! │ N+100   arch 13:02      │
//! │ compact urgent · 14:40  │
//! │ strain  504×3 · 429×1   │
//! │ energy  72 · focused    │
//! ╰─────────────────────────╯
//! ```
//!
//! Every value comes from a `TuiEvent` the TUI already emits — the pane is
//! pure accumulation, no new backend wiring.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    layout::Rect,
    Frame,
};
use std::time::Instant;

use super::chat::ChatPalette;
use super::component::{Component, TuiEvent};

pub struct HealthPane {
    /// When the pane came up — drives the uptime readout.
    started: Instant,

    /// Latest context pressure (0.0–1.0).
    pressure: f32,
    /// Last compaction warning: (tier, "HH:MM").
    last_compaction: Option<(u8, String)>,

    /// Whether an N+1 subconscious pass is in flight right now.
    n1_active: bool,
    /// Completed N+1 passes this session.
    n1_passes: u32,
    /// Surfacings promoted to the cockpit this session.
    surfacings: u32,
    /// Last N+25 reflection time.
    last_reflection: Option<String>,
    /// Last N+100 archivist synthesis time.
    last_archivist: Option<String>,

    /// Inference strain tallies — 504s are the timeout Casey watches for.
    strain_504: u32,
    strain_429: u32,
    strain_other: u32,

    /// Backend connectivity.
    backend_mode: String,
    backend_healthy: bool,

    /// Agent felt-state.
    energy: u8,
    mood: String,

    palette: ChatPalette,
}

impl HealthPane {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            pressure: 0.0,
            last_compaction: None,
            n1_active: false,
            n1_passes: 0,
            surfacings: 0,
            last_reflection: None,
            last_archivist: None,
            strain_504: 0,
            strain_429: 0,
            strain_other: 0,
            backend_mode: "—".to_string(),
            backend_healthy: false,
            energy: 0,
            mood: "—".to_string(),
            palette: ChatPalette::default(),
        }
    }

    fn timestamp_now() -> String {
        chrono::Local::now().format("%H:%M").to_string()
    }

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
            _ => return,
        };
        self.palette = ChatPalette::from_atmosphere(atm);
    }

    /// Colour for the current pressure, tiered like the compaction warnings.
    fn pressure_color(&self) -> Color {
        if self.pressure >= 0.95 {
            Color::Rgb(220, 100, 100) // critical
        } else if self.pressure >= 0.90 {
            Color::Rgb(230, 150, 80) // urgent
        } else if self.pressure >= 0.80 {
            Color::Rgb(220, 200, 110) // warn
        } else {
            Color::Rgb(120, 200, 120) // easy
        }
    }

    /// A 10-cell block bar for a 0.0–1.0 ratio.
    fn bar(ratio: f32, cells: usize) -> String {
        let ratio = ratio.clamp(0.0, 1.0);
        let filled = (ratio * cells as f32).round() as usize;
        let filled = filled.min(cells);
        let mut s = String::with_capacity(cells);
        for i in 0..cells {
            s.push(if i < filled { '█' } else { '░' });
        }
        s
    }

    /// One `label  value` row with a dim label and a coloured value.
    fn row(&self, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!(" {:<8}", label),
            Style::default().fg(self.palette.agent_dim),
        )];
        spans.extend(value);
        Line::from(spans)
    }

    fn build_lines(&self) -> Vec<Line<'static>> {
        let p = &self.palette;
        let dim = Style::default().fg(p.agent_dim);
        let mut lines: Vec<Line> = Vec::new();

        // ── context pressure ──────────────────────────────────────────
        let pc = self.pressure_color();
        let pct = (self.pressure * 100.0).round() as u16;
        lines.push(self.row("context", vec![
            Span::styled(Self::bar(self.pressure, 10), Style::default().fg(pc)),
            Span::styled(format!("  {pct:>3}%"), Style::default().fg(pc).add_modifier(Modifier::BOLD)),
        ]));

        // ── backend ───────────────────────────────────────────────────
        let (dot, dot_c) = if self.backend_healthy {
            ("●", Color::Rgb(120, 200, 120))
        } else {
            ("○", Color::Rgb(200, 120, 100))
        };
        lines.push(self.row("backend", vec![
            Span::styled(format!("{dot} "), Style::default().fg(dot_c)),
            Span::styled(self.backend_mode.clone(), Style::default().fg(p.agent_primary)),
        ]));

        lines.push(Line::from(""));

        // ── N+1 subconscious ──────────────────────────────────────────
        let n1_mark = if self.n1_active { "◌ active" } else { "idle" };
        lines.push(self.row("N+1", vec![
            Span::styled(
                format!("{} pass · {}", self.n1_passes, n1_mark),
                Style::default().fg(if self.n1_active { p.surfacing } else { p.agent_primary }),
            ),
        ]));
        lines.push(self.row("◈ surf", vec![
            Span::styled(format!("{} surfaced", self.surfacings), Style::default().fg(p.surfacing)),
        ]));

        // ── N+25 reflection ───────────────────────────────────────────
        lines.push(self.row("◎ N+25", vec![match &self.last_reflection {
            Some(t) => Span::styled(t.clone(), Style::default().fg(p.reflection)),
            None => Span::styled("—", dim),
        }]));

        // ── N+100 archivist ───────────────────────────────────────────
        lines.push(self.row("◉ N+100", vec![match &self.last_archivist {
            Some(t) => Span::styled(t.clone(), Style::default().fg(p.archivist)),
            None => Span::styled("—", dim),
        }]));

        // ── last compaction warning ───────────────────────────────────
        lines.push(self.row("⚠ compact", vec![match &self.last_compaction {
            Some((tier, t)) => {
                let label = match tier {
                    3 => "critical",
                    2 => "urgent",
                    _ => "warn",
                };
                Span::styled(format!("{label} · {t}"), Style::default().fg(p.compaction))
            }
            None => Span::styled("none", dim),
        }]));

        lines.push(Line::from(""));

        // ── inference strain ──────────────────────────────────────────
        let strain_spans: Vec<Span> = if self.strain_504 + self.strain_429 + self.strain_other == 0 {
            vec![Span::styled("clear", Style::default().fg(Color::Rgb(120, 200, 120)))]
        } else {
            let mut v = Vec::new();
            if self.strain_504 > 0 {
                v.push(Span::styled(
                    format!("504×{}", self.strain_504),
                    Style::default().fg(Color::Rgb(220, 100, 100)).add_modifier(Modifier::BOLD),
                ));
            }
            if self.strain_429 > 0 {
                if !v.is_empty() { v.push(Span::styled(" · ", dim)); }
                v.push(Span::styled(format!("429×{}", self.strain_429), Style::default().fg(p.compaction)));
            }
            if self.strain_other > 0 {
                if !v.is_empty() { v.push(Span::styled(" · ", dim)); }
                v.push(Span::styled(format!("err×{}", self.strain_other), Style::default().fg(p.compaction)));
            }
            v
        };
        lines.push(self.row("strain", strain_spans));

        // ── energy + mood ─────────────────────────────────────────────
        lines.push(self.row("energy", vec![
            Span::styled(Self::bar(self.energy as f32 / 100.0, 5), Style::default().fg(p.agent_primary)),
            Span::styled(format!("  {}", self.energy), Style::default().fg(p.agent_primary)),
        ]));
        lines.push(self.row("mood", vec![
            Span::styled(self.mood.clone(), Style::default().fg(p.agent_primary).add_modifier(Modifier::ITALIC)),
        ]));

        lines
    }

    fn uptime(&self) -> String {
        let secs = self.started.elapsed().as_secs();
        let (h, m) = (secs / 3600, (secs % 3600) / 60);
        if h > 0 {
            format!("{h}:{m:02} up")
        } else {
            format!("{m}m up")
        }
    }
}

impl Component for HealthPane {
    fn name(&self) -> &str {
        "health"
    }

    fn handle_event(&mut self, event: &TuiEvent) -> bool {
        match event {
            TuiEvent::PressureChanged(p, _limit) => { self.pressure = *p; true }
            TuiEvent::CompactionWarning { pressure, tier } => {
                self.pressure = *pressure;
                self.last_compaction = Some((*tier, Self::timestamp_now()));
                true
            }
            TuiEvent::Archivist { pressure, .. } => {
                self.pressure = *pressure;
                self.last_archivist = Some(Self::timestamp_now());
                true
            }
            TuiEvent::Reflection { .. } => {
                self.last_reflection = Some(Self::timestamp_now());
                true
            }
            TuiEvent::Surfacing { .. } => { self.surfacings += 1; true }
            TuiEvent::SubconsciousPass(active) => {
                if *active {
                    self.n1_active = true;
                } else if self.n1_active {
                    self.n1_active = false;
                    self.n1_passes += 1;
                }
                true
            }
            TuiEvent::InferenceStrain { status, .. } => {
                match *status {
                    504 => self.strain_504 += 1,
                    429 => self.strain_429 += 1,
                    _ => self.strain_other += 1,
                }
                true
            }
            TuiEvent::BackendStatus { mode, healthy } => {
                self.backend_mode = mode.clone();
                self.backend_healthy = *healthy;
                true
            }
            TuiEvent::EnergyChanged(e) => { self.energy = *e; true }
            TuiEvent::MoodChanged(m) => { self.mood = m.clone(); true }
            TuiEvent::AtmosphereChanged(name) => { self.set_palette_from_name(name); true }
            // Redraw once a minute so the uptime clock advances.
            TuiEvent::Tick(t) => *t % 60 == 0,
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
            .border_style(Style::default().fg(self.palette.agent_primary).add_modifier(Modifier::DIM))
            .title(Span::styled(
                format!(" Health · {} ", self.uptime()),
                Style::default().fg(self.palette.agent_primary).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width < 6 || inner.height < 1 {
            return;
        }
        let lines = self.build_lines();
        // Clip to the available height — vitals are most-important-first.
        let visible: Vec<Line> = lines.into_iter().take(inner.height as usize).collect();
        frame.render_widget(Paragraph::new(visible), inner);
    }
}
