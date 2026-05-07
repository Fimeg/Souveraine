//! Lightweight markdown → `Vec<Line<'static>>` renderer for the TUI chat.
//!
//! Pattern lifted from `jcode-tui-markdown` (jcode uses `pulldown-cmark = 0.12`
//! and renders to ratatui Lines). This is a much smaller subset focused on
//! what an agent will produce in a coding-oriented chat:
//!
//! - Headings (h1–h3)
//! - Bold (`**`), italic (`*` or `_`), inline code (`` ` ``)
//! - Fenced code blocks (```` ``` ````) with optional language label
//! - Bullet and ordered lists
//! - Links rendered as `text (url)` in dim color
//! - Block quotes
//!
//! Returns ratatui `Line<'static>` so the chat module can drop the output
//! straight into a `Paragraph`.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const CODE_BG: Color = Color::Rgb(38, 38, 46);
const CODE_FG: Color = Color::Rgb(220, 220, 230);
const LINK_DIM: Color = Color::Rgb(140, 160, 200);
const QUOTE_BAR: Color = Color::Rgb(120, 100, 150);
const HEADING: Color = Color::Rgb(255, 200, 120);
const BULLET: Color = Color::Rgb(180, 180, 180);

/// Render markdown to a Vec of styled lines.
pub fn render(md: &str, default_fg: Color) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(md, opts);

    let mut renderer = Renderer::new(default_fg);
    for ev in parser {
        renderer.handle(ev);
    }
    renderer.flush();
    renderer.lines
}

struct Renderer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    in_code_block: bool,
    code_lang: Option<String>,
    list_stack: Vec<ListMode>,
    quote_depth: usize,
    default_fg: Color,
    /// Pending list item bullet to emit on next text.
    pending_bullet: Option<String>,
}

enum ListMode {
    Bullet,
    Ordered(u64),
}

impl Renderer {
    fn new(default_fg: Color) -> Self {
        Self {
            lines: Vec::new(),
            current: Vec::new(),
            style_stack: vec![Style::default().fg(default_fg)],
            in_code_block: false,
            code_lang: None,
            list_stack: Vec::new(),
            quote_depth: 0,
            default_fg,
            pending_bullet: None,
        }
    }

    fn current_style(&self) -> Style {
        *self.style_stack.last().unwrap()
    }

    fn push_style(&mut self, s: Style) {
        self.style_stack.push(s);
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    fn flush(&mut self) {
        if !self.current.is_empty() {
            let line = Line::from(std::mem::take(&mut self.current));
            self.lines.push(line);
        }
    }

    fn newline(&mut self) {
        self.flush();
    }

    fn quote_prefix(&self) -> Option<Span<'static>> {
        if self.quote_depth > 0 {
            Some(Span::styled(
                "▌ ".repeat(self.quote_depth),
                Style::default().fg(QUOTE_BAR),
            ))
        } else {
            None
        }
    }

    fn ensure_line_started(&mut self) {
        if self.current.is_empty() {
            if let Some(p) = self.quote_prefix() {
                self.current.push(p);
            }
            if let Some(b) = self.pending_bullet.take() {
                self.current.push(Span::styled(b, Style::default().fg(BULLET)));
            }
        }
    }

