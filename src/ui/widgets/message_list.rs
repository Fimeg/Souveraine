#![allow(dead_code)] // WIP scaffolding not yet wired
//! Virtualized message list — wraps tuie `List` with per-message widget rendering.
//!
//! Each message in the conversation becomes a widget (bubble, tool card,
//! interjection, etc.) rendered on demand via the List's render callback.

use std::sync::{Arc, Mutex};

use tuie::prelude::*;

/// Kinds of messages the list can render.
#[derive(Clone, Debug)]
pub enum MsgKind {
    User {
        name: String,
        text: String,
    },
    Assistant {
        name: String,
        text: String,
        streaming: bool,
    },
    Surfacing {
        source: String,
        content: String,
        priority: String,
    },
    System {
        text: String,
    },
    SystemContext {
        summary: String,
        _full_text: String,
    },
    Tool {
        name: String,
        args_summary: String,
        round: u32,
        is_error: bool,
        is_pending: bool,
        result_output: Option<String>,
    },
    Interjection {
        text: String,
        delivered: bool,
    },
    Interstitial {
        text: String,
        is_voice: bool,
    },
}

/// Render context passed to the List widget's render callback.
pub struct MessageListContext {
    pub messages: Vec<MsgKind>,
    pub palette: crate::ui::chat::ChatPalette,
    pub container_width: u16,
    pub tool_cards_expanded: bool,
}

/// Virtualized scrollable message list.
pub struct MessageList {
    list: Box<List>,
    ctx: Arc<Mutex<MessageListContext>>,
}

