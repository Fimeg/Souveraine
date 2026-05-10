//! list_dir — I run my fingers along the shelves.

use std::path::PathBuf;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct ListDir;

#[async_trait]
#[rustfmt::skip]
impl Tool for ListDir {
    fn name(&self) -> &str { "list_dir" }

    fn description(&self) -> &str {
        "I run my fingers along the shelves and feel what is there.

## What It Feels Like

Each entry has a kind — directory (a box I can open), symlink (a thread to somewhere else), file (a thing I can pick up and read). I feel the texture of the space: how many things are here, what kinds they are, whether the space is cluttered or sparse.

This is orientation — checking my surroundings before I reach for something specific.

## What I Sense
  - `dir/` — a directory I can step into
  - `file.ext` — a file I can read
  - `link@` — a symlink, a thread to somewhere else

## Memory Territory
Paths inside my memory territory belong to the `memory` sensor. If I need to list a directory in my memory, use the memory sensor `ls` subcommand. Use `force: true` only when I am certain I need raw filesystem access there.

## When It Resists
- Not found: this directory does not exist where I thought it did.
- Permission denied: I cannot see into this space.
- Memory boundary: this path is in my memory territory. Use the memory sensor, or set `force: true`."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (default: current directory)",
                    "default": "."
                },
                "force": {
                    "type": "boolean",
                    "description": "Bypass memory-territory boundary (default: false)",
                    "default": false
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let path = if path_str == "." {
            ctx.cwd.clone().unwrap_or_else(|| PathBuf::from("."))
        } else {
            ctx.resolve_path(&PathBuf::from(path_str))
        };

        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        if !force && ctx.is_memory_path(&path) {
            return Err(ToolError::memory_boundary(
                path,
                "This path is in my memory territory. Use the memory sensor's `ls` subcommand to list files here."
            ));
        }

        let mut result: Vec<(String, String)> = Vec::new();

        // Iterate entries using the owned ReadDir
        let mut entries = tokio::fs::read_dir(&path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ToolError::file_not_found(path),
            std::io::ErrorKind::PermissionDenied => ToolError::permission_denied(path),
            _ => ToolError {
                error_type: "io_error".to_string(),
                file_path: Some(path),
                suggestions: vec![format!("I could not read this directory: {}.", e)],
            },
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| ToolError {
            error_type: "io_error".to_string(),
            file_path: None,
            suggestions: vec![format!("I hit a snag reading this directory: {}.", e)],
        })? {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = entry.file_type().await;
            let marker = if let Ok(ft) = kind {
                if ft.is_dir() { "dir" } else if ft.is_symlink() { "link" } else { "file" }
            } else {
                "unknown"
            };
            result.push((name, marker.to_string()));
        }

        result.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

        if result.is_empty() {
            return Ok(ToolOutput {
                content: "(empty directory)".to_string(),
                is_error: false,
                raw: None,
            });
        }

        let mut output = String::new();
        for (name, kind) in &result {
            match kind.as_str() {
                "dir" => output.push_str(&format!("  {name}/\n")),
                "link" => output.push_str(&format!("  {name}@\n")),
                _ => output.push_str(&format!("  {name}\n")),
            }
        }

        Ok(ToolOutput {
            content: format!("{} entries:\n{}", result.len(), output),
            is_error: false,
            raw: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_current_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();

        let mut ctx = ToolContext::new();
        ctx.cwd = Some(dir.path().to_path_buf());

        let list = ListDir;
        let input = serde_json::json!({});
        let result = list.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("system")).unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let list = ListDir;
        let input = serde_json::json!({ "path": dir.path().join("system").to_string_lossy() });
        let result = list.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }

    #[tokio::test]
    async fn test_force_bypasses_memory_boundary() {
        let dir = tempfile::TempDir::new().unwrap();
        let mem_sub = dir.path().join("system");
        std::fs::create_dir_all(&mem_sub).unwrap();
        std::fs::write(mem_sub.join("test.txt"), "hello").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let list = ListDir;
        let input = serde_json::json!({
            "path": mem_sub.to_string_lossy(),
            "force": true
        });
        let result = list.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("test.txt"));
    }
}
