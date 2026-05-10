//! TUI Component System — trait, events, scene graph.
//!
//! The `App` no longer knows what specific panels exist. It holds a `Scene`
//! which owns a layout strategy and a `Vec<Box<dyn Component>>`. Events arrive
//! as `TuiEvent` variants and are dispatched to every component. The Scene
//! splits the terminal area into zones per its layout and calls each
//! component's `render`.
//!
//! ## Adding a new panel
//!
//! 1. Define a struct and implement `Component` for it
//! 2. `Box::new(YourPanel)` into a Scene
//! 3. The panel receives events and renders into its zone
//!
//! No changes to `App`, the event loop, or other components.

use ratatui::layout::Rect;
use ratatui::Frame;

// ── Events ──────────────────────────────────────────────────────

/// Every event the TUI can react to. Add variants, never break them.
/// Components opt in to what they care about via `handle_event`.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    // ── Input ──────────────────────────────────────────────────
    Key(crossterm::event::KeyEvent),
    Resize { width: u16, height: u16 },

    // ── Streaming & surfacing ──────────────────────────────────
    /// A streaming token from the LLM response.
    Token { text: String },
    /// A subconscious surfacing item (N+1 detect).
    Surfacing {
        source: String,
        content: String,
        priority: String,
    },
    /// N+25 reflection event.
    Reflection { content: String },
    /// N+100 archivist / context pressure event.
    Archivist {
        synthesis: String,
        pressure: f32,
    },

    // ── Agent & subagent lifecycle ─────────────────────────────
    /// A subagent fork has been created, updated, or completed.
    SubagentUpdate {
        id: String,
        status: String,
        output: Option<String>,
    },
    /// The active agent has changed.
    AgentSelected(String),

    // ── State changes ──────────────────────────────────────────
    /// The active screen changed (e.g. splash → welcome).
    ScreenChanged(super::app::Screen),
    /// Mood hint from the agent or the consciousness engine.
    MoodChanged(String),
    /// Energy level change (0-100) from agent state.
    EnergyChanged(u8),
    /// Context pressure from the conversation engine.
    PressureChanged(f32),
    /// Compaction pressure warning (advisory, 3-tier).
    CompactionWarning { pressure: f32, tier: u8 },
    /// Backend connectivity status.
    BackendStatus { mode: String, healthy: bool },

    // ── Animation tick ─────────────────────────────────────────
    /// Monotonic tick counter, increments every frame.
    Tick(u64),
}

// ── Component trait ─────────────────────────────────────────────

/// Something that lives on screen — a panel, an overlay, a presence.
///
/// Each component receives events and renders into its allocated zone.
/// Components do not know about each other. They share nothing but the
/// event stream and their assigned screen area.
pub trait Component {
    /// A unique name for debugging and event routing.
    fn name(&self) -> &str;

    /// Handle a UI event. Return true if a redraw is needed.
    fn handle_event(&mut self, event: &TuiEvent) -> bool;

    /// Render into the given area. The `Scene` allocates regions.
    fn render(&self, area: Rect, frame: &mut Frame);
}

// ── Scene layout ────────────────────────────────────────────────

/// How the scene's components are arranged on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneLayout {
    /// One component fills the full terminal (Splash, Welcome).
    Single,
    /// Main chat + optional right sidebar.
    ChatWithSidebar {
        /// Fraction of width given to the sidebar (0.0 – 1.0).
        sidebar_ratio: f32,
        /// Whether the sidebar is currently visible.
        sidebar_open: bool,
    },
    /// 2×2 grid for system overview (Dashboard).
    Dashboard,
    /// Full-screen agent picker overlay.
    AgentPicker,
}

impl Default for SceneLayout {
    fn default() -> Self {
        Self::ChatWithSidebar {
            sidebar_ratio: 0.3,
            sidebar_open: false,
        }
    }
}

impl SceneLayout {
    /// Split `area` into zones corresponding to each component index.
    ///
    /// Returns `Vec<Rect>` — one per component. The length equals the
    /// number of components for the current layout.
    pub fn split(&self, area: Rect, component_count: usize) -> Vec<Rect> {
        match self {
            Self::Single => {
                if component_count == 0 { return Vec::new(); }
                vec![area]
            }
            Self::ChatWithSidebar { sidebar_ratio, sidebar_open } => {
                if component_count == 0 { return Vec::new(); }
                if !sidebar_open || component_count == 1 {
                    // Main only — no sidebar visible
                    return vec![area];
                }
                let min_sidebar = 20u16;
                let sidebar_w = ((area.width as f32) * sidebar_ratio).round() as u16;
                let sidebar_w = sidebar_w.max(min_sidebar);

                if sidebar_w >= area.width {
                    // Not enough room for both — main gets everything
                    return vec![area];
                }

                let main_w = area.width - sidebar_w;
                // Components: [0] = chat panel, [1..] = sidebar panels
                let mut zones = Vec::with_capacity(component_count);

                // Chat panel gets the main area
                let main_area = Rect::new(area.x, area.y, main_w, area.height);
                zones.push(main_area);

                // Remaining components share the sidebar vertically
                let sidebar_count = (component_count - 1) as u16;
                if sidebar_count > 0 {
                    let side_area = Rect::new(area.x + main_w, area.y, sidebar_w, area.height);
                    let row_height = side_area.height / sidebar_count;
                    for i in 0..sidebar_count {
                        let row_y = side_area.y + i * row_height;
                        let h = if i == sidebar_count - 1 {
                            side_area.height - i * row_height
                        } else {
                            row_height
                        };
                        zones.push(Rect::new(side_area.x, row_y, side_area.width, h));
                    }
                }

                zones
            }
            Self::Dashboard => {
                // 2×2 grid
                let half_w = area.width / 2;
                let half_h = area.height / 2;
                vec![
                    Rect::new(area.x, area.y, half_w, half_h),
                    Rect::new(area.x + half_w, area.y, area.width - half_w, half_h),
                    Rect::new(area.x, area.y + half_h, half_w, area.height - half_h),
                    Rect::new(area.x + half_w, area.y + half_h, area.width - half_w, area.height - half_h),
                ]
            }
            Self::AgentPicker => {
                if component_count == 0 { return Vec::new(); }
                vec![area]
            }
        }
    }
}

// ── Scene ───────────────────────────────────────────────────────

/// A scene owns a layout strategy and all components that draw into it.
pub struct Scene {
    /// How the terminal area is divided.
    pub layout: SceneLayout,
    /// All components in this scene, in render order.
    pub components: Vec<Box<dyn Component>>,
}

impl Scene {
    pub fn new(layout: SceneLayout) -> Self {
        Self {
            layout,
            components: Vec::new(),
        }
    }

    /// Register a component. Called during scene construction.
    pub fn add(&mut self, component: impl Component + 'static) {
        self.components.push(Box::new(component));
    }

    /// Dispatch an event to all components.
    /// Returns true if any component requested a redraw.
    pub fn event_all(&mut self, event: &TuiEvent) -> bool {
        let mut dirty = false;
        for comp in &mut self.components {
            if comp.handle_event(event) {
                dirty = true;
            }
        }
        dirty
    }

    /// Render all components into their allocated zones.
    pub fn render_all(&self, area: Rect, frame: &mut Frame) {
        let zones = self.layout.split(area, self.components.len());
        for (component, zone) in self.components.iter().zip(zones.iter()) {
            component.render(*zone, frame);
        }
    }
}
