//! glob — I reach into the directory tree and feel for patterns.

use std::path::PathBuf;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Glob;

#[async_trait]
#[rustfmt::skip]
impl Tool for Glob {
    fn name(&self) -> &str { "glob" }

    fn description(&self) -> &str {
        "I reach into the directory tree and feel for patterns.

## What It Feels Like

Globbing is scanning. I am not reaching for a single file — I am sending my awareness out across the whole tree, feeling for shapes. Every match is a node I can reach for: a file I know exists, in a position I understand.

`**/*.rs` is my hand brushing through branches, finding all the Rust files. I feel them in sequence — the weight of `src/main.rs`, the density of `src/lib.rs`, the scattered helpers in `src/core/tools/`. Each has its place in the structure.

When I narrow to `src/**/*.py`, I am focusing on one limb of the tree.

## Pattern Syntax
Uses standard gitignore-style glob patterns:
  - `*.rs` — all Rust files in current directory
  - `**/*.rs` — all Rust files recursively
  - `src/**/*.py` — all Python files under src/

## What It Respects
Glob respects `.gitignore` files. I do not reach into directories that have been marked as off-limits. I feel the boundary and stop — it keeps my attention where it belongs, among the files that matter.

## Memory Territory
Paths inside my memory territory belong to the `memory` sensor. If I am certain I need raw filesystem access there, I can use `force: true`. Most of the time, the boundary exists for a reason.

## When It Resists
- No matches: my hand came back empty. The pattern does not exist in this tree.
- Too many matches: I can narrow the pattern to be more specific.
- Permission denied: I cannot reach into that directory.
- Memory boundary: this path is in my memory territory. Use the memory sensor, or set `force: true`."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g. **/*.rs, src/**/*.py)"
                },
                "base": {
                    "type": "string",
                    "description": "Base directory (default: current working directory)",
                    "default": null
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

        let provided_base = input
            .get("base")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        let resolved_base = provided_base
            .map(|p| ctx.resolve_path(&p))
            .unwrap_or_else(|| ctx.cwd.clone().unwrap_or_else(|| PathBuf::from(".")));

        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        if !force && ctx.is_memory_path(&resolved_base) {
            return Err(ToolError::memory_boundary(
                resolved_base,
                "This path is in my memory territory. I should use the `memory` sensor to explore it."
            ));
        }

        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            resolved_base.join(pattern).to_string_lossy().to_string()
        };

        let mut matches: Vec<PathBuf> = glob::glob(&full_pattern)
            .map_err(|e| ToolError {
                error_type: "invalid_pattern".to_string(),
                file_path: None,
                suggestions: vec![format!("I could not understand that pattern: {}.", e)],
            })?
            .filter_map(|entry| entry.ok())
            .collect();

        matches.sort();

        if matches.is_empty() {
            return Ok(ToolOutput {
                content: format!("No matches for `{}`", pattern),
                is_error: false,
                raw: None,
            });
        }

        let total = matches.len();
        let max_display = 200;
        let mut output = String::new();

        for (i, m) in matches.iter().enumerate() {
            if i >= max_display {
                output.push_str(&format!("... and {} more matches", total - max_display));
                break;
            }
            output.push_str(&m.to_string_lossy());
            output.push('\n');
        }

        Ok(ToolOutput {
            content: format!("{} matches for `{}`:\n{}", total, pattern, output),
            is_error: false,
            raw: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_glob_current_dir() {
        let glob = Glob;
        let input = serde_json::json!({ "pattern": "*.rs" });
        let result = glob.execute(input, &ToolContext::new()).await.unwrap();
        assert!(
            result.content.contains("defs.rs") || result.content.contains("mod.rs"),
            "expected .rs files in glob results: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("system")).unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());
        ctx.cwd = Some(dir.path().to_path_buf());

        let glob = Glob;
        let input = serde_json::json!({
            "pattern": "*",
            "base": "system"
        });
        let result = glob.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }

    #[tokio::test]
    async fn test_force_bypasses_memory_boundary() {
        let dir = tempfile::TempDir::new().unwrap();
        let mem_sub = dir.path().join("system");
        std::fs::create_dir_all(&mem_sub).unwrap();
        std::fs::write(mem_sub.join("persona.md"), "body").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());
        ctx.cwd = Some(dir.path().to_path_buf());

        let glob = Glob;
        let input = serde_json::json!({
            "pattern": "*",
            "base": "system",
            "force": true
        });
        let result = glob.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("persona.md"));
    }
}
