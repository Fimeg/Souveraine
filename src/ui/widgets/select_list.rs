#![allow(dead_code)] // WIP scaffolding not yet wired
//! `SelectList` — the one clickable, stylable, keyboard-navigable list.
//!
//! Every list-bearing screen used to reinvent selection: a `selected: usize`
//! field, a hand-matched `Up`/`Down`/`Enter` block in `override_on_input`, an
//! `Rc<Cell<Option<usize>>>` activation channel, and a method that rebuilt every
//! row to re-apply the highlight — keyboard only, never clickable. This widget
//! owns all of that once.
//!
//! - Rows come from `set_items(Vec<StyledString>)`. The caller formats each
//!   row's *content* (bold names, coloured methods, …); the list owns the
//!   *selection* styling (prefix + accent, or an accent border for cards) so it
//!   stays consistent everywhere.
//! - Virtualised through tuie's [`List`], so a thousand rows cost the visible
//!   window.
//! - **Single left-click on a row activates it** (selects + fires). Arrow keys
//!   (plus Home/End/PageUp/PageDown) move the selection; Enter activates.
//! - Hover tints the row under the pointer.
//!
//! Activation surfaces as an [`ActivateEvent`] emitted from the `SelectList`'s
//! own id. A screen reads it in `after_on_event`:
//!
//! ```ignore
//! fn after_on_event(&mut self, event: &mut WidgetEvent) {
//!     if let Some(&ActivateEvent(i)) = event.get_by::<ActivateEvent>(self.list_id) {
//!         self.selection.set(Some(i));
//!     }
//! }
//! ```

use chord_macro::chord;
use tuie::prelude::*;

use crate::ui::theme;

/// Emitted from a [`SelectList`] when a row is activated (clicked, or selected
/// and confirmed with Enter). Carries the row index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivateEvent(pub usize);

/// Emitted from a [`ListRow`] when it is clicked. Internal to this module — the
/// owning `SelectList` translates it into selection + [`ActivateEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowClicked(usize);

// ── Row ─────────────────────────────────────────────────────────────────────

/// One rendered row. Clickable (hit-tested, no focus needed), hover-aware.
struct ListRow {
    root: Box<Pane>,
    index: usize,
    selected: bool,
    active: Color,
    bordered: bool,
}

impl ListRow {
    fn new(
        index: usize,
        content: Box<dyn Widget>,
        selected: bool,
        active: Color,
        dim: Color,
        bordered: bool,
    ) -> Box<Self> {
        let body = Pane::new().horizontal().children([
            Text::new().content(
                StyledStr::new(if selected { "\u{25b6} " } else { "  " }).fg(if selected {
                    active
                } else {
                    dim
                }),
            ) as Box<dyn Widget>,
            content,
        ]);

        let root = if bordered {
            Pane::new()
                .vertical()
                .bordered()
                .border_style(Style::new().fg(if selected { active } else { dim }).dim())
                .padding(Spacing::new().horizontal(1))
                .children([body])
        } else {
            Pane::new().vertical().children([body])
        };

        let mut row = Box::new(Self {
            root,
            index,
            selected,
            active,
            bordered,
        });
        row.apply_style(false);
        row
    }

    /// Repaint the row body for the current (selected, hovered) state. Selection
    /// is structural (prefix + border); hover adds a subtle accent tint so the
    /// row under the pointer reads as live without recolouring its content.
    fn apply_style(&mut self, hovered: bool) {
        let style = if hovered {
            Style::new().fg(self.active)
        } else if self.selected && !self.bordered {
            Style::new().fg(self.active).dim()
        } else {
            Style::new()
        };
        self.root.set_style(style);
    }
}

impl DelegateWidget for ListRow {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        false
    }

    fn after_on_state_change(&mut self, state: WidgetState) {
        let hovered = matches!(state, WidgetState::Hover | WidgetState::FocusedHover);
        self.apply_style(hovered);
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        // Rows are hit-tested for the pointer only; keyboard goes to the focused
        // `SelectList`, never here.
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(LeftRelease) => {
                let size = self.get_rect_size();
                let inside = Axis2D::all(|a| event.pos[a] >= 0.0 && event.pos[a] < size[a] as f32);
                if inside {
                    tuie::emit(self.get_id(), RowClicked(self.index));
                }
                InputResult::Handled
            }
            _ => InputResult::Rejected,
        }
    }
}

