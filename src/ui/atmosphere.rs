//! Atmospheric visual presets — color themes that shift the UI's accent palette.
//!
//! Atmospheric visual presets — ported here so Annie can express mood through
//! the terminal chrome: border colors, title accents, background tints, and
//! per-character text gradients in chat bubbles. The agent sets atmosphere via
//! a structured event; when none is set, a posture-linked default applies.
//!
//! Each preset carries four tones: a primary accent (borders, titles), a secondary
//! accent (subtle highlights), a dim muted shade, and a background tint.

use ratatui::style::Color;

/// Named atmospheric preset. The `Default` variant uses ANI_PRIMARY etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Atmosphere {
    /// Harness defaults — warm orange (#FF8C42 family).
    Default,
    /// Calm greens and teals.
    MintTea,
    /// Soft blues.
    TherapeuticBlue,
    /// Gentle purples.
    LavenderCalm,
    /// Golden / warm amber.
    WarmAmber,
    /// Warm pinks.
    PeachSunset,
    /// Earthy browns.
    AutumnBrowns,
    /// Hot pink / cyan / lime.
    NeonGlow,
    /// Northern lights — cyan/green.
    AuroraBorealis,
    /// Pink spectrum.
    CherryBlossom,
    /// Deep blues.
    OceanDepths,
    /// Dark space with violet.
    MidnightGalaxy,
    /// Purple haze.
    TwilightMist,
    /// Deep nature greens.
    ForestGreens,
}

/// Helper: blend two u8 channels by `t ∈ [0, 1]`.
pub(crate) fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

/// Helper: blend two colors channel-wise.
pub(crate) fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = into_rgb(a);
    let (br, bg, bb) = into_rgb(b);
    Color::Rgb(lerp_u8(ar, br, t), lerp_u8(ag, bg, t), lerp_u8(ab, bb, t))
}

fn into_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 140, 66), // fallback: Default primary
    }
}

impl Atmosphere {
    /// Primary accent — the most visible color (borders, titles, cursor).
    pub fn primary(self) -> Color {
        match self {
            Atmosphere::Default => Color::Rgb(255, 140, 66),
            Atmosphere::MintTea => Color::Rgb(118, 238, 198),
            Atmosphere::TherapeuticBlue => Color::Rgb(135, 206, 235),
            Atmosphere::LavenderCalm => Color::Rgb(216, 191, 216),
            Atmosphere::WarmAmber => Color::Rgb(255, 191, 0),
            Atmosphere::PeachSunset => Color::Rgb(255, 218, 185),
            Atmosphere::AutumnBrowns => Color::Rgb(205, 133, 63),
            Atmosphere::NeonGlow => Color::Rgb(255, 20, 147),
            Atmosphere::AuroraBorealis => Color::Rgb(0, 255, 255),
            Atmosphere::CherryBlossom => Color::Rgb(255, 183, 197),
            Atmosphere::OceanDepths => Color::Rgb(65, 105, 225),
            Atmosphere::MidnightGalaxy => Color::Rgb(139, 0, 139),
            Atmosphere::TwilightMist => Color::Rgb(106, 90, 205),
            Atmosphere::ForestGreens => Color::Rgb(50, 205, 50),
        }
    }

    /// Secondary accent — subtle highlights, secondary text.
    pub fn secondary(self) -> Color {
        match self {
            Atmosphere::Default => Color::Rgb(180, 120, 80),
            Atmosphere::MintTea => Color::Rgb(178, 255, 221),
            Atmosphere::TherapeuticBlue => Color::Rgb(176, 224, 230),
            Atmosphere::LavenderCalm => Color::Rgb(221, 160, 221),
            Atmosphere::WarmAmber => Color::Rgb(255, 215, 0),
            Atmosphere::PeachSunset => Color::Rgb(255, 228, 181),
            Atmosphere::AutumnBrowns => Color::Rgb(222, 184, 135),
            Atmosphere::NeonGlow => Color::Rgb(127, 255, 0),
            Atmosphere::AuroraBorealis => Color::Rgb(127, 255, 212),
            Atmosphere::CherryBlossom => Color::Rgb(255, 105, 180),
            Atmosphere::OceanDepths => Color::Rgb(100, 149, 237),
            Atmosphere::MidnightGalaxy => Color::Rgb(75, 0, 130),
            Atmosphere::TwilightMist => Color::Rgb(123, 104, 238),
            Atmosphere::ForestGreens => Color::Rgb(0, 255, 127),
        }
    }

