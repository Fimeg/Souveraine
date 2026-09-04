#![allow(dead_code)] // WIP scaffolding not yet wired
//! Clickable text link widget.

use std::cell::Cell;
use std::process::{Command, Stdio};

use chord_macro::chord;
use tuie::prelude::*;

use crate::ui::theme;

/// Focusable text link that opens a URL when activated.
pub struct Link {
    layout: Layout,
    label: String,
    url: String,
    state: Cell<WidgetState>,
}

impl Link {
    fn natural_size(&self) -> Vec2<u16> {
        let margin = self.layout.get_margin_total();
        Vec2::new(self.label.len() as u16 + margin.x, 1 + margin.y)
    }

    fn open_url(&self) {
        let mut cmd = if cfg!(target_os = "macos") {
            Command::new("open")
        } else if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd.exe");
            cmd.args(["/c", "start", ""]);
            cmd
        } else {
            Command::new("xdg-open")
        };
        cmd.arg(&self.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.spawn();
    }
}

impl Widget for Link {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn get_layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    fn get_name(&self) -> &'static str {
        "Link"
    }

    fn measure_constraints(&mut self) -> Constraints {
        let size = self.natural_size();
        Constraints {
            min_size: size,
            max_size: size,
            preferred_size: size,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn on_state_change(&mut self, state: WidgetState) {
        self.state.set(state);
        self.dirty_paint();
    }

    fn render(&self, mut ctx: RenderContext) {
        let base = self.layout.style;
        let style = if matches!(self.state.get(), WidgetState::Active) {
            theme::get_accent()
                .apply(base)
                .underline(UnderlineType::Single)
        } else if self.in_focus_chain() {
            theme::get_accent()
                .bold()
                .apply(base)
                .underline(UnderlineType::Single)
        } else {
            base.underline(UnderlineType::Single)
        };

        ctx.set_style(base);
        ctx.clear();
        ctx.set_style(style);
        write!(ctx, "{}", self.label);
    }

    fn on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(Enter) => {
                self.open_url();
            }
            chord!(LeftRelease) => {
                let size = self.get_rect_size();
                if Axis2D::all(|a| event.pos[a] >= 0.0 && event.pos[a] < size[a] as f32) {
                    tuie::focus_widget(self.get_id());
                    self.open_url();
                }
            }
            chord!(LeftClick) => {
                tuie::focus_widget(self.get_id());
            }
            _ => return InputResult::Rejected,
        }
        InputResult::Handled
    }
}

impl Link {
    /// Creates a link with the given visible `label` that opens `url` when activated.
    pub fn new(label: &str, url: &str) -> Box<Self> {
        Box::new(Self {
            layout: Layout::new(),
            label: label.to_string(),
            url: url.to_string(),
            state: Cell::new(WidgetState::None),
        })
    }
}
