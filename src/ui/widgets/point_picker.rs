//! Grid-based point picker widget.

use std::cell::Cell;

use chord_macro::chord;
use tuie::prelude::*;

use crate::ui::theme;

/// Interactive grid for selecting a point cell.
pub struct PointPicker {
    layout: Layout,
    size: Cell<Vec2<u16>>,
    selected: Cell<Vec2<u16>>,
    pressed: Cell<Option<Vec2<u16>>>,
}

impl PointPicker {
    const CELL_W: u16 = 3;
    const CELL_H: u16 = 1;

    fn grid_px(&self) -> Vec2<u16> {
        let size = self.size.get();
        Vec2::new(size.x * Self::CELL_W, size.y * Self::CELL_H)
    }

    fn hit_cell(&self, pos: Vec2<i32>) -> Option<Vec2<u16>> {
        let grid = self.grid_px();
        if pos.x < 0 || pos.x >= grid.x as i32 {
            return None;
        }
        if pos.y < 0 || pos.y >= grid.y as i32 {
            return None;
        }
        Some(Vec2::new(
            (pos.x / Self::CELL_W as i32) as u16,
            (pos.y / Self::CELL_H as i32) as u16,
        ))
    }

    fn set_pressed(&self, pressed: Option<Vec2<u16>>) {
        if self.pressed.get() != pressed {
            self.pressed.set(pressed);
            tuie::dirty_paint();
        }
    }

    fn select_cell(&self, cell: Vec2<u16>) {
        if self.selected.get() != cell {
            self.selected.set(cell);
            tuie::dirty_paint();
            tuie::emit(self.get_id(), ChangeEvent(cell));
        }
    }
}

impl Widget for PointPicker {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "PointPicker"
    }

    fn measure_constraints(&mut self) -> Constraints {
        let margin = self.layout.get_margin_total();
        let grid = self.grid_px();
        let size = Vec2::new(grid.x + margin.x, grid.y + margin.y);
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
        ctx.set_style(self.layout.style);
        ctx.clear();

        let size = self.size.get();
        let selected = self.selected.get();
        let pressed = self.pressed.get();
        let focused = self.in_focus_chain();

        for row in 0..size.y {
            for col in 0..size.x {
                let cell = Vec2::new(col, row);
                let is_selected = cell == selected;
                let is_pressed = pressed == Some(cell);
                let accent = theme::get_accent_color();
                let style = if is_selected {
                    if focused {
                        Style::new().fg(accent).bold()
                    } else {
                        Style::new().fg(Color::Foreground).bold()
                    }
                } else if is_pressed {
                    Style::new().fg(accent)
                } else {
                    Style::new().fg(Color::grey256(7))
                };
                let marker = if is_selected && !is_pressed {
                    "●"
                } else {
                    "·"
                };
                let x = (col * Self::CELL_W + Self::CELL_W / 2) as i32;
                let y = (row * Self::CELL_H) as i32;
                ctx.move_to(Vec2::new(x, y));
                ctx.set_style(style);
                write!(ctx, "{}", marker);
            }
        }
    }

    fn on_state_change(&mut self, _state: WidgetState) {
        tuie::dirty_paint();
    }

    fn on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        let size = self.size.get();
        let selected = self.selected.get();

        match &event.chord {
            chord!(Left | h) => {
                if selected.x > 0 {
                    self.select_cell(Vec2::new(selected.x - 1, selected.y));
                }
            }
            chord!(Right | l) => {
                if selected.x + 1 < size.x {
                    self.select_cell(Vec2::new(selected.x + 1, selected.y));
                }
            }
            chord!(Up | k) => {
                if selected.y > 0 {
                    self.select_cell(Vec2::new(selected.x, selected.y - 1));
                }
            }
            chord!(Down | j) => {
                if selected.y + 1 < size.y {
                    self.select_cell(Vec2::new(selected.x, selected.y + 1));
                }
            }
            chord!(LeftClick) => {
                if let Some(cell) = self.hit_cell(event.pos.map(|v| v as i32)) {
                    tuie::focus_widget(self.get_id());
                    self.set_pressed(Some(cell));
                }
            }
            chord!(LeftRelease) => {
                let pressed = self.pressed.get();
                self.set_pressed(None);
                if let Some(cell) = self.hit_cell(event.pos.map(|v| v as i32)) {
                    if pressed == Some(cell) {
                        self.select_cell(cell);
                    }
                }
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }
}

impl PointPicker {
    /// Creates a [`PointPicker`] with the top-left cell selected.
    pub fn new() -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            size: Cell::new(Vec2::new(3, 3)),
            selected: Cell::new(Vec2::of(0)),
            pressed: Cell::new(None),
        })
    }

    /// Sets the initially selected `point`.
    pub fn point(self: Box<Self>, point: Vec2<u16>) -> Box<Self> {
        self.selected.set(point);
        self
    }
}
