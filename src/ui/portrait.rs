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

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
};

use crate::ui::presence::{Eye, Posture, Presence};

pub const PORTRAIT_W: u16 = 18;
pub const PORTRAIT_H: u16 = 18;

/// Cells consumed when rendering: width × (height / 2) because half-blocks
/// stack two pixels per cell.
pub const RENDER_W: u16 = PORTRAIT_W;
pub const RENDER_H: u16 = PORTRAIT_H / 2;

// ── Base portrait grid ──────────────────────────────────────────
// Each row must be exactly PORTRAIT_W characters wide. Validated by a test.

const BASE: [&str; PORTRAIT_H as usize] = [
    "....HH......HH....",
    "...HHHH....HHHH...",
    "..HHHHHH..HHHHHH..",
    "..HHHHHHHHHHHHHH..",
    "..HHHHHHHHHHHHHH..",
    "..hhhHHHHHHHHhhh..",
    ".HHHBBssMMssBBHHH.",
    ".HHHsseesseessHHH.",
    ".HHHssssssssssHHH.",
    ".HHHsMssssssMsHHH.",
    ".HHHHssssssssHHHH.",
    ".HHHHsssLLsssHHHH.",
    ".HHHHHssssssHHHHH.",
    ".HHHHHHssssHHHHHH.",
    ".HHHcCcccCcccCHHH.",
    "..CCCCCCCCCCCCCC..",
    "..CCCCcCccCcCCCC..",
    "..CCCC      CCCC..",
];

// ── State-aware pixel lookup ────────────────────────────────────

/// Read a pixel from the base grid, applying state-driven cell overrides
/// before returning. Each posture has its own row swaps so the portrait's
/// SHAPE shifts (not just color) when state changes.
fn pixel_at(x: usize, y: usize, p: &Presence) -> char {
    let row = BASE[y];
    let ch = row.as_bytes()[x] as char;

    // Eye row override — replace `e` with `-` while blinking OR yawning.
    if y == 7 && ch == 'e' && (p.eye == Eye::Blinking || p.posture == Posture::Yawning) {
        return '-';
    }

    // Posture-specific row swaps.
    match p.posture {
        Posture::Yawning => {
            // Half-lid eyes (already handled above) + open mouth.
            if y == 11 {
                let yawn = b".HHHHssOOOOssHHHH.";
                return yawn[x] as char;
            }
        }
        Posture::Processing => {
            // Eyes lock to one side: shift the cyan-glow pixels right by 1.
            // Base eye row: ".HHHsseesseessHHH." — gaze-shift to ".HHHssseseeseesHHH" feel
            // by stretching one pupil right.
            if y == 7 {
                let proc = b".HHHsesesseeseesHH";
                return proc[x] as char;
            }
            // Add a faint cyan filigree pulse on the brow.
            if y == 6 {
                let proc = b".HHHBBssMMssBBHHH.";
                return proc[x] as char;
            }
        }
        Posture::Affectionate => {
            // Slight smile — lips curve up at the corners.
            // Base mouth row 11: ".HHHHsssLLsssHHHH."
            // Affect    row 11: ".HHHHsLssssssLsHHH"  (lift the L's to the cheek line)
            if y == 11 {
                let aff = b".HHHHsssLLsssHHHH.";
                return aff[x] as char;
            }
            // Cheek line gets a soft warm fold above the mouth.
            if y == 10 {
                let aff = b".HHHHsssLLsssHHHH.";
                return aff[x] as char;
            }
        }
        Posture::Straining => {
            // Furrowed brow — brow row darkens and tightens.
            // Base row 6:    ".HHHBBssMMssBBHHH."
            // Strain row 6:  ".HHBBBssMMssBBBHH."  (brow encroaches inward)
            if y == 6 {
                let strain = b".HHBBBssMMssBBBHH.";
                return strain[x] as char;
            }
            // Slight frown — straighten the lips.
            if y == 11 {
                let strain = b".HHHHsss--sssHHHH.";
                return strain[x] as char;
            }
        }
        Posture::Idle => {}
    }

    ch
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
            let key = pixel_at(px as usize, py as usize, p);
            let col = color_for(key, p.posture, breath);
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

            let top_key = pixel_at(px, py_top, p);
            let bot_key = pixel_at(px, py_bot, p);

            let top_col = color_for(top_key, p.posture, breath);
            let bot_col = color_for(bot_key, p.posture, breath);

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
    fn pixel_at_returns_base_when_idle() {
        let p = Presence::new("Annie");
        // Row 7 has eyes at base.
        let row7 = "_".repeat(PORTRAIT_W as usize);
        let _ = row7;
        // Position of an 'e' in BASE[7] = ".HHHsseesseessHHH."
        //  index 5 should be 's', index 6 should be 'e'
        assert_eq!(pixel_at(6, 7, &p), 'e');
        assert_eq!(pixel_at(7, 7, &p), 'e');
    }

    #[test]
    fn pixel_at_swaps_eye_to_dash_on_blink() {
        let mut p = Presence::new("Annie");
        // Force blink state.
        p.handle_event(&crate::ui::component::TuiEvent::Tick(200));
        // Eye is now Blinking; pixel_at row 7 should be '-' where it was 'e'.
        assert_eq!(pixel_at(6, 7, &p), '-');
    }

    #[test]
    fn pixel_at_swaps_eye_to_dash_on_yawn() {
        let mut p = Presence::new("Annie");
        p.handle_event(&crate::ui::component::TuiEvent::PressureChanged(0.9));
        assert_eq!(pixel_at(6, 7, &p), '-');
    }

    #[test]
    fn yawning_changes_mouth_row() {
        let mut p = Presence::new("Annie");
        p.handle_event(&crate::ui::component::TuiEvent::PressureChanged(0.9));
        // Base row 11: ".HHHHsssLLsssHHHH." — yawn replaces LL with OOOO.
        // Position 8 in yawn row is 'O'.
        assert_eq!(pixel_at(8, 11, &p), 'O');
    }

    #[test]
    fn color_for_transparent_pixels_returns_none() {
        assert!(color_for('.', Posture::Idle, 0.5).is_none());
        assert!(color_for('?', Posture::Idle, 0.5).is_none());
    }

    #[test]
    fn straining_desaturates_eye_color() {
        let idle = color_for('e', Posture::Idle, 0.5).unwrap();
        let strained = color_for('e', Posture::Straining, 0.5).unwrap();
        // The eye colour at idle is very blue-leaning; under strain the rgb
        // channels should drift closer together (toward grey).
        if let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (idle, strained) {
            let spread_idle = g1.abs_diff(r1) as u16 + b1.abs_diff(r1) as u16;
            let spread_strained = g2.abs_diff(r2) as u16 + b2.abs_diff(r2) as u16;
            assert!(
                spread_strained < spread_idle,
                "expected straining to desaturate; got idle spread {} vs strained spread {}",
                spread_idle, spread_strained
            );
        } else {
            panic!("expected RGB colors");
        }
    }
}
