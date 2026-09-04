//! Horizontal slider for a bounded scalar (e.g. a 0.0–1.0 float).
//!
//! Values are carried as integers in `min..=max` (the field grid scales floats
//! by 100, so 0.70 travels as `70`). Emits `ChangeEvent<i32>` carrying the new
//! absolute value whenever it changes — by arrow key, click, or drag.

use std::cell::Cell;

use tuie::input::key::Key;
use tuie::input::mouse::MouseButton;
use tuie::input::trigger::Trigger;
use tuie::prelude::*;

const TRACK_WIDTH: u16 = 16;
/// Eighth-block ramp used to render sub-cell fill at the leading edge.
const EIGHTHS: [&str; 8] = [
    "\u{258f}", "\u{258e}", "\u{258d}", "\u{258c}", "\u{258b}", "\u{258a}", "\u{2589}", "\u{2588}",
];

/// Horizontal slider over an integer range. Renders a filled track followed by
/// the value as a two-decimal fraction (`value / 100`).
pub struct Slider {
    layout: Layout,
    value: Cell<i32>,
    min: i32,
    max: i32,
    dragging: Cell<bool>,
    accent: Color,
    track: Color,
}

impl Slider {
    fn fraction(&self) -> f32 {
        let span = (self.max - self.min).max(1) as f32;
        ((self.value.get() - self.min) as f32 / span).clamp(0.0, 1.0)
    }

    fn set_value(&self, value: i32) {
        let clamped = value.clamp(self.min, self.max);
        if self.value.get() != clamped {
            self.value.set(clamped);
            tuie::dirty_paint();
            tuie::emit(self.get_id(), ChangeEvent(clamped));
        }
    }

    /// Maps a click x-position over the track to a value.
    fn value_at_x(&self, x: i32) -> i32 {
        let clamped = x.clamp(0, TRACK_WIDTH as i32);
        let frac = clamped as f32 / TRACK_WIDTH as f32;
        self.min + (frac * (self.max - self.min) as f32).round() as i32
    }
}

impl Widget for Slider {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "Slider"
    }

    fn measure_constraints(&mut self) -> Constraints {
        let margin = self.layout.get_margin_total();
        // track + space + "0.00"
        let size = Vec2::new(TRACK_WIDTH + 5 + margin.x, 1 + margin.y);
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
        let base = self.layout.style;
        ctx.set_style(base);
        ctx.clear();

        let focused = self.in_focus_chain();
        let fill_color = if focused {
            self.accent
        } else {
            base.get_fg().unwrap_or(self.accent)
        };
        let eighths = (self.fraction() * TRACK_WIDTH as f32 * 8.0).round() as u32;
        let full = (eighths / 8) as u16;
        let remainder = (eighths % 8) as usize;

        ctx.move_to((0, 0).into());
        ctx.set_style(base.fg(fill_color));
        for _ in 0..full.min(TRACK_WIDTH) {
            write!(ctx, "\u{2588}");
        }
        let mut drawn = full.min(TRACK_WIDTH);
        if remainder > 0 && drawn < TRACK_WIDTH {
            write!(ctx, "{}", EIGHTHS[remainder - 1]);
            drawn += 1;
        }
        ctx.set_style(base.fg(self.track));
        for _ in drawn..TRACK_WIDTH {
            write!(ctx, "\u{2591}");
        }

        let value = self.value.get() as f32 / 100.0;
        ctx.set_style(if focused {
            base.fg(self.accent).bold()
        } else {
            base
        });
        write!(ctx, " {:.2}", value);
    }

    fn on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        let value = self.value.get();
        match &event.chord.trigger {
            Trigger::Key(Key::Arrow(Direction2D::Left)) => {
                self.set_value(value - 1);
            }
            Trigger::Key(Key::Arrow(Direction2D::Right)) => {
                self.set_value(value + 1);
            }
            Trigger::MouseDown(MouseButton::Left) => {
                tuie::focus_widget(self.get_id());
                self.dragging.set(true);
                self.set_value(self.value_at_x(event.pos.x as i32));
            }
            Trigger::MouseDrag(MouseButton::Left) => {
                if self.dragging.get() {
                    self.set_value(self.value_at_x(event.pos.x as i32));
                }
            }
            Trigger::MouseUp(MouseButton::Left) => {
                self.dragging.set(false);
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }
}

impl Slider {
    /// Creates a slider over `min..=max` with the given `accent` color.
    pub fn new(min: i32, max: i32, accent: Color) -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            value: Cell::new(min),
            min,
            max,
            dragging: Cell::new(false),
            accent,
            track: Color::grey256(6),
        })
    }

    /// Sets the initial value.
    pub fn value(self: Box<Self>, value: i32) -> Box<Self> {
        self.value.set(value.clamp(self.min, self.max));
        self
    }
}
