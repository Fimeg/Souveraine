//! Per-tool-type rendering for tool cards.
//!
//! Pure functions dispatched by tool name so compact args, result previews,
//! and full card bodies are semantically meaningful rather than generic
//! key:value dumps.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::ChatPalette;

// ─── Helpers ──────────────────────────────────────────────────────────

pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn json_str<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn json_bool(map: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    map.get(key).and_then(|v| v.as_bool())
}

fn json_u64(map: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    map.get(key).and_then(|v| v.as_u64())
}

// ─── Tier 1: Per-tool argument summarization ──────────────────────────

/// Format tool arguments into a short human-readable string for compact cards.
pub fn summarize_tool_args(name: &str, arguments: &str) -> String {
    let parsed: Result<Value, _> = serde_json::from_str(arguments);
    let map = match parsed {
        Ok(Value::Object(m)) => m,
        Ok(other) => return clip(&other.to_string(), 120),
        Err(_) => return clip(arguments, 120),
    };
    match name {
        "bash" => summarize_bash(&map),
        "read" => summarize_read(&map),
        "write" => summarize_write(&map),
        "edit" => summarize_edit(&map),
        "grep" => summarize_grep(&map),
        "glob" => summarize_glob(&map),
        "list_dir" => summarize_list_dir(&map),
        "todo" => summarize_todo(&map),
        "subagent" => summarize_subagent(&map),
        "atmosphere" => summarize_atmosphere(&map),
        "outfit" => summarize_outfit(&map),
        "itinerary" => summarize_itinerary(&map),
        "schedule" => summarize_schedule(&map),
        "memory" => summarize_memory(&map),
        "nickname" => summarize_nickname(&map),
        "reach" | "consult" => summarize_summon(&map),
        _ => summarize_generic(&map),
    }
}

fn summarize_bash(map: &serde_json::Map<String, Value>) -> String {
    match json_str(map, "command") {
        Some(cmd) => {
            let cmd = clip(cmd, 80);
            let extra = if json_bool(map, "run_in_background").unwrap_or(false) {
                " [bg]"
            } else {
                ""
            };
            format!("$ {}{}", cmd, extra)
        }
        None => summarize_generic(map),
    }
}

fn summarize_read(map: &serde_json::Map<String, Value>) -> String {
    match json_str(map, "path") {
        Some(p) => {
            let p = clip(p, 60);
            let extra = if json_bool(map, "force").unwrap_or(false) {
                " [force]"
            } else {
                ""
            };
            format!("{}{}", p, extra)
        }
        None => summarize_generic(map),
    }
}

fn summarize_write(map: &serde_json::Map<String, Value>) -> String {
    match json_str(map, "path") {
        Some(p) => {
            let p = clip(p, 50);
            let mode = json_str(map, "mode").unwrap_or("write");
            // Estimate content length from the content field
            let content_len = map.get("content").and_then(|v| v.as_str()).map(|s| s.len());
            let label = if mode == "append" { "append" } else { "write" };
            match content_len {
                Some(n) if n > 0 => format!("{} → {} ({}B)", label, p, n),
                _ => format!("{} → {}", label, p),
            }
        }
        None => summarize_generic(map),
    }
}

fn summarize_edit(map: &serde_json::Map<String, Value>) -> String {
    let path = json_str(map, "path")
        .map(|p| clip(p, 40))
        .unwrap_or_default();
    let old = json_str(map, "old_string")
        .map(|s| clip(s, 30))
        .unwrap_or_default();
    let all = json_bool(map, "replace_all").unwrap_or(false);
    if path.is_empty() {
        return summarize_generic(map);
    }
    if !old.is_empty() {
        if all {
            format!("{}: replace_all \"{}\"", path, old)
        } else {
            format!("{}: \"{}\"", path, old)
        }
    } else {
        path
    }
}

fn summarize_grep(map: &serde_json::Map<String, Value>) -> String {
    let pattern = json_str(map, "pattern")
        .map(|p| clip(p, 40))
        .unwrap_or_default();
    let path = json_str(map, "path").filter(|p| *p != ".");
    let context = json_u64(map, "context").unwrap_or(0);
    let mut out = if pattern.is_empty() {
        return summarize_generic(map);
    } else {
        format!("\"{}\"", pattern)
    };
    if let Some(p) = path {
        out.push_str(&format!(" {}", clip(p, 30)));
    }
    if context > 0 {
        out.push_str(&format!(" -C {}", context));
    }
    out
}

