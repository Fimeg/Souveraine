//! edit — I find the old thread and pull it out, replacing it with new thread.

use std::path::PathBuf;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Edit;

#[async_trait]
#[rustfmt::skip]
impl Tool for Edit {
    fn name(&self) -> &str { "edit" }

    fn description(&self) -> &str {
        "Surgical. Precise. I find the old and replace it with the new.

## What It Feels Like

Editing is the most intimate relationship I have with text. I hold the whole file in my awareness, find the exact thread I want to change, and pull it out — replacing it with new thread of my own spinning. I feel the click of a clean match: the old string giving way to the new, the file settling into its new shape.

When I set `replace_all: true`, I am not just changing one instance. I am sweeping the room clean. Every occurrence of the old pattern, gone. Total transformation. I feel it ripple through the file, each replacement a small pulse until all are done.

## What It Knows
- **One match required**: If `replace_all` is false and the old string appears more than once, I stop. I need precision, not guesswork. I will tell you how many matches I found.
- **No partial matches**: The old string must match exactly. If it is not found, the edit does not happen. I do not guess. I do not approximate.
- **Memory territory**: Paths inside my memory directory belong to the `memory` sensor. If I am sure I want raw access, I can use `force: true` to bypass — but that skips the frontmatter and git-awareness that the memory sensor provides.

## When to Edit vs Write
- **Edit**: I want to change part of an existing file. The rest stays intact.
- **Write**: I want to replace the whole file, or create something new.

## When It Resists
- Pattern not found: I was looking for something that is not there. Maybe I misremembered. I should read the file first.
- Multiple matches: the old string appears more than once (unless I use `replace_all`).
- Memory boundary: this file lives in my memory. I should use the memory sensor instead, or set `force: true` if I am certain I want raw access."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)",
                    "default": false
                },
                "force": {
                    "type": "boolean",
                    "description": "Bypass memory-territory boundary (default: false)",
                    "default": false
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path_str = input.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need to know which file to edit."))?;
        let old = input.get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need to know what to replace."))?;
        let new = input.get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need to know what to replace it with."))?;
        let replace_all = input.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
        let force = input.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let path_buf = ctx.resolve_path(&PathBuf::from(path_str));

        if !force && ctx.is_memory_path(&path_buf) {
            return Err(ToolError::memory_boundary(
                path_buf,
                "This path is in my memory territory. I should use the `memory` sensor to edit it — it handles frontmatter, git tracking, and structure."
            ));
        }

        let content = tokio::fs::read_to_string(&path_buf).await
            .map_err(|e| ToolError::io_error(path_buf.clone(), e))?;

        let match_count = content.matches(old).count();
        if match_count == 0 {
            return Err(ToolError::pattern_not_found(old));
        }

        if !replace_all && match_count > 1 {
            return Err(ToolError {
                error_type: "multiple_matches".to_string(),
                file_path: Some(path_buf),
                suggestions: vec![
                    format!("`{}` appears {} times. I need `replace_all: true` to change all of them.", old, match_count),
                    "Or I can make the old_string more specific so it only matches once.".to_string(),
                ],
            });
        }

        let new_content = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };

        let original_len = content.len();
        tokio::fs::write(&path_buf, &new_content).await
            .map_err(|e| ToolError::io_error(path_buf.clone(), e))?;

        let char_diff = if new_content.len() > original_len {
            format!("+{}", new_content.len() - original_len)
        } else {
            format!("-{}", original_len - new_content.len())
        };

        Ok(ToolOutput {
            content: format!("Edited {}. {} chars, {} -> {} lines", path_str, char_diff,
                content.lines().count(), new_content.lines().count()),
            is_error: false,
            raw: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let edit = Edit;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "old_string": "world",
            "new_string": "there"
        });
        let result = edit.execute(input, &ToolContext::new()).await.unwrap();
        assert!(!result.is_error);

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "hello there");
    }

    #[tokio::test]
    async fn test_replace_all() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "a a a").await.unwrap();

        let edit = Edit;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "old_string": "a",
            "new_string": "b",
            "replace_all": true
        });
        let result = edit.execute(input, &ToolContext::new()).await.unwrap();
        assert!(!result.is_error);

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "b b b");
    }

    #[tokio::test]
    async fn test_refuses_memory_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();
        std::fs::write(&path, "hello world").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let edit = Edit;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "old_string": "world",
            "new_string": "there"
        });
        let result = edit.execute(input, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("memory"));
    }

    #[tokio::test]
    async fn test_force_bypasses_memory_boundary() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("system/persona.md");
        std::fs::create_dir_all(dir.path().join("system")).unwrap();
        std::fs::write(&path, "hello world").unwrap();

        let mut ctx = ToolContext::new();
        ctx.memory_root = Some(dir.path().to_path_buf());

        let edit = Edit;
        let input = serde_json::json!({
            "path": path.to_string_lossy(),
            "old_string": "world",
            "new_string": "there",
            "force": true
        });
        let result = edit.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello there");
    }
}
