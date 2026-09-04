//! Horizontal progress bar widget.

use tuie::{field, prelude::*};

/// Horizontal progress bar.
pub(crate) struct ProgressBar {
    layout: Layout,
    progress: f32,
}

impl Widget for ProgressBar {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "ProgressBar"
    }

    fn measure_constraints(&mut self) -> Constraints {
        Constraints {
            min_size: Vec2::new(0, 1),
            max_size: Vec2::new(u16::MAX, 1),
            preferred_size: Vec2::new(0, 1),
        }
    }

    fn render(&self, mut ctx: RenderContext) {
        let width = ctx.size.x;
        if width == 0 {
            return;
        }
        let eighths = (self.progress * width as f32 * 8.0).round() as u32;
        let full_cells = (eighths / 8) as u16;
        let remainder = (eighths % 8) as u8;

        let mut row = String::new();
        for _ in 0..full_cells.min(width) {
            row.push('\u{2588}');
        }
        if remainder > 0 && full_cells < width {
            let partial = match remainder {
                7 => '\u{2589}',
                6 => '\u{258A}',
                5 => '\u{258B}',
                4 => '\u{258C}',
                3 => '\u{258D}',
                2 => '\u{258E}',
                1 => '\u{258F}',
                _ => unreachable!(),
            };
            row.push(partial);
        }

        ctx.set_style(self.layout.style);
        ctx.move_to((0, 0).into());
        ctx.write(&row);
    }
}

impl ProgressBar {
    pub(crate) fn new() -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            progress: 0.0,
        })
    }

    fn clamp_and_dirty(&mut self) {
        self.progress = self.progress.clamp(0.0, 1.0);
        self.dirty_paint();
    }

    field!(progress: f32; clamp_and_dirty);
}
