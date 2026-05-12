//! Annie's half-block portrait — Tier 1 of the Presence visual stack.
//!
//! Renders a hand-crafted stylized portrait directly into ratatui's frame
//! buffer using upper/lower half-block characters (`▀` / `▄` / `█`) so each
//! cell holds two vertically-stacked pixels. This is the C2 implementation:
//! recognizable Annie (twin-tails, cyan filigree, forehead diamond), seven
//! visible states, no new dependencies.
//!
//! Tier 2 (C4) will replace this with full PNG rendering via image protocols
//! (kitty/sixel) where supported and fall back to this module elsewhere. The
//! pixel grid lives here as both the Tier 1 art and the fallback art.
//!
//! ## Grid
//!
//! The pixel grid is [`PORTRAIT_W`] × [`PORTRAIT_H`] pixels. Each row of the
//! const arrays is exactly `PORTRAIT_W` characters; each character is a
//! palette key. Rendered, this becomes `PORTRAIT_W` × ([`PORTRAIT_H`] / 2)
//! terminal cells.
//!
//! ## Palette keys
//!
//! - `.` background (transparent — does not write)
//! - `H` hair primary (platinum/white)
//! - `h` hair shadow (dimmer)
//! - `s` skin
//! - `S` skin shadow
//! - `B` brow
//! - `e` eye open (cyan glow)
//! - `-` eye closed
//! - `L` lips
//! - `O` mouth open (yawn variant)
//! - `M` forehead diamond + cheek filigree (cyan)
//! - `C` collar dark
//! - `c` collar cyan accent
//! - ` ` (space) skin (V-neck opening)

use std::path::Path;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
};

use crate::ui::presence::{Posture, Presence};

// ── Loaded per-agent portrait (Tier 2 source) ───────────────────

/// Pixel-grid portrait loaded from a PNG/JPEG on disk and downsampled to
/// `PORTRAIT_W × PORTRAIT_H` colors. When attached to a [`Presence`], the
/// renderer pulls non-overlay pixels from here instead of the hand-coded
/// palette grid, while still painting eye/mouth/brow rows from the
/// state-aware overlays so the seven animation states keep working.
#[derive(Debug, Clone)]
pub struct PortraitSource {
    pub pixels: Vec<Color>, // PORTRAIT_W * PORTRAIT_H, row-major
}

impl PortraitSource {
    /// Sample the pixel at `(x, y)` from the source grid. Returns `None` if
    /// the indices are out of range (caller falls back to the palette grid).
    pub fn at(&self, x: usize, y: usize) -> Option<Color> {
        let idx = y * PORTRAIT_W as usize + x;
        self.pixels.get(idx).copied()
    }

    /// Decode an image file (PNG or JPEG), resize to portrait grid dims,
    /// and produce a colored pixel array. Returns `None` on any I/O or
    /// decode error — but now logs the reason so we can see why a PNG
    /// didn't take instead of silently falling back to the silhouette.
    pub fn from_path(path: &Path) -> Option<Self> {
        let img = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "portrait decode failed"
                );
                return None;
            }
        };
        let resized = img.resize_exact(
            PORTRAIT_W as u32,
            PORTRAIT_H as u32,
            image::imageops::FilterType::Lanczos3,
        );
        let rgb = resized.to_rgb8();
        let pixels = rgb
            .pixels()
            .map(|p| Color::Rgb(p[0], p[1], p[2]))
            .collect();
        Some(Self { pixels })
    }
}


pub const PORTRAIT_W: u16 = 18;
pub const PORTRAIT_H: u16 = 18;

/// Cells consumed when rendering: width × (height / 2) because half-blocks
/// stack two pixels per cell.
pub const RENDER_W: u16 = PORTRAIT_W;
pub const RENDER_H: u16 = PORTRAIT_H / 2;

// ── Base portrait grid — clean silhouette, no face ──────────────
// This is the EMERGENCY default. The hand-crafted "Annie face" version
// read as a creepy llama (Casey's words), so this is now a faceless
// silhouette: hair, neck, collar. State animation lives in border color,
// breath luminance pulse, and posture-driven color modulation — never in
// per-pixel row swaps at this resolution. True facial animation belongs
// in a future TTS/STT-integrated system, not in half-blocks.
//
// When a `PortraitSource` is loaded (per-agent PNG/JPEG from agent memfs
// `assets/`), all pixels come from the source and this silhouette is not
// rendered.
//
// Each row must be exactly PORTRAIT_W characters wide. Validated by a test.
const BASE: [&str; PORTRAIT_H as usize] = [
    "....HHHHHHHHHH....",
    "...HHHHHHHHHHHH...",
    "..HHHHHHHHHHHHHH..",
    "..HHHHHHHHHHHHHH..",
    "..HHHHHHHHHHHHHH..",
    "..HHHHHHHHHHHHHH..",
    "..HHHHssssssssHH..",
    "..HHHssssssssssH..",
    "..HHHssssssssssH..",
    "..HHHssssssssssH..",
    "..HHHssssssssssH..",
    "..HHHssssssssssH..",
    "..HHHHsssssssHHH..",
    "..HHHHHHHHHHHHHH..",
    "..HHHCCCCCCCCHHH..",
    "..CCCCCCCCCCCCCC..",
    "..CCCCCCCCCCCCCC..",
    "..CCCC      CCCC..",
];

