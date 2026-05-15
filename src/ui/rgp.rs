//! Ratty Graphics Protocol — inline 3D rendering when running inside ratty.
//!
//! The GPU surface lives behind the terminal text. 3D objects are anchored to
//! cell regions and rendered by ratty's Bevy+wgpu pipeline. When RGP is not
//! available (any other terminal), all functions here are silent no-ops and
//! the TUI falls through to ratatui-image or the half-block silhouette.
//!
//! ## Posture → animation mapping
//!
//! Each posture maps to a set of RGP visual parameters (scale, rotation,
//! brightness, color tint, animate on/off). The mapping is applied via
//! [`Graphic::apply_posture`] which calls `update()` to push changes to
//! ratty. The model must have animation clips for `animate: true` to have
//! visible effect — otherwise the model stands still regardless.

#[cfg(feature = "rgp")]
use ratatui_ratty::{ObjectFormat, RattyGraphic, RattyGraphicSettings};

use ratatui::layout::Rect;

use crate::ui::presence::Posture;

/// Whether the current terminal supports the Ratty Graphics Protocol.
///
/// Checks `TERM_PROGRAM=ratty` — ratty sets this on spawn.
pub fn is_available() -> bool {
    std::env::var("TERM_PROGRAM")
        .map(|v| v.eq_ignore_ascii_case("ratty"))
        .unwrap_or(false)
}

/// Object IDs for the various 3D slots in the TUI.
pub mod ids {
    pub const PORTRAIT: u32 = 1;
    pub const ACCENT_LEFT: u32 = 10;
    pub const ACCENT_RIGHT: u32 = 11;
}

/// Posture-driven visual parameters for an RGP object.
///
/// Derived from the agent's current posture and applied via
/// [`RattyGraphicSettings`] fields before calling `update()`.
struct PostureParams {
    animate: bool,
    scale: f32,
    brightness: f32,
    color: Option<[u8; 3]>,
}

impl From<Posture> for PostureParams {
    fn from(p: Posture) -> Self {
        match p {
            Posture::Idle => Self {
                animate: true,
                scale: 1.0,
                brightness: 0.8,
                color: None,
            },
            Posture::Alert => Self {
                animate: true,
                scale: 1.0,
                brightness: 1.0,
                color: None,
            },
            Posture::Thinking => Self {
                animate: false,
                scale: 1.0,
                brightness: 0.7,
                color: Some([80, 140, 200]),
            },
            Posture::Processing => Self {
                animate: false,
                scale: 1.0,
                brightness: 1.1,
                color: Some([255, 180, 80]),
            },
            Posture::Affectionate => Self {
                animate: true,
                scale: 1.0,
                brightness: 0.9,
                color: Some([255, 180, 200]),
            },
            Posture::Straining => Self {
                animate: false,
                scale: 0.95,
                brightness: 0.5,
                color: Some([100, 80, 120]),
            },
            Posture::Yawning => Self {
                animate: true,
                scale: 0.95,
                brightness: 0.6,
                color: Some([60, 80, 120]),
            },
            Posture::Listening => Self {
                animate: false,
                scale: 1.0,
                brightness: 1.0,
                color: Some([80, 200, 220]),
            },
            Posture::Speaking => Self {
                animate: true,
                scale: 1.0,
                brightness: 1.1,
                color: Some([255, 200, 120]),
            },
        }
    }
}

/// A managed 3D object that can be placed, updated, and cleared.
///
/// When compiled without `rgp` feature or when not running in ratty,
/// all methods are silent no-ops.
pub struct Graphic {
    #[cfg(feature = "rgp")]
    inner: Option<RattyGraphic<'static>>,
    #[cfg(not(feature = "rgp"))]
    _phantom: (),
    registered: bool,
    /// Last posture applied — used to skip redundant `update()` calls.
    last_posture: Option<Posture>,
}

impl Graphic {
    /// Create a new graphic for a GLB asset.
    #[cfg(feature = "rgp")]
    pub fn from_glb(id: u32, path: &str) -> Self {
        if !is_available() {
            return Self { inner: None, registered: false, last_posture: None };
        }
        let settings = RattyGraphicSettings::new(path.to_string())
            .id(id)
            .format(ObjectFormat::Glb)
            .animate(true)
            .scale(1.0)
            .brightness(0.8);
        Self {
            inner: Some(RattyGraphic::new(settings)),
            registered: false,
            last_posture: None,
        }
    }

