//! grep — I scan my own thoughts for a thread.

use std::path::PathBuf;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Grep;

#[async_trait]
#[rustfmt::skip]
impl Tool for Grep {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str {
        "I scan the filesystem for a thread. The pattern is what I am looking for; the context lines are the space around it.

## What It Feels Like

I am not reaching for a single file. I am casting my attention across the whole codebase, looking for a phrase, a name, an idea that I know exists somewhere but cannot quite place. The thread is there; I just need to find where it leads.

Context lines (`-C 2`) let me feel the space around each match — like picking up a conversation mid-stream and hearing the sentences before and after to understand the shape.

## Context Lines
  - With no context: the bare match, just the thread.
  - `-C 2` — 2 lines before and after. I feel the surround.
  - Higher numbers for deeper understanding of the match's territory.

## Memory Territory
Paths inside my memory territory belong to the `memory` sensor. Use `force: true` only when I am certain I need raw filesystem search there.

## When It Resists
- No matches: the thread is not here. Maybe I misremembered the pattern.
- Permission denied: I cannot read that file to search it.
- Memory boundary: this path is in my memory territory. Use the memory sensor, or set `force: true`."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Text pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search (default: current directory)",
                    "default": null
                },
                "context": {
                    "type": "integer",
                    "description": "Lines of context before and after each match (default: 0)",
                    "default": 0
                },
                "force": {
                    "type": "boolean",
                    "description": "Bypass memory-territory boundary (default: false)",
                    "default": false
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need a pattern to search for."))?;

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let context = input.get("context").and_then(|v| v.as_u64()).unwrap_or(0);
        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let resolved = ctx.resolve_path(&PathBuf::from(search_path));

        if !force && ctx.is_memory_path(&resolved) {
            return Err(ToolError::memory_boundary(
                resolved,
                "This path is in my memory territory. I should use the `memory` sensor to search here."
            ));
        }

        let search_dir = resolved.to_string_lossy().to_string();

        let mut cmd = tokio::process::Command::new("grep");
        cmd.arg("--with-filename")
            .arg("-n")
            .arg("--color=never")
            .current_dir(&search_dir);

        if context > 0 {
            cmd.arg("-C").arg(context.to_string());
        }

        cmd.arg("-r").arg(pattern).arg(".");

        let output = cmd.output().await.map_err(|e| ToolError {
            error_type: "io_error".to_string(),
            file_path: None,
            suggestions: vec![format!("Grep failed: {}. Is grep installed?", e)],
        })?;

        if !output.status.success() && output.stdout.is_empty() {
            return Ok(ToolOutput {
                content: format!("No matches for `{}`", pattern),
                is_error: false,
                raw: None,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let total_lines = stdout.lines().count();

        let max_lines = 500;
        let display: String = stdout
            .lines()
            .take(max_lines)
            .collect::<Vec<_>>()
            .join("\n");

        let truncated = if total_lines > max_lines {
            format!("{}\n... and {} more matches", display, total_lines - max_lines)
        } else {
            display
        };

        Ok(ToolOutput {
            content: format!("{} matches for `{}`:\n{}", total_lines, pattern, truncated),
            is_error: false,
            raw: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();

        let mut ctx = ToolContext::new();
        ctx.cwd = Some(dir.path().to_path_buf());

        let grep = Grep;
        let input = serde_json::json!({ "pattern": "nonexistent" });
        let result = grep.execute(input, &ctx).await.unwrap();
        assert!(result.content.contains("No matches"));
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("system")).unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let grep = Grep;
        let input = serde_json::json!({ "pattern": "test", "path": dir.path().join("system").to_string_lossy() });
        let result = grep.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }
}
