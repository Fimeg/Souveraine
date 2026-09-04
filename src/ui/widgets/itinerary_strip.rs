//! Itinerary strip widget — thin route line between header and messages.
//!
//! Shows a single-line text strip when `ChatState.itinerary_line` is
//! non-empty, displaying the current route like:
//! `route: step 1 → step 2 → step 3`
//!
//! When hidden (empty line), the strip collapses to zero height so it
//! does not consume layout space.

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;

pub struct ItineraryStrip {
    text: Box<Text>,
    visible: bool,
}

impl DelegateWidget for ItineraryStrip {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl ItineraryStrip {
    /// Create a hidden strip. Call `set_line` to show content.
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let _color = theme::to_tuie_color(palette.agent_dim);

        let content = StyledString::new();
        let mut text = Text::new().content(content);
        text.set_min_height(Some(0));
        text.set_max_height(Some(0));

        Box::new(Self {
            text,
            visible: false,
        })
    }

    /// Set the itinerary text. Empty string hides the strip.
    pub fn set_line(&mut self, line: &str, palette: &ChatPalette) {
        if line.is_empty() {
            self.text.set_content(StyledString::new());
            self.text.set_min_height(Some(0));
            self.text.set_max_height(Some(0));
            self.visible = false;
        } else {
            let color = theme::to_tuie_color(palette.agent_dim);

            let mut content = StyledString::new();
            content.push_span(
                StyledStr::new(&format!("  {}\n", line))
                    .fg(color)
                    .italic()
                    .dim(),
            );

            self.text.set_content(content);
            self.text.set_min_height(Some(1));
            self.text.set_max_height(Some(1));
            self.visible = true;
        }

        self.text.dirty_layout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;

    #[test]
    fn itinerary_starts_hidden() {
        let palette = ChatPalette::default();
        let mut widget = ItineraryStrip::new(&palette);
        // Height 1: when hidden the widget collapses to zero height, so
        // rendering should be empty or a single blank line.
        let term = Emulator::new(&mut *widget, Vec2::new(80, 1));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.trim().is_empty(),
            "expected empty/minimal output from hidden strip, got: {rendered:?}"
        );
    }

    #[test]
    fn itinerary_shows_route() {
        let palette = ChatPalette::default();
        let mut widget = ItineraryStrip::new(&palette);
        widget.set_line("step1 → step2 → step3", &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(80, 3));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("step1") || rendered.contains("step2"),
            "expected route text in output, got: {rendered:?}"
        );
    }

    #[test]
    fn itinerary_hide_after_show() {
        let palette = ChatPalette::default();
        let mut widget = ItineraryStrip::new(&palette);
        widget.set_line("step1 → step2 → step3", &palette);
        widget.set_line("", &palette);
        let term = Emulator::new(&mut *widget, Vec2::new(80, 1));
        let rendered = term.get_snapshot_text();
        assert!(
            !rendered.contains("step1") && !rendered.contains("step2"),
            "expected route text absent after hide, got: {rendered:?}"
        );
    }
}
