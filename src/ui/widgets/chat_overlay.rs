//! ChatOverlay widget — popup overlays for slash-complete and conversation picker.
//!
//! Renders a bordered popup near the input area with a virtualized list of items.
//! Two modes:
//! - SlashComplete: list of slash commands matching `/` input
//! - ConversationPicker: list of past conversations for `/resume` or `/convos`
//!
//! The overlay is a tuie widget that lives in the widget tree but only renders
//! when its item count is non-zero. When hidden, borders are off and the list
//! is empty, so the widget takes effectively zero space.

use tuie::prelude::*;

use crate::ui::chat::{ChatPalette, SlashDef};
use crate::ui::theme;

// ─── Render context ───────────────────────────────────────────────────────

/// Which data the overlay is currently showing.
enum OverlayKind {
    /// Overlay is hidden — no items rendered.
    None,
    /// Slash command completion list.
    SlashCommands {
        selected: usize,
        matches: Vec<&'static SlashDef>,
    },
    /// Past conversation picker.
    Conversations {
        selected: usize,
        conversations: Vec<crate::backend::ConversationInfo>,
    },
}

/// Context passed to the List renderer — owns the overlay data and palette.
struct OverlayContext {
    kind: OverlayKind,
    palette: ChatPalette,
}

/// Render a single list item as a [`Text`] widget.
///
/// Called by tuie's virtualized [`List`] for each visible index. Returns
/// [`None`] when the index is out of range (should not happen with correct
/// item counts).
fn render_item(ctx: &mut OverlayContext, index: usize) -> Option<Box<dyn Widget>> {
    match &ctx.kind {
        OverlayKind::None => None,

        OverlayKind::SlashCommands { selected, matches } => {
            if index >= matches.len() {
                return None;
            }
            let cmd = matches[index];
            let sel = index == *selected;
            let primary = theme::to_tuie_color(ctx.palette.agent_primary);
            let dim = theme::to_tuie_color(ctx.palette.agent_dim);

            let mut content = StyledString::new();
            if sel {
                content.push_span(
                    StyledStr::new(&format!("  {} ", cmd.name))
                        .fg(primary)
                        .bold(),
                );
                content.push_span(StyledStr::new(cmd.hint).fg(dim));
            } else {
                content.push_span(StyledStr::new(&format!("  {} ", cmd.name)));
                content.push_span(StyledStr::new(cmd.hint).fg(dim));
            }
            Some(Text::new().content(content))
        }

        OverlayKind::Conversations {
            selected,
            conversations,
        } => {
            // Index 0 is the header / instructions line.
            if index == 0 {
                let dim = theme::to_tuie_color(ctx.palette.agent_dim);
                let mut content = StyledString::new();
                content.push_span(
                    StyledStr::new(
                        " Conversations — up/down select  Enter resume  n new  Esc dismiss",
                    )
                    .fg(dim),
                );
                return Some(Text::new().content(content));
            }

            let conv_idx = index.saturating_sub(1);
            if conv_idx >= conversations.len() {
                return None;
            }
            let conv = &conversations[conv_idx];
            let sel = conv_idx == *selected;
            let primary = theme::to_tuie_color(ctx.palette.agent_primary);

            let short_id = &conv.id[..8.min(conv.id.len())];
            let summary = conv.summary.as_deref().unwrap_or("(no summary)");
            let label = format!("  {} . {} msgs . {}", short_id, conv.message_count, summary,);

            let mut content = StyledString::new();
            if sel {
                content.push_span(StyledStr::new(&label).fg(primary).bold());
            } else {
                content.push_span(StyledStr::new(&label));
            }
            Some(Text::new().content(content))
        }
    }
}

// ─── Widget ───────────────────────────────────────────────────────────────

/// Floating overlay widget for the chat screen.
///
/// Delegates to an inner [`Pane`] containing a virtualized [`List`].
/// The pane is bordered when visible, borderless when hidden.
pub struct ChatOverlay {
    pane: Box<Pane>,
    list_id: WidgetId,
}

