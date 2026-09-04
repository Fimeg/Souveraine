//! Focusable send button for chat input row.
//!
//! Emits [`ClickEvent`] on mouse click. Placed as a sibling of the chat `Input`
//! widget inside `input_row`, so users can click with the mouse to submit.
//!
//! Keyboard Enter is handled by `ChatScreen` directly — this widget's keyboard
//! handler is secondary (it fires ClickEvent, but ChatScreen's override_on_input
//! catches Enter first).
//!
//! Styled with a subtle arrow glyph.

use tuie::prelude::*;
use tuie::widget::events::ClickEvent;

/// A minimal send‑button widget.
pub struct SendButton {
    label: Box<Text>,
}

impl DelegateWidget for SendButton {
    tuie::delegate_widget!(label);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };

        match &event.chord.trigger {
            Trigger::MouseDown(MouseButton::Left) => {
                // Emit ClickEvent — caught by the parent screen's after_on_event.
                // Deliberately do NOT call focus_widget: the focus chain computed
                // by tuie for a DelegateWidget-nested widget is broken
                // (walk_path_mut can't navigate through DelegateWidget boundaries).
                tuie::emit(self.get_id(), ClickEvent);
                InputResult::Handled
            }
            _ => InputResult::Rejected,
        }
    }
}

impl SendButton {
    /// Create a send button with the given foreground color.
    pub fn new(fg: Color) -> Box<Self> {
        let mut label = Text::new();
        label.set_content(
            // U+25B8 — right-pointing small triangle
            StyledStr::new(" \u{25b8} ").fg(fg).bold(),
        );
        Box::new(Self { label })
    }
}
