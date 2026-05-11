use std::time::Duration;
use tokio::time::Instant;
use std::io::{stdout, Write};

/// Animation system for sexy terminal effects
pub struct Animator {
    start_time: Instant,
}

impl Animator {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }
    
    /// Breathing glow effect (sine wave)
    /// Returns intensity 0.0-1.0 based on time
    pub fn breathe(&self, speed_ms: u64) -> f32 {
        let elapsed = self.start_time.elapsed().as_millis() as f64;
        let cycle = (elapsed / speed_ms as f64) * 2.0 * std::f64::consts::PI;
        ((cycle.sin() + 1.0) / 2.0) as f32
    }
    
    /// Pulsing between two colors
    pub fn pulse_color(&self, color1: (u8, u8, u8), color2: (u8, u8, u8), speed_ms: u64) -> (u8, u8, u8) {
        let t = self.breathe(speed_ms);
        (
            (color1.0 as f32 * (1.0 - t) + color2.0 as f32 * t) as u8,
            (color1.1 as f32 * (1.0 - t) + color2.1 as f32 * t) as u8,
            (color1.2 as f32 * (1.0 - t) + color2.2 as f32 * t) as u8,
        )
    }
}

/// Typing animation for responses
pub async fn typewrite(text: &str, wpm: u64) {
    // 120 WPM = ~10 chars/sec = 100ms per char
    let delay_ms = 60000 / (wpm * 5); // chars per word ~5
    
    for ch in text.chars() {
        print!("{}", ch);
        stdout().flush().unwrap();
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

/// Gradient text across a string
pub fn gradient(text: &str, start_hue: f32) -> String {
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            let hue = (start_hue + i as f32 * 3.0) % 360.0;
            let (r, g, b) = hsl_to_rgb(hue, 0.8, 0.6);
            format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, ch)
        })
        .collect()
}

/// Smooth spinner using braille patterns
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Wave animation for progress
pub const WAVE: &[&str] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂"];

/// Persona signature colors
pub mod colors {
    use ratatui::style::Color;

    // Ani: Warm, gentle, inviting
    pub const ANI_PRIMARY: Color = Color::Rgb(255, 140, 66);    // Warm orange
    pub const ANI_SECONDARY: Color = Color::Rgb(255, 200, 150); // Light peach
    pub const ANI_DIM: Color = Color::Rgb(180, 120, 80);        // Muted brown-orange

    // Jean-Luc: Cool, precise, technical
    pub const JEANLUC_PRIMARY: Color = Color::Rgb(66, 133, 244);   // Blue
    pub const JEANLUC_SECONDARY: Color = Color::Rgb(150, 200, 255); // Light blue
    pub const JEANLUC_DIM: Color = Color::Rgb(80, 100, 140);       // Steel

    // Eione: Creative, flowing, purple
    pub const EIONE_PRIMARY: Color = Color::Rgb(155, 89, 182);    // Purple
    pub const EIONE_SECONDARY: Color = Color::Rgb(200, 150, 220);  // Light purple
    pub const EIONE_DIM: Color = Color::Rgb(120, 80, 140);         // Muted

    // Subconscious surfacing: Gray, dim, italic
    pub const SUBCONSCIOUS: Color = Color::Rgb(128, 128, 128);
}

/// Get breathing color for active element
pub fn breathing_color(base: (u8, u8, u8), intensity: f32) -> (u8, u8, u8) {
    // Shift brightness slightly
    let factor = 0.8 + (intensity * 0.4); // 0.8 to 1.2
    (
        (base.0 as f32 * factor).min(255.0) as u8,
        (base.1 as f32 * factor).min(255.0) as u8,
        (base.2 as f32 * factor).min(255.0) as u8,
    )
}

/// HSL to RGB conversion for gradients
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

