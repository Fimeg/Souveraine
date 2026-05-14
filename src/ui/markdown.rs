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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::chat::ChatPalette;

/// Palette of markdown-specific colours derived from the agent's atmosphere.
/// Constructed from a `ChatPalette` at the call site in chat.rs.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownPalette {
    pub code_bg: Color,
    pub code_fg: Color,
    pub link_dim: Color,
    pub quote_bar: Color,
    pub heading: Color,
    pub bullet: Color,
}

impl MarkdownPalette {
    pub fn from_chat_palette(cpal: &ChatPalette) -> Self {
        let (pr, pg, pb) = match cpal.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let r = |c: Color| -> u8 { match c { Color::Rgb(r, _, _) => r, _ => 0 } };
        let g = |c: Color| -> u8 { match c { Color::Rgb(_, g, _) => g, _ => 0 } };
        let b = |c: Color| -> u8 { match c { Color::Rgb(_, _, b) => b, _ => 0 } };
        let bg = cpal.bg;
        Self {
            code_bg: Color::Rgb(r(bg).saturating_add(22).min(255), g(bg).saturating_add(22).min(255), b(bg).saturating_add(34).min(255)),
            code_fg: Color::Rgb(pr.max(160), pg.max(160), pb.max(200)),
            link_dim: Color::Rgb((pr / 3).saturating_add(100).min(220), (pg / 3).saturating_add(120).min(220), (pb / 2).saturating_add(100).min(220)),
            quote_bar: Color::Rgb((pr / 3).saturating_add(80).min(200), (pg / 3).saturating_add(70).min(180), (pb / 2).saturating_add(80).min(200)),
            heading: Color::Rgb(pr.max(200), pg.max(160), pb.max(80)),
            bullet: Color::Rgb((pr / 2).saturating_add(80), (pg / 2).saturating_add(80), (pb / 2).saturating_add(80)),
        }
    }
}

/// Render markdown to a Vec of styled lines.
pub fn render(md: &str, default_fg: Color, mdpal: &MarkdownPalette) -> Vec<Line<'static>> {
    render_with_width(md, default_fg, None, mdpal)
}

/// Render markdown to a Vec of styled lines, optionally wrapping any line
/// wider than `max_width` cells. Pattern lifted from jcode's
/// `render_markdown_with_width` — when the width is known up-front (e.g.
/// inside a chat bubble), wrapping at render time prevents `Paragraph`'s
/// internal re-wrap from inflating the visual line count and breaking
/// scroll math. Span styling is preserved across wrap points.
pub fn render_with_width(
    md: &str,
    default_fg: Color,
    max_width: Option<usize>,
    mdpal: &MarkdownPalette,
) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(md, opts);

    let mut renderer = Renderer::new(default_fg, mdpal);
    for ev in parser {
        renderer.handle(ev);
    }
    renderer.flush();

    match max_width {
        Some(w) if w > 0 => wrap_lines(renderer.lines, w),
        _ => renderer.lines,
    }
}

/// Wrap every line in `lines` to `max_width` cells. Span-aware (styles
/// carry through wrap points), gutter-aware (wrapped continuation lines
/// under a bullet or numbered marker are indented to align under the
/// bullet's text). Tries balanced minimum-raggedness wrap first; falls
/// back to greedy word/char-level wrap when balanced wrap can't apply.
pub fn wrap_lines(lines: Vec<Line<'static>>, max_width: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return lines;
    }
    lines
        .into_iter()
        .flat_map(|l| wrap_line(l, max_width))
        .collect()
}

/// Wrap a single styled line, using the default bullet-aware gutter for
/// continuation indentation.
pub fn wrap_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    wrap_line_with_gutter(line, max_width, bullet_gutter)
}