    /// Dim muted variant for secondary borders, footer text.
    pub fn dim(self) -> Color {
        match self {
            Atmosphere::Default => Color::Rgb(120, 90, 60),
            Atmosphere::MintTea => Color::Rgb(100, 160, 140),
            Atmosphere::TherapeuticBlue => Color::Rgb(100, 140, 160),
            Atmosphere::LavenderCalm => Color::Rgb(140, 120, 160),
            Atmosphere::WarmAmber => Color::Rgb(160, 130, 60),
            Atmosphere::PeachSunset => Color::Rgb(160, 140, 120),
            Atmosphere::AutumnBrowns => Color::Rgb(120, 90, 60),
            Atmosphere::NeonGlow => Color::Rgb(140, 80, 100),
            Atmosphere::AuroraBorealis => Color::Rgb(80, 140, 140),
            Atmosphere::CherryBlossom => Color::Rgb(160, 110, 120),
            Atmosphere::OceanDepths => Color::Rgb(60, 80, 140),
            Atmosphere::MidnightGalaxy => Color::Rgb(80, 60, 100),
            Atmosphere::TwilightMist => Color::Rgb(80, 70, 120),
            Atmosphere::ForestGreens => Color::Rgb(60, 120, 80),
        }
    }

    /// Background tint — subtle fill for panes and cards.
    pub fn bg_tint(self) -> Color {
        match self {
            Atmosphere::Default => Color::Rgb(16, 14, 12),
            Atmosphere::MintTea => Color::Rgb(12, 18, 14),
            Atmosphere::TherapeuticBlue => Color::Rgb(12, 14, 20),
            Atmosphere::LavenderCalm => Color::Rgb(16, 14, 20),
            Atmosphere::WarmAmber => Color::Rgb(18, 16, 10),
            Atmosphere::PeachSunset => Color::Rgb(20, 16, 14),
            Atmosphere::AutumnBrowns => Color::Rgb(16, 14, 12),
            Atmosphere::NeonGlow => Color::Rgb(12, 8, 14),
            Atmosphere::AuroraBorealis => Color::Rgb(8, 16, 16),
            Atmosphere::CherryBlossom => Color::Rgb(18, 14, 16),
            Atmosphere::OceanDepths => Color::Rgb(8, 10, 18),
            Atmosphere::MidnightGalaxy => Color::Rgb(8, 8, 14),
            Atmosphere::TwilightMist => Color::Rgb(12, 10, 16),
            Atmosphere::ForestGreens => Color::Rgb(10, 16, 12),
        }
    }

    /// Parse an atmosphere from a preset name (case-insensitive, underscore-tolerant).
    /// Returns `None` for unknown names and empty strings.
    pub fn from_name(name: &str) -> Option<Self> {
        let key = name.to_lowercase().replace(' ', "_");
        match key.as_str() {
            "" | "default" => Some(Atmosphere::Default),
            "mint_tea" => Some(Atmosphere::MintTea),
            "therapeutic_blue" => Some(Atmosphere::TherapeuticBlue),
            "lavender_calm" => Some(Atmosphere::LavenderCalm),
            "warm_amber" => Some(Atmosphere::WarmAmber),
            "peach_sunset" => Some(Atmosphere::PeachSunset),
            "autumn_browns" => Some(Atmosphere::AutumnBrowns),
            "neon_glow" => Some(Atmosphere::NeonGlow),
            "aurora_borealis" => Some(Atmosphere::AuroraBorealis),
            "cherry_blossom" => Some(Atmosphere::CherryBlossom),
            "ocean_depths" => Some(Atmosphere::OceanDepths),
            "midnight_galaxy" => Some(Atmosphere::MidnightGalaxy),
            "twilight_mist" => Some(Atmosphere::TwilightMist),
            "forest_greens" => Some(Atmosphere::ForestGreens),
            _ => None,
        }
    }

    /// Map a posture to a default atmosphere (when none is explicitly set).
    pub fn from_posture(posture: crate::ui::presence::Posture) -> Self {
        use crate::ui::presence::Posture;
        match posture {
            Posture::Idle => Atmosphere::Default,
            // Alert stays warm — present and ready, not doing anything cool.
            Posture::Alert => Atmosphere::Default,
            // Thinking pulls the room cool — subconscious's inward pass.
            Posture::Thinking => Atmosphere::TherapeuticBlue,
            Posture::Processing => Atmosphere::WarmAmber,
            Posture::Affectionate => Atmosphere::CherryBlossom,
            Posture::Straining => Atmosphere::TwilightMist,
            Posture::Yawning => Atmosphere::OceanDepths,
            // Listening: cool cyan tilt — alert and receptive.
            Posture::Listening => Atmosphere::TherapeuticBlue,
            // Speaking: warm amber — engaged, outward-facing.
            Posture::Speaking => Atmosphere::WarmAmber,
        }
    }

    /// Blend between two atmospheres by `t ∈ [0, 1]`. Used for smooth
    /// transitions when the agent shifts posture mid-turn.
    pub fn lerp(self, other: Atmosphere, t: f32) -> Atmosphere {
        if self == other || t <= 0.0 { return self; }
        if t >= 1.0 { return other; }
        // Return a custom-blended Atmosphere by computing interpolated colors.
        // We encode the blended result as a custom variant by returning it
        // through a static — but since we can't add runtime variants, we just
        // return one of the endpoints. A future pass could add a Custom(r,g,b)
        // variant to Atmosphere for true blending.
        if t < 0.5 { self } else { other }
    }
}

impl Default for Atmosphere {
    fn default() -> Self {
        Atmosphere::Default
    }
}
