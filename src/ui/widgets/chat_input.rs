//! Chat input — helper functions for the bare tuie `Input` widget.
//!
//! Following tuie-demo: the `Input` is placed as a direct child of a `Pane`
//! (not wrapped in a `DelegateWidget`), and the editable input we read by id
//! carries **no** placeholder (the only placeholdered input in tuie-demo is a
//! decorative one whose text is never read — see `new_chat_input`).
//!
//! `ChatScreen::override_on_input` intercepts a bare Enter to submit (reading
//! the current text via `read_input_text` and clearing the input). Alt+Enter /
//! Ctrl+Enter insert a literal newline via [`insert_newline`]. All other keys
//! fall through to the `Input` via the focus chain for normal editing.

use tuie::prelude::*;

/// Create a chat Input widget wired for submit-on-Enter.
///
/// The Input is **multiline**.  `ChatScreen` intercepts a bare Enter (no
/// modifier) to submit, consuming it before it reaches the Input.  For
/// Alt+Enter / Ctrl+Enter it calls [`insert_newline`] to add a literal
/// newline, because the default editor bindings only newline on a *bare*
/// Enter (which never reaches them here).
///
/// We deliberately do **not** set a tuie placeholder.  `Input`'s
/// `get_delegate()` returns the *placeholder* `Text` while empty and the
/// *content* `Text` once non-empty; because `get_id()` forwards through the
/// delegate, a placeholder makes the Input's id flip at the empty↔non-empty
/// boundary.  Any id we capture for focus/lookups then goes stale on the first
/// keystroke, and keyboard routing within a single input batch breaks (the
/// focus chain still points at the old id).  No placeholder ⇒ stable id.
pub fn new_chat_input() -> (Box<Input>, WidgetId<Input>) {
    let mut input = Input::new();
    input.set_multiline(true);
    let input_id = input.get_id();
    (input, input_id)
}

/// Get the current text from the Input widget.
pub fn get_input_text(input: &Input) -> String {
    let s = input.get_string();
    // Strip trailing newline if Input captured Enter before TuieApp could.
    s.trim_end_matches('\n').to_string()
}

/// Get the current text from an Input stored in a pane tree.
pub fn read_input_text(root: &dyn Widget, input_id: WidgetId<Input>) -> String {
    root.get_widget(input_id)
        .map(get_input_text)
        .unwrap_or_default()
}

/// Clear the Input widget.
pub fn clear_input(root: &mut dyn Widget, input_id: WidgetId<Input>) {
    if let Some(input) = root.get_widget_mut(input_id) {
        input.set_content("");
    }
}

/// Insert a literal newline at the cursor.
///
/// The default editor bindings only insert `\n` for a *bare* `Enter`
/// (`chord!(Enter)`), and `ChatScreen` intercepts bare Enter to submit.  So a
/// modifier-Enter (Alt/Ctrl) never reaches the editor as a newline — we insert
/// it directly here instead.
pub fn insert_newline(root: &mut dyn Widget, input_id: WidgetId<Input>) {
    if let Some(input) = root.get_widget_mut(input_id) {
        let (editor, text) = input.get_editor_mut();
        editor.insert_char(text, '\n');
        input.dirty_layout();
    }
}