/// Procedural bloom for the splash screen — terminal peony.
///
/// Renders a radial flower using braille characters that grows from
/// center outward. Three render modes cycle: braille dots, block fills,
/// and ASCII characters. Inspired by peonia.html.
pub mod bloom {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use crate::ui::color_support::rgb;

    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789@#$%&*";
    /// Sharp spike chars for wicked edges
    const SPIKES: &[char] = &['▲', '△', '⤴', '⤵', '➚', '➘', '✸', '✦', '⬆', '⬇'];

    pub struct BloomState {
        pub progress: f32,
        pub mode: u8,
        mode_timer: f32,
        /// Flash intensity for glitch transitions (0.0-1.0)
        pub flash: f32,
        /// Which flower variant (for color cycling like peonia)
        pub variant: u8,
    }

    impl BloomState {
        pub fn new() -> Self {
            Self { progress: 0.0, mode: 0, mode_timer: 0.0, flash: 0.0, variant: 0 }
        }

        pub fn advance(&mut self, dt: f32) {
            self.progress = (self.progress + dt * 0.25).min(1.0);
            self.flash *= 0.92; // decay flash
            self.mode_timer += dt;
            if self.mode_timer > 1.8 {
                self.mode_timer = 0.0;
                self.mode = (self.mode + 1) % 3;
                self.flash = 1.0; // glitch spark on mode change
                self.variant = (self.variant + 1) % 3;
            }
        }
    }