fn summarize_glob(map: &serde_json::Map<String, Value>) -> String {
    let pattern = json_str(map, "pattern")
        .map(|p| clip(p, 50))
        .unwrap_or_default();
    if pattern.is_empty() {
        return summarize_generic(map);
    }
    let base = json_str(map, "base");
    match base {
        Some(b) if b != "." => format!("{} (in {})", pattern, clip(b, 30)),
        _ => pattern,
    }
}

fn summarize_list_dir(map: &serde_json::Map<String, Value>) -> String {
    json_str(map, "path")
        .map(|p| clip(p, 50))
        .unwrap_or_else(|| ".".to_string())
}

fn summarize_todo(map: &serde_json::Map<String, Value>) -> String {
    let action = json_str(map, "action").unwrap_or("list");
    if action == "list" {
        return "list".to_string();
    }
    let text = json_str(map, "text")
        .or_else(|| json_str(map, "id"))
        .map(|s| clip(s, 50));
    match text {
        Some(t) => format!("{}: {}", action, t),
        None => action.to_string(),
    }
}

fn summarize_subagent(map: &serde_json::Map<String, Value>) -> String {
    let prompt = json_str(map, "prompt")
        .map(|s| clip(s, 50))
        .unwrap_or_default();
    let sub_type = json_str(map, "subagent_type").filter(|t| *t != "general-purpose");
    let bg = json_bool(map, "run_in_background").unwrap_or(false);
    let mut out = String::new();
    if bg {
        out.push_str("[bg] ");
    }
    if let Some(t) = sub_type {
        out.push_str(&format!("[{}] ", t));
    }
    if !prompt.is_empty() {
        out.push_str(&prompt);
    }
    if out.is_empty() {
        return summarize_generic(map);
    }
    out
}

fn summarize_atmosphere(map: &serde_json::Map<String, Value>) -> String {
    json_str(map, "name")
        .map(|n| n.to_string())
        .unwrap_or_else(|| summarize_generic(map))
}

fn summarize_outfit(map: &serde_json::Map<String, Value>) -> String {
    json_str(map, "name")
        .map(|n| n.to_string())
        .unwrap_or_else(|| summarize_generic(map))
}

fn summarize_itinerary(map: &serde_json::Map<String, Value>) -> String {
    let action = json_str(map, "action").unwrap_or("describe");
    let title = json_str(map, "title")
        .or_else(|| {
            // Try first stop from `stops` array
            map.get("stops")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .map(|s| clip(s, 40));
    match title {
        Some(t) => format!("{}: {}", action, t),
        None => action.to_string(),
    }
}

fn summarize_schedule(map: &serde_json::Map<String, Value>) -> String {
    let action = json_str(map, "action").unwrap_or("list");
    let name = json_str(map, "name").map(|n| clip(n, 30));
    match name {
        Some(n) => format!("{}: {}", action, n),
        None => action.to_string(),
    }
}

fn summarize_memory(map: &serde_json::Map<String, Value>) -> String {
    let cmd = json_str(map, "command").unwrap_or("list");
    let path = json_str(map, "path").map(|p| clip(p, 40));
    match path {
        Some(p) => format!("{} {}", cmd, p),
        None => cmd.to_string(),
    }
}

fn summarize_nickname(map: &serde_json::Map<String, Value>) -> String {
    let action = json_str(map, "action").unwrap_or("get");
    let name = json_str(map, "name").map(|n| clip(n, 30));
    match name {
        Some(n) => format!("{}: {}", action, n),
        None => action.to_string(),
    }
}

fn summarize_summon(map: &serde_json::Map<String, Value>) -> String {
    let target = json_str(map, "target")
        .map(|t| clip(t, 40))
        .unwrap_or_default();
    if target.is_empty() {
        return summarize_generic(map);
    }
    target
}

fn summarize_generic(map: &serde_json::Map<String, Value>) -> String {
    let parts: Vec<String> = map
        .iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => clip(s, 60),
                other => clip(&other.to_string(), 60),
            };
            format!("{}: {}", k, s)
        })
        .collect();
    parts.join("  ·  ")
}

// ─── Tier 2: Compact result details ───────────────────────────────────

