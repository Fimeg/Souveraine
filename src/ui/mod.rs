pub mod animation;
pub mod app;
pub mod buddy;
pub mod buddy_panel;
pub mod chat;
pub mod cockpit_panel;
pub mod component;
pub mod markdown;

pub use app::App;
pub use buddy::{BuddyState, CompanionSprite, BuddyPosition};
pub use buddy_panel::BuddyPanel;
pub use cockpit_panel::CockpitPane;
pub use component::{Component, Scene, SceneLayout, TuiEvent};
