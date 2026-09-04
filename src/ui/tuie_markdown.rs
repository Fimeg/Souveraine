use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use tuie::prelude::{Color, Style, StyledStr, StyledString, UnderlineType};

use crate::ui::chat::ChatPalette;
use crate::ui::theme::to_tuie_color;

#[derive(Debug, Clone, Copy)]
pub struct MarkdownPalette {
    pub code_bg: Color,
    pub code_fg: Color,
    pub link_dim: Color,
    pub quote_bar: Color,
    pub heading: Color,
    pub bullet: Color,
    pub default_fg: Color,
}

impl MarkdownPalette {
    pub fn from_chat_palette(cpal: &ChatPalette) -> Self {
        let (pr, pg, pb) = match cpal.agent_primary {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            _ => (255, 140, 66),
        };
        let (br, bg_g, bb) = match cpal.bg {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            _ => (20, 20, 30),
        };
        Self {
            code_bg: Color::Rgb(
                br.saturating_add(22),
                bg_g.saturating_add(22),
                bb.saturating_add(34),
            ),
            code_fg: Color::Rgb(pr.max(160), pg.max(160), pb.max(200)),
            link_dim: Color::Rgb(
                (pr / 3).saturating_add(100).min(220),
                (pg / 3).saturating_add(120).min(220),
                (pb / 2).saturating_add(100).min(220),
            ),
            quote_bar: Color::Rgb(
                (pr / 3).saturating_add(80).min(200),
                (pg / 3).saturating_add(70).min(180),
                (pb / 2).saturating_add(80).min(200),
            ),
            heading: Color::Rgb(pr.max(200), pg.max(160), pb.max(80)),
            bullet: Color::Rgb(
                (pr / 2).saturating_add(80),
                (pg / 2).saturating_add(80),
                (pb / 2).saturating_add(80),
            ),
            default_fg: to_tuie_color(cpal.agent_dim),
        }
    }
}

pub fn render(md: &str, palette: &MarkdownPalette) -> StyledString {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(md, opts);

    let mut r = Renderer::new(palette);
    for ev in parser {
        r.handle(ev);
    }
    r.flush();
    r.out
}

struct StyleEntry {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    italic: bool,
    dim: bool,
    strikethrough: bool,
    underline: bool,
}

impl StyleEntry {
    fn to_style(&self) -> Style {
        let mut s = Style::new();
        if let Some(fg) = self.fg {
            s.set_fg(Some(fg));
        }
        if let Some(bg) = self.bg {
            s.set_bg(Some(bg));
        }
        if self.bold {
            s.set_bold(true);
        }
        if self.italic {
            s.set_italic(true);
        }
        if self.dim {
            s.set_dim(true);
        }
        if self.strikethrough {
            s.set_strikethrough(true);
        }
        if self.underline {
            s.set_underline(Some(UnderlineType::Single));
        }
        s
    }
}

struct Renderer<'a> {
    out: StyledString,
    style_stack: Vec<StyleEntry>,
    in_code_block: bool,
    code_lang: Option<String>,
    list_stack: Vec<ListMode>,
    quote_depth: usize,
    pending_bullet: Option<String>,
    at_line_start: bool,
    palette: &'a MarkdownPalette,
}

enum ListMode {
    Bullet,
    Ordered(u64),
}