    fn handle(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                self.ensure_line_started();
                let style = self.current_style();
                let style = if self.in_code_block {
                    Style::default().fg(CODE_FG).bg(CODE_BG)
                } else {
                    style
                };
                // Code blocks may include newlines inside one Text event.
                let text = t.into_string();
                let mut first = true;
                for chunk in text.split('\n') {
                    if !first {
                        self.flush();
                    }
                    first = false;
                    if !chunk.is_empty() {
                        self.ensure_line_started();
                        self.current.push(Span::styled(chunk.to_string(), style));
                    }
                }
            }
            Event::Code(c) => {
                self.ensure_line_started();
                self.current.push(Span::styled(
                    format!("`{}`", c.into_string()),
                    Style::default().fg(CODE_FG).bg(CODE_BG),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                self.flush();
            }
            Event::Rule => {
                self.flush();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::Rgb(80, 80, 80)),
                )));
            }
            // Pulldown-cmark events we don't render specially fall through.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.lines.push(Line::from(Span::raw("")));
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    _ => "#### ",
                };
                self.current.push(Span::styled(
                    prefix.to_string(),
                    Style::default().fg(HEADING).add_modifier(Modifier::BOLD),
                ));
                self.push_style(Style::default().fg(HEADING).add_modifier(Modifier::BOLD));
            }
            Tag::Paragraph => {
                // Blank line between paragraphs (but not inside list items
                // or quotes — pulldown emits the right structure).
            }
            Tag::Strong => {
                let s = self.current_style().add_modifier(Modifier::BOLD);
                self.push_style(s);
            }
            Tag::Emphasis => {
                let s = self.current_style().add_modifier(Modifier::ITALIC);
                self.push_style(s);
            }
            Tag::Strikethrough => {
                let s = self.current_style().add_modifier(Modifier::CROSSED_OUT);
                self.push_style(s);
            }
            Tag::Link { dest_url, .. } => {
                self.push_style(
                    Style::default()
                        .fg(LINK_DIM)
                        .add_modifier(Modifier::UNDERLINED),
                );
                // Defer the URL; emit on Tag::End(Link).
                self.code_lang = Some(dest_url.into_string());
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.in_code_block = true;
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(s) => Some(s.into_string()),
                    pulldown_cmark::CodeBlockKind::Indented => None,
                };
                self.code_lang = lang.clone();
                let label = lang.filter(|s| !s.is_empty()).unwrap_or_else(|| "code".to_string());
                self.lines.push(Line::from(Span::styled(
                    format!("┌─ {} ", label),
                    Style::default().fg(Color::Rgb(120, 120, 130)),
                )));
            }
            Tag::List(start) => {
                self.flush();
                self.list_stack.push(match start {
                    Some(n) => ListMode::Ordered(n),
                    None => ListMode::Bullet,
                });
            }
            Tag::Item => {
                self.flush();
                let bullet = match self.list_stack.last_mut() {
                    Some(ListMode::Bullet) => "  • ".to_string(),
                    Some(ListMode::Ordered(n)) => {
                        let s = format!("  {}. ", n);
                        *n += 1;
                        s
                    }
                    None => "  • ".to_string(),
                };
                self.pending_bullet = Some(bullet);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush();
            }
            TagEnd::Paragraph => {
                self.flush();
                self.lines.push(Line::from(Span::raw("")));
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                self.pop_style();
            }
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.code_lang.take() {
                    self.current.push(Span::styled(
                        format!(" ({})", url),
                        Style::default()
                            .fg(LINK_DIM)
                            .add_modifier(Modifier::DIM),
                    ));
                }
            }
            TagEnd::CodeBlock => {
                self.flush();
                self.in_code_block = false;
                self.code_lang = None;
                self.lines.push(Line::from(Span::styled(
                    "└─".to_string(),
                    Style::default().fg(Color::Rgb(120, 120, 130)),
                )));
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                self.flush();
            }
            TagEnd::Item => {
                self.flush();
                self.pending_bullet = None;
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.flush();
            }
            _ => {}
        }
        // Suppress unused-variable warnings for the default fg color in
        // contexts where it's not directly referenced.
        let _ = self.default_fg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_plain_paragraph() {
        let lines = render("hello world", Color::White);
        assert!(!lines.is_empty());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("hello world"));
    }

    #[test]
    fn renders_inline_code() {
        let lines = render("call `foo()` then", Color::White);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("`foo()`"));
    }

    #[test]
    fn renders_fenced_code_block_with_lang() {
        let md = "```rust\nfn main() {}\n```";
        let lines = render(md, Color::White);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("rust"));
        assert!(joined.contains("fn main()"));
    }

    #[test]
    fn renders_heading() {
        let lines = render("# Big\n\nbody", Color::White);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("# Big"));
        assert!(joined.contains("body"));
    }

    #[test]
    fn renders_bullet_list() {
        let lines = render("- one\n- two", Color::White);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("• one"));
        assert!(joined.contains("• two"));
    }
}
