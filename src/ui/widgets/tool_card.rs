#![allow(dead_code)] // WIP scaffolding not yet wired
//! Tool call card widget — compact and expanded tool display.
//!
//! Compact mode renders a single line: `⟳ tool_name  ·  args_summary  ·  r2`.
//! Expanded mode renders a full bubble with arguments and result body.

use tuie::prelude::*;

/// Whether the tool card is compact (one-line) or expanded (full bubble).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolCardMode {
    Compact,
    Expanded,
}

/// A card displaying a tool invocation — name, arguments, round, and result.
pub struct ToolCard {
    text: Box<Text>,
    name: String,
    args_summary: String,
    round: u32,
    is_error: bool,
    is_pending: bool,
    result_output: Option<String>,
    mode: ToolCardMode,
    glyph_style: Style,
    name_style: Style,
    dim_style: Style,
    container_width: u16,
}

impl DelegateWidget for ToolCard {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl ToolCard {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            text: Text::new(),
            name: String::new(),
            args_summary: String::new(),
            round: 1,
            is_error: false,
            is_pending: true,
            result_output: None,
            mode: ToolCardMode::Compact,
            glyph_style: Style::new(),
            name_style: Style::new(),
            dim_style: Style::new(),
            container_width: 120,
        })
    }

    // ── Builder methods ──────────────────────────────────────────────────────

    pub fn name(mut self: Box<Self>, name: impl Into<String>) -> Box<Self> {
        self.name = name.into();
        self.rebuild();
        self
    }

    pub fn args_summary(mut self: Box<Self>, summary: impl Into<String>) -> Box<Self> {
        self.args_summary = summary.into();
        self.rebuild();
        self
    }

    pub fn round(mut self: Box<Self>, r: u32) -> Box<Self> {
        self.round = r;
        self.rebuild();
        self
    }

    pub fn is_error(mut self: Box<Self>, err: bool) -> Box<Self> {
        self.is_error = err;
        self.rebuild();
        self
    }

    pub fn is_pending(mut self: Box<Self>, pending: bool) -> Box<Self> {
        self.is_pending = pending;
        self.rebuild();
        self
    }

    pub fn mode(mut self: Box<Self>, m: ToolCardMode) -> Box<Self> {
        self.mode = m;
        self.rebuild();
        self
    }

    pub fn glyph_style(mut self: Box<Self>, s: Style) -> Box<Self> {
        self.glyph_style = s;
        self.rebuild();
        self
    }

    pub fn name_style(mut self: Box<Self>, s: Style) -> Box<Self> {
        self.name_style = s;
        self.rebuild();
        self
    }

    pub fn dim_style(mut self: Box<Self>, s: Style) -> Box<Self> {
        self.dim_style = s;
        self.rebuild();
        self
    }

    pub fn container_width(mut self: Box<Self>, w: u16) -> Box<Self> {
        self.container_width = w;
        self.rebuild();
        self
    }

    // ── Mutator methods ──────────────────────────────────────────────────────

    pub fn set_mode(&mut self, m: ToolCardMode) {
        self.mode = m;
        self.rebuild();
    }

    pub fn set_pending(&mut self, pending: bool) {
        self.is_pending = pending;
        self.rebuild();
    }

    pub fn set_error(&mut self, err: bool) {
        self.is_error = err;
        self.rebuild();
    }

    pub fn set_result_details(&mut self, output: &str) {
        self.result_output = if output.is_empty() {
            None
        } else {
            Some(output.to_string())
        };
        self.rebuild();
    }

    pub fn result_output(mut self: Box<Self>, output: Option<String>) -> Box<Self> {
        self.result_output = output;
        self.rebuild();
        self
    }

    // ── Rebuild ──────────────────────────────────────────────────────────────

    fn rebuild(&mut self) {
        let glyph = match (self.is_pending, self.is_error) {
            (true, _) => '⟳',
            (false, true) => '✗',
            (false, false) => '✓',
        };

        match self.mode {
            ToolCardMode::Compact => self.build_compact(glyph),
            ToolCardMode::Expanded => self.build_expanded(glyph),
        }
        self.text.dirty_layout();
    }

    fn build_compact(&mut self, glyph: char) {
        let mut content = StyledString::new();
        let reserved = self.name.chars().count() + 14;
        let arg_budget = (self.container_width as usize)
            .saturating_sub(reserved + 6)
            .clamp(20, 120);
        let args = clip(&self.args_summary, arg_budget);

        // "  ⟳ tool_name  ·  args  ·  r2"
        let line = if self.round > 1 {
            if args.is_empty() {
                format!("  {glyph} {}  ·  r{}", self.name, self.round)
            } else {
                format!("  {glyph} {}  ·  {}  ·  r{}", self.name, args, self.round)
            }
        } else {
            if args.is_empty() {
                format!("  {glyph} {}", self.name)
            } else {
                format!("  {glyph} {}  ·  {}", self.name, args)
            }
        };

        content.push_str(&line);

        // Style the glyph region "  {glyph}" — 2 leading spaces plus the glyph,
        // measured in bytes (the glyph is a multi-byte char, e.g. ✓/✗/⟳ are 3 bytes each).
        let glyph_end = (2 + glyph.len_utf8()).min(line.len());
        content.style_range(0..glyph_end, |s| *s = self.glyph_style);

        // Style the tool name (from glyph_end to "  ·" or "  ·  r")
        let name_end = glyph_end + 1 + self.name.len();
        let name_end = name_end.min(line.len());
        content.style_range(glyph_end..name_end, |s| *s = self.name_style);

        // Style the rest as dim
        if name_end < line.len() {
            content.style_range(name_end..line.len(), |s| *s = self.dim_style);
        }

        self.text.set_content(content);
    }

    fn build_expanded(&mut self, glyph: char) {
        // Header: "  ⟳ tool_name  ·  r3" (same as compact header)
        let mut content = StyledString::new();
        let header = format!("  {glyph} {}  ·  r{}", self.name, self.round);
        content.push_str(&header);

        let header_len = header.len();
        // "  {glyph}" in bytes — the glyph is multi-byte (✓/✗/⟳ = 3 bytes each),
        // so a literal 0..3 would split it and panic when the span is sliced.
        let glyph_end = (2 + glyph.len_utf8()).min(header_len);
        content.style_range(0..glyph_end, |s| *s = self.glyph_style);
        content.style_range(glyph_end..header_len, |s| *s = self.name_style);

        // Arguments body — full, not clipped.
        content.push_str("\n");
        if self.args_summary.is_empty() {
            content.push_str("  (no arguments)");
        } else {
            // Indent each line of args for readability.
            for (i, line) in self.args_summary.lines().enumerate() {
                if i > 0 {
                    content.push_str("\n");
                }
                content.push_str(&format!("    {line}"));
            }
        };
        let body_start = header_len + 1;
        let body_end = content.as_ref().len();
        content.style_range(body_start..body_end, |s| *s = self.dim_style);

        // Result output, if any.
        if let Some(ref output) = self.result_output {
            content.push_str("\n\n");
            let result_marker = if self.is_error { "stderr:" } else { "stdout:" };
            content.push_str(&format!("  {result_marker}"));
            for line in output.lines() {
                content.push_str("\n    ");
                content.push_str(line);
            }
            let result_start = body_end + 2;
            let result_end = content.as_ref().len();
            let result_color = if self.is_error {
                Color::RED
            } else {
                Color::BRIGHT_BLACK
            };
            content.style_range(result_start..result_end, |s| {
                *s = Style::new().fg(result_color)
            });
        }

        self.text.set_content(content);
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
