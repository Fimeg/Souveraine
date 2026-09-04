#![allow(dead_code)] // WIP scaffolding not yet wired
//! Horizontal toggle control with mutually exclusive labeled segments.

use std::cell::Cell;

use chord_macro::chord;
use tuie::prelude::*;

use crate::ui::theme;

/// Horizontal toggle with mutually exclusive labeled segments.
pub struct SegmentedControl {
    layout: Layout,
    labels: Vec<String>,
    selected: Cell<usize>,
    pressed: Cell<Option<usize>>,
    disabled: Cell<u32>,
}

impl SegmentedControl {
    fn hit_segment(&self, pos: Vec2<i32>) -> Option<usize> {
        let size = self.get_rect_size();
        if pos.y < 0 || pos.y >= size.y as i32 {
            return None;
        }
        let mut x: i32 = 0;
        for (i, label) in self.labels.iter().enumerate() {
            let width = label.len() as i32 + 2;
            if pos.x >= x && pos.x < x + width {
                return Some(i);
            }
            x += width;
        }
        None
    }

    fn set_pressed(&self, pressed: Option<usize>) {
        if self.pressed.get() != pressed {
            self.pressed.set(pressed);
            tuie::dirty_paint();
        }
    }

    fn is_disabled(&self, index: usize) -> bool {
        self.disabled.get() & (1 << index) != 0
    }

    fn select_index(&self, index: usize) {
        if index >= self.labels.len() || self.is_disabled(index) {
            return;
        }
        if self.selected.get() != index {
            self.selected.set(index);
            tuie::dirty_paint();
        }
        tuie::emit(self.get_id(), ChangeEvent(index));
    }

    fn step(&self, from: usize, dir: i32) -> Option<usize> {
        let n = self.labels.len() as i32;
        let mut i = from as i32 + dir;
        while i >= 0 && i < n {
            if !self.is_disabled(i as usize) {
                return Some(i as usize);
            }
            i += dir;
        }
        None
    }

    fn content_width(&self) -> u16 {
        self.labels.iter().map(|l| l.len() as u16 + 2).sum()
    }
}

impl Widget for SegmentedControl {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "SegmentedControl"
    }

    fn measure_constraints(&mut self) -> Constraints {
        let margin = self.layout.get_margin_total();
        let size = Vec2::new(self.content_width() + margin.x, 1 + margin.y);
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
        let selected_style = if self.in_focus_chain() {
            Style::new().fg(Color::BLACK).bg(accent).bold()
        } else {
            base.reverse().bold()
        };
        let pressed_style = base.fg(accent);

        ctx.set_style(base);
        ctx.clear();

        for (i, label) in self.labels.iter().enumerate() {
            let is_pressed = pressed == Some(i);
            let is_selected = i == selected;
            let is_disabled = self.is_disabled(i);
            let needs_separator = i > 0 && !is_selected && i - 1 != selected;
            let mut style = if is_selected {
                selected_style
            } else if is_pressed {
                pressed_style
            } else {
                base
            };
            if is_disabled {
                style = style.dim();
            }

            if needs_separator {
                ctx.set_style(base.fg(Color::Rgb(80, 80, 80)));
                write!(ctx, "▏");
                ctx.set_style(style);
                write!(ctx, "{} ", label);
            } else {
                ctx.set_style(style);
                write!(ctx, " {} ", label);
            }
        }
    }

    fn on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        let selected = self.selected.get();

        match &event.chord {
            chord!(Left | h) => {
                if let Some(i) = self.step(selected, -1) {
                    self.select_index(i);
                }
            }
            chord!(Right | l) => {
                if let Some(i) = self.step(selected, 1) {
                    self.select_index(i);
                }
            }
            chord!(LeftClick) => {
                if let Some(i) = self.hit_segment(event.pos.map(|v| v as i32)) {
                    if !self.is_disabled(i) {
                        tuie::focus_widget(self.get_id());
                        self.set_pressed(Some(i));
                    }
                }
            }
            chord!(LeftRelease) => {
                let pressed = self.pressed.get();
                self.set_pressed(None);
                if let Some(i) = self.hit_segment(event.pos.map(|v| v as i32)) {
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

impl SegmentedControl {
    /// Creates a [`SegmentedControl`] with one segment per label.
    pub fn new(labels: &[&str]) -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            labels: labels.iter().map(|&l| l.to_string()).collect(),
            selected: Cell::new(0),
            pressed: Cell::new(None),
            disabled: Cell::new(0),
        })
    }

    /// Sets the segment at `index` as initially disabled.
    pub fn disabled(self: Box<Self>, index: usize) -> Box<Self> {
        let mask = self.disabled.get();
        self.disabled.set(mask | (1 << index));
        self
    }

    /// Sets the initially selected segment by `index`.
    pub fn selected(self: Box<Self>, index: usize) -> Box<Self> {
        self.selected.set(index);
        self
    }

    /// Sets whether the segment at `index` is disabled.
    pub fn set_disabled(&mut self, index: usize, disabled: bool) {
        let mask = self.disabled.get();
        let new_mask = if disabled {
            mask | (1 << index)
        } else {
            mask & !(1 << index)
        };
        if new_mask != mask {
            self.disabled.set(new_mask);
            tuie::dirty_paint();
        }
    }

    /// Returns the selected segment index.
    pub fn get_selected(&self) -> usize {
        self.selected.get()
    }

    /// Sets the selected segment by `index` without emitting a [`ChangeEvent`].
    pub fn set_selected(&mut self, index: usize) {
        if index >= self.labels.len() || self.is_disabled(index) {
            return;
        }
        if self.selected.get() != index {
            self.selected.set(index);
            tuie::dirty_paint();
        }
    }
}
