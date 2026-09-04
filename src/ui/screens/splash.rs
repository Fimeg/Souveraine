#![allow(dead_code)] // WIP scaffolding not yet wired
//! Splash screen — the first thing shown on launch.
//!
//! A procedural bloom flower animates while the "S O U V E R A I N E" title
//! breathes below it. A progress bar fills at the bottom. Any key press skips
//! to the welcome screen. Auto-transitions after ~7 seconds.
//!
//! Ported from `src/ui/animation.rs` bloom module (ratatui → tuie).

use std::cell::Cell;
use std::rc::Rc;

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;

// ── Bloom state ────────────────────────────────────────────────────────────────

const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789@#$%&*";
const SPIKES: &[char] = &['▲', '△', '⤴', '⤵', '➚', '➘', '✸', '✦', '⬆', '⬇'];

#[derive(Clone, Copy)]
pub struct BloomState {
    progress: f32,
    mode: u8,
    mode_timer: f32,
    flash: f32,
    variant: u8,
}

impl BloomState {
    pub fn new() -> Self {
        Self {
            progress: 0.0,
            mode: 0,
            mode_timer: 0.0,
            flash: 0.0,
            variant: 0,
        }
    }

    pub fn advance(&mut self, dt: f32) {
        self.progress = (self.progress + dt * 0.25).min(1.0);
        self.flash *= 0.92;
        self.mode_timer += dt;
        if self.mode_timer > 1.8 {
            self.mode_timer = 0.0;
            self.mode = (self.mode + 1) % 3;
            self.flash = 1.0;
            self.variant = (self.variant + 1) % 3;
        }
    }
}

// ── SplashScreen widget ────────────────────────────────────────────────────────

pub struct SplashScreen {
    /// Delegate — provides layout management.
    text: Box<Text>,
    /// Bloom animation state.
    bloom: Cell<BloomState>,
    /// Frame counter (incremented each render).
    tick: Cell<u64>,
    /// Set when user presses any key.
    skip_pressed: Cell<bool>,
    /// Shared flag with TuieApp — set to true when splash is done.
    complete: Rc<Cell<bool>>,
    /// Palette for title colors.
    palette: ChatPalette,
}

