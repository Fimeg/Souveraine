//! Btw pane widget — "by the way" interject feature display.
//!
//! Shows the state of a btw fork in chat:
//! - Idle: hidden (zero height)
//! - Forking: question text with spinner
//! - Streaming: question + live response with cursor
//! - Complete: question, response, green checkmark + forked conversation ID
//! - Error: question + error in red
//!
//! Uses a bordered Pane with a title and content Text widget.

use tuie::prelude::*;

use crate::ui::chat::{BtwState, ChatPalette};
use crate::ui::theme;

/// Pane that displays btw (by the way) fork state in chat.
pub struct BtwPane {
    pane: Box<Pane>,
    content_id: WidgetId<Text>,
}

impl DelegateWidget for BtwPane {
    tuie::delegate_widget!(pane);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl BtwPane {
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let primary = theme::to_tuie_color(palette.agent_primary);

        let mut content_id = WidgetId::EMPTY;
        let mut title = StyledString::new();
        title.push_span(StyledStr::new(" btw ").fg(primary).bold());

        let body = Text::new().content(StyledStr::new("")).id(&mut content_id);

        let pane = Pane::new()
            .vertical()
            .bordered()
            .border_style(Style::new().fg(primary).dim())
            .min_height(0)
            .max_height(0)
            .children([Text::new().content(title) as Box<dyn Widget>, body]);

        Box::new(Self { pane, content_id })
    }

    /// Update displayed btw state.
    pub fn set_state(&mut self, state: &BtwState, palette: &ChatPalette) {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let content = match state {
            BtwState::Idle => {
                self.pane.set_min_height(Some(0));
                self.pane.set_max_height(Some(0));
                return;
            }
            BtwState::Forking { question } => {
                self.pane.set_min_height(None);
                self.pane.set_max_height(None);
                let mut s = StyledString::new();
                s.push_span(StyledStr::new(&format!("Forking: \"{}\"\n", question)).fg(primary));
                s.push_span(StyledStr::new("...").fg(dim));
                s
            }
            BtwState::Streaming {
                question,
                response_so_far,
            } => {
                self.pane.set_min_height(None);
                self.pane.set_max_height(None);
                let mut s = StyledString::new();
                s.push_span(StyledStr::new(&format!("Q: {}\n", question)).fg(dim));
                s.push_span(StyledStr::new(response_so_far).fg(primary));
                s.push_span(StyledStr::new("_").fg(primary));
                s
            }
            BtwState::Complete {
                question,
                response,
                forked_id,
            } => {
                self.pane.set_min_height(None);
                self.pane.set_max_height(None);
                let mut s = StyledString::new();
                s.push_span(StyledStr::new(&format!("Q: {}\n", question)).fg(dim));
                s.push_span(StyledStr::new(&format!("A: {}\n", response)).fg(primary));
                s.push_span(
                    StyledStr::new(&format!("OK  Forked: {}\n", forked_id)).fg(Color::GREEN),
                );
                s
            }
            BtwState::Error { question, error } => {
                self.pane.set_min_height(None);
                self.pane.set_max_height(None);
                let mut s = StyledString::new();
                s.push_span(StyledStr::new(&format!("Q: {}\n", question)).fg(dim));
                s.push_span(StyledStr::new(&format!("Error: {}\n", error)).fg(Color::RED));
                s
            }
        };

        if let Some(t) = self.pane.get_widget_mut(self.content_id) {
            t.set_content(content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::{BtwState, ChatPalette};
    use tuie::emulator::Emulator;

    #[test]
    fn btw_pane_starts_hidden() {
        let palette = ChatPalette::default();
        let mut widget = BtwPane::new(&palette);
        let mut term = Emulator::new(&mut *widget, Vec2::new(80, 10));
        let output = term.get_snapshot_text();
        assert!(
            !output.contains("Forking") && !output.contains("Error") && !output.contains("OK"),
            "expected idle/empty output, got: {output:?}"
        );
    }

    #[test]
    fn btw_pane_shows_forking() {
        let palette = ChatPalette::default();
        let mut widget = BtwPane::new(&palette);
        let state = BtwState::Forking {
            question: "test question".into(),
        };
        widget.set_state(&state, &palette);
        let mut term = Emulator::new(&mut *widget, Vec2::new(80, 10));
        let output = term.get_snapshot_text();
        assert!(
            output.contains("Forking") || output.contains("test question"),
            "expected forking content, got: {output:?}"
        );
    }

    #[test]
    fn btw_pane_shows_streaming() {
        let palette = ChatPalette::default();
        let mut widget = BtwPane::new(&palette);
        let state = BtwState::Streaming {
            question: "q".into(),
            response_so_far: "streaming response".into(),
        };
        widget.set_state(&state, &palette);
        let mut term = Emulator::new(&mut *widget, Vec2::new(80, 10));
        let output = term.get_snapshot_text();
        assert!(
            output.contains("streaming response"),
            "expected streaming response text, got: {output:?}"
        );
    }

    #[test]
    fn btw_pane_shows_complete() {
        let palette = ChatPalette::default();
        let mut widget = BtwPane::new(&palette);
        let state = BtwState::Complete {
            question: "q".into(),
            response: "done".into(),
            forked_id: "abc123".into(),
        };
        widget.set_state(&state, &palette);
        let mut term = Emulator::new(&mut *widget, Vec2::new(80, 10));
        let output = term.get_snapshot_text();
        assert!(
            output.contains("OK") || output.contains("abc123"),
            "expected complete content, got: {output:?}"
        );
    }

    #[test]
    fn btw_pane_shows_error() {
        let palette = ChatPalette::default();
        let mut widget = BtwPane::new(&palette);
        let state = BtwState::Error {
            question: "q".into(),
            error: "fail".into(),
        };
        widget.set_state(&state, &palette);
        let mut term = Emulator::new(&mut *widget, Vec2::new(80, 10));
        let output = term.get_snapshot_text();
        assert!(
            output.contains("fail") || output.contains("Error"),
            "expected error content, got: {output:?}"
        );
    }
}
