//! Ratty Graphics Protocol — inline 3D rendering when running inside ratty.
//!
//! The GPU surface lives behind the terminal text. 3D objects are anchored to
//! cell regions and rendered by ratty's Bevy+wgpu pipeline. When RGP is not
//! available (any other terminal), all functions here are silent no-ops and
//! the TUI falls through to ratatui-image or the half-block silhouette.

#[cfg(feature = "rgp")]
use ratatui_ratty::{ObjectFormat, RattyGraphic, RattyGraphicSettings};

use ratatui::layout::Rect;

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
}

impl Graphic {
    /// Create a new graphic for a GLB asset.
    #[cfg(feature = "rgp")]
    pub fn from_glb(id: u32, path: &str) -> Self {
        if !is_available() {
            return Self { inner: None, registered: false };
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
        }
    }

    #[cfg(not(feature = "rgp"))]
    pub fn from_glb(_id: u32, _path: &str) -> Self {
        Self { _phantom: (), registered: false }
    }

    /// Create a graphic from in-memory GLB bytes.
    #[cfg(feature = "rgp")]
    pub fn from_glb_bytes(id: u32, name: &str, bytes: &[u8]) -> Self {
        if !is_available() {
            return Self { inner: None, registered: false };
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
        }
    }

    #[cfg(not(feature = "rgp"))]
    pub fn from_glb_bytes(_id: u32, _name: &str, _bytes: &[u8]) -> Self {
        Self { _phantom: (), registered: false }
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
