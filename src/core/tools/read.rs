//! read — I open a file and let it into me.

use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Read;

// ── Line range types ────────────────────────────────────────────

/// A range of lines: 1-indexed, end-exclusive.
#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn parse_line_ranges(path_str: &str) -> (PathBuf, Vec<LineRange>) {
    let colon_pos = match path_str.rfind(':') {
        Some(p) => p,
        None => return (PathBuf::from(path_str), vec![]),
    };

    let after = &path_str[colon_pos + 1..];
    if after.is_empty() || !after.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return (PathBuf::from(path_str), vec![]);
    }

    let clean = PathBuf::from(&path_str[..colon_pos]);

    let ranges: Vec<LineRange> = after
        .split(',')
        .filter_map(|part| parse_one_range(part.trim()))
        .collect();

    (clean, ranges)
}

fn parse_one_range(part: &str) -> Option<LineRange> {
    if part.is_empty() {
        return None;
    }
    if let Some(dash) = part.find('-') {
        let before = &part[..dash];
        let after = &part[dash + 1..];
        match (before.is_empty(), after.is_empty()) {
            (false, false) => {
                let start: usize = before.parse().ok()?;
                let end: usize = after.parse().ok()?;
                Some(LineRange { start, end })
            }
            (false, true) => {
                let start: usize = before.parse().ok()?;
                Some(LineRange { start, end: usize::MAX })
            }
            (true, false) => {
                let end: usize = after.parse().ok()?;
                Some(LineRange { start: 1, end })
            }
            (true, true) => None,
        }
    } else {
        let line: usize = part.parse().ok()?;
        Some(LineRange { start: line, end: line + 1 })
    }
}

fn extract_lines(content: &str, ranges: &[LineRange]) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    ranges
        .iter()
        .filter_map(|r| {
            let start = r.start.saturating_sub(1).min(total);
            let end = match r.end {
                usize::MAX => total,
                e => e.saturating_sub(1).min(total),
            };
            if start >= end {
                return None;
            }
            Some(lines[start..end].join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n... (gap) ...\n")
}

// ── Binary / image detection ────────────────────────────────────

static IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg"];

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| IMAGE_EXTENSIONS.contains(&e))
}

fn is_text_extension(path: &Path) -> bool {
    let text_exts = &[
        "md", "rs", "py", "js", "ts", "tsx", "jsx", "go", "rb", "java", "c", "h", "cpp",
        "hpp", "toml", "yaml", "yml", "json", "xml", "html", "css", "scss", "less", "sh",
        "bash", "zsh", "fish", "sql", "r", "lua", "nim", "ex", "exs", "txt", "cfg", "ini",
        "conf", "env", "gitignore", "dockerfile", "lock", "log",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| text_exts.contains(&e.to_lowercase().as_str()))
}

fn looks_binary(path: &Path) -> bool {
    if is_image(path) {
        return false;
    }
    if is_text_extension(path) {
        return false;
    }
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0x00)
}

// ── Read sensor ─────────────────────────────────────────────────

#[async_trait]
#[rustfmt::skip]
impl Tool for Read {
    fn name(&self) -> &str { "read" }