    pub fn render(buf: &mut Buffer, area: Rect, state: &BloomState, tick: u64) {
        if area.width < 20 || area.height < 10 {
            return;
        }

        let cx = area.width as f32 / 2.0;
        let cy = area.height as f32 / 3.8; // higher up to leave room for stem
        let max_r = (cx.min(cy * 2.0) * 0.65).min(26.0);
        let bloom = ease_out_back(state.progress); // sharper attack than ease_in_out
        let t = tick as f32 * 0.08;
        let flash = state.flash;

        // --- Stem ---
        if bloom > 0.05 {
            let stem_progress = ((bloom - 0.05) / 0.5).min(1.0);
            let stem_len = area.height as f32 * 0.28;
            let stem_top = cy + max_r * 0.3;
            let stem_visible = stem_len * stem_progress;
            let stem_bot = stem_top + stem_visible;
            for y in (stem_top as u16)..(stem_bot as u16).min(area.y + area.height) {
                let tt = (y as f32 - stem_top) / stem_len;
                let curve = (tt * std::f32::consts::PI * 0.3).sin() * 6.0
                          + (tt * std::f32::consts::PI * 0.8).sin() * 2.0;
                let col = (area.x as f32 + cx + curve) as u16;
                if col < area.x + area.width && y < area.y + area.height {
                    let c = buf.get_mut(col, y);
                    let stem_shade = (40.0 + (1.0 - tt) * 30.0) as u8;
                    c.set_char('▐');
                    c.set_style(Style::default().fg(rgb(50, stem_shade, 25)));
                }
            }
            // Small leaf at ~40% up
            if stem_progress > 0.4 {
                let leaf_y = stem_top + stem_visible * 0.35;
                let leaf_x = area.x as f32 + cx + (0.35 * std::f32::consts::PI * 0.3).sin() * 6.0;
                let leaf_chars = ['/', '\\', '|', '—'];
                for (li, lc) in leaf_chars.iter().enumerate() {
                    let lx = leaf_x as u16 + li as u16;
                    let ly = leaf_y as u16 - 1 + li as u16;
                    if lx < area.x + area.width && ly < area.y + area.height {
                        let c = buf.get_mut(lx, ly);
                        c.set_char(*lc);
                        c.set_style(Style::default().fg(rgb(55, 100, 30)));
                    }
                }
            }
        }

        // --- Sharp petals with pointed tips ---
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
                let tip_sharpness = 0.3 + inner_boost * 0.5; // inner layers sharper

                for step in 0..((petal_len * 2.2) as u32) {
                    let s = step as f32 / (petal_len * 2.2);
                    // Sharp pointed tip: use triangular width falloff instead of smooth sine
                    let width_factor = if s < 0.5 {
                        s / 0.5 * (1.0 - tip_sharpness * 0.3)
                    } else {
                        (1.0 - s) / 0.5 * (1.0 - tip_sharpness * 0.5)
                    };
                    let width_factor = width_factor.max(0.0);
                    if width_factor < 0.15 {
                        continue;
                    }

                    let px = cx + (angle.cos() * (dist + s * petal_len));
                    let py = cy + (angle.sin() * (dist + s * petal_len)) * 0.45;

                    let col = area.x + px as u16;
                    let row = area.y + py as u16;
                    if col >= area.x + area.width || row >= area.y + area.height {
                        continue;
                    }

                    let depth = lr * 0.5 + (1.0 - lr) * 0.5;
                    let breathe = ((t * 0.6 + layer as f32 * 0.4).sin() * 0.5 + 0.5) * 0.12;
                    let mut intensity = (depth + breathe) * layer_bloom;
                    // Flash boost on mode transitions
                    if flash > 0.1 {
                        intensity = (intensity + flash * 0.5).min(1.0);
                    }

                    let (r, g, b) = petal_color(state.variant, lr, intensity, layer, i);
                    let ch = render_char(state.mode, s, i, layer, tick, tip_sharpness);

                    let cell = buf.get_mut(col, row);
                    cell.set_char(ch);
                    cell.set_style(Style::default().fg(rgb(r, g, b)));
                }
            }
        }

        // --- Thorns/spikes radiating outward between outer petals ---
        if bloom > 0.5 {
            let spike_bloom = ((bloom - 0.5) / 0.4).min(1.0);
            let num_spikes = 16u32;
            for i in 0..num_spikes {
                let angle = (std::f32::consts::TAU / num_spikes as f32) * i as f32
                    + (t * 0.05).sin() * 0.2;
                let spike_dist = max_r * 0.85 * spike_bloom;
                for si in 0..3 {
                    let sd = spike_dist + si as f32 * 1.5;
                    let sx = cx + angle.cos() * sd;
                    let sy = cy + angle.sin() * sd * 0.45;
                    let col = area.x + sx as u16;
                    let row = area.y + sy as u16;
                    if col < area.x + area.width && row < area.y + area.height {
                        let cell = buf.get_mut(col, row);
                        cell.set_char(SPIKES[i as usize % SPIKES.len()]);
                        let spike_alpha = (0.3 + spike_bloom * 0.5) * (1.0 - si as f32 * 0.3);
                        let sr = (200.0 * spike_alpha) as u8;
                        let sg = (60.0 * spike_alpha) as u8;
                        let sb = (100.0 * spike_alpha) as u8;
                        cell.set_style(Style::default().fg(rgb(sr, sg, sb)));
                    }
                }
            }
        }

        // --- Starburst center (bright core with radiating dots) ---
        if bloom > 0.25 {
            let core_bright = ((bloom - 0.25) / 0.4).min(1.0);
            // Radiating starburst lines from center
            for ri in 0..8 {
                let r_angle = std::f32::consts::TAU / 8.0 * ri as f32 + t * 0.04;
                for rd in 1..4 {
                    let dist = rd as f32 * 1.8 * core_bright;
                    let sx = cx + r_angle.cos() * dist;
                    let sy = cy + r_angle.sin() * dist * 0.45;
                    let col = area.x + sx as u16;
                    let row = area.y + sy as u16;
                    if col < area.x + area.width && row < area.y + area.height {
                        let cell = buf.get_mut(col, row);
                        let bright = (230.0 - rd as f32 * 30.0) as u8;
                        cell.set_char('✦');
                        cell.set_style(Style::default().fg(rgb(bright, bright, 200)));
                    }
                }
            }

            // Central pistil
            let pistil_alpha = ((bloom - 0.25) / 0.4).min(1.0);
            let pr = (200.0 * pistil_alpha + flash * 55.0) as u8;
            let pg = (100.0 * pistil_alpha) as u8;
            let pb = (60.0 * pistil_alpha) as u8;
            let cc = area.x + cx as u16;
            let cr = area.y + cy as u16;
            if cc < area.x + area.width && cr < area.y + area.height {
                let cell = buf.get_mut(cc, cr);
                cell.set_char('⬟');
                cell.set_style(Style::default().fg(rgb(pr.min(255), pg.min(255), pb.min(255))));
            }
        }
    }

    fn petal_color(variant: u8, lr: f32, intensity: f32, layer: u32, petal: u32) -> (u8, u8, u8) {
        let hash = ((petal * 73 + layer * 137) % 256) as f32 / 256.0;
        let inner_boost = (1.0 - lr) * 0.3;

        match variant {
            // Variant 0: Hot magenta-crimson (wicked)
            0 => {
                let r = lerp(200.0, 255.0, lr) * intensity * (0.9 + hash * 0.2 + inner_boost);
                let g = lerp(30.0, 130.0, lr) * intensity * 0.6;
                let b = lerp(80.0, 200.0, lr) * intensity * 0.5;
                (r.max(8.0) as u8, g.max(3.0) as u8, b.max(3.0) as u8)
            }
            // Variant 1: Deep violet-ember
            1 => {
                let r = lerp(180.0, 240.0, lr) * intensity * (0.85 + hash * 0.2);
                let g = lerp(40.0, 100.0, lr) * intensity * 0.5;
                let b = lerp(160.0, 240.0, lr) * intensity * 0.8;
                (r.max(8.0) as u8, g.max(3.0) as u8, b.max(3.0) as u8)
            }
            // Variant 2: Fiery orange-gold
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
                // ASCII chars — use sharper glyphs at the tip
                let idx = ((s * 12.0) as usize + petal as usize + layer as usize) % CHARS.len();
                if s > 0.8 && sharpness > 0.5 {
                    // Sharp tip gets pointy chars
                    let tip_chars = &['⭒', '⬡', '◆', '◎', '⬢', '✦', '△'];
                    tip_chars[(petal as usize + layer as usize) % tip_chars.len()]
                } else {
                    CHARS[idx] as char
                }
            }
            1 => {
                // Block fills with sharper edge transitions
                let density = if s < 0.3 {
                    s / 0.3 * 0.8 + 0.2  // sharper ramp
                } else if s > 0.75 {
                    (1.0 - s) / 0.25 * 0.6  // sharp falloff at tip
                } else {
                    0.8
                };
                if density > 0.75 { '█' }
                else if density > 0.5 { '▓' }
                else if density > 0.3 { '▒' }
                else { '░' }
            }
            _ => {
                // Braille — use sparser patterns near edges for sharper look
                let braille_base = 0x2800u32;
                let density_mask = if s > 0.7 {
                    ((1.0 - s) / 0.3 * 128.0) as u32  // fewer dots at tip
                } else {
                    255u32
                };
                let dots = ((s * 8.0) as u32 + tick as u32 + petal * 7 + layer * 13) & density_mask;
                char::from_u32(braille_base + dots.min(255)).unwrap_or('·')
            }
        }
    }

    /// Ease-out-back: sharp attack with slight overshoot for "wicked" pop
    fn ease_out_back(x: f32) -> f32 {
        let c1 = 1.70158;
        let c3 = c1 + 1.0;
        1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
    }

    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_gradient() {
        let text = "Ani";
        let result = gradient(text, 30.0); // Orange-ish start
        assert!(result.contains("\x1b[38;2;")); // Contains ANSI color code
    }
    
    #[test]
    fn test_breathe() {
        let animator = Animator::new();
        let val = animator.breathe(1000);
        assert!(val >= 0.0 && val <= 1.0);
    }
}
