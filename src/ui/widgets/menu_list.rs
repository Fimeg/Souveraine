//! Menu list widget — selectable items with labels, descriptions, and availability.

use tuie::prelude::*;

/// A single menu item.
pub struct MenuItem {
    pub label: String,
    pub description: String,
    pub available: bool,
}

/// Vertical list of menu items. The selected index is tracked externally.
pub struct MenuList {
    text: Box<Text>,
    items: Vec<MenuItem>,
    selected: usize,
    active_color: Color,
    dim_color: Color,
    unavailable_color: Color,
}

impl DelegateWidget for MenuList {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool { true }
}

impl MenuList {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            text: Text::new(),
            items: Vec::new(),
            selected: 0,
            active_color: Color::YELLOW,
            dim_color: Color::BRIGHT_BLACK,
            unavailable_color: Color::BRIGHT_BLACK,
        })
    }

    pub fn set_items(mut self: Box<Self>, items: Vec<MenuItem>) -> Box<Self> {
        self.items = items;
        self.rebuild();
        self
    }

    pub fn set_colors(
        mut self: Box<Self>,
        active: Color,
        dim: Color,
        unavailable: Color,
    ) -> Box<Self> {
        self.active_color = active;
        self.dim_color = dim;
        self.unavailable_color = unavailable;
        self.rebuild();
        self
    }

    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
            self.rebuild();
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.rebuild();
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
            self.rebuild();
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_label(&self) -> Option<&str> {
        self.items.get(self.selected).map(|i| i.label.as_str())
    }

    fn rebuild(&mut self) {
        let mut content = StyledString::new();
        for (i, item) in self.items.iter().enumerate() {
            let is_selected = i == self.selected;
            let prefix = if is_selected { "▶" } else { " " };

            let color = if !item.available {
                self.unavailable_color
            } else if is_selected {
                self.active_color
            } else {
                self.dim_color
            };

            let mut line = format!("  {prefix} {:<14}", item.label);
            if item.available {
                line.push_str(&format!("- {}", item.description));
            } else {
                line.push_str(&format!("- {}  (coming soon)", item.description));
            }
            line.push('\n');

            content.push_span(StyledStr::new(&line).fg(color));
        }

        self.text.set_content(content);
        self.text.dirty_layout();
    }
}