// ── Render context ──────────────────────────────────────────────────────────

/// State the virtualised renderer reads to build each visible row. Lives inside
/// the [`List`]; `SelectList` mutates it through `List::get_context_mut`.
struct RowCtx {
    rows: Vec<StyledString>,
    selected: usize,
    active: Color,
    dim: Color,
    bordered: bool,
}

fn render_row(ctx: &mut RowCtx, index: usize) -> Option<Box<dyn Widget>> {
    let content = ctx.rows.get(index)?.clone();
    let widget = Text::new().content(content) as Box<dyn Widget>;
    Some(ListRow::new(
        index,
        widget,
        index == ctx.selected,
        ctx.active,
        ctx.dim,
        ctx.bordered,
    ))
}

// ── SelectList ──────────────────────────────────────────────────────────────

/// The list. Build it, hand it rows, place it in a screen, read [`ActivateEvent`].
pub struct SelectList {
    root: Box<Pane>,
    list_id: WidgetId<List>,
    len: usize,
    selected: usize,
    active: Color,
    dim: Color,
    bordered: bool,
    gap: u8,
}

impl SelectList {
    /// A new, empty list. Colours default to the theme accent / dim; override
    /// with [`colors`](Self::colors). Call [`items`](Self::items) to populate.
    pub fn new() -> Box<Self> {
        Box::new(Self {
            root: Pane::new().vertical().flex(1),
            list_id: WidgetId::EMPTY,
            len: 0,
            selected: 0,
            active: theme::get_accent_color(),
            dim: Color::BRIGHT_BLACK,
            bordered: false,
            gap: 0,
        })
    }

    /// Accent (selected/hover) and dim (idle prefix/border) colours.
    pub fn colors(mut self: Box<Self>, active: Color, dim: Color) -> Box<Self> {
        self.active = active;
        self.dim = dim;
        self
    }

    /// Render each row inside an accent-bordered card (agent-picker style)
    /// rather than as a plain prefixed line.
    pub fn bordered(mut self: Box<Self>) -> Box<Self> {
        self.bordered = true;
        self
    }

    /// Vertical gap (in rows) between items.
    pub fn gap(mut self: Box<Self>, gap: u8) -> Box<Self> {
        self.gap = gap;
        self
    }

    /// Set the initial selection (clamped on [`items`](Self::items)).
    pub fn selected(mut self: Box<Self>, index: usize) -> Box<Self> {
        self.selected = index;
        self
    }

    /// Populate the list. Each `StyledString` is one row's content; selection
    /// styling is applied by the list. Builds the inner [`List`] and finalises.
    pub fn items(mut self: Box<Self>, rows: Vec<StyledString>) -> Box<Self> {
        self.len = rows.len();
        if self.selected >= self.len {
            self.selected = self.len.saturating_sub(1);
        }

        let mut list = List::new()
            .vertical()
            .flex(1)
            .scrollbar_style(ScrollbarStyle::new())
            .scroll(Scrollbar::AutoHide);
        if self.gap > 0 {
            list = list.gap(self.gap);
        }
        list.set_renderer(
            RowCtx {
                rows,
                selected: self.selected,
                active: self.active,
                dim: self.dim,
                bordered: self.bordered,
            },
            render_row,
        );
        list.set_item_count(self.len);
        self.list_id = list.get_id();
        self.root = Pane::new().vertical().flex(1).children([list]);
        self
    }

    /// Replace the rows in place (after the list is already built), keeping the
    /// current selection where it still fits. For screens whose contents change
    /// — toggles, adds, deletes.
    pub fn set_items(&mut self, rows: Vec<StyledString>) {
        self.len = rows.len();
        if self.selected >= self.len {
            self.selected = self.len.saturating_sub(1);
        }
        let selected = self.selected;
        if let Some(list) = self.root.get_widget_mut(self.list_id) {
            if let Some(ctx) = list.get_context_mut::<RowCtx>() {
                ctx.rows = rows;
                ctx.selected = selected;
            }
            list.set_item_count(self.len);
            list.invalidate_all();
        }
        self.root.dirty_layout();
    }

    /// The currently highlighted row.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Jump the highlight to a specific row.
    pub fn select(&mut self, index: usize) {
        self.set_selected(index);
    }

    /// Move the highlight up one row. For a screen that drives the list from its
    /// own `override_on_input` (the list is not the focused widget).
    pub fn move_up(&mut self) {
        self.move_by(-1);
    }

