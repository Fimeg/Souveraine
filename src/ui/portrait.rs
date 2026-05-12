//! Annie's half-block silhouette — the fallback when no terminal image
//! protocol (kitty/sixel) is available. Also the "presence" overlay card.
//!
//! Renders a hand-crafted stylized portrait directly into ratatui's frame
//! buffer using upper/lower half-block characters (`▀` / `▄` / `█`) so each
//! cell holds two vertically-stacked pixels. This is the C2 implementation:
//! recognizable Annie (twin-tails, cyan filigree, forehead diamond), seven
//! visible states, no new dependencies.
//!
//! The only path for photo portraits is `ratatui-image` (`Image` widget in
//! `App::image_protocol`), which renders via kitty/sixel and falls back
//! to unicode halfblocks internally. This module is the *pixel-art fallback*,
//! not a photo pipeline.
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

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
};

use crate::ui::presence::{Posture, Presence};

pub const PORTRAIT_W: u16 = 18;
pub const PORTRAIT_H: u16 = 18;

/// Cells consumed when rendering: width × (height / 2) because half-blocks
/// stack two pixels per cell.
pub const RENDER_W: u16 = PORTRAIT_W;
pub const RENDER_H: u16 = PORTRAIT_H / 2;

// ── Base portrait grid — clean silhouette, no face ──────────────────────
// This is the EMERGENCY default. The hand-crafted "Annie face" version
// read as a creepy llama (Casey's words), so this is now a faceless
// silhouette: hair, neck, collar. State animation lives in border color,
// breath luminance pulse, and posture-driven color modulation — never in
// per-pixel row swaps at this resolution. True facial animation belongs
// in a future TTS/STT-integrated system, not in half-blocks.
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

// ── State-aware pixel lookup ────────────────────────────────────────────

/// Read a pixel from the base silhouette grid. No facial features — state
/// expression lives in border color, breath, and posture-driven modulation.
fn pixel_at(x: usize, y: usize, _p: &Presence) -> char {
    BASE[y].as_bytes()[x] as char
}

// ── Palette ─────────────────────────────────────────────────────────────

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

// ── Render ──────────────────────────────────────────────────────────────

/// Resolve a single pixel's color from the silhouette palette, modulated
/// by posture and breath.
fn pixel_color(px: usize, py: usize, p: &Presence, breath: f32) -> Option<Color> {
    let key = pixel_at(px, py, p);
    color_for(key, p.posture, breath)
}

/// Render the portrait scaled-up by `scale` (1 = native half-block density).
/// Each grid pixel becomes a `scale × scale` square. Use this for the
/// presence-mode fullscreen view. Cells outside `area` are skipped.
pub fn render_scaled(buf: &mut Buffer, area: Rect, p: &Presence, scale: u16) {
    if scale == 0 { return; }
    let scale = scale.max(1);
    let breath = p.animator.breathe(2500);

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

            let cx0 = area.x + (px as u16) * cell_w;
            let cy0 = area.y + ((py / 2) as u16) * cell_h;
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

// ── Tests ───────────────────────────────────────────────────────────────

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
}
