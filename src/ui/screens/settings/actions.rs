#![allow(dead_code)] // WIP scaffolding not yet wired
//! Settings action dispatch — maps widget events to settings mutations.
//!
//! Each action corresponds to a user interaction with a settings field widget.
//! The SettingsScreen processes actions in `after_on_event` and applies them
//! to the shared [`SettingsView`].

use crate::ui::settings::FieldLoc;

/// Actions that can be performed in the settings screen.
#[derive(Debug, Clone)]
pub enum SettingsAction {
    /// Toggle a bool field.
    ToggleBool(FieldLoc),
    /// Cycle an enum field by a signed delta.
    CycleEnum(FieldLoc, i32),
    /// Adjust a numeric field by a signed delta.
    AdjustNumber(FieldLoc, i32),
    /// Commit a text field's current value.
    CommitText(FieldLoc, String),
    /// Focus the fields column from categories.
    FocusFields,
    /// Focus the categories column from fields.
    FocusCategories,
    /// Move selection up or down in the focused column.
    MoveSelection(i32),
    /// Save the config to disk.
    Save,
    /// Save and return to the welcome screen.
    SaveAndGoBack,
    /// Discard changes and return to the welcome screen.
    Discard,
    /// Fetch available models from the provider.
    FetchModels,
    /// Models have been fetched (callback from async).
    ModelsFetched(Vec<String>),
    /// Push a model picker sub-page for the given field.
    OpenModelPicker(FieldLoc),
    /// Select a model from the picker and apply it.
    SelectModel(FieldLoc, String),
    /// Push a text editor sub-page for the given field.
    OpenTextEditor(FieldLoc),
    /// Pop the current PageLayout sub-page.
    PopPage,
    /// No action.
    None,
}
