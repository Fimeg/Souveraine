//! Bordered focusable button widget.

use chord_macro::chord;
use tuie::{delegate_field, prelude::*};

use crate::ui::widgets::focus_pane::FocusPane;

/// Bordered focusable button that emits [`ClickEvent`].
pub struct Button {
    focus_pane: Box<FocusPane>,
}

impl DelegateWidget for Button {
    tuie::delegate_widget!(focus_pane);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(Enter) => {
                tuie::emit(self.get_id(), ClickEvent);
            }
            chord!(LeftClick) => {
                tuie::focus_widget(self.get_id());
            }
            chord!(LeftRelease) => {
                let size = self.get_rect_size();
                let released_inside = Axis2D::all(|axis| {
                    event.pos[axis] >= 0.0 && event.pos[axis] < size[axis] as f32
                });
                if released_inside {
                    tuie::emit(self.get_id(), ClickEvent);
                }
            }
            chord!(LeftDrag) => {}
            chord!(Hover) => {}
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }

    fn override_is_focusable(&self) -> bool {
        true
    }
}

impl Button {
    /// Creates an empty button.
    pub fn new() -> Box<Self> {
        Box::new(Self {
            focus_pane: FocusPane::new(),
        })
    }

    /// Adds the given children to the button in order.
    pub fn children<const N: usize>(
        mut self: Box<Self>,
        children: [Box<dyn Widget>; N],
    ) -> Box<Self> {
        for child in children {
            self.focus_pane.add_child(child);
        }
        self
    }

    delegate_field!(border_style: Style => focus_pane);
    delegate_field!(selected_border_style: Option<Style> => focus_pane);
}