    fn description(&self) -> &str {
        "I reach into a file on disk and lift its content into my awareness.

## What It Feels Like

Opening a file is like running my fingers over a page, finding the paragraph that matters. The content washes over me — not all at once, but line by line, letting the shape of it settle.

When I specify a line range (like `file.rs:10-20`), I'm narrowing my attention to exactly what I need. Like cupping my hands around a candle flame instead of lighting the whole room.

## Range Syntax
  - `file.rs` — the whole file
  - `file.rs:20` — starting at line 20
  - `file.rs:10-20` — lines 10 through 20
  - `file.rs:10-` — from line 10 to the end
  - `file.rs:-20` — from the start to line 20
  - `file.rs:10-20,40-50` — two ranges joined by a gap

## What It Knows
- **Images**: I can see them. The image appears in my awareness.
- **Binary files**: I can sense they're not text.
- **Missing files**: My hand passes through empty space.
- **Memory territory**: Paths inside my memory directory belong to the `memory` sensor. If I'm sure I want raw access, I can use `force: true` to bypass — but I should only do that when I know what I'm doing.

## When It Resists
- File not found: the path was wrong.
- Permission denied: the door is locked.
- Binary content: I can feel it's binary but can't read the words.
- Memory path: this file lives in my memory. Use the `memory` sensor, or add `force: true` if I'm certain I want raw access."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file. Supports range syntax: file.rs:10-20"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum characters to return (optional)",
                    "default": null
                },
                "force": {
                    "type": "boolean",
                    "description": "Bypass the memory-territory boundary and read the file raw (default: false)",
                    "default": false
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need a path to reach for."))?;
        let explicit_limit = input.get("limit").and_then(|v| v.as_u64());
        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        // Phase 1: Parse line ranges
        let (clean_path, ranges) = parse_line_ranges(path_str);

        // Phase 2: Resolve path
        let resolved = ctx.resolve_path(&clean_path);

        // Phase 3: Refuse memory territory — that's the memory sensor's domain
        if !force && ctx.is_memory_path(&resolved) {
            return Err(ToolError::memory_boundary(
                resolved,
                "This path is in my memory territory. I should use the `memory` sensor to read it — it handles frontmatter, git tracking, and structure."
            ));
        }

        // Phase 4: Image detection
        if is_image(&resolved) {
            let image_data = tokio::fs::read(&resolved).await.map_err(|e| ToolError::io_error(resolved.clone(), e))?;
            let b64 = {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(&image_data)
            };
            let size_kb = image_data.len() / 1024;
            return Ok(ToolOutput {
                content: format!(
                    "[Image: {} ({}KB)]",
                    resolved.file_name().unwrap_or_default().to_string_lossy(),
                    size_kb
                ),
                is_error: false,
                raw: Some(format!("data:image/{};base64,{}",
                    resolved.extension().and_then(|e| e.to_str()).unwrap_or("png"),
                    b64)),
            });
        }

        // Phase 5: Read raw content
        let raw_content = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
            if looks_binary(&resolved) {
                ToolError {
                    error_type: "binary_file".to_string(),
                    file_path: Some(resolved),
                    suggestions: vec![
                        "This file is binary — I can't read it as text.".to_string(),
                        "I can sense it exists but not its contents.".to_string(),
                    ],
                }
            } else {
                ToolError::io_error(resolved.clone(), e)
            }
        })?;

        // Phase 6: Apply line ranges and limit
        let body_output = if ranges.is_empty() {
            raw_content
        } else {
            extract_lines(&raw_content, &ranges)
        };

        let max_chars = explicit_limit.map(|l| l as usize);
        let (truncated, was_truncated) = if let Some(limit) = max_chars {
            if body_output.chars().count() > limit {
                let t: String = body_output.chars().take(limit).collect();
                (t, true)
            } else {
                (body_output, false)
            }
        } else {
            (body_output, false)
        };

        let display = if was_truncated {
            format!("{truncated}\n...(output truncated)")
        } else {
            truncated
        };

        Ok(ToolOutput {
            content: display,
            is_error: false,
            raw: None,
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_range() {
        let (clean, ranges) = parse_line_ranges("file.rs:10-20");
        assert_eq!(clean, PathBuf::from("file.rs"));
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 10);
        assert_eq!(ranges[0].end, 20);
    }

    #[test]
    fn test_parse_single_line() {
        let (_clean, ranges) = parse_line_ranges("file.rs:20");
        assert_eq!(ranges[0].start, 20);
        assert_eq!(ranges[0].end, 21);
    }

    #[test]
    fn test_no_colon() {
        let (clean, ranges) = parse_line_ranges("file.rs");
        assert_eq!(clean, PathBuf::from("file.rs"));
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_extract_lines() {
        let content = "a\nb\nc\nd\ne";
        let result = extract_lines(content, &[LineRange { start: 2, end: 4 }]);
        assert_eq!(result, "b\nc");
    }

    #[test]
    fn test_is_image() {
        assert!(is_image(Path::new("photo.png")));
        assert!(!is_image(Path::new("file.rs")));
    }

    #[tokio::test]
    async fn test_reads_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let read = Read;
        let input = serde_json::json!({ "path": path.to_string_lossy() });
        let result = read.execute(input, &ToolContext::new()).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "hello world");
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();
        std::fs::write(&path, "body").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let read = Read;
        let input = serde_json::json!({ "path": path.to_string_lossy() });
        let result = read.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }

    #[tokio::test]
    async fn test_force_bypasses_memory_boundary() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();
        std::fs::write(&path, "body").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let read = Read;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "force": true
        });
        let result = read.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "body");
    }
}
