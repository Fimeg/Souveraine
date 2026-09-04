//! Footer widget — keybind hints, conversation ID, context pressure.
//!
//! Single-row status bar at the bottom of the chat screen, matching the
//! ratatui footer layout: centered pipe-separated segments.

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;

pub struct Footer {
    text: Box<Text>,
    agent_name: String,
    mode: String,
    model: String,
    pressure: f32,
    context_limit: Option<usize>,
    conversation_id: String,
    cockpit_open: bool,
    tools_expanded: bool,
    dim_color: Color,
    primary_color: Color,
}

impl DelegateWidget for Footer {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl Footer {
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let text = Text::new().center();
        Box::new(Self {
            text,
            agent_name: String::new(),
            mode: String::new(),
            model: String::new(),
            pressure: 0.0,
            context_limit: None,
            conversation_id: String::new(),
            cockpit_open: false,
            tools_expanded: false,
            dim_color: theme::to_tuie_color(palette.agent_dim),
            primary_color: theme::to_tuie_color(palette.agent_primary),
        })
    }

    pub fn set_agent_name(&mut self, name: &str) {
        self.agent_name = name.to_string();
        self.rebuild();
    }

    pub fn set_mode(&mut self, mode: &str) {
        self.mode = mode.to_string();
        self.rebuild();
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
        self.rebuild();
    }

    pub fn set_pressure(&mut self, pressure: f32) {
        self.pressure = pressure;
        self.rebuild();
    }

    pub fn set_context_limit(&mut self, limit: Option<usize>) {
        self.context_limit = limit;
        self.rebuild();
    }

    pub fn set_conversation_id(&mut self, id: &str) {
        self.conversation_id = id.to_string();
        self.rebuild();
    }

    pub fn set_cockpit_open(&mut self, open: bool) {
        self.cockpit_open = open;
        self.rebuild();
    }

    pub fn set_tools_expanded(&mut self, expanded: bool) {
        self.tools_expanded = expanded;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let dim = self.dim_color;
        let primary = self.primary_color;
        let mut c = StyledString::new();

        // Left: agent + mode
        if !self.agent_name.is_empty() {
            c.push_span(
                StyledStr::new(&format!(" {} ", self.agent_name))
                    .fg(primary)
                    .bold(),
            );
            c.push_span(StyledStr::new("· ").fg(dim));
        }
        c.push_span(StyledStr::new(&self.mode).fg(dim));

        // Keybind hints
        let cockpit_hint = if self.cockpit_open {
            "Tab close cockpit"
        } else {
            "Tab cockpit"
        };
        let tool_hint = if self.tools_expanded {
            "Ctrl+T collapse tools"
        } else {
            "Ctrl+T expand tools"
        };
        c.push_span(
            StyledStr::new(&format!(
                "  |  Esc menu · Enter send · {cockpit_hint} · {tool_hint}"
            ))
            .fg(dim),
        );

        // Conversation ID
        if !self.conversation_id.is_empty() {
            let short = if self.conversation_id.len() > 8 {
                &self.conversation_id[..8]
            } else {
                &self.conversation_id
            };
            c.push_span(StyledStr::new(&format!("  |  conv {short}")).fg(dim));
        }

        // Model
        if !self.model.is_empty() {
            c.push_span(StyledStr::new(&format!("  |  {}", self.model)).fg(dim));
        }

        // Pressure bar
        let p = self.pressure.clamp(0.0, 1.0);
        let bar_width: usize = 16;
        let filled = (p * bar_width as f32).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        let bar_color = pressure_color(p);
        let pct = (p * 100.0).round() as u32;

        c.push_span(StyledStr::new("  ").fg(dim));
        if filled > 0 {
            c.push_span(StyledStr::new(&"\u{2588}".repeat(filled)).fg(bar_color));
        }
        if empty > 0 {
            c.push_span(StyledStr::new(&"\u{2591}".repeat(empty)).fg(dim));
        }

        let ctx_label = match self.context_limit {
            Some(limit) => format!(" {:>3}% of {} ", pct, limit),
            None => format!(" {:>3}% ", pct),
        };
        c.push_span(StyledStr::new(&ctx_label).fg(dim));

        self.text.set_content(c);
    }
}

fn pressure_color(pressure: f32) -> Color {
    let p = pressure.clamp(0.0, 1.0);
    if p < 0.5 {
        let t = p / 0.5;
        Color::Rgb((255.0 * t) as u8, 255, 0)
    } else {
        let t = (p - 0.5) / 0.5;
        Color::Rgb(255, (255.0 * (1.0 - t)) as u8, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;

    #[test]
    fn footer_renders_agent_name() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_agent_name("TestAgent");
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("TestAgent"),
            "expected agent name in output, got: {rendered:?}"
        );
    }

    #[test]
    fn footer_renders_keybind_hints() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_mode("local");
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Enter send"),
            "expected keybind hints, got: {rendered:?}"
        );
        assert!(
            rendered.contains("Tab cockpit"),
            "expected cockpit hint, got: {rendered:?}"
        );
    }

    #[test]
    fn footer_renders_conversation_id() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_conversation_id("abcdef1234567890");
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("conv abcdef12"),
            "expected truncated conv id, got: {rendered:?}"
        );
    }

    #[test]
    fn footer_renders_pressure_bar() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_pressure(0.52);
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("52%"),
            "expected pressure percentage, got: {rendered:?}"
        );
    }

    #[test]
    fn footer_renders_model() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_model("test-model");
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("test-model"),
            "expected model name, got: {rendered:?}"
        );
    }

    #[test]
    fn footer_cockpit_hint_toggles() {
        let palette = ChatPalette::default();
        let mut widget = Footer::new(&palette);
        widget.set_cockpit_open(false);
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        assert!(term.get_snapshot_text().contains("Tab cockpit"));

        widget.set_cockpit_open(true);
        let term = Emulator::new(&mut *widget, Vec2::new(120, 3));
        assert!(term.get_snapshot_text().contains("Tab close cockpit"));
    }
}
