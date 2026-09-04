//! Atmosphere → tuie Theme mapping.
//!
//! tuie's `harmonious` feature generates a 256-color palette from an 8-color
//! `Theme` struct. We map Souveraine's `ChatPalette` (derived from the active
//! `Atmosphere`) onto tuie's Theme slots so the terminal palette tracks the
//! agent's atmospheric shift — live, without restarting.

use tuie::prelude::{Color, Style};
use tuie::theme::Theme;

use crate::ui::atmosphere::Atmosphere;
use crate::ui::chat::ChatPalette;

/// Map a ChatPalette onto tuie's 8-color Theme slots.
///
/// The mapping is approximate but intentional — each role in ChatPalette
/// lands on the Theme slot that best expresses its semantic weight:
///
/// | ChatPalette role     | Theme slot  | Rationale                        |
/// |----------------------|-------------|----------------------------------|
/// | `bg`                 | `bg`        | terminal background              |
/// | `agent_dim`          | `fg`        | default foreground / muted text  |
/// | `user_accent`        | `cyan`      | cool, distinct from agent colors |
/// | `tool_accent`        | `green`     | "go" signal, success             |
/// | `agent_primary`      | `yellow`    | warmth, attention                |
/// | `surfacing`          | `magenta`   | consciousness surfacing          |
/// | `compaction`         | `red`       | urgency, warning                 |
/// | `reflection`         | `blue`      | calm, reflective                 |
pub fn chat_palette_to_theme(palette: &ChatPalette) -> Theme {
    let fg = to_tuie_rgb(palette.agent_dim);
    let bg = to_tuie_rgb(palette.bg);
    let default = to_tuie_rgb(ratatui::style::Color::Rgb(128, 128, 128));
    let mut indexed = [default; 16];
    // Map palette slots to ANSI color indices.
    indexed[1] = to_tuie_rgb(palette.compaction); // red
    indexed[2] = to_tuie_rgb(palette.tool_accent); // green
    indexed[3] = to_tuie_rgb(palette.agent_primary); // yellow
    indexed[4] = to_tuie_rgb(palette.reflection); // blue
    indexed[5] = to_tuie_rgb(palette.surfacing); // magenta
    indexed[6] = to_tuie_rgb(palette.user_accent); // cyan
                                                   // Fill unused slots with reasonable dark variants.
    indexed[0] = to_tuie_rgb(palette.bg); // black → bg
    indexed[7] = to_tuie_rgb(palette.agent_primary); // white → primary
    indexed[8] = to_tuie_rgb(ratatui::style::Color::Rgb(64, 64, 64)); // bright black
    for i in 9..16 {
        indexed[i] = indexed[i - 8]; // bright variants = dim variants
    }
    Theme::new(fg, bg, indexed)
}

/// Apply an atmosphere to tuie's global palette.
///
/// Builds a Theme from the atmosphere's ChatPalette, generates the 256-color
/// harmonious palette, and applies it globally. Calls `dirty_layout()` so all
/// widgets re-render on the next frame.
pub fn apply_atmosphere(atm: Atmosphere) {
    let palette = ChatPalette::from_atmosphere(atm);
    let theme = chat_palette_to_theme(&palette);
    tuie::theme::harmonious::apply_palette(tuie::theme::harmonious::Palette::from_theme(theme));
    tuie::dirty_layout();
}

/// Convert a ratatui Color to a tuie Rgb.
///
/// tuie uses its own `Rgb` type (in `tuie::util::rgb`) for the Theme struct,
/// while the rest of the widget system uses `tuie::prelude::Color`. This
/// helper bridges ratatui colors (still used by ChatPalette during the dual-
/// stack transition) into tuie's Theme format.
fn to_tuie_rgb(c: ratatui::style::Color) -> tuie::util::rgb::Rgb {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => tuie::util::rgb::Rgb::new(r, g, b),
        _ => tuie::util::rgb::Rgb::new(255, 140, 66), // fallback: Default primary
    }
}