/// Full-power wrap: `gutter_fn` decides, per-line, whether continuation
/// lines should be prefixed with a styled gutter (e.g. spaces matching a
/// bullet's text-start column). Returns `(spans, width)` or `None`.
/// Pattern from `jcode/crates/jcode-tui-markdown/src/markdown_wrap.rs::wrap_line`.
pub fn wrap_line_with_gutter(
    line: Line<'static>,
    max_width: usize,
    gutter_fn: impl Fn(&Line<'static>) -> Option<(Vec<Span<'static>>, usize)> + Copy,
) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![line];
    }
    let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if UnicodeWidthStr::width(line_text.as_str()) <= max_width {
        return vec![line];
    }

    let alignment = line.alignment;

    // Continuation gutter — only kept if it's positive width and narrower
    // than the line budget. Otherwise the wrap would loop on the prefix.
    let gutter = gutter_fn(&line).and_then(|(spans, w)| {
        if w == 0 || w >= max_width { None } else { Some((spans, w)) }
    });

    // Try the balanced minimum-raggedness DP first. It produces visually
    // smoother wraps (favors lines of equal width). Only succeeds for
    // lines with clean word boundaries; falls through to greedy wrap when
    // the line has tabs, doubled spaces, leading/trailing whitespace, or
    // single-word content.
    if let Some(balanced) = wrap_line_balanced(&line, max_width, gutter.as_ref()) {
        return balanced;
    }

    let initial_gutter_width = gutter.as_ref().map(|(_, w)| *w).unwrap_or(0);

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::with_capacity(line.spans.len());
    let mut current_width: usize = 0;
    let mut current_has_content = false;
    let mut pending_gutter = false;

    fn flush(
        result: &mut Vec<Line<'static>>,
        current: &mut Vec<Span<'static>>,
        current_width: &mut usize,
        current_has_content: &mut bool,
        pending_gutter: &mut bool,
        alignment: Option<ratatui::layout::Alignment>,
        gutter_present: bool,
    ) {
        if current.is_empty() {
            return;
        }
        let mut nl = Line::from(std::mem::take(current));
        if let Some(a) = alignment {
            nl = nl.alignment(a);
        }
        result.push(nl);
        *current_width = 0;
        *current_has_content = false;
        *pending_gutter = gutter_present;
    }

    let seed_gutter = |current: &mut Vec<Span<'static>>,
                       current_width: &mut usize,
                       pending: &mut bool| {
        if *pending {
            if let Some((g_spans, g_w)) = &gutter {
                current.extend(g_spans.iter().cloned());
                *current_width = *g_w;
            }
            *pending = false;
        }
    };

    for span in line.spans {
        let style = span.style;
        let text = span.content.into_owned();
        let mut remaining = text.as_str();

        while !remaining.is_empty() {
            let (chunk, rest): (String, &str) = if let Some(space_idx) = remaining.find(' ') {
                let (word, after) = remaining.split_at(space_idx);
                let mut buf = String::with_capacity(word.len() + 1);
                buf.push_str(word);
                buf.push(' ');
                let rest = if after.len() > 1 { &after[1..] } else { "" };
                (buf, rest)
            } else {
                (remaining.to_string(), "")
            };
            remaining = rest;

            let chunk_w = UnicodeWidthStr::width(chunk.as_str());

            if current_width + chunk_w > max_width && current_has_content {
                flush(
                    &mut result,
                    &mut current,
                    &mut current_width,
                    &mut current_has_content,
                    &mut pending_gutter,
                    alignment,
                    gutter.is_some(),
                );
            }

            if chunk_w > max_width {
                // Char-level split for monster tokens.
                let mut part = String::new();
                let mut part_w = 0usize;
                for c in chunk.chars() {
                    seed_gutter(&mut current, &mut current_width, &mut pending_gutter);
                    let cw = c.width().unwrap_or(0);
                    if current_width + part_w + cw > max_width && (current_width + part_w) > 0 {
                        if !part.is_empty() {
                            current.push(Span::styled(std::mem::take(&mut part), style));
                            let before = current_width;
                            current_width += part_w;
                            if before + part_w > initial_gutter_width {
                                current_has_content = true;
                            }
                            part_w = 0;
                        }
                        if current_has_content {
                            flush(
                                &mut result,
                                &mut current,
                                &mut current_width,
                                &mut current_has_content,
                                &mut pending_gutter,
                                alignment,
                                gutter.is_some(),
                            );
                        }
                        seed_gutter(&mut current, &mut current_width, &mut pending_gutter);
                    }
                    part.push(c);
                    part_w += cw;
                }
                if !part.is_empty() {
                    seed_gutter(&mut current, &mut current_width, &mut pending_gutter);
                    current.push(Span::styled(part, style));
                    let before = current_width;
                    current_width += part_w;
                    if before + part_w > initial_gutter_width {
                        current_has_content = true;
                    }
                }
            } else {
                seed_gutter(&mut current, &mut current_width, &mut pending_gutter);
                current.push(Span::styled(chunk, style));
                let before = current_width;
                current_width += chunk_w;
                if before + chunk_w > initial_gutter_width {
                    current_has_content = true;
                }
            }
        }
    }

    if !current.is_empty() && current_has_content {
        let mut nl = Line::from(current);
        if let Some(a) = alignment {
            nl = nl.alignment(a);
        }
        result.push(nl);
    }

    if result.is_empty() {
        let mut empty = Line::from("");
        if let Some(a) = alignment {
            empty = empty.alignment(a);
        }
        result.push(empty);
    }

    result
}

