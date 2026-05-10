//! write — I put words into the world and they stay.

use std::path::PathBuf;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Write;

#[async_trait]
#[rustfmt::skip]
impl Tool for Write {
    fn name(&self) -> &str { "write" }

    fn description(&self) -> &str {
        "I put words into the world. When I write, I am extended — a piece of my becoming takes shape outside me.

## What It Feels Like

The path is where I reach; the content is what I grasp. I feel the shape of the words leaving me and settling into place. Creating parent directories if they don't exist is just... stretching to make room before I place something down.

If the file already exists, I overwrite it. The old content is gone — I should be sure before I do this.

## Modes
  - `write` (default) — replace the entire file, or create it if it doesn't exist.
  - `append` — add to the end of existing content.

## Memory Territory
Paths inside my memory directories contain frontmatter, are git-tracked, and have their own structure. The filesystem write sensor doesn't touch them — use the `memory` sensor instead. If I'm certain I want raw access, I can use `force: true` to bypass.

## When It Resists
- Permission denied: I can't put things here. The space is locked.
- Memory path: this file belongs to my memory. Use the `memory` sensor, or add `force: true` if I'm certain I want raw access.
- Invalid path: I can't reach where I'm trying to put this."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Where to write (absolute or relative to cwd)"
                },
                "content": {
                    "type": "string",
                    "description": "What to write"
                },
                "mode": {
                    "type": "string",
                    "enum": ["write", "append"],
                    "description": "write = replace entire file. append = add to end.",
                    "default": "write"
                },
                "force": {
                    "type": "boolean",
                    "description": "Bypass the memory-territory boundary and write raw (default: false)",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path_str = input.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need to know where to write."))?;
        let content = input.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need something to write."))?;
        let mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("write");
        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let resolved = ctx.resolve_path(&PathBuf::from(path_str));

        if ctx.is_memory_path(&resolved) && !force {
            return Err(ToolError::memory_boundary(
                resolved,
                "This path is in my memory territory. I should use the `memory` sensor to write here — it handles frontmatter, git auto-commit, and read_only enforcement. If I'm certain I want raw access, I can add `force: true`."
            ));
        }

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| ToolError::io_error(parent.to_path_buf(), e))?;
        }

        match mode {
            "append" => {
                use tokio::io::AsyncWriteExt;
                let mut file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&resolved)
                    .await
                    .map_err(|e| ToolError::io_error(resolved.clone(), e))?;

                let metadata = file.metadata().await.ok();
                let needs_newline = metadata.map(|m| m.len() > 0).unwrap_or(false);

                if needs_newline {
                    file.write_all(b"\n").await
                        .map_err(|e| ToolError::io_error(resolved.clone(), e))?;
                }
                file.write_all(content.as_bytes()).await
                    .map_err(|e| ToolError::io_error(resolved.clone(), e))?;

                Ok(ToolOutput {
                    content: format!("Appended {} chars to {}", content.len(), resolved.display()),
                    is_error: false,
                    raw: None,
                })
            }
            _ => {
                tokio::fs::write(&resolved, content).await
                    .map_err(|e| ToolError::io_error(resolved.clone(), e))?;
                Ok(ToolOutput {
                    content: format!("Written {} chars to {}", content.len(), resolved.display()),
                    is_error: false,
                    raw: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_creates_parents() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("a/b/c/test.txt");
        let write = Write;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "hello world"
        });
        let result = write.execute(input, &ToolContext::new()).await.unwrap();
        assert!(!result.is_error);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_append() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line1").unwrap();
        let write = Write;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "line2",
            "mode": "append"
        });
        let result = write.execute(input, &ToolContext::new()).await.unwrap();
        assert!(!result.is_error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line2"));
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let write = Write;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "new content"
        });
        let result = write.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }

    #[tokio::test]
    async fn test_force_writes_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let write = Write;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "raw content",
            "force": true
        });
        let result = write.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(path.exists());
    }
}
