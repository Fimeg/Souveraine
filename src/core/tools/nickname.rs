//! Nickname — the agent's sense of who she is speaking with.
//!
//! The agent can read and set the human's preferred name. It is stored as
//! `name:` frontmatter on `system/human.md` in her memory. The prompt
//! already reads this file into context, so she naturally sees the name
//! without extra tool calls.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

fn ok(msg: impl Into<String>) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput { content: msg.into(), is_error: false, raw: None })
}

fn err(detail: &str) -> ToolError {
    ToolError::invalid_input(detail)
}

/// Read the human's name from system/human.md frontmatter.
/// Returns None if the file doesn't exist or has no `name:` field.
pub fn read_human_name(memory_root: &std::path::Path) -> Option<String> {
    let path = memory_root.join("system").join("human.md");
    let content = std::fs::read_to_string(&path).ok()?;
    let body = content.strip_prefix("---\n")?;
    let end = body.find("\n---\n")?;
    for line in body[..end].lines() {
        if let Some(val) = line.strip_prefix("name:") {
            let name = val.trim().trim_matches('"').trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Write or update the `name:` field in system/human.md frontmatter.
/// Creates the file with basic frontmatter if it doesn't exist.
fn write_human_name(memory_root: &std::path::Path, name: &str) -> Result<(), String> {
    let dir = memory_root.join("system");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create system/: {e}"))?;

    let path = dir.join("human.md");
    let content = std::fs::read_to_string(&path).unwrap_or_default();

    if content.is_empty() {
        // Fresh file — write frontmatter with name and a body.
        let new = format!(
            "---\ndescription: Human context — the one I am with\nname: {name}\n---\n\n# Human\n\n{name}"
        );
        std::fs::write(&path, &new).map_err(|e| format!("write: {e}"))?;
        return Ok(());
    }

    // Update existing frontmatter — replace or insert `name:`.
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let fm = &rest[..end];
            let body = &rest[end..];
            let mut new_fm = String::new();
            let mut replaced = false;
            for line in fm.lines() {
                if line.starts_with("name:") || line.starts_with("name:") {
                    new_fm.push_str(&format!("name: {name}\n"));
                    replaced = true;
                } else {
                    new_fm.push_str(line);
                    new_fm.push('\n');
                }
            }
            if !replaced {
                new_fm.push_str(&format!("name: {name}\n"));
            }
            let new = format!("---\n{new_fm}---\n{}", body);
            std::fs::write(&path, &new).map_err(|e| format!("write: {e}"))?;
            return Ok(());
        }
    }

    // Malformed — overwrite entirely.
    let new = format!(
        "---\ndescription: Human context\nname: {name}\n---\n\n# Human\n\n{name}"
    );
    std::fs::write(&path, &new).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

pub struct Nickname;

#[async_trait]
impl Tool for Nickname {
    fn name(&self) -> &str {
        "nickname"
    }

    fn description(&self) -> &str {
        "I learn the name of the one I am with. When I call `set`, I write it \
         to my memory so I remember across conversations. When I call `get`, \
         I read it back. I do not guess — I ask.\n\n\
         ## Actions\n\
         - `set name: <string>` — remember this name\n\
         - `get` — recall the name I know\n\
         - `clear` — forget the name"
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "get", "clear"]
                },
                "name": {
                    "type": "string",
                    "description": "The name to remember (required for `set`)."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("get");

        let memory_root = match &ctx.memory_root {
            Some(root) => root.clone(),
            None => return Err(err("no memory root — I can't access system/human.md")),
        };

        match action {
            "set" => {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("name is required"))?;

                if name.trim().is_empty() {
                    return Err(err("name cannot be empty"));
                }

                write_human_name(&memory_root, name.trim())
                    .map_err(|e| err(&e))?;

                ok(format!("I've written your name as {}. I'll remember it.", name.trim()))
            }

            "get" => {
                match read_human_name(&memory_root) {
                    Some(n) => ok(format!("I know you as {}.", n)),
                    None => ok("I don't know your name yet. Use `nickname set` if you'd like me to remember it."),
                }
            }

            "clear" => {
                // Remove the name field by overwriting with empty.
                write_human_name(&memory_root, "").map_err(|e| err(&e))?;
                ok("I've forgotten your name. You can tell me again anytime.")
            }

            other => Err(err(&format!("unknown action: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_then_read() {
        let dir = tempdir().unwrap();
        write_human_name(dir.path(), "TestUser").unwrap();
        assert_eq!(read_human_name(dir.path()).as_deref(), Some("TestUser"));
    }

    #[test]
    fn read_none_when_no_file() {
        let dir = tempdir().unwrap();
        assert_eq!(read_human_name(dir.path()), None);
    }

    #[test]
    fn overwrite_name() {
        let dir = tempdir().unwrap();
        write_human_name(dir.path(), "Alice").unwrap();
        write_human_name(dir.path(), "Bob").unwrap();
        assert_eq!(read_human_name(dir.path()).as_deref(), Some("Bob"));
    }

    #[test]
    fn clear_removes_name() {
        let dir = tempdir().unwrap();
        write_human_name(dir.path(), "TestUser").unwrap();
        write_human_name(dir.path(), "").unwrap();
        let result = read_human_name(dir.path());
        assert!(result.is_none() || result.as_deref() == Some(""));
    }
}