/// Default gutter detector. If a line starts with `  • ` / `  - ` / `  * ` /
/// `  · ` (Souveraine's bullet markers, see `Renderer::start::Tag::Item`)
/// or `  N. ` (numbered list), return spaces of equal display width so
/// continuation lines hang under the text, not under the bullet.
fn bullet_gutter(line: &Line<'static>) -> Option<(Vec<Span<'static>>, usize)> {
    let first = line.spans.first()?;
    let text = first.content.as_ref();
    // Bullet form: leading whitespace, a bullet char, then a single space.
    let trimmed = text.trim_start();
    let leading = text.len() - trimmed.len();
    let bullet_match = ["• ", "- ", "* ", "· "]
        .iter()
        .find(|m| trimmed.starts_with(*m))
        .map(|m| leading + m.len());
    // Numbered form: digits, then ". " or ") ".
    let numbered_match = if bullet_match.is_none() {
        let mut digits = 0;
        for c in trimmed.chars() {
            if c.is_ascii_digit() { digits += 1; } else { break; }
        }
        if digits > 0 {
            let after = &trimmed[digits..];
            if after.starts_with(". ") || after.starts_with(") ") {
                Some(leading + digits + 2)
            } else { None }
        } else { None }
    } else { None };
    let prefix_chars = bullet_match.or(numbered_match)?;
    let prefix_str: String = text[..prefix_chars].to_string();
    let width = UnicodeWidthStr::width(prefix_str.as_str());
    if width == 0 { return None; }
    Some((vec![Span::raw(" ".repeat(width))], width))
}

// ── Balanced wrap (minimum-raggedness DP) ────────────────────────────────
//
// Lifted from jcode-tui-markdown/src/markdown_wrap.rs. jcode gates this on
// non-Left alignment because their text is left-aligned by default; ours is
// too, but visually-balanced wraps (lines of equal width) look better even
// when left-aligned, so we apply it for all alignments. Guards match jcode:
// reject tabs, doubled spaces, leading/trailing whitespace, single-word
// content, or any single word wider than the budget — those fall back to
// greedy wrap.

#[derive(Clone)]
struct StyledPiece {
    text: String,
    style: Style,
}

#[derive(Clone)]
struct WrapToken {
    word: Vec<StyledPiece>,
    spaces: Vec<StyledPiece>,
    word_width: usize,
    space_width: usize,
}

fn wrap_line_balanced(
    line: &Line<'static>,
    max_width: usize,
    gutter: Option<&(Vec<Span<'static>>, usize)>,
) -> Option<Vec<Line<'static>>> {
    let flat_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if UnicodeWidthStr::width(flat_text.as_str()) <= max_width || !flat_text.contains(' ') {
        return None;
    }
    if flat_text.starts_with(char::is_whitespace)
        || flat_text.ends_with(char::is_whitespace)
        || flat_text.contains("  ")
        || flat_text.contains('\t')
    {
        return None;
    }

    let tokens = tokenize_for_balanced(line)?;
    let gutter_w = gutter.map(|(_, w)| *w).unwrap_or(0);
    let first_budget = max_width;
    let cont_budget = max_width.saturating_sub(gutter_w);
    if cont_budget == 0 {
        return None;
    }
    if tokens.len() < 3 || tokens.iter().any(|t| t.word_width > cont_budget) {
        return None;
    }

    let (breaks, line_count) = balanced_breaks(&tokens, first_budget, cont_budget)?;
    if line_count <= 1 {
        return None;
    }

    let mut result = Vec::with_capacity(line_count);
    let mut start = 0usize;
    let mut line_idx = 0usize;
    while start < tokens.len() {
        let end = breaks[start];
        if end <= start {
            return None;
        }
        let mut spans = Vec::new();
        if line_idx > 0 {
            if let Some((g_spans, _)) = gutter {
                spans.extend(g_spans.iter().cloned());
            }
        }
        spans.extend(build_balanced_spans(&tokens[start..end]));
        let mut nl = Line::from(spans);
        if let Some(a) = line.alignment {
            nl = nl.alignment(a);
        }
        result.push(nl);
        start = end;
        line_idx += 1;
    }
    Some(result)
}

