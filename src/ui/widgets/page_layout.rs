//! Stack-based page navigation widget.

use tuie::prelude::*;

struct PageEntry {
    saved_widget: Option<WidgetId>,
    content: Option<Box<dyn Widget>>,
}

/// Page navigation stack.
pub(crate) struct PageLayout {
    pane: Box<Pane>,
    stack: Vec<PageEntry>,
    top_id: WidgetId,
}

impl DelegateWidget for PageLayout {
    tuie::delegate_widget!(pane);
}

impl PageLayout {
    pub(crate) fn new(home: Box<dyn Widget>) -> Box<Self> {
        let top_id = home.get_id();
        let pane = Pane::new()
            .vertical()
            .flex(1)
            .min_width(0)
            .min_height(0)
            .children([home]);

        Box::new(Self {
            pane,
            stack: vec![PageEntry {
                saved_widget: None,
                content: None,
            }],
            top_id,
        })
    }

    pub(crate) fn push(&mut self, content: Box<dyn Widget>) {
        if let Some(entry) = self.stack.last_mut() {
            entry.saved_widget = tuie::get_focused_widget();
        }

        let old = self.pane.remove(self.top_id);
        if let Some(entry) = self.stack.last_mut() {
            entry.content = old;
        }

        self.top_id = content.get_id();
        self.pane.add_child(content);
        self.stack.push(PageEntry {
            saved_widget: None,
            content: None,
        });
        tuie::dirty_layout();
    }

    pub(crate) fn pop(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }

        self.pane.remove(self.top_id);
        self.stack.pop();

        if let Some(entry) = self.stack.last_mut() {
            if let Some(content) = entry.content.take() {
                self.top_id = content.get_id();
                self.pane.add_child(content);
            }

            if let Some(id) = entry.saved_widget {
                tuie::focus_widget(id);
            }
        }
    }
}
