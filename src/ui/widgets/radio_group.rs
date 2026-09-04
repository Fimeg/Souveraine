//! Vertical radio group control.

use std::cell::Cell;

use chord_macro::chord;
use tuie::prelude::*;

use crate::ui::theme;

/// Vertical group of mutually exclusive labeled radio options.
pub struct RadioGroup {
    layout: Layout,
    labels: Vec<String>,
    selected: Cell<usize>,
    pressed: Cell<Option<usize>>,
}

impl RadioGroup {
    fn hit_option(&self, pos: Vec2<i32>) -> Option<usize> {
        let size = self.get_rect_size();
        if pos.x < 0 || pos.x >= size.x as i32 {
            return None;
        }
        if pos.y < 0 || (pos.y as usize) >= self.labels.len() {
            return None;
        }
        Some(pos.y as usize)
    }

    fn set_pressed(&self, pressed: Option<usize>) {
        if self.pressed.get() != pressed {
            self.pressed.set(pressed);
            tuie::dirty_paint();
        }
    }

    fn select_index(&self, index: usize) {
        if index >= self.labels.len() {
            return;
        }
        if self.selected.get() != index {
            self.selected.set(index);
            tuie::dirty_paint();
        }
        tuie::emit(self.get_id(), ChangeEvent(index));
    }

    fn content_width(&self) -> u16 {
        self.labels
            .iter()
            .map(|l| l.len() as u16)
            .max()
            .unwrap_or(0)
            + 4
    }
}

impl Widget for RadioGroup {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "RadioGroup"
    }

    fn measure_constraints(&mut self) -> Constraints {
        let margin = self.layout.get_margin_total();
        let size = Vec2::new(
            self.content_width() + margin.x,
            self.labels.len() as u16 + margin.y,
        );
        Constraints {
            min_size: size,
            max_size: size,
            preferred_size: size,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn render(&self, mut ctx: RenderContext) {
        let selected = self.selected.get();
        let pressed = self.pressed.get();
        let base = self.layout.style;
        let accent = theme::get_accent_color();
        let marker_style = if self.in_focus_chain() {
            base.fg(accent).bold()
        } else {
            base.bold()
        };
        let pressed_style = base.fg(accent);

        ctx.set_style(base);
        ctx.clear();

        for (i, label) in self.labels.iter().enumerate() {
            let is_pressed = pressed == Some(i);
            let is_selected = i == selected;
            let marker = if is_selected { "(*)" } else { "( )" };
            let row_style = if is_pressed {
                pressed_style
            } else if is_selected {
                marker_style
            } else {
                base
            };

            ctx.move_to((0, i as i32).into());
            ctx.set_style(row_style);
            write!(ctx, "{}", marker);
            ctx.set_style(base);
            write!(ctx, " {}", label);
        }
    }

    fn on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        let count = self.labels.len();
        let selected = self.selected.get();

        match &event.chord {
            chord!(Up | k) => {
                if selected > 0 {
                    self.select_index(selected - 1);
                }
            }
            chord!(Down | j) => {
                if selected + 1 < count {
                    self.select_index(selected + 1);
                }
            }
            chord!(LeftClick) => {
                if let Some(i) = self.hit_option(event.pos.map(|v| v as i32)) {
                    tuie::focus_widget(self.get_id());
                    self.set_pressed(Some(i));
                }
            }
            chord!(LeftRelease) => {
                let pressed = self.pressed.get();
                self.set_pressed(None);
                if let Some(i) = self.hit_option(event.pos.map(|v| v as i32)) {
                    if pressed == Some(i) {
                        self.select_index(i);
                    }
                }
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }
}

impl RadioGroup {
    /// Creates a [`RadioGroup`] with one option per label.
    pub fn new(labels: &[&str]) -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            labels: labels.iter().map(|&l| l.to_string()).collect(),
            selected: Cell::new(0),
            pressed: Cell::new(None),
        })
    }

    /// Sets the initially selected option by `index`.
    pub fn selected(self: Box<Self>, index: usize) -> Box<Self> {
        self.selected.set(index);
        self
    }
}