fn tokenize_for_balanced(line: &Line<'static>) -> Option<Vec<WrapToken>> {
    let mut tokens: Vec<WrapToken> = Vec::new();
    let mut word: Vec<StyledPiece> = Vec::new();
    let mut spaces: Vec<StyledPiece> = Vec::new();
    let mut word_width = 0usize;
    let mut space_width = 0usize;
    let mut seen_word_char = false;
    let mut in_spaces = false;

    for span in &line.spans {
        let style = span.style;
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if ch.is_whitespace() {
                if !seen_word_char {
                    return None;
                }
                in_spaces = true;
                push_piece(&mut spaces, ch, style);
                space_width += cw;
            } else {
                if in_spaces {
                    tokens.push(WrapToken {
                        word: std::mem::take(&mut word),
                        spaces: std::mem::take(&mut spaces),
                        word_width,
                        space_width,
                    });
                    word_width = 0;
                    space_width = 0;
                    in_spaces = false;
                }
                seen_word_char = true;
                push_piece(&mut word, ch, style);
                word_width += cw;
            }
        }
    }
    if !seen_word_char || in_spaces {
        return None;
    }
    tokens.push(WrapToken {
        word,
        spaces,
        word_width,
        space_width,
    });
    Some(tokens)
}

fn push_piece(pieces: &mut Vec<StyledPiece>, ch: char, style: Style) {
    if let Some(last) = pieces.last_mut() {
        if last.style == style {
            last.text.push(ch);
            return;
        }
    }
    pieces.push(StyledPiece { text: ch.to_string(), style });
}

fn balanced_breaks(
    tokens: &[WrapToken],
    first_budget: usize,
    cont_budget: usize,
) -> Option<(Vec<usize>, usize)> {
    // DP over token slack — cost = sum of slack² across lines + line count.
    // `dp[i]` = min cost of wrapping tokens[i..]; `breaks[i]` = exclusive
    // end of the first line of that suffix. The first line uses `first_budget`,
    // continuation lines use `cont_budget` (gutter-adjusted).
    let n = tokens.len();
    let mut dp = vec![usize::MAX; n + 1];
    let mut breaks = vec![0usize; n];
    let mut counts = vec![usize::MAX; n + 1];
    dp[n] = 0;
    counts[n] = 0;

    for start in (0..n).rev() {
        // First line of the suffix gets the appropriate budget — only the
        // very first line of the original line uses `first_budget`. For
        // jcode-style we just use cont_budget everywhere except start=0.
        let budget = if start == 0 { first_budget } else { cont_budget };
        let mut line_width = 0usize;
        for end in start..n {
            line_width = if end == start {
                tokens[end].word_width
            } else {
                line_width
                    .saturating_add(tokens[end - 1].space_width)
                    .saturating_add(tokens[end].word_width)
            };
            if line_width > budget {
                break;
            }
            if dp[end + 1] == usize::MAX {
                continue;
            }
            let slack = budget - line_width;
            let cost = slack.saturating_mul(slack).saturating_add(dp[end + 1]);
            let lines_used = counts[end + 1].saturating_add(1);
            let better = cost < dp[start]
                || (cost == dp[start] && lines_used < counts[start]);
            if better {
                dp[start] = cost;
                breaks[start] = end + 1;
                counts[start] = lines_used;
            }
        }
    }
    if dp[0] == usize::MAX { None } else { Some((breaks, counts[0])) }
}

fn build_balanced_spans(tokens: &[WrapToken]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, t) in tokens.iter().enumerate() {
        for p in &t.word {
            spans.push(Span::styled(p.text.clone(), p.style));
        }
        if idx + 1 < tokens.len() {
            for p in &t.spaces {
                spans.push(Span::styled(p.text.clone(), p.style));
            }
        }
    }
    spans
}

struct Renderer<'a> {
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
    palette: &'a MarkdownPalette,
}

enum ListMode {
    Bullet,
    Ordered(u64),
}

impl<'a> Renderer<'a> {
    fn new(default_fg: Color, palette: &'a MarkdownPalette) -> Self {
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
            palette,
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
                Style::default().fg(self.palette.quote_bar),
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
                self.current.push(Span::styled(b, Style::default().fg(self.palette.bullet)));
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
                    Style::default().fg(self.palette.code_fg).bg(self.palette.code_bg)
                } else {
                    style
                };
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
                    Style::default().fg(self.palette.code_fg).bg(self.palette.code_bg),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                self.flush();
            }
            Event::Rule => {
                self.flush();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(self.palette.bullet),
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
                    Style::default().fg(self.palette.heading).add_modifier(Modifier::BOLD),
                ));
                self.push_style(Style::default().fg(self.palette.heading).add_modifier(Modifier::BOLD));
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
                        .fg(self.palette.link_dim)
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
                    Style::default().fg(self.palette.code_fg).add_modifier(Modifier::DIM),
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
                            .fg(self.palette.link_dim)
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
                    Style::default().fg(self.palette.code_fg).add_modifier(Modifier::DIM),
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