// ── State-aware pixel lookup ────────────────────────────────────

/// Read a pixel from the base silhouette grid. The default silhouette has no
/// facial features, so no per-pixel row swaps are needed — state expression
/// at this resolution lives in border color, breath luminance pulse, and the
/// posture-driven color modulation in [`color_for`].
fn pixel_at(x: usize, y: usize, _p: &Presence) -> char {
    BASE[y].as_bytes()[x] as char
}

// ── Palette ─────────────────────────────────────────────────────

/// Resolve a pixel key to an RGB color, modulated by posture + breath_phase.
///
/// Returns `None` for `.` (transparent — the renderer skips this pixel).
fn color_for(key: char, posture: Posture, breath: f32) -> Option<Color> {
    let warm = matches!(posture, Posture::Affectionate);
    let strained = matches!(posture, Posture::Straining);
    let processing = matches!(posture, Posture::Processing);
    let yawning = matches!(posture, Posture::Yawning);

    // Subtle pulse on cyan elements driven by breath.
    let cyan_lum = (170.0 + breath * 60.0).clamp(120.0, 235.0) as u8;
    let cyan_lum = if processing { cyan_lum.saturating_add(25).min(255) } else { cyan_lum };

    let base = match key {
        '.' => return None,
        ' ' => (240, 200, 170),                    // V-neck skin
        'H' => (220, 215, 215),
        'h' => (160, 155, 160),
        's' => if warm { (250, 200, 185) } else { (240, 200, 175) },
        'S' => (200, 150, 130),
        'B' => (45, 35, 35),
        'e' => (60, cyan_lum, cyan_lum.saturating_add(20)),
        '-' => (70, 50, 50),                        // eye-closed line
        'L' => if warm { (235, 145, 155) } else { (200, 110, 125) },
        'O' => (50, 30, 35),                        // open-mouth interior (yawn)
        'M' => (70, cyan_lum, cyan_lum),
        'C' => (20, 20, 28),
        'c' => (60, cyan_lum.saturating_sub(20), cyan_lum.saturating_sub(20)),
        _ => return None,
    };

    let (r, g, b) = base;

    // Yawning droops everything subtly — pull luminance down.
    let dim = if yawning { 0.82 } else { 1.0 };

    // Straining desaturates toward greyscale.
    let sat = if strained { 0.55 } else { 1.0 };
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as f32;
    let mix = |c: u8| {
        let f = (c as f32 * sat + avg * (1.0 - sat)) * dim;
        f.clamp(0.0, 255.0) as u8
    };

    Some(Color::Rgb(mix(r), mix(g), mix(b)))
}

// ── Render ──────────────────────────────────────────────────────

/// Resolve a single pixel's color: source-when-loaded, modulated by posture
/// and breath. With no source the silhouette palette grid is used.
fn pixel_color(px: usize, py: usize, p: &Presence, breath: f32) -> Option<Color> {
    if let Some(source) = p.portrait_source.as_ref() {
        if let Some(c) = source.at(px, py) {
            return Some(modulate(c, p.posture, breath));
        }
    }
    let key = pixel_at(px, py, p);
    color_for(key, p.posture, breath)
}

/// Apply posture-driven modulation to a source pixel — desaturate when
/// straining, dim when yawning, breathe luminance on cyan-leaning hues,
/// warm-tint on affection. This is how state shows on a loaded portrait
/// at half-block resolution: across-the-image tone, not per-pixel swaps.
fn modulate(c: Color, posture: Posture, breath: f32) -> Color {
    let Color::Rgb(r, g, b) = c else { return c };

    let strained = matches!(posture, Posture::Straining);
    let yawning = matches!(posture, Posture::Yawning);
    let warm = matches!(posture, Posture::Affectionate);
    let processing = matches!(posture, Posture::Processing);

    let avg = ((r as u16 + g as u16 + b as u16) / 3) as f32;
    let sat = if strained { 0.55 } else { 1.0 };
    let dim = if yawning { 0.82 } else { 1.0 };
    // Subtle breath pulse — only on the cyan-leaning pixels so skin stays calm.
    let cyan_lean = b > r && b > g;
    let breath_gain = if cyan_lean {
        1.0 + breath * 0.10 * if processing { 1.6 } else { 1.0 }
    } else { 1.0 };
    // Warm tint shifts the red/green channels up a little.
    let warm_r = if warm { 1.06 } else { 1.0 };
    let warm_g = if warm { 1.02 } else { 1.0 };

    let mix = |c: u8, warm_chan: f32| {
        let f = (c as f32 * sat + avg * (1.0 - sat)) * dim * breath_gain * warm_chan;
        f.clamp(0.0, 255.0) as u8
    };
    Color::Rgb(mix(r, warm_r), mix(g, warm_g), mix(b, 1.0))
}