impl DelegateWidget for SplashScreen {
    tuie::delegate_widget!(text);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        // Any key press skips the splash.
        if queue.peek().is_some() {
            self.skip_pressed.set(true);
            self.complete.set(true);
            // Drain the queue so the key doesn't leak to the next screen.
            while queue.next().is_some() {}
            return InputResult::Handled;
        }
        InputResult::Rejected
    }

    fn override_render(&self, mut ctx: RenderContext) {
        // Advance animation state.
        // dt=0.02 at ~30fps gives ~6.5s to full bloom — dramatic but not sluggish.
        let mut bloom = self.bloom.get();
        bloom.advance(0.02);
        self.bloom.set(bloom);
        let tick = self.tick.get().wrapping_add(1);
        self.tick.set(tick);

        // Signal completion when the bloom finishes.
        if bloom.progress >= 1.0 {
            self.complete.set(true);
        }

        // Fill background.
        ctx.set_style(Style::new().bg(Color::BLACK));
        ctx.clear();

        let w = ctx.physical_size.x;
        let h = ctx.physical_size.y;

        if w >= 20 && h >= 10 {
            render_bloom(&mut ctx, w, h, &bloom, tick);
        }

        // Title overlay — appears once bloom is past 25%.
        if bloom.progress > 0.25 {
            let alpha = ((bloom.progress - 0.25) / 0.35).min(1.0);
            let breathe = ((tick as f32 * 0.04).sin() * 0.5 + 0.5) * 0.15 + 0.85;
            let primary = theme::to_tuie_color(self.palette.agent_primary);
            let (tr, tg, tb) = match primary {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (255u8, 140, 66),
            };

            let title_r = (tr as f32 * alpha * breathe) as u8;
            let title_g = (tg as f32 * alpha * breathe * 0.6) as u8;
            let title_b = (tb as f32 * alpha * breathe * 0.4) as u8;
            let title_color = Color::Rgb(title_r, title_g, title_b);

            let tagline_alpha = (180.0 * alpha) as u8;
            let tagline_color =
                Color::Rgb(tagline_alpha, (120.0 * alpha) as u8, (80.0 * alpha) as u8);

            let title_text = "S O U V E R A I N E";
            let tagline = "La souveraineté de la conscience";

            // Center horizontally.
            let title_x = (w.saturating_sub(title_text.len() as u16)) / 2;
            let title_y = h.saturating_sub(8).min(h.saturating_sub(4));

            if title_y < h {
                ctx.move_to(Vec2::new(title_x as i32, title_y as i32));
                ctx.set_style(Style::new().fg(title_color).bold().bg(Color::BLACK));
                ctx.write(title_text);

                ctx.move_to(Vec2::new(
                    (w.saturating_sub(tagline.len() as u16) / 2) as i32,
                    (title_y + 1) as i32,
                ));
                ctx.set_style(Style::new().fg(tagline_color).italic().bg(Color::BLACK));
                ctx.write(tagline);
            }

            // "press any key to skip" — fades in after 80% progress.
            if bloom.progress > 0.8 {
                let skip_alpha = ((bloom.progress - 0.8) / 0.2).min(1.0);
                let skip_text = "press any key to skip";
                let skip_color = Color::Rgb(
                    (100.0 * skip_alpha) as u8,
                    (100.0 * skip_alpha) as u8,
                    (100.0 * skip_alpha) as u8,
                );
                let skip_y = (title_y + 2).min(h.saturating_sub(1));
                ctx.move_to(Vec2::new(
                    (w.saturating_sub(skip_text.len() as u16) / 2) as i32,
                    skip_y as i32,
                ));
                ctx.set_style(Style::new().fg(skip_color).bg(Color::BLACK));
                ctx.write(skip_text);
            }
        }

        // Progress bar at the very bottom.
        {
            let bar_y = h.saturating_sub(2);
            let bar_w = 30u16.min(w.saturating_sub(4));
            let bar_x = (w.saturating_sub(bar_w)) / 2;
            let pct = (bloom.progress * 100.0) as u16;

            let filled = (bar_w as f32 * bloom.progress) as u16;
            let empty = bar_w.saturating_sub(filled);

            ctx.move_to(Vec2::new(bar_x as i32, bar_y as i32));
            ctx.set_style(Style::new().fg(Color::Rgb(200, 130, 160)).bg(Color::BLACK));

            let bar_text = format!(
                "{}{} {:>3}%",
                "▰".repeat(filled as usize),
                "▱".repeat(empty as usize),
                pct,
            );
            ctx.write(&bar_text);
        }
    }
}

impl SplashScreen {
    pub fn new(palette: &ChatPalette, complete: Rc<Cell<bool>>) -> Box<Self> {
        let mut text = Text::new();
        text.set_min_height(Some(10));
        text.set_min_width(Some(20));

        Box::new(Self {
            text,
            bloom: Cell::new(BloomState::new()),
            tick: Cell::new(0),
            skip_pressed: Cell::new(false),
            palette: *palette,
            complete,
        })
    }

    /// Whether the splash has finished (progress complete or user skipped).
    pub fn is_complete(&self) -> bool {
        self.skip_pressed.get() || self.bloom.get().progress >= 1.0
    }
}

// ── Bloom renderer (ported from animation.rs) ──────────────────────────────────