impl DelegateWidget for ChatOverlay {
    tuie::delegate_widget!(pane);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl ChatOverlay {
    /// Create a new, initially-hidden overlay.
    pub fn new(palette: &ChatPalette) -> Box<Self> {
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut list = List::new().vertical();
        list.set_flex(1);
        let list_id = list.get_id().untyped();
        list.set_renderer(
            OverlayContext {
                kind: OverlayKind::None,
                palette: *palette,
            },
            render_item,
        );

        // The pane wraps the list with a border that is toggled on/off.
        let pane = Pane::new()
            .vertical()
            .border_style(Style::new().fg(dim))
            .y_scroll(Scrollbar::AutoHide)
            .child(list as Box<dyn Widget>);

        let mut this = Box::new(Self { pane, list_id });
        this.hide();
        this
    }

    /// Show slash command completions.
    ///
    /// `selected` is the highlighted index, `matches` are the commands that
    /// matched the current input. Up to 8 items are shown.
    pub fn show_slash_commands(
        &mut self,
        selected: usize,
        matches: &[&'static SlashDef],
        palette: &ChatPalette,
    ) {
        let count = matches.len().min(8);
        let dim = theme::to_tuie_color(palette.agent_dim);

        if let Some(list) = self.pane.get_child_mut(self.list_id) {
            if let Some(list) = list.downcast_mut::<List>() {
                list.reset();
                list.set_renderer(
                    OverlayContext {
                        kind: OverlayKind::SlashCommands {
                            selected,
                            matches: matches.to_vec(),
                        },
                        palette: *palette,
                    },
                    render_item,
                );
                list.set_item_count(count);
            }
        }

        self.pane.set_bordered(true);
        self.pane.set_border(Some(Border::SINGLE));
        self.pane.set_border_style(Style::new().fg(dim));
        // Reserve height: one row per visible item plus the top/bottom border.
        // Without this the flex-only List negotiates down to ~0 rows in the
        // chat root's vertical stack and the overlay never appears.
        self.pane.set_height(Some(count.max(1) as u16 + 2));
    }

    /// Show the conversation picker.
    ///
    /// `selected` is the highlighted index into `conversations`. Shows a
    /// header line plus up to 12 conversation rows.
    pub fn show_conversations(
        &mut self,
        selected: usize,
        conversations: &[crate::backend::ConversationInfo],
        palette: &ChatPalette,
    ) {
        // +1 for the instruction header line; max 12 conversations visible.
        let visible_convs = conversations.len().min(12);
        let count = visible_convs + 1;
        let primary = theme::to_tuie_color(palette.agent_primary);

        if let Some(list) = self.pane.get_child_mut(self.list_id) {
            if let Some(list) = list.downcast_mut::<List>() {
                list.reset();
                list.set_renderer(
                    OverlayContext {
                        kind: OverlayKind::Conversations {
                            selected,
                            conversations: conversations.to_vec(),
                        },
                        palette: *palette,
                    },
                    render_item,
                );
                list.set_item_count(count);
            }
        }

        self.pane.set_bordered(true);
        self.pane.set_border(Some(Border::SINGLE));
        self.pane.set_border_style(Style::new().fg(primary));
        self.pane.set_height(Some(count.max(1) as u16 + 2));
    }

    /// Hide the overlay.
    ///
    /// Sets the item count to 0 and removes the border so the widget takes
    /// effectively zero space in the layout.
    pub fn hide(&mut self) {
        if let Some(list) = self.pane.get_child_mut(self.list_id) {
            if let Some(list) = list.downcast_mut::<List>() {
                list.reset();
                list.set_item_count(0);
            }
        }
        self.pane.set_bordered(false);
        // Release the reserved height so the hidden overlay collapses to zero.
        self.pane.set_height(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;

    fn snapshot(overlay: &mut Box<ChatOverlay>, size: Vec2<u16>) -> String {
        let mut term = Emulator::new(&mut **overlay, size);
        term.get_snapshot_text()
    }

    #[test]
    fn overlay_starts_hidden() {
        let mut overlay = ChatOverlay::new(&ChatPalette::default());
        let output = snapshot(&mut overlay, Vec2::new(60, 5));
        assert!(
            !output.contains("/help"),
            "hidden overlay leaked /help: {output:?}"
        );
        assert!(
            !output.contains("/clear"),
            "hidden overlay leaked /clear: {output:?}"
        );
    }

    #[test]
    fn overlay_shows_slash_commands() {
        let matches: Vec<&'static SlashDef> =
            crate::ui::chat::SLASH_COMMANDS.iter().take(2).collect();
        let mut overlay = ChatOverlay::new(&ChatPalette::default());
        overlay.show_slash_commands(0, &matches, &ChatPalette::default());
        let output = snapshot(&mut overlay, Vec2::new(60, 5));
        assert!(
            output.contains("/help"),
            "expected /help in output: {output:?}"
        );
        assert!(
            output.contains("/clear"),
            "expected /clear in output: {output:?}"
        );
    }

    #[test]
    fn overlay_hide_clears() {
        let matches: Vec<&'static SlashDef> =
            crate::ui::chat::SLASH_COMMANDS.iter().take(2).collect();
        let mut overlay = ChatOverlay::new(&ChatPalette::default());
        overlay.show_slash_commands(0, &matches, &ChatPalette::default());
        overlay.hide();
        let output = snapshot(&mut overlay, Vec2::new(60, 5));
        assert!(!output.contains("/help"), "hide leaked /help: {output:?}");
        assert!(!output.contains("/clear"), "hide leaked /clear: {output:?}");
    }

    #[test]
    fn overlay_shows_conversations() {
        let convos = vec![crate::backend::ConversationInfo {
            id: "abc12345".into(),
            agent_id: "agent1".into(),
            summary: Some("test conv".into()),
            message_count: 5,
            updated_at: "2024-01-01".into(),
        }];
        let mut overlay = ChatOverlay::new(&ChatPalette::default());
        overlay.show_conversations(0, &convos, &ChatPalette::default());
        let output = snapshot(&mut overlay, Vec2::new(80, 5));
        assert!(
            output.contains("abc12345") || output.contains("test conv"),
            "expected conv id or summary in output: {output:?}",
        );
    }
}
