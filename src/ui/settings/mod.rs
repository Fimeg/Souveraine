//! TUI settings editor — browse and edit `souveraine.toml` live.
//!
//! Two-panel layout: categories (left) · fields (right). Edits are made on a
//! cloned copy of the config; Ctrl+S persists to disk and propagates to the
//! running system. Esc with unsaved changes prompts for confirmation.

mod types;
mod view;
mod key_handling;
mod draw;

pub use types::{PanelFocus, Category, FieldLoc, EditableValue, SettingsMode};
pub use view::{ActiveAgentSettings, SettingsView};
pub use key_handling::SettingsAction;
pub use draw::draw;