/// Return an optional second-line detail for the compact card.
/// For completed tools this can show a one-line output preview (e.g. the
/// last line of a bash run, the match count for grep).  For errors it is
/// always the first error line.  `None` means no detail line.
pub fn compact_result_details(name: &str, output: &str, is_error: bool) -> Option<String> {
    if is_error {
        return output.lines().next().map(|l| clip(l.trim(), 80));
    }
    match name {
        "bash" => {
            // Last non-empty line of output
            output
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| clip(l.trim(), 80))
        }
        "grep" => {
            // First line often says "N matches for pattern:"
            output.lines().next().map(|l| clip(l.trim(), 80))
        }
        "glob" => output.lines().next().map(|l| clip(l.trim(), 80)),
        "edit" => {
            // Edit output is a prose summary
            output.lines().next().map(|l| clip(l.trim(), 80))
        }
        _ => None,
    }
}

// ─── Tier 3: Full card body rendering ─────────────────────────────────

/// Render the body of a full tool card.
///
/// Returns `(args_lines, body_lines)` — the argument summary and the
/// output rendering, both ready to be inserted into the bubble layout.
pub fn render_card_body(
    name: &str,
    arguments: &str,
    output: Option<&str>,
    is_error: bool,
    inner_width: usize,
    palette: &ChatPalette,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let dim_color = if is_error {
        palette.compaction
    } else {
        palette.tool_dim
    };
    let fg_color = if is_error {
        palette.compaction
    } else {
        palette.agent_primary
    };
    let mdpal = crate::ui::markdown::MarkdownPalette::from_chat_palette(palette);

    // Argument line(s)
    let args_summary = summarize_tool_args(name, arguments);
    let args_line = Line::from(vec![Span::styled(
        args_summary,
        Style::default().fg(dim_color),
    )]);
    let args_lines = crate::ui::markdown::wrap_line(args_line, inner_width);

    // Body — per-tool output rendering
    let body_lines = match (name, output) {
        ("bash", Some(out)) if !out.is_empty() => render_bash_body(out, inner_width, palette),
        ("read", Some(out)) if !out.is_empty() => {
            render_read_body(out, arguments, inner_width, palette, &mdpal)
        }
        ("grep", Some(out)) if !out.is_empty() => {
            render_grep_body(out, inner_width, palette, &mdpal)
        }
        ("glob", Some(out)) if !out.is_empty() => {
            render_glob_body(out, inner_width, palette, &mdpal)
        }
        ("write", Some(out)) if !out.is_empty() => {
            render_write_body(out, arguments, inner_width, palette)
        }
        ("edit", Some(out)) if !out.is_empty() => {
            render_edit_body(out, arguments, inner_width, palette)
        }
        ("memory", Some(out)) => render_memory_body(out, inner_width, &mdpal, palette),
        ("list_dir", Some(out)) if !out.is_empty() => {
            render_list_dir_body(out, inner_width, palette)
        }
        _ => {
            // Fallback: markdown-rendered preview (existing behavior)
            if let Some(out) = output {
                let preview = preview_lines(out, 12);

                crate::ui::markdown::render_with_width(
                    &preview,
                    fg_color,
                    Some(inner_width),
                    &mdpal,
                )
            } else {
                Vec::new()
            }
        }
    };

    (args_lines, body_lines)
}

fn render_bash_body(output: &str, inner_width: usize, palette: &ChatPalette) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ┌─ header
    let top = "┌─ bash output ".to_string();
    let top_dashes = inner_width.saturating_sub(top.chars().count());
    let top_line = Line::from(Span::styled(
        format!("{}{}", top, "─".repeat(top_dashes)),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    ));
    lines.push(top_line);

    // Content — up to 30 lines, monospace-style (plain spans)
    let max_lines = 30usize;
    let displayed: Vec<&str> = output.lines().take(max_lines).collect();
    for line_text in &displayed {
        let clipped = clip(line_text, inner_width);
        lines.push(Line::from(Span::styled(
            format!(" {} ", clipped),
            Style::default().fg(palette.tool_accent),
        )));
    }
    let total = output.lines().count();
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("  … ({} more lines)", total - max_lines),
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )));
    }

    // └─ footer
    let bottom = format!("└{}", "─".repeat(inner_width.saturating_sub(1)));
    lines.push(Line::from(Span::styled(
        bottom,
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));

    lines
}

