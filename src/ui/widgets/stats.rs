//! Shared stat-rendering helpers used across dashboard-style widgets.
//!
//! `push_bar` lived in `chat_sidebar` and `breathe_color` in `welcome`; the
//! unified agent screen is a third caller for both, so they graduate here as
//! `pub(crate)` helpers instead of gaining a third verbatim copy.

use tuie::prelude::*;

/// Append a bar like `[████████░░] ` to `out` using `width` blocks.
///
/// `fraction` is 0.0–1.0; filled blocks use `color`, empty blocks and
/// brackets use dim gray.
pub(crate) fn push_bar(out: &mut StyledString, fraction: f32, width: usize, color: Color) {
    let filled = ((fraction.clamp(0.0, 1.0)) * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    let bracket = Color::BRIGHT_BLACK;

    out.push_span(StyledStr::new("[").fg(bracket));
    if filled > 0 {
        out.push_span(StyledStr::new(&"█".repeat(filled)).fg(color));
    }
    if empty > 0 {
        out.push_span(StyledStr::new(&"░".repeat(empty)).fg(bracket));
    }
    out.push_span(StyledStr::new("] ").fg(bracket));
}

/// Gently pulse `base`'s brightness with a slow sine — palette-agnostic, so it
/// works whatever atmosphere colour an agent currently wears.
///
/// `tick` is a wrapping counter advanced on each animation step (~every 90 ms).
pub(crate) fn breathe_color(base: Color, tick: u64) -> Color {
    let (r, g, b) = match base {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255u8, 140, 66),
    };
    let phase = (tick as f32 * 0.06).sin() * 0.5 + 0.5; // 0..1
    let f = 0.78 + phase * 0.22; // 0.78..1.0
    Color::Rgb(
        (r as f32 * f) as u8,
        (g as f32 * f) as u8,
        (b as f32 * f) as u8,
    )
}
