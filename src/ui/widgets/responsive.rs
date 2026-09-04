#![allow(dead_code)] // WIP scaffolding not yet wired
//! Responsive container — swaps between a wide and a narrow subtree.
//!
//! `Responsive` holds two child widgets: a `wide` arrangement and a `narrow`
//! one. During layout it measures the width it was allocated and lays out /
//! renders **only** the arrangement that fits, switching at `breakpoint`
//! columns. This is how the welcome screen offers two viewable modes (a
//! side-by-side dashboard when there's room, a stacked column when narrow) the
//! same way the old ratatui dashboard did — but as a reusable primitive any
//! screen can wrap around any pair of layouts.
//!
//! ## Traversal model
//!
//! Both children are exposed to [`Widget::each_child`] so that
//! [`WidgetMethods::get_widget`] lookups resolve ids in *either* arrangement —
//! a caller can keep a `WidgetId` for a widget in the narrow tree and still
//! reach it while the wide tree is showing. Focus, hit-testing, painting and
//! positioning, however, only ever touch the active child, so the inactive
//! arrangement is inert: never drawn, never focusable, never hit.
//!
//! Modelled on tuie's own `Stack` container.

use tuie::prelude::*;

/// A container that shows `wide` at or above `breakpoint` columns and `narrow`
/// below it.
pub struct Responsive {
    layout: Layout,
    wide: Box<dyn Widget>,
    narrow: Box<dyn Widget>,
    breakpoint: u16,
    /// Which arrangement is currently active. Recomputed every `layout_flow`.
    wide_active: bool,
}

impl Responsive {
    /// Create a responsive container that switches at `breakpoint` columns.
    pub fn new(breakpoint: u16, wide: Box<dyn Widget>, narrow: Box<dyn Widget>) -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            wide,
            narrow,
            breakpoint,
            wide_active: true,
        })
    }

    /// Returns true when the wide arrangement is currently shown.
    pub fn is_wide(&self) -> bool {
        self.wide_active
    }

    fn active(&self) -> &dyn Widget {
        if self.wide_active {
            &*self.wide
        } else {
            &*self.narrow
        }
    }

    fn active_mut(&mut self) -> &mut dyn Widget {
        if self.wide_active {
            &mut *self.wide
        } else {
            &mut *self.narrow
        }
    }

    fn inactive(&self) -> &dyn Widget {
        if self.wide_active {
            &*self.narrow
        } else {
            &*self.wide
        }
    }

    fn contains_pos(child: &dyn Widget, pos: Vec2<f32>) -> bool {
        let cp = child.get_pos();
        let cs = child.get_rect_size().map(|v| v as i32);
        Axis2D::all(|a| pos[a] >= cp[a] as f32 && pos[a] < (cp[a] + cs[a]) as f32)
    }
}

