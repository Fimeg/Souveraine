#![allow(dead_code)] // WIP scaffolding not yet wired
//! App-wide key chord handler.

use chord_macro::chord;
use tuie::prelude::*;

/// Root widget wrapper that intercepts app-wide key chords.
pub struct GlobalChords {
    inner: Box<dyn Widget>,
}

impl DelegateWidget for GlobalChords {
    tuie::delegate_widget!(inner);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(Tab) if queue.is_unhandled() => {
                queue.next();
                tuie::focus_next_tab_order(Sign::Positive);
            }
            chord!(Shift + Tab) if queue.is_unhandled() => {
                queue.next();
                tuie::focus_next_tab_order(Sign::Negative);
            }
            chord!(Ctrl + z) => {
                queue.next();
                tuie::suspend();
            }
            #[cfg(feature = "gui")]
            chord!(Ctrl + Char('+')) => {
                queue.next();
                let cur = tuie::gui::config::get().font_size;
                tuie::gui::set_font_size((cur + 1.0).min(72.0));
            }
            #[cfg(feature = "gui")]
            chord!(Ctrl + Char('-')) => {
                queue.next();
                let cur = tuie::gui::config::get().font_size;
                tuie::gui::set_font_size((cur - 1.0).max(6.0));
            }
            chord!(Ctrl + (c | q)) => {
                queue.next();
                tuie::quit(0);
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }
}

impl GlobalChords {
    /// Wraps `inner` in a [`GlobalChords`] handler.
    pub fn new(inner: Box<dyn Widget>) -> Box<Self> {
        Box::new(Self { inner })
    }
}
