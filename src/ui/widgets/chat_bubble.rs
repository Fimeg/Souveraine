#![allow(dead_code)] // WIP scaffolding not yet wired
//! Chat message bubble widget — ASCII-art bordered message container.
//!
//! Renders the `╭─── title ───╮` / `│ body...` / `╰── footer ──╯` pattern
//! with left/right/center alignment. Delegates rendering to a tuie `Text`
//! widget via `DelegateWidget`.

use tuie::prelude::*;

/// Horizontal alignment for the bubble within its container.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BubbleAlign {
    Left,
    Right,
    Center,
}

/// A chat bubble with ASCII-art borders, title, body, and optional footer.
///
/// Uses the `DelegateWidget` pattern — all rendering is delegated to an
/// inner `Text` widget whose content is rebuilt whenever the bubble's
/// fields change.
pub struct ChatBubble {
    text: Box<Text>,
    title: String,
    body_lines: Vec<String>,
    body_styled: Option<StyledString>,
    max_width: usize,
    align: BubbleAlign,
    border_style: Style,
    footer: Option<String>,
    container_width: u16,
}

impl DelegateWidget for ChatBubble {
    tuie::delegate_widget!(text);

    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl ChatBubble {
    /// Creates an empty bubble. Call the builder methods before rendering.
    pub fn new() -> Box<Self> {
        Box::new(Self {
            text: Text::new(),
            title: String::new(),
            body_lines: Vec::new(),
            body_styled: None,
            max_width: 80,
            align: BubbleAlign::Left,
            border_style: Style::new(),
            footer: None,
            container_width: 120,
        })
    }

    // ── Builder methods ──────────────────────────────────────────────────────

    pub fn title(mut self: Box<Self>, title: impl Into<String>) -> Box<Self> {
        self.title = title.into();
        self.rebuild();
        self
    }

    pub fn body(mut self: Box<Self>, lines: Vec<String>) -> Box<Self> {
        self.body_lines = lines;
        self.rebuild();
        self
    }

    pub fn body_from_text(mut self: Box<Self>, text: &str, max_width: usize) -> Box<Self> {
        self.body_lines = wrap_text_lines(text, max_width);
        self.body_styled = None;
        self.rebuild();
        self
    }

    pub fn body_styled(mut self: Box<Self>, styled: StyledString) -> Box<Self> {
        self.body_styled = Some(styled);
        self.body_lines.clear();
        self.rebuild();
        self
    }

    pub fn max_width(mut self: Box<Self>, w: usize) -> Box<Self> {
        self.max_width = w;
        self.rebuild();
        self
    }

    pub fn align(mut self: Box<Self>, a: BubbleAlign) -> Box<Self> {
        self.align = a;
        self.rebuild();
        self
    }

    pub fn border_style(mut self: Box<Self>, s: Style) -> Box<Self> {
        self.border_style = s;
        self.rebuild();
        self
    }

    pub fn footer(mut self: Box<Self>, f: Option<String>) -> Box<Self> {
        self.footer = f;
        self.rebuild();
        self
    }

    pub fn container_width(mut self: Box<Self>, w: u16) -> Box<Self> {
        self.container_width = w;
        self.rebuild();
        self
    }

    // ── Mutator methods ──────────────────────────────────────────────────────

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
        self.rebuild();
    }

    pub fn set_body(&mut self, lines: Vec<String>) {
        self.body_lines = lines;
        self.rebuild();
    }

    pub fn set_footer(&mut self, footer: Option<String>) {
        self.footer = footer;
        self.rebuild();
    }

    pub fn set_border_style(&mut self, s: Style) {
        self.border_style = s;
        self.rebuild();
    }

    pub fn set_container_width(&mut self, w: u16) {
        self.container_width = w;
        self.rebuild();
    }

    // ── Layout calculation ───────────────────────────────────────────────────

    /// Compute inner content width (widest body line, clamped to max_width).
    fn compute_inner(&self) -> usize {
        let title_w = self.title.chars().count() + 2; // " title "
        let footer_w = self
            .footer
            .as_ref()
            .map(|f| f.chars().count() + 2)
            .unwrap_or(0);
        let body_w = if let Some(styled) = &self.body_styled {
            styled
                .as_str()
                .split('\n')
                .map(unicode_display_width)
                .max()
                .unwrap_or(0)
        } else {
            self.body_lines
                .iter()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0)
        };