impl Widget for Responsive {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "Responsive"
    }

    fn get_flow_axis(&self) -> Axis2D {
        self.active().get_flow_axis()
    }

    fn measure_constraints(&mut self) -> Constraints {
        // Constrain both arrangements so either can be flowed immediately when
        // the breakpoint is crossed.
        constrain_child(&mut *self.wide);
        constrain_child(&mut *self.narrow);
        // We fill whatever space the parent offers; the active child is then
        // forced to that size during `layout_flow`.
        Constraints {
            min_size: Vec2::of(0),
            max_size: Vec2::of(u16::MAX),
            preferred_size: Vec2::of(u16::MAX),
        }
    }

    fn layout_flow(&mut self, allocated: Vec2<u16>) -> Vec2<u16> {
        self.wide_active = allocated[Axis2D::X] >= self.breakpoint;
        flow_child(self.active_mut(), allocated);
        allocated
    }

    fn layout_measure(&self, allocated: Vec2<u16>) -> Vec2<u16> {
        let wide = allocated[Axis2D::X] >= self.breakpoint;
        let child: &dyn Widget = if wide { &*self.wide } else { &*self.narrow };
        measure_child(child, allocated);
        allocated
    }

    fn layout_position(&mut self) {
        let content_pos = self.layout.rect.pos;
        let child = self.active_mut();
        let margin = child.get_layout().get_margin_before().map(|v| v as i32);
        child.set_pos(content_pos + margin);
        child.layout_position();
    }

    fn render(&self, mut ctx: RenderContext) {
        let content_pos = self.layout.rect.pos;
        let child = self.active();
        let offset = child.get_pos() - content_pos;
        ctx.render_child(child, offset);
    }

    fn each_child(&self, f: &mut dyn FnMut(&dyn Widget), _direction: Sign) {
        // Both children are visible to id lookups; the active one first.
        f(self.active());
        f(self.inactive());
    }

    fn each_child_mut(&mut self, f: &mut dyn FnMut(&mut dyn Widget), _direction: Sign) {
        // Borrow rules: resolve the active flag before splitting the borrows.
        if self.wide_active {
            f(&mut *self.wide);
            f(&mut *self.narrow);
        } else {
            f(&mut *self.narrow);
            f(&mut *self.wide);
        }
    }

    fn find_descendant(
        &self,
        predicate: &dyn Fn(&dyn Widget) -> bool,
        mut path: Option<&mut Vec<WidgetId>>,
    ) -> Option<WidgetId> {
        // Focus traversal only ever sees the active arrangement.
        let child = self.active();
        if let Some(found) = child.find_descendant(predicate, path.as_deref_mut()) {
            if let Some(p) = &mut path {
                p.push(child.get_id());
            }
            return Some(found);
        }
        if predicate(child) {
            if let Some(p) = &mut path {
                p.push(child.get_id());
            }
            return Some(child.get_id());
        }
        None
    }

    fn descendant_at_pos(
        &self,
        pos: Vec2<f32>,
        mut path: Option<&mut Vec<WidgetId>>,
    ) -> Option<WidgetId> {
        let child = self.active();
        if !Self::contains_pos(child, pos) {
            return None;
        }
        let hit = child
            .descendant_at_pos(pos, path.as_deref_mut())
            .unwrap_or_else(|| child.get_id());
        if let Some(p) = &mut path {
            p.push(child.get_id());
        }
        Some(hit)
    }

    fn find_descendant_at_pos(
        &self,
        pos: Vec2<f32>,
        predicate: &dyn Fn(&dyn Widget) -> bool,
        mut path: Option<&mut Vec<WidgetId>>,
    ) -> Option<WidgetId> {
        let child = self.active();
        if !Self::contains_pos(child, pos) {
            return None;
        }
        if let Some(found) = child.find_descendant_at_pos(pos, predicate, path.as_deref_mut()) {
            if let Some(p) = &mut path {
                p.push(child.get_id());
            }
            return Some(found);
        }
        if predicate(child) {
            if let Some(p) = &mut path {
                p.push(child.get_id());
            }
            return Some(child.get_id());
        }
        None
    }

    fn can_scroll(&self, direction: Direction2D) -> bool {
        self.active().can_scroll(direction)
    }

    fn get_cursor(&self, selected: Option<WidgetId>) -> Option<(CursorShape, Vec2<i32>)> {
        let selected = selected?;
        self.layout.get_child_cursor(self.active(), selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;

    fn label(s: &str) -> Box<dyn Widget> {
        Text::new().content(s).flex(1)
    }

    #[test]
    fn switches_arrangement_on_width() {
        let mut root = Responsive::new(100, label("WIDEMODE"), label("NARROWMODE")).flex(1);

        // At/above the breakpoint the wide arrangement is shown.
        let mut term = Emulator::new(&mut *root, Vec2::new(120, 10));
        let wide = term.get_snapshot_text();
        assert!(
            wide.contains("WIDEMODE"),
            "expected wide arrangement, got: {wide:?}"
        );
        assert!(
            !wide.contains("NARROWMODE"),
            "narrow leaked into wide: {wide:?}"
        );

        // Shrinking below the breakpoint swaps to the narrow arrangement.
        term.update(&mut *root, &[RuntimeEvent::Resize(Vec2::new(70, 10))]);
        let narrow = term.get_snapshot_text();
        assert!(
            narrow.contains("NARROWMODE"),
            "expected narrow arrangement, got: {narrow:?}"
        );
        assert!(
            !narrow.contains("WIDEMODE"),
            "wide leaked into narrow: {narrow:?}"
        );

        // Growing back restores the wide arrangement.
        term.update(&mut *root, &[RuntimeEvent::Resize(Vec2::new(120, 10))]);
        let wide_again = term.get_snapshot_text();
        assert!(
            wide_again.contains("WIDEMODE"),
            "expected wide again, got: {wide_again:?}"
        );
    }
}