/// Render the portrait scaled-up by `scale` (1 = native half-block density).
/// Each grid pixel becomes a `scale × scale` square. Use this for the
/// presence-mode fullscreen view. Cells outside `area` are skipped.
pub fn render_scaled(buf: &mut Buffer, area: Rect, p: &Presence, scale: u16) {
    if scale == 0 { return; }
    let scale = scale.max(1);
    let breath = p.animator.breathe(2500);

    // Each grid pixel is `scale` cells wide and `scale` cells tall after the
    // half-block density (which already collapses 2 pixels per cell vertically).
    // To keep the aspect roughly square with scale, we use scale horizontally
    // and scale/2 (min 1) vertically since terminal cells are taller than wide.
    let cell_w = scale;
    let cell_h = (scale / 2).max(1);

    let need_w = PORTRAIT_W * cell_w;
    let need_h = (PORTRAIT_H / 2) * cell_h.max(1);
    if area.width < need_w || area.height < need_h { return; }

    for py in 0..PORTRAIT_H {
        for px in 0..PORTRAIT_W {
            let col = pixel_color(px as usize, py as usize, p, breath);
            if col.is_none() { continue; }
            let col = col.unwrap();

            // Each pixel paints a cell_w × cell_h block. Since two pixels
            // share a terminal row (half-blocks), top pixels use ▀ and bottom
            // pixels use ▄, but at scale > 1 we just use █ everywhere because
            // the pixels are already painted as full cells.
            let cx0 = area.x + (px as u16) * cell_w;
            let cy0 = area.y + ((py / 2) as u16) * cell_h
                + if py % 2 == 1 { 0 } else { 0 }; // vertical halves merged at scale>1
            for dx in 0..cell_w {
                for dy in 0..cell_h {
                    let x = cx0 + dx;
                    let y = cy0 + dy;
                    if x >= area.x + area.width || y >= area.y + area.height { continue; }
                    let cell = buf.get_mut(x, y);
                    cell.set_char('█');
                    cell.set_fg(col);
                }
            }
        }
    }
}

/// Render the portrait at native density (half-block precision).
pub fn render(buf: &mut Buffer, area: Rect, p: &Presence) {
    if area.width < RENDER_W || area.height < RENDER_H {
        return;
    }

    let breath = p.animator.breathe(2500);

    for cy in 0..RENDER_H {
        for cx in 0..RENDER_W {
            let px = cx as usize;
            let py_top = (cy * 2) as usize;
            let py_bot = py_top + 1;

            let top_col = pixel_color(px, py_top, p, breath);
            let bot_col = pixel_color(px, py_bot, p, breath);

            if top_col.is_none() && bot_col.is_none() {
                continue;
            }

            let dest_x = area.x + cx;
            let dest_y = area.y + cy;
            if dest_x >= area.x + area.width || dest_y >= area.y + area.height {
                continue;
            }

            let cell = buf.get_mut(dest_x, dest_y);
            match (top_col, bot_col) {
                (Some(fg), Some(bg)) if fg == bg => {
                    cell.set_char('█');
                    cell.set_fg(fg);
                }
                (Some(fg), Some(bg)) => {
                    cell.set_char('▀');
                    cell.set_fg(fg);
                    cell.set_bg(bg);
                }
                (Some(fg), None) => {
                    cell.set_char('▀');
                    cell.set_fg(fg);
                }
                (None, Some(bg)) => {
                    cell.set_char('▄');
                    cell.set_fg(bg);
                }
                (None, None) => {}
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_base_rows_have_expected_width() {
        for (i, row) in BASE.iter().enumerate() {
            assert_eq!(
                row.len(),
                PORTRAIT_W as usize,
                "row {} has wrong width: {:?} (len={})",
                i,
                row,
                row.len()
            );
        }
    }

    #[test]
    fn render_dimensions_match() {
        assert_eq!(RENDER_W, PORTRAIT_W);
        assert_eq!(RENDER_H, PORTRAIT_H / 2);
    }

    #[test]
    fn color_for_transparent_pixels_returns_none() {
        assert!(color_for('.', Posture::Idle, 0.5).is_none());
        assert!(color_for('?', Posture::Idle, 0.5).is_none());
    }

    #[test]
    fn modulate_dims_on_yawn() {
        let base = Color::Rgb(200, 200, 200);
        let yawn = modulate(base, Posture::Yawning, 0.5);
        if let Color::Rgb(r, _, _) = yawn {
            assert!(r < 200, "yawn should dim luminance; got {}", r);
        } else {
            panic!("expected RGB");
        }
    }

    #[test]
    fn modulate_desaturates_on_strain() {
        let blue = Color::Rgb(60, 60, 220);
        let strained = modulate(blue, Posture::Straining, 0.5);
        if let Color::Rgb(r, _, b) = strained {
            // Strain pulls channels toward the average — blue and red should
            // be closer together than they started.
            assert!(b - r < 220 - 60, "strain should desaturate");
        } else {
            panic!("expected RGB");
        }
    }
}
