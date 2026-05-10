pub mod animation;
pub mod app;
pub mod buddy;
pub mod chat;
pub mod component;
pub mod markdown;

pub use app::App;
pub use buddy::{BuddyState, CompanionSprite, BuddyPosition};
pub use component::{Component, Scene, SceneLayout, TuiEvent};