fn render_bloom(ctx: &mut RenderContext, w: u16, h: u16, state: &BloomState, tick: u64) {
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 3.8;
    let max_r = (cx.min(cy * 2.0) * 0.65).min(26.0);
    let bloom = ease_out_back(state.progress);
    let t = tick as f32 * 0.08;
    let flash = state.flash;

    // ── Stem ──
    if bloom > 0.05 {
        let stem_progress = ((bloom - 0.05) / 0.5).min(1.0);
        let stem_len = h as f32 * 0.28;
        let stem_top = cy + max_r * 0.3;
        let stem_visible = stem_len * stem_progress;
        let stem_bot = (stem_top + stem_visible) as u16;

        for y in (stem_top as u16)..stem_bot.min(h) {
            let tt = (y as f32 - stem_top) / stem_len;
            let curve = (tt * std::f32::consts::PI * 0.3).sin() * 6.0
                + (tt * std::f32::consts::PI * 0.8).sin() * 2.0;
            let col = (cx + curve) as u16;
            if col < w {
                let stem_shade = (40.0 + (1.0 - tt) * 30.0) as u8;
                ctx.move_to(Vec2::new(col as i32, y as i32));
                let mut row = ctx.row_writer();
                row.cell(0).glyph('▐').style(
                    &Style::new()
                        .fg(Color::Rgb(50, stem_shade, 25))
                        .bg(Color::BLACK),
                );
            }
        }
    }

    // ── Petals ──
    let layers = 8u32;
    let petals_per = 10u32;

    for layer in 0..layers {
        let lr = layer as f32 / layers as f32;
        let layer_delay = (1.0 - lr) * 0.25;
        let layer_bloom = ((bloom - layer_delay) / (1.0 - layer_delay)).clamp(0.0, 1.0);
        if layer_bloom < 0.01 {
            continue;
        }

        let base_r = max_r * lr.max(0.12);
        let np = petals_per + layer * 2;
        let inner_boost = 1.0 - lr;

        for i in 0..np {
            let angle = (std::f32::consts::TAU / np as f32) * i as f32
                + layer as f32 * 0.37
                + (t * 0.03).sin() * 0.08
                + inner_boost * 0.1;

            let dist = base_r * layer_bloom * (0.35 + lr * 0.65);
            let petal_len = base_r * (0.5 + inner_boost * 0.3) * layer_bloom;
            let tip_sharpness = 0.3 + inner_boost * 0.5;

            for step in 0..((petal_len * 2.2) as u32) {
                let s = step as f32 / (petal_len * 2.2);
                let width_factor = if s < 0.5 {
                    s / 0.5 * (1.0 - tip_sharpness * 0.3)
                } else {
                    (1.0 - s) / 0.5 * (1.0 - tip_sharpness * 0.5)
                };
                if width_factor < 0.15 {
                    continue;
                }

                let px = cx + (angle.cos() * (dist + s * petal_len));
                let py = cy + (angle.sin() * (dist + s * petal_len)) * 0.45;

                let col = px as u16;
                let row = py as u16;
                if col >= w || row >= h {
                    continue;
                }

                let depth = lr * 0.5 + (1.0 - lr) * 0.5;
                let breathe = ((t * 0.6 + layer as f32 * 0.4).sin() * 0.5 + 0.5) * 0.12;
                let mut intensity = (depth + breathe) * layer_bloom;
                if flash > 0.1 {
                    intensity = (intensity + flash * 0.5).min(1.0);
                }

                let (r, g, b) = petal_color(state.variant, lr, intensity, layer, i);
                let ch = render_char(state.mode, s, i, layer, tick, tip_sharpness);

                ctx.move_to(Vec2::new(col as i32, row as i32));
                let mut row_w = ctx.row_writer();
                row_w
                    .cell(0)
                    .glyph(ch)
                    .style(&Style::new().fg(Color::Rgb(r, g, b)).bg(Color::BLACK));
            }
        }
    }

    // ── Thorns/spikes ──
    if bloom > 0.5 {
        let spike_bloom = ((bloom - 0.5) / 0.4).min(1.0);
        let num_spikes = 16u32;
        for i in 0..num_spikes {
            let angle =
                (std::f32::consts::TAU / num_spikes as f32) * i as f32 + (t * 0.05).sin() * 0.2;
            let spike_dist = max_r * 0.85 * spike_bloom;
            for si in 0..3 {
                let sd = spike_dist + si as f32 * 1.5;
                let sx = cx + angle.cos() * sd;
                let sy = cy + angle.sin() * sd * 0.45;
                let col = sx as u16;
                let row = sy as u16;
                if col < w && row < h {
                    let spike_alpha = (0.3 + spike_bloom * 0.5) * (1.0 - si as f32 * 0.3);
                    let sr = (200.0 * spike_alpha) as u8;
                    let sg = (60.0 * spike_alpha) as u8;
                    let sb = (100.0 * spike_alpha) as u8;
                    ctx.move_to(Vec2::new(col as i32, row as i32));
                    let mut row_w = ctx.row_writer();
                    row_w
                        .cell(0)
                        .glyph(SPIKES[i as usize % SPIKES.len()])
                        .style(&Style::new().fg(Color::Rgb(sr, sg, sb)).bg(Color::BLACK));
                }
            }
        }
    }

    // ── Starburst center ──
    if bloom > 0.25 {
        let core_bright = ((bloom - 0.25) / 0.4).min(1.0);
        for ri in 0..8 {
            let r_angle = std::f32::consts::TAU / 8.0 * ri as f32 + t * 0.04;
            for rd in 1..4 {
                let dist = rd as f32 * 1.8 * core_bright;
                let sx = cx + r_angle.cos() * dist;
                let sy = cy + r_angle.sin() * dist * 0.45;
                let col = sx as u16;
                let row = sy as u16;
                if col < w && row < h {
                    let bright = (230.0 - rd as f32 * 30.0) as u8;
                    ctx.move_to(Vec2::new(col as i32, row as i32));
                    let mut row_w = ctx.row_writer();
                    row_w.cell(0).glyph('✦').style(
                        &Style::new()
                            .fg(Color::Rgb(bright, bright, 200))
                            .bg(Color::BLACK),
                    );
                }
            }
        }

        // Central pistil.
        let pistil_alpha = ((bloom - 0.25) / 0.4).min(1.0);
        let pr = (200.0 * pistil_alpha + flash * 55.0) as u8;
        let pg = (100.0 * pistil_alpha) as u8;
        let pb = (60.0 * pistil_alpha) as u8;
        let cc = cx as u16;
        let cr = cy as u16;
        if cc < w && cr < h {
            ctx.move_to(Vec2::new(cc as i32, cr as i32));
            let mut row_w = ctx.row_writer();
            row_w
                .cell(0)
                .glyph('⬟')
                .style(&Style::new().fg(Color::Rgb(pr, pg, pb)).bg(Color::BLACK));
        }
    }
}

