#![allow(dead_code)] // WIP scaffolding not yet wired
//! Horizontal divider widget.

use tuie::prelude::*;

/// One-cell-tall horizontal divider.
pub(crate) struct HorizontalRule {
    layout: Layout,
}

impl Widget for HorizontalRule {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "HorizontalRule"
    }

    fn measure_constraints(&mut self) -> Constraints {
        Constraints {
            min_size: Vec2::new(0, 1),
            max_size: Vec2::new(u16::MAX, 1),
            preferred_size: Vec2::new(0, 1),
        }
    }

    fn render(&self, mut ctx: RenderContext) {
        ctx.set_style(self.layout.style);
        ctx.fill("─");
    }
}

impl HorizontalRule {
    /// Creates a [`HorizontalRule`] with a dim foreground.
    pub(crate) fn new() -> Box<Self> {
        let mut layout = Layout::new();
        layout.style = Style::new().fg(Color::grey256(6));
        Box::new(Self { layout })
    }
}