/// Convert a ratatui Color to a tuie Color.
///
/// Used when building tuie `Style` objects from `ChatPalette` colors during
/// the transition period. Once ratatui is fully removed, ChatPalette can
/// directly use tuie Color.
pub fn to_tuie_color(c: ratatui::style::Color) -> Color {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ratatui::style::Color::Reset => Color::Foreground,
        ratatui::style::Color::Black => Color::BLACK,
        ratatui::style::Color::Red => Color::RED,
        ratatui::style::Color::Green => Color::GREEN,
        ratatui::style::Color::Yellow => Color::YELLOW,
        ratatui::style::Color::Blue => Color::BLUE,
        ratatui::style::Color::Magenta => Color::MAGENTA,
        ratatui::style::Color::Cyan => Color::CYAN,
        ratatui::style::Color::Gray => Color::BRIGHT_BLACK,
        ratatui::style::Color::DarkGray => Color::BRIGHT_BLACK,
        ratatui::style::Color::LightRed => Color::BRIGHT_RED,
        ratatui::style::Color::LightGreen => Color::BRIGHT_GREEN,
        ratatui::style::Color::LightYellow => Color::BRIGHT_YELLOW,
        ratatui::style::Color::LightBlue => Color::BRIGHT_BLUE,
        ratatui::style::Color::LightMagenta => Color::BRIGHT_MAGENTA,
        ratatui::style::Color::LightCyan => Color::BRIGHT_CYAN,
        ratatui::style::Color::White => Color::WHITE,
        _ => Color::Rgb(255, 140, 66),
    }
}

// ── Accent color ───────────────────────────────────────────────────────────
//
// The accent is not a separately-tracked value. `apply_atmosphere` feeds the
// agent's ChatPalette through tuie's harmonious palette, where `agent_primary`
// lands on the `yellow` theme slot (see `chat_palette_to_theme`). So the accent
// is simply `Color::YELLOW`, which harmonious resolves to the agent's primary at
// render time — tracking every atmosphere shift for free, no thread-local state.

/// The agent's primary accent color, resolved through the active atmosphere.
pub fn get_accent_color() -> Color {
    Color::YELLOW
}

/// The agent's primary accent as a `Style`.
pub fn get_accent() -> Style {
    Style::new().fg(Color::YELLOW)
}

// ── GUI color scheme (light/dark) ─────────────────────────────────────────

/// Selectable GUI color scheme options with display labels.
#[cfg(feature = "gui")]
pub const COLOR_SCHEMES: &[(&str, Theme, Theme)] = &[
    ("Century", Theme::CENTURY_LIGHT, Theme::CENTURY_DARK),
    ("One", Theme::ONE_LIGHT, Theme::ONE_DARK),
    ("Solarized", Theme::SOLARIZED_LIGHT, Theme::SOLARIZED_DARK),
    ("Gruvbox", Theme::GRUVBOX_LIGHT, Theme::GRUVBOX_DARK),
    (
        "Everforest",
        Theme::EVERFOREST_LIGHT,
        Theme::EVERFOREST_DARK,
    ),
];

/// Selectable GUI appearance options with display labels, where `None` follows the OS appearance.
#[cfg(feature = "gui")]
pub const APPEARANCES: &[(Option<tuie::ansi::ColorScheme>, &str)] = &[
    (Some(tuie::ansi::ColorScheme::Light), "Light"),
    (Some(tuie::ansi::ColorScheme::Dark), "Dark"),
    (None, "System"),
];

/// Sets the GUI color scheme to the entry at `index`.
#[cfg(feature = "gui")]
pub fn set_color_scheme(index: usize) {
    let (_, light, dark) = COLOR_SCHEMES[index];
    tuie::gui::config::update(|cfg| {
        cfg.light_theme = light;
        cfg.dark_theme = dark;
    });
    tuie::gui::reapply_theme();
}

/// Returns the index into [`COLOR_SCHEMES`] matching the active GUI themes.
#[cfg(feature = "gui")]
pub fn get_color_scheme_index() -> usize {
    let cfg = tuie::gui::config::get();
    COLOR_SCHEMES
        .iter()
        .position(|(_, l, d)| *l == cfg.light_theme && *d == cfg.dark_theme)
        .unwrap_or(0)
}

/// Sets the GUI appearance override.
#[cfg(feature = "gui")]
pub fn set_appearance(appearance: Option<tuie::ansi::ColorScheme>) {
    tuie::gui::config::update(|cfg| cfg.appearance = appearance);
    tuie::gui::reapply_theme();
}

/// Returns the index into [`APPEARANCES`] of the current GUI appearance.
#[cfg(feature = "gui")]
pub fn get_appearance_index() -> usize {
    let current = tuie::gui::config::get().appearance;
    APPEARANCES
        .iter()
        .position(|(a, _)| *a == current)
        .unwrap_or(2)
}