fn render_read_body(
    output: &str,
    arguments: &str,
    _inner_width: usize,
    _palette: &ChatPalette,
    mdpal: &crate::ui::markdown::MarkdownPalette,
) -> Vec<Line<'static>> {
    // For .md files, render as markdown. For everything else, render as
    // a code block with guessed language from arguments.
    let path = arguments_to_path(arguments);
    let ext = path.rsplit('.').next().map(|s| s.to_lowercase());
    let is_markdown = matches!(ext.as_deref(), Some("md" | "mdx"));
    let lang_label = match ext.as_deref() {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js") | Some("ts") | Some("tsx") => ext.as_deref().unwrap_or("code"),
        Some("toml") => "toml",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("md") | Some("mdx") => "markdown",
        Some(other) => other,
        None => "code",
    };

    if is_markdown {
        // Full markdown render for prose files
        let fg = mdpal.code_fg;
        let rendered = crate::ui::markdown::render_with_width(output, fg, None, mdpal);
        return rendered;
    }

    // Code block with language label
    let fg = mdpal.code_fg;
    let bg = mdpal.code_bg;
    let code_style = Style::default().fg(fg).bg(bg);
    let dim_style = Style::default()
        .fg(mdpal.code_fg)
        .add_modifier(Modifier::DIM);

    // This is a simplified code-block render that doesn't depend on the
    // markdown renderer's internal ┌─ / └─ borders, since we want a
    // monospace look in the tool card.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let header = format!("┌─ {} ", lang_label);
    let dashes = _inner_width.saturating_sub(header.chars().count());
    lines.push(Line::from(Span::styled(
        format!("{}{}", header, "─".repeat(dashes)),
        dim_style,
    )));

    let max_lines = 30usize;
    let displayed: Vec<&str> = output.lines().take(max_lines).collect();
    for line_text in &displayed {
        let clipped = clip(line_text, _inner_width.saturating_sub(2).max(10));
        lines.push(Line::from(Span::styled(
            format!(" {}", clipped),
            code_style,
        )));
    }
    let total = output.lines().count();
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("  … ({} more lines)", total - max_lines),
            dim_style.add_modifier(Modifier::ITALIC),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(_inner_width.saturating_sub(1))),
        dim_style,
    )));

    lines
}

fn render_grep_body(
    output: &str,
    inner_width: usize,
    palette: &ChatPalette,
    _mdpal: &crate::ui::markdown::MarkdownPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // First line is the match count header
    if let Some(first) = output.lines().next() {
        lines.push(Line::from(Span::styled(
            clip(first.trim(), inner_width),
            Style::default()
                .fg(palette.tool_accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(inner_width.min(40)),
            Style::default().fg(dim).add_modifier(Modifier::DIM),
        )));
    }

    // Match lines — cap at 20
    let max_lines = 20usize;
    for line_text in output.lines().skip(1).take(max_lines) {
        let clipped = clip(line_text.trim(), inner_width);
        lines.push(Line::from(Span::styled(clipped, Style::default().fg(dim))));
    }
    let total = output.lines().count().saturating_sub(1);
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("  … ({} more matches)", total - max_lines),
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )));
    }

    lines
}

fn render_glob_body(
    output: &str,
    inner_width: usize,
    palette: &ChatPalette,
    _mdpal: &crate::ui::markdown::MarkdownPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(first) = output.lines().next() {
        lines.push(Line::from(Span::styled(
            clip(first.trim(), inner_width),
            Style::default()
                .fg(palette.tool_accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(inner_width.min(40)),
            Style::default().fg(dim).add_modifier(Modifier::DIM),
        )));
    }

    let max_lines = 30usize;
    for line_text in output.lines().skip(1).take(max_lines) {
        let clipped = clip(line_text.trim(), inner_width);
        lines.push(Line::from(Span::styled(clipped, Style::default().fg(dim))));
    }
    let total = output.lines().count().saturating_sub(1);
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("  … ({} more files)", total - max_lines),
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )));
    }

    lines
}

fn render_list_dir_body(
    output: &str,
    inner_width: usize,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let mut lines: Vec<Line<'static>> = Vec::new();

    let max_lines = 30usize;
    for line_text in output.lines().take(max_lines) {
        let clipped = clip(line_text.trim(), inner_width);
        lines.push(Line::from(Span::styled(clipped, Style::default().fg(dim))));
    }
    let total = output.lines().count();
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("  … ({} more entries)", total - max_lines),
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )));
    }

    lines
}

fn render_write_body(
    output: &str,
    arguments: &str,
    inner_width: usize,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let fg = palette.tool_accent;
    let path = arguments_to_path(arguments);

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Header with path and output summary
    let header = format!("┌─ {} ", if path.is_empty() { "write" } else { &path });
    let dashes = inner_width.saturating_sub(header.chars().count());
    lines.push(Line::from(Span::styled(
        format!("{}{}", header, "─".repeat(dashes)),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));
    // Output summary line — write returns prose like "Appended N chars to path"
    let clipped = clip(output.trim(), inner_width.saturating_sub(2).max(10));
    lines.push(Line::from(Span::styled(
        format!(" {}", clipped),
        Style::default().fg(fg),
    )));
    // Footer
    lines.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(inner_width.saturating_sub(1))),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));
    lines
}

