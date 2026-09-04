//! TUI settings editor — browse and edit `souveraine.toml` live.
//!
//! Two-panel layout: categories (left) · fields (right). Edits are made on a
//! cloned copy of the config; Ctrl+S persists to disk and propagates to the
//! running system. Esc with unsaved changes prompts for confirmation.

mod draw;
mod key_handling;
mod types;
mod view;

pub use draw::draw;
pub use key_handling::SettingsAction;
pub use types::{Category, CategoryGroup, EditableValue, FieldLoc, SettingsMode};
pub use view::{ActiveAgentSettings, SettingsView};