        let widest = title_w.max(footer_w).max(body_w);
        widest.min(self.max_width.saturating_sub(4).max(8))
    }

    fn pad_str(&self, outer: usize) -> String {
        let pad = match self.align {
            BubbleAlign::Left => 2,
            BubbleAlign::Right => (self.container_width as usize).saturating_sub(outer + 2),
            BubbleAlign::Center => (self.container_width as usize).saturating_sub(outer) / 2,
        };
        " ".repeat(pad)
    }

    // ── Rebuild ──────────────────────────────────────────────────────────────

    /// Rebuild the inner `Text` content with styled ASCII-art borders.
    fn rebuild(&mut self) {
        let inner = self.compute_inner();
        let outer = inner + 4;
        let pad = self.pad_str(outer);
        let mut content = StyledString::new();
        let border = self.border_style;

        // ╭─── title ───╮
        let title_text = format!(" {} ", self.title);
        let dashes = outer.saturating_sub(2 + title_text.chars().count());
        let left_dash = "\u{2500}".repeat(dashes / 2);
        let right_dash = "\u{2500}".repeat(dashes - dashes / 2);
        let top_line = format!("{pad}\u{256d}{left_dash}{title_text}{right_dash}\u{256e}\n");
        let s = content.as_ref().len();
        content.push_str(&top_line);
        content.style_range(s..content.as_ref().len(), |style| *style = border);

        if let Some(styled) = &self.body_styled {
            let body_text = styled.as_str();
            for line_str in body_text.split('\n') {
                let line_byte_start = line_str.as_ptr() as usize - body_text.as_ptr() as usize;
                for sub in wrap_sub_lines(line_str, inner) {
                    let sub_width = unicode_display_width(sub.text);
                    let inner_pad = inner.saturating_sub(sub_width);
                    let left = format!("{pad}\u{2502} ");
                    let s = content.as_ref().len();
                    content.push_str(&left);
                    content.style_range(s..content.as_ref().len(), |style| *style = border);
                    let abs_start = line_byte_start + sub.byte_offset;
                    let abs_end = abs_start + sub.text.len();
                    push_styled_slice(&mut content, styled, abs_start, abs_end);
                    let right = format!("{} \u{2502}\n", " ".repeat(inner_pad));
                    let s = content.as_ref().len();
                    content.push_str(&right);
                    content.style_range(s..content.as_ref().len(), |style| *style = border);
                }
            }
        } else {
            // Plain text body lines: │ body text... │
            for line in &self.body_lines {
                let chunk_width = line.chars().count();
                let inner_pad = inner.saturating_sub(chunk_width);
                let body_line =
                    format!("{pad}\u{2502} {line}{} \u{2502}\n", " ".repeat(inner_pad),);
                let s = content.as_ref().len();
                content.push_str(&body_line);
                content.style_range(s..content.as_ref().len(), |style| *style = border);
            }
        }

        // ╰── footer ──╯ (or ───╯ if no footer)
        let bottom_line = match &self.footer {
            Some(f) if !f.is_empty() => {
                let ftext = format!(" {} ", f);
                let fdashes = outer.saturating_sub(2 + ftext.chars().count());
                let fl = "\u{2500}".repeat(fdashes / 2);
                let fr = "\u{2500}".repeat(fdashes - fdashes / 2);
                format!("{pad}\u{2570}{fl}{ftext}{fr}\u{256f}")
            }
            _ => {
                let bottom_dashes = "\u{2500}".repeat(outer - 2);
                format!("{pad}\u{2570}{bottom_dashes}\u{256f}")
            }
        };
        let s = content.as_ref().len();
        content.push_str(&bottom_line);
        content.style_range(s..content.as_ref().len(), |style| *style = border);

        self.text.set_content(content);
        self.text.dirty_layout();
    }
}

fn unicode_display_width(s: &str) -> usize {
    tuie::display_width(s)
}

/// Copy a byte-range slice from a `StyledString` into `out`, preserving span styles.
fn push_styled_slice(out: &mut StyledString, src: &StyledString, start: usize, end: usize) {
    if start >= end {
        return;
    }
    for (chunk, style) in src.iter_chunks(start..end) {
        out.push_span(StyledStr::new(chunk).style(style));
    }
}

struct SubLine<'a> {
    text: &'a str,
    byte_offset: usize,
}

fn wrap_sub_lines(line: &str, max_width: usize) -> Vec<SubLine<'_>> {
    if max_width == 0 {
        return vec![SubLine {
            text: line,
            byte_offset: 0,
        }];
    }
    let mut subs = Vec::new();
    let mut offset = 0;
    let mut remaining = line;
    while !remaining.is_empty() {
        let w = unicode_display_width(remaining);
        if w <= max_width {
            subs.push(SubLine {
                text: remaining,
                byte_offset: offset,
            });
            break;
        }
        // Find the byte position where display width exceeds max_width.
        // Try to break at the last space within the limit.
        let mut byte_pos = 0;
        let mut disp_w = 0;
        let mut last_space = None;
        for ch in remaining.chars() {
            let ch_w = unicode_display_width(&remaining[byte_pos..byte_pos + ch.len_utf8()]);
            if disp_w + ch_w > max_width {
                break;
            }
            byte_pos += ch.len_utf8();
            disp_w += ch_w;
            if ch == ' ' {
                last_space = Some(byte_pos);
            }
        }
        let break_at = if let Some(sp) = last_space {
            if sp > max_width / 4 {
                sp
            } else {
                byte_pos
            }
        } else {
            byte_pos.max(1)
        };
        subs.push(SubLine {
            text: &remaining[..break_at],
            byte_offset: offset,
        });
        offset += break_at;
        remaining = &remaining[break_at..];
    }
    if subs.is_empty() {
        subs.push(SubLine {
            text: "",
            byte_offset: 0,
        });
    }
    subs
}

/// Simple word-wrapping: split text into lines at `max_width` characters.
pub fn wrap_text_lines(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.chars().count() + 1 + word.chars().count() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