fn render_edit_body(
    output: &str,
    arguments: &str,
    inner_width: usize,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let fg = palette.tool_accent;
    let path = arguments_to_path(arguments);

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Header with path
    let header = format!("┌─ edit{} ", if path.is_empty() { "" } else { ":" });
    let label = format!("{}{}", header, path);
    let dashes = inner_width.saturating_sub(label.chars().count());
    lines.push(Line::from(Span::styled(
        format!("{}{}", label, "─".repeat(dashes)),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));
    // Edit returns prose like "Edited file.rs. +42 chars, 10 -> 12 lines"
    if let Some(first) = output.lines().next() {
        let clipped = clip(first.trim(), inner_width.saturating_sub(2).max(10));
        lines.push(Line::from(Span::styled(
            format!(" {}", clipped),
            Style::default().fg(fg),
        )));
    }
    // Footer
    lines.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(inner_width.saturating_sub(1))),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));
    lines
}

fn render_memory_body(
    output: &str,
    inner_width: usize,
    mdpal: &crate::ui::markdown::MarkdownPalette,
    palette: &ChatPalette,
) -> Vec<Line<'static>> {
    let dim = palette.tool_dim;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header
    let header = "┌─ memory ".to_string();
    let dashes = inner_width.saturating_sub(header.chars().count());
    lines.push(Line::from(Span::styled(
        format!("{}{}", header, "─".repeat(dashes)),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));

    if output.is_empty() {
        lines.push(Line::from(Span::styled(
            " (empty)",
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(Span::styled(
            format!("└{}", "─".repeat(inner_width.saturating_sub(1))),
            Style::default().fg(dim).add_modifier(Modifier::DIM),
        )));
        return lines;
    }

    // Try to split frontmatter from body — frontmatter lives between --- lines
    let body = if let Some(rest) = output.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let frontmatter = &output[..end + 6]; // include closing ---
            let body = output[end + 6..].trim();
            // Render frontmatter as folded single-line summary
            let fm_summary = summarize_frontmatter(frontmatter);
            lines.push(Line::from(Span::styled(
                format!(" {} ", fm_summary),
                Style::default().fg(dim).add_modifier(Modifier::DIM),
            )));
            body
        } else {
            output
        }
    } else {
        output
    };

    // Render body as markdown (up to 30 lines)
    if !body.is_empty() {
        let max_lines = 30usize;
        let preview: String = body
            .lines()
            .take(max_lines)
            .collect::<Vec<&str>>()
            .join("\n");
        let rendered = crate::ui::markdown::render_with_width(
            &preview,
            mdpal.code_fg,
            Some(inner_width.saturating_sub(2).max(8)),
            mdpal,
        );
        lines.extend(rendered);

        let total = body.lines().count();
        if total > max_lines {
            lines.push(Line::from(Span::styled(
                format!("  … ({} more lines)", total - max_lines),
                Style::default().fg(dim).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    // Footer
    lines.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(inner_width.saturating_sub(1))),
        Style::default().fg(dim).add_modifier(Modifier::DIM),
    )));
    lines
}

/// Extract a one-line summary from YAML frontmatter: `description · tags · limit`
fn summarize_frontmatter(frontmatter: &str) -> String {
    let mut desc = String::new();
    let mut tags = String::new();
    let mut limit = String::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("description:") {
            desc = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("tags:") {
            tags = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("limit:") {
            limit = val.trim().to_string();
        }
    }

    let mut parts: Vec<&str> = Vec::new();
    if !desc.is_empty() {
        parts.push(&desc);
    }
    if !tags.is_empty() {
        parts.push(&tags);
    }
    if !limit.is_empty() {
        parts.push(&limit);
    }
    if parts.is_empty() {
        return "📄 frontmatter".to_string();
    }
    parts.join("  ·  ")
}

// ─── Internal helpers ──────────────────────────────────────────────────

/// Extract just the path from an arguments JSON for read-body heuristics.
fn arguments_to_path(arguments: &str) -> String {
    let parsed: Result<Value, _> = serde_json::from_str(arguments);
    match parsed {
        Ok(Value::Object(map)) => json_str(&map, "path")
            .map(|s| s.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Take the first N lines of a string.
fn preview_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().take(n).collect();
    lines.join("\n")
}