    /// Move the highlight down one row.
    pub fn move_down(&mut self) {
        self.move_by(1);
    }

    /// Fire [`ActivateEvent`] for the current selection (the keyboard "Enter"
    /// path; clicks activate themselves).
    pub fn activate_selected(&mut self) {
        let sel = self.selected;
        self.activate(sel);
    }

    fn move_by(&mut self, delta: i32) {
        if self.len == 0 {
            return;
        }
        let max = self.len as i32 - 1;
        let next = (self.selected as i32 + delta).clamp(0, max) as usize;
        if next != self.selected {
            self.set_selected(next);
        }
    }

    fn set_selected(&mut self, index: usize) {
        if index >= self.len {
            return;
        }
        self.selected = index;
        if let Some(list) = self.root.get_widget_mut(self.list_id) {
            if let Some(ctx) = list.get_context_mut::<RowCtx>() {
                ctx.selected = index;
            }
            list.invalidate_all();
            list.ensure_visible(index);
        }
        self.root.dirty_layout();
    }

    fn activate(&mut self, index: usize) {
        tuie::emit(self.get_id(), ActivateEvent(index));
    }
}

impl DelegateWidget for SelectList {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        use tuie::input::key::Key;
        use tuie::input::trigger::Trigger;

        if let Some(event) = queue.peek() {
            if let Trigger::Key(key) = &event.chord.trigger {
                let handled = match key {
                    Key::Arrow(Direction2D::Up) => Some(-1i32),
                    Key::Arrow(Direction2D::Down) => Some(1),
                    Key::PageUp => Some(-10),
                    Key::PageDown => Some(10),
                    _ => None,
                };
                if let Some(delta) = handled {
                    queue.next();
                    self.move_by(delta);
                    return InputResult::Handled;
                }
                match key {
                    Key::Home => {
                        queue.next();
                        self.set_selected(0);
                        return InputResult::Handled;
                    }
                    Key::End => {
                        queue.next();
                        self.set_selected(self.len.saturating_sub(1));
                        return InputResult::Handled;
                    }
                    Key::Enter => {
                        queue.next();
                        let sel = self.selected;
                        self.activate(sel);
                        return InputResult::Handled;
                    }
                    _ => {}
                }
            }
        }
        self.get_delegate_mut().on_input(queue)
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        if let Some(&RowClicked(index)) = event.get::<RowClicked>() {
            self.set_selected(index);
            self.activate(index);
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;

    fn rows(n: usize) -> Vec<StyledString> {
        (0..n)
            .map(|i| {
                let mut s = StyledString::new();
                s.push_span(StyledStr::new(&format!("row{i}")));
                s
            })
            .collect()
    }

    #[test]
    fn renders_visible_rows() {
        let mut list = SelectList::new().items(rows(3));
        let term = Emulator::new(&mut *list, Vec2::new(20, 10));
        let snap = term.get_snapshot_text();
        assert!(snap.contains("row0"), "got: {snap:?}");
        assert!(snap.contains("row2"), "got: {snap:?}");
    }

    #[test]
    fn keyboard_move_clamps_at_bounds() {
        let mut list = SelectList::new().items(rows(3));
        let _ = Emulator::new(&mut *list, Vec2::new(20, 10));

        assert_eq!(list.selected_index(), 0);
        list.move_up(); // already at top — clamps
        assert_eq!(list.selected_index(), 0);

        list.move_down();
        assert_eq!(list.selected_index(), 1);

        list.move_down();
        list.move_down(); // past the end — clamps
        assert_eq!(list.selected_index(), 2);
    }

    #[test]
    fn set_items_clamps_overflowing_selection() {
        let mut list = SelectList::new().items(rows(5));
        let _ = Emulator::new(&mut *list, Vec2::new(20, 10));
        list.select(4);
        assert_eq!(list.selected_index(), 4);

        list.set_items(rows(2));
        assert!(
            list.selected_index() <= 1,
            "selection should clamp to new len"
        );
    }

    #[test]
    fn empty_list_is_inert() {
        let mut list = SelectList::new().items(rows(0));
        let _ = Emulator::new(&mut *list, Vec2::new(20, 10));
        // No panic, selection pinned at 0.
        list.move_down();
        list.move_up();
        assert_eq!(list.selected_index(), 0);
    }
}
