#![allow(dead_code)] // WIP scaffolding not yet wired
//! Bordered pane that highlights itself when selected or active.

use tuie::render::border;
use tuie::{delegate_field, field, prelude::*};

use crate::ui::theme;

/// Bordered [`Pane`] wrapper that highlights its border when focused.
pub(crate) struct FocusPane {
    pane: Box<Pane>,
    border_style: Style,
    selected_border_style: Option<Style>,
}

impl FocusPane {
    fn refresh_border(&mut self) {
        let cfg = border::config::get();
        let focused = tuie::runtime::in_focus_chain(self.pane.get_id());
        if focused {
            let style = self
                .selected_border_style
                .unwrap_or_else(|| cfg.selected_style.apply(theme::get_accent()));
            self.pane.set_border_style(style);
            self.pane.set_border(Some(cfg.selected_border));
        } else {
            self.pane.set_border_style(self.border_style);
            self.pane.set_border(Some(cfg.border));
        }
    }

    fn sync_border_style(&mut self) {
        self.refresh_border();
    }
}

impl DelegateWidget for FocusPane {
    tuie::delegate_widget!(pane);

    fn after_on_state_change(&mut self, _widget_state: WidgetState) {
        self.pane.dirty_layout();
    }

    fn after_before_layout(&mut self) {
        self.refresh_border();
    }
}

impl FocusPane {
    /// Creates an empty bordered focus pane.
    pub(crate) fn new() -> Box<Self> {
        let mut pane = Pane::new();
        pane.set_bordered(true);
        Box::new(Self {
            pane,
            border_style: Style::new(),
            selected_border_style: None,
        })
    }

    /// Appends a child widget.
    pub(crate) fn add_child(&mut self, widget: Box<dyn Widget>) {
        self.pane.add_child(widget);
    }

    /// Appends `children` to the pane.
    pub(crate) fn children<const N: usize>(
        mut self: Box<Self>,
        children: [Box<dyn Widget>; N],
    ) -> Box<Self> {
        for child in children {
            self.pane.add_child(child);
        }
        self
    }

    field!(border_style: Style; sync_border_style);
    field!(selected_border_style: Option<Style>);

    delegate_field!(orientation: Axis2D => pane);
    delegate_field!(gap: u8 => pane);
}