impl<'a> Renderer<'a> {
    fn new(palette: &'a MarkdownPalette) -> Self {
        Self {
            out: StyledString::new(),
            style_stack: vec![StyleEntry {
                fg: Some(palette.default_fg),
                bg: None,
                bold: false,
                italic: false,
                dim: false,
                strikethrough: false,
                underline: false,
            }],
            in_code_block: false,
            code_lang: None,
            list_stack: Vec::new(),
            quote_depth: 0,
            pending_bullet: None,
            at_line_start: true,
            palette,
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().unwrap().to_style()
    }

    fn push_style(&mut self, entry: StyleEntry) {
        self.style_stack.push(entry);
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    fn emit(&mut self, text: &str, style: Style) {
        self.out.push_span(StyledStr { text, style });
    }

    fn newline(&mut self) {
        self.out.push_str("\n");
        self.at_line_start = true;
    }

    fn flush(&mut self) {
        // trim trailing newlines
        let text_bytes = self.out.as_ref().as_bytes();
        let mut trailing = 0;
        for &b in text_bytes.iter().rev() {
            if b == b'\n' {
                trailing += 1;
            } else {
                break;
            }
        }
        if trailing > 1 {
            self.out.drop_end(trailing - 1);
        }
    }

    fn ensure_line_started(&mut self) {
        if self.at_line_start {
            if self.quote_depth > 0 {
                let bar = "\u{258c} ".repeat(self.quote_depth);
                let style = Style::new().fg(self.palette.quote_bar);
                self.emit(&bar, style);
            }
            if let Some(bullet) = self.pending_bullet.take() {
                let style = Style::new().fg(self.palette.bullet);
                self.emit(&bullet, style);
            }
            self.at_line_start = false;
        }
    }

    fn handle(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                let text = t.into_string();
                if self.in_code_block {
                    let style = Style::new()
                        .fg(self.palette.code_fg)
                        .bg(self.palette.code_bg);
                    for (i, line) in text.split('\n').enumerate() {
                        if i > 0 {
                            self.newline();
                            self.at_line_start = false; // suppress quote prefix inside code
                        }
                        if !line.is_empty() {
                            self.emit(line, style);
                        }
                    }
                } else {
                    for (i, chunk) in text.split('\n').enumerate() {
                        if i > 0 {
                            self.newline();
                        }
                        if !chunk.is_empty() {
                            self.ensure_line_started();
                            let style = self.current_style();
                            self.emit(chunk, style);
                        }
                    }
                }
            }
            Event::Code(c) => {
                self.ensure_line_started();
                let code_text = format!("`{}`", c.into_string());
                let style = Style::new()
                    .fg(self.palette.code_fg)
                    .bg(self.palette.code_bg);
                self.emit(&code_text, style);
            }
            Event::SoftBreak | Event::HardBreak => {
                self.newline();
            }
            Event::Rule => {
                self.newline();
                let rule = "\u{2500}".repeat(40);
                let style = Style::new().fg(self.palette.bullet);
                self.emit(&rule, style);
                self.newline();
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                self.newline();
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    _ => "#### ",
                };
                self.at_line_start = false;
                let style = Style::new().fg(self.palette.heading).bold();
                self.emit(prefix, style);
                self.push_style(StyleEntry {
                    fg: Some(self.palette.heading),
                    bg: None,
                    bold: true,
                    italic: false,
                    dim: false,
                    strikethrough: false,
                    underline: false,
                });
            }
            Tag::Paragraph => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with("\n\n") {
                    if !self.out.as_ref().ends_with('\n') {
                        self.newline();
                    }
                    self.newline();
                }
            }
            Tag::Strong => {
                let prev = self.style_stack.last().unwrap();
                self.push_style(StyleEntry {
                    bold: true,
                    fg: prev.fg,
                    bg: prev.bg,
                    italic: prev.italic,
                    dim: prev.dim,
                    strikethrough: prev.strikethrough,
                    underline: prev.underline,
                });
            }
            Tag::Emphasis => {
                let prev = self.style_stack.last().unwrap();
                self.push_style(StyleEntry {
                    italic: true,
                    fg: prev.fg,
                    bg: prev.bg,
                    bold: prev.bold,
                    dim: prev.dim,
                    strikethrough: prev.strikethrough,
                    underline: prev.underline,
                });
            }
            Tag::Strikethrough => {
                let prev = self.style_stack.last().unwrap();
                self.push_style(StyleEntry {
                    strikethrough: true,
                    fg: prev.fg,
                    bg: prev.bg,
                    bold: prev.bold,
                    italic: prev.italic,
                    dim: prev.dim,
                    underline: prev.underline,
                });
            }
            Tag::Link { dest_url, .. } => {
                self.push_style(StyleEntry {
                    fg: Some(self.palette.link_dim),
                    bg: None,
                    bold: false,
                    italic: false,
                    dim: false,
                    strikethrough: false,
                    underline: true,
                });
                self.code_lang = Some(dest_url.into_string());
            }
            Tag::CodeBlock(kind) => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                self.in_code_block = true;
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(s) => Some(s.into_string()),
                    pulldown_cmark::CodeBlockKind::Indented => None,
                };
                self.code_lang = lang.clone();
                let label = lang
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "code".into());
                let header = format!("\u{250c}\u{2500} {} ", label);
                let style = Style::new().fg(self.palette.code_fg).dim();
                self.emit(&header, style);
                self.newline();
                self.at_line_start = false;
            }
            Tag::List(start) => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                self.list_stack.push(match start {
                    Some(n) => ListMode::Ordered(n),
                    None => ListMode::Bullet,
                });
            }
            Tag::Item => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                let bullet = match self.list_stack.last_mut() {
                    Some(ListMode::Bullet) => "  \u{2022} ".to_string(),
                    Some(ListMode::Ordered(n)) => {
                        let s = format!("  {}. ", n);
                        *n += 1;
                        s
                    }
                    None => "  \u{2022} ".to_string(),
                };
                self.pending_bullet = Some(bullet);
            }
            Tag::BlockQuote(_) => {
                if !self.out.as_ref().is_empty() && !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                self.quote_depth += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                self.newline();
            }
            TagEnd::Paragraph => {
                if !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                self.pop_style();
            }
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.code_lang.take() {
                    let url_text = format!(" ({})", url);
                    let style = Style::new().fg(self.palette.link_dim).dim();
                    self.emit(&url_text, style);
                }
            }
            TagEnd::CodeBlock => {
                if !self.out.as_ref().ends_with('\n') {
                    self.newline();
                }
                self.in_code_block = false;
                self.code_lang = None;
                let footer = "\u{2514}\u{2500}";
                let style = Style::new().fg(self.palette.code_fg).dim();
                self.at_line_start = false;
                self.emit(footer, style);
                self.newline();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.pending_bullet = None;
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_palette() -> MarkdownPalette {
        MarkdownPalette {
            code_bg: Color::Rgb(30, 30, 40),
            code_fg: Color::Rgb(200, 200, 200),
            link_dim: Color::Rgb(100, 100, 140),
            quote_bar: Color::Rgb(80, 80, 80),
            heading: Color::Rgb(255, 200, 100),
            bullet: Color::Rgb(150, 150, 150),
            default_fg: Color::Foreground,
        }
    }

    #[test]
    fn renders_plain_paragraph() {
        let out = render("hello world", &test_palette());
        assert!(out.as_ref().contains("hello world"));
    }

    #[test]
    fn renders_inline_code() {
        let out = render("call `foo()` then", &test_palette());
        assert!(out.as_ref().contains("`foo()`"));
    }

    #[test]
    fn renders_fenced_code_block() {
        let out = render("```rust\nfn main() {}\n```", &test_palette());
        assert!(out.as_ref().contains("rust"));
        assert!(out.as_ref().contains("fn main()"));
    }

    #[test]
    fn renders_heading() {
        let out = render("# Big\n\nbody", &test_palette());
        assert!(out.as_ref().contains("# Big"));
        assert!(out.as_ref().contains("body"));
    }

    #[test]
    fn renders_bullet_list() {
        let out = render("- one\n- two", &test_palette());
        assert!(out.as_ref().contains("\u{2022} one"));
        assert!(out.as_ref().contains("\u{2022} two"));
    }

    #[test]
    fn renders_blockquote() {
        let out = render("> quoted text", &test_palette());
        assert!(out.as_ref().contains("quoted text"));
        assert!(out.as_ref().contains("\u{258c}"));
    }
}