    #[cfg(not(feature = "rgp"))]
    pub fn from_glb(_id: u32, _path: &str) -> Self {
        Self { _phantom: (), registered: false, last_posture: None }
    }

    /// Create a graphic from in-memory GLB bytes.
    #[cfg(feature = "rgp")]
    pub fn from_glb_bytes(id: u32, name: &str, bytes: &[u8]) -> Self {
        if !is_available() {
            return Self { inner: None, registered: false, last_posture: None };
        }
        let settings = RattyGraphicSettings::new(name.to_string())
            .id(id)
            .format(ObjectFormat::Glb)
            .animate(true)
            .scale(1.0)
            .brightness(0.8);
        let graphic = RattyGraphic::new(settings);
        let _ = graphic.register_payload(bytes);
        Self {
            inner: Some(graphic),
            registered: true,
            last_posture: None,
        }
    }

    #[cfg(not(feature = "rgp"))]
    pub fn from_glb_bytes(_id: u32, _name: &str, _bytes: &[u8]) -> Self {
        Self { _phantom: (), registered: false, last_posture: None }
    }

    /// Register the asset with ratty (sends the file path).
    pub fn register(&mut self) {
        #[cfg(feature = "rgp")]
        if let Some(ref g) = self.inner {
            if !self.registered {
                let _ = g.register();
                self.registered = true;
            }
        }
    }

    /// Render the 3D object into a ratatui buffer area.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        #[cfg(feature = "rgp")]
        if let Some(ref g) = self.inner {
            use ratatui::widgets::Widget;
            g.render(area, buf);
        }
        let _ = (area, buf);
    }

    /// Update properties (call after mutating settings).
    pub fn update(&self) {
        #[cfg(feature = "rgp")]
        if let Some(ref g) = self.inner {
            let _ = g.update();
        }
    }

    /// Set scale.
    pub fn set_scale(&mut self, scale: f32) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            g.settings_mut().scale = scale;
        }
        let _ = scale;
    }

    /// Set rotation (degrees).
    pub fn set_rotation(&mut self, rotation: [f32; 3]) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            g.settings_mut().rotation = rotation;
        }
        let _ = rotation;
    }

    /// Set brightness.
    pub fn set_brightness(&mut self, brightness: f32) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            g.settings_mut().brightness = brightness;
        }
        let _ = brightness;
    }

    /// Set color tint.
    pub fn set_color(&mut self, rgb: [u8; 3]) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            g.settings_mut().color = Some(rgb);
        }
        let _ = rgb;
    }

    /// Set animation on/off.
    pub fn set_animate(&mut self, animate: bool) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            g.settings_mut().animate = animate;
        }
        let _ = animate;
    }

    /// Remove the 3D object from the screen.
    pub fn clear(&self) {
        #[cfg(feature = "rgp")]
        if let Some(ref g) = self.inner {
            let _ = g.clear();
        }
    }

    /// Whether this graphic is backed by a real RGP widget.
    pub fn is_active(&self) -> bool {
        #[cfg(feature = "rgp")]
        { self.inner.is_some() }
        #[cfg(not(feature = "rgp"))]
        { false }
    }

    /// Apply posture-driven visual parameters (scale, brightness, color, animate).
    /// Skips the update if the posture hasn't changed since last call.
    /// Safe to call on every tick — the `last_posture` check is cheap.
    pub fn apply_posture(&mut self, posture: Posture) {
        #[cfg(feature = "rgp")]
        if let Some(ref mut g) = self.inner {
            if self.last_posture == Some(posture) {
                return;
            }
            let params = PostureParams::from(posture);
            g.settings_mut().animate = params.animate;
            g.settings_mut().scale = params.scale;
            g.settings_mut().brightness = params.brightness;
            g.settings_mut().color = params.color;
            let _ = g.update();
            self.last_posture = Some(posture);
        }
        let _ = posture;
    }
}

impl Drop for Graphic {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Load a portrait GLB from an agent's assets directory if present.
///
/// Looks for `assets/portrait.glb` or `assets/model.glb` in the agent's memfs.
pub fn load_portrait_glb(agent_assets_dir: &std::path::Path) -> Option<Graphic> {
    if !is_available() {
        return None;
    }
    for name in ["portrait.glb", "model.glb"] {
        let p = agent_assets_dir.join(name);
        if p.exists() {
            let mut g = Graphic::from_glb(ids::PORTRAIT, &p.to_string_lossy());
            g.register();
            return Some(g);
        }
    }
    None
}
