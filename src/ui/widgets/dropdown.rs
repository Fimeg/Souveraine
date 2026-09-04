#![allow(dead_code)] // WIP scaffolding not yet wired
//! Dropdown select — a compact trigger that opens a floating popup menu.
//!
//! The trigger renders `value ▾` and, on activation, opens a tuie [`Popup`]
//! anchored below it containing one [`FlatButton`] per option. Selecting an
//! option emits `ChangeEvent<usize>` from the dropdown's own id (so a parent
//! can map it back to a field) and closes the popup. Best for medium-length
//! enums; very long lists (model catalogs) are better served by a full picker.
//!
//! [`FlatButton`]: crate::ui::widgets::flat_button::FlatButton

use std::cell::Cell;
use std::rc::Rc;

use tuie::input::key::Key;
use tuie::input::mouse::MouseButton;
use tuie::input::trigger::Trigger;
use tuie::prelude::*;

use crate::ui::widgets::flat_button::FlatButton;
use crate::ui::widgets::focus_pane::FocusPane;

/// Self-contained popup host: forwards its subtree's events to a closure. This
/// is how the selected option travels back out of the (separately-rooted)
/// popup widget tree.
struct MenuHost {
    root: Box<Pane>,
    on_event: Box<dyn Fn(&WidgetEvent)>,
}

impl MenuHost {
    fn new(root: Box<Pane>, on_event: impl Fn(&WidgetEvent) + 'static) -> Box<Self> {
        Box::new(Self {
            root,
            on_event: Box::new(on_event),
        })
    }
}

impl DelegateWidget for MenuHost {
    tuie::delegate_widget!(root);

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        (self.on_event)(event);
    }
}

/// Compact select control that opens a popup menu of options.
pub struct Dropdown {
    trigger: Box<FocusPane>,
    labels: Vec<String>,
    selected: usize,
    accent: Color,
}

impl DelegateWidget for Dropdown {
    tuie::delegate_widget!(trigger);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        match &event.chord.trigger {
            Trigger::Key(Key::Enter) => {
                self.open_menu();
            }
            Trigger::MouseDown(MouseButton::Left) => {
                tuie::focus_widget(self.get_id());
            }
            Trigger::MouseUp(MouseButton::Left) => {
                let size = self.get_rect_size();
                let inside = Axis2D::all(|a| event.pos[a] >= 0.0 && event.pos[a] < size[a] as f32);
                if inside {
                    self.open_menu();
                }
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }

    fn override_is_focusable(&self) -> bool {
        true
    }
}

impl Dropdown {
    /// Creates a dropdown over `labels` with `selected` initially chosen.
    pub fn new(labels: &[&str], selected: usize) -> Box<Self> {
        let label = labels.get(selected).copied().unwrap_or("");
        let text = Text::new().content(format!(" {label} \u{25be}"));
        let mut trigger = FocusPane::new();
        trigger.add_child(text as Box<dyn Widget>);
        Box::new(Self {
            trigger,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            selected,
            accent: Color::YELLOW,
        })
    }

    /// Returns the currently selected index.
    pub fn get_selected(&self) -> usize {
        self.selected
    }

    fn open_menu(&self) {
        let dd_id = self.get_id();
        let accent = self.accent;
        let mut row_ids: Vec<WidgetId> = Vec::new();
        let mut list = Pane::new().vertical().gap(0);
        for (i, label) in self.labels.iter().enumerate() {
            let mut id = WidgetId::EMPTY;
            let marker = if i == self.selected { "\u{203a}" } else { " " };
            let row = FlatButton::new()
                .child(Text::new().content(format!(" {marker} {label} ")))
                .id(&mut id);
            row_ids.push(id.untyped());
            list = list.children([row as Box<dyn Widget>]);
        }

        let body = Pane::new()
            .vertical()
            .bordered()
            .border_style(Style::new().fg(accent).dim())
            .style(Style::new().bg(Color::grey256(3)))
            .max_height(14)
            .children([list as Box<dyn Widget>]);

        let popup_slot: Rc<Cell<Option<WidgetId>>> = Rc::new(Cell::new(None));
        let slot = popup_slot.clone();
        let host = MenuHost::new(body, move |event| {
            if event.of::<ClickEvent>() {
                if let Some(idx) = row_ids.iter().position(|&id| event.source == id) {
                    tuie::emit(dd_id, ChangeEvent(idx));
                    if let Some(pid) = slot.get() {
                        tuie::close_popup(pid);
                    }
                }
            }
        });
        popup_slot.set(Some(host.get_id().untyped()));
        tuie::open_popup(
            Popup::new(host as Box<dyn Widget>)
                .placement(Placement::side(
                    Direction2D::Down,
                    Sign::Positive,
                    Align::Start,
                ))
                .dismissible(),
        );
    }
}