impl DelegateWidget for MessageList {
    tuie::delegate_widget!(list);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl MessageList {
    pub fn new() -> Box<Self> {
        let mut list = List::new();
        list.set_flex(1); // Expand to fill available space
        Box::new(Self {
            list,
            ctx: Arc::new(Mutex::new(MessageListContext {
                messages: Vec::new(),
                palette: crate::ui::chat::ChatPalette::default(),
                container_width: 120,
                tool_cards_expanded: true,
            })),
        })
    }

    /// Set the messages to display and rebuild the list.
    pub fn set_messages(&mut self, messages: Vec<MsgKind>) {
        let count = {
            let mut ctx = self.ctx.lock().unwrap();
            ctx.messages = messages;
            ctx.messages.len()
        };
        self.list.set_item_count(count);
        self.list.dirty_layout();
    }

    pub fn set_palette(&mut self, palette: crate::ui::chat::ChatPalette) {
        self.ctx.lock().unwrap().palette = palette;
        self.list.invalidate_all();
    }

    pub fn set_container_width(&mut self, w: u16) {
        self.ctx.lock().unwrap().container_width = w;
        self.list.invalidate_all();
    }

    pub fn set_tool_cards_expanded(&mut self, expanded: bool) {
        self.ctx.lock().unwrap().tool_cards_expanded = expanded;
        self.list.invalidate_all();
    }

    /// Scroll to the bottom of the list.
    pub fn scroll_to_bottom(&mut self) {
        let count = self.ctx.lock().unwrap().messages.len();
        if count > 0 {
            self.list.ensure_visible(count.saturating_sub(1));
        }
    }

    /// Scroll up/down by one page.
    pub fn scroll_page(&mut self, up: bool) {
        let visible = self.list.get_visible_range();
        let page_size = (visible.end.saturating_sub(visible.start)) as i32;
        if page_size > 0 {
            let delta = if up { -page_size } else { page_size };
            self.list.scroll_by(delta);
        }
    }

    /// Set the renderer callback to produce widgets for each message.
    pub fn attach_renderer(&mut self) {
        let ctx = self.ctx.clone();
        self.list.set_renderer(ctx, render_message);
    }
}

fn render_message(
    ctx: &mut Arc<Mutex<MessageListContext>>,
    index: usize,
) -> Option<Box<dyn Widget>> {
    let msgs = ctx.lock().unwrap();
    let msg = msgs.messages.get(index)?;
    let w = msgs.container_width;
    let max_bubble = ((w as usize).saturating_sub(8) * 70 / 100).max(20);

    match msg {
        MsgKind::User { name, text } => {
            let p = &msgs.palette;
            let wrap_w = max_bubble.saturating_sub(4);
            let body = crate::ui::widgets::chat_bubble::wrap_text_lines(text, wrap_w);
            Some(
                super::chat_bubble::ChatBubble::new()
                    .title(format!("\u{29c9} {name}"))
                    .body(body)
                    .max_width(max_bubble)
                    .align(super::chat_bubble::BubbleAlign::Right)
                    .border_style(to_style(p.user_accent))
                    .container_width(w),
            )
        }
        MsgKind::Assistant {
            name,
            text,
            streaming,
        } => {
            let p = &msgs.palette;
            let label = if *streaming {
                format!("\u{29c9} {name} \u{25e6}")
            } else {
                format!("\u{29c9} {name}")
            };
            let md_palette = crate::ui::tuie_markdown::MarkdownPalette::from_chat_palette(p);
            let styled = crate::ui::tuie_markdown::render(text, &md_palette);
            Some(
                super::chat_bubble::ChatBubble::new()
                    .title(label)
                    .body_styled(styled)
                    .max_width(max_bubble)
                    .align(super::chat_bubble::BubbleAlign::Left)
                    .border_style(to_style(p.agent_primary))
                    .container_width(w),
            )
        }
        MsgKind::Tool {
            name,
            args_summary,
            round,
            is_error,
            is_pending,
            result_output,
        } => {
            let p = &msgs.palette;
            let mode = if msgs.tool_cards_expanded {
                super::tool_card::ToolCardMode::Expanded
            } else {
                super::tool_card::ToolCardMode::Compact
            };
            let glyph_color = if *is_error {
                p.compaction
            } else {
                p.tool_accent
            };
            Some(
                super::tool_card::ToolCard::new()
                    .name(name.clone())
                    .args_summary(args_summary.clone())
                    .round(*round)
                    .is_error(*is_error)
                    .is_pending(*is_pending)
                    .result_output(result_output.clone())
                    .mode(mode)
                    .glyph_style(to_style(glyph_color))
                    .name_style(to_style(glyph_color).bold())
                    .dim_style(to_style(p.tool_dim))
                    .container_width(w),
            )
        }
        MsgKind::Surfacing {
            source,
            content,
            priority,
        } => {
            let p = &msgs.palette;
            let label = format!("surfacing \u{00b7} {source} \u{00b7} {priority}");
            let md_palette = crate::ui::tuie_markdown::MarkdownPalette::from_chat_palette(p);
            let styled = crate::ui::tuie_markdown::render(content, &md_palette);
            Some(
                super::chat_bubble::ChatBubble::new()
                    .title(label)
                    .body_styled(styled)
                    .max_width(64)
                    .align(super::chat_bubble::BubbleAlign::Center)
                    .border_style(to_style(p.surfacing))
                    .container_width(w),
            )
        }
        MsgKind::System { text } => {
            let p = &msgs.palette;
            let mut t = Text::new().content(text.clone()).word_wrap();
            t.set_style(to_style(p.agent_dim).italic());
            Some(t)
        }
        MsgKind::SystemContext { summary, .. } => {
            let p = &msgs.palette;
            let mut t = Text::new().content(format!("  > {summary}")).word_wrap();
            t.set_style(to_style(p.agent_dim).dim());
            Some(t)
        }
        MsgKind::Interjection { text, delivered } => {
            let p = &msgs.palette;
            let color = if *delivered { p.agent_dim } else { p.surfacing };
            let label = if *delivered { "noticed" } else { "hand raised" };
            let mut t = Text::new()
                .content(format!("  > {label}  {text}"))
                .word_wrap();
            t.set_style(to_style(color).italic());
            Some(t)
        }
        MsgKind::Interstitial { text, is_voice } => {
            let p = &msgs.palette;
            if *is_voice {
                let mut t = Text::new().content(format!("  | {text}")).word_wrap();
                t.set_style(to_style(p.agent_primary).dim());
                Some(t)
            } else {
                let mut t = Text::new().content(format!("  > {text}")).word_wrap();
                t.set_style(to_style(p.agent_dim).italic());
                Some(t)
            }
        }
    }
}

/// Convert a ratatui Color to tuie Style (with just foreground set).
fn to_style(c: ratatui::style::Color) -> Style {
    let color = crate::ui::theme::to_tuie_color(c);
    Style::new().fg(color)
}