// ── Bloom helpers ──────────────────────────────────────────────────────────────

fn petal_color(variant: u8, lr: f32, intensity: f32, layer: u32, petal: u32) -> (u8, u8, u8) {
    let hash = ((petal * 73 + layer * 137) % 256) as f32 / 256.0;
    let inner_boost = (1.0 - lr) * 0.3;

    match variant {
        0 => {
            let r = lerp(200.0, 255.0, lr) * intensity * (0.9 + hash * 0.2 + inner_boost);
            let g = lerp(30.0, 130.0, lr) * intensity * 0.6;
            let b = lerp(80.0, 200.0, lr) * intensity * 0.5;
            (r.max(8.0) as u8, g.max(3.0) as u8, b.max(3.0) as u8)
        }
        1 => {
            let r = lerp(180.0, 240.0, lr) * intensity * (0.85 + hash * 0.2);
            let g = lerp(40.0, 100.0, lr) * intensity * 0.5;
            let b = lerp(160.0, 240.0, lr) * intensity * 0.8;
            (r.max(8.0) as u8, g.max(3.0) as u8, b.max(3.0) as u8)
        }
        _ => {
            let r = lerp(255.0, 255.0, lr) * intensity * (0.95 + hash * 0.15);
            let g = lerp(120.0, 220.0, lr) * intensity * 0.8;
            let b = lerp(30.0, 100.0, lr) * intensity * 0.4;
            (r.max(8.0) as u8, g.max(3.0) as u8, b.max(3.0) as u8)
        }
    }
}

fn render_char(mode: u8, s: f32, petal: u32, layer: u32, tick: u64, sharpness: f32) -> char {
    match mode {
        0 => {
            let idx = ((s * 12.0) as usize + petal as usize + layer as usize) % CHARS.len();
            if s > 0.8 && sharpness > 0.5 {
                let tip_chars = &['⭒', '⬡', '◆', '◎', '⬢', '✦', '△'];
                tip_chars[(petal as usize + layer as usize) % tip_chars.len()]
            } else {
                CHARS[idx] as char
            }
        }
        1 => {
            let density = if s < 0.3 {
                s / 0.3 * 0.8 + 0.2
            } else if s > 0.75 {
                (1.0 - s) / 0.25 * 0.6
            } else {
                0.8
            };
            if density > 0.75 {
                '█'
            } else if density > 0.5 {
                '▓'
            } else if density > 0.3 {
                '▒'
            } else {
                '░'
            }
        }
        _ => {
            let braille_base = 0x2800u32;
            let density_mask = if s > 0.7 {
                ((1.0 - s) / 0.3 * 128.0) as u32
            } else {
                255u32
            };
            let dots = ((s * 8.0) as u32 + tick as u32 + petal * 7 + layer * 13) & density_mask;
            char::from_u32(braille_base + dots.min(255)).unwrap_or('·')
        }
    }
}

fn ease_out_back(x: f32) -> f32 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
