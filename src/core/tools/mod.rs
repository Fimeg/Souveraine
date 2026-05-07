//! Tools — The entity's hands
//!
//! Standard tools the model can invoke to interact with the filesystem
//! and terminal. Each tool implements the Tool trait.
//!
//! Based on the Claude Code / claw-code tool patterns.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::debug;

use crate::core::memory;

/// Tool definition sent to the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Read a file from the filesystem
pub async fn read_file(path: &str) -> Result<String> {
    debug!("📖 Reading file: {}", path);
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("Reading file: {path}"))?;
    Ok(content)
}

/// Write content to a file
pub async fn write_file(path: &str, content: &str) -> Result<String> {
    debug!("✍️ Writing file: {}", path);
    if let Some(parent) = std::path::Path::new(path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Creating parent dirs for: {path}"))?;
    }
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("Writing file: {path}"))?;
    Ok(format!("Written {} bytes to {}", content.len(), path))
}

/// Edit a file by replacing a string
pub async fn edit_file(path: &str, old_string: &str, new_string: &str) -> Result<String> {
    debug!("✏️ Editing file: {}", path);
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("Reading file for edit: {path}"))?;

    if !content.contains(old_string) {
        return Err(anyhow::anyhow!("String to replace not found in {path}"));
    }

    let new_content = content.replace(old_string, new_string);
    tokio::fs::write(path, &new_content)
        .await
        .with_context(|| format!("Writing edited file: {path}"))?;

    let diff_lines = content.lines().count() - new_content.lines().count();
    Ok(format!(
        "Edited {}. Changed {} chars, {} lines",
        path,
        content.len() - new_content.len(),
        diff_lines
    ))
}

/// Run a bash command
pub async fn run_bash(command: &str, _timeout_secs: u64) -> Result<String> {
    debug!("⚙️ Running: {}", command);
    let output = Command::new("bash")
        .arg("-c")
        .arg(command)
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("Running command: {command}"))?;

    let mut result = String::new();

    if !output.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Truncate very long output
        if stdout.len() > 10000 {
            result.push_str(&stdout[..10000]);
            result.push_str("\n... (output truncated)");
        } else {
            result.push_str(&stdout);
        }
    }

    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !result.is_empty() {
            result.push('\n');
        }
        if stderr.len() > 5000 {
            result.push_str(&stderr[..5000]);
            result.push_str("\n... (stderr truncated)");
        } else {
            result.push_str(&stderr);
        }
    }

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Command exited with code {:?}:\n{}",
            output.status.code(),
            result
        ));
    }

    Ok(result)
}

/// List a directory
pub async fn list_dir(path: &str) -> Result<String> {
    debug!("📁 Listing: {}", path);
    let entries = tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("Listing directory: {path}"))?;

    let mut result = String::new();
    use futures::StreamExt;
    let mut stream = tokio_stream::wrappers::ReadDirStream::new(entries);

    while let Some(entry) = stream.next().await {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = entry.file_type().await?;
        if kind.is_dir() {
            result.push_str(&format!("  {name}/\n"));
        } else if kind.is_symlink() {
            result.push_str(&format!("  {name}@\n"));
        } else {
            result.push_str(&format!("  {name}\n"));
        }
    }

    Ok(result)
}

/// Execute a tool by name with JSON input
pub async fn execute_tool(tool_name: &str, input: &str) -> ToolResult {
    let tool_use_id = format!("tool-{}", chrono::Utc::now().timestamp_millis());

    // Parse JSON input
    let parsed: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                tool_use_id,
                tool_name: tool_name.to_string(),
                output: format!("Failed to parse tool input JSON: {e}"),
                is_error: true,
            };
        }
    };

    match tool_name {
        "memory" => {
            let command = parsed.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let cmd = match command {
                "read" => {
                    let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    memory::MemoryCommand::Read { path }
                }
                "write" => {
                    let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let content = parsed.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    memory::MemoryCommand::Write { path, content }
                }
                "append" => {
                    let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let content = parsed.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    memory::MemoryCommand::Append { path, content }
                }
                "ls" => {
                    let path = parsed.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
                    memory::MemoryCommand::Ls { path }
                }
                "status" => memory::MemoryCommand::Status,
                "init" => {
                    let agent_id = parsed.get("agent_id").and_then(|v| v.as_str()).unwrap_or("default").to_string();
                    memory::MemoryCommand::Init { agent_id }
                }
                "delete" => {
                    let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    memory::MemoryCommand::Delete { path }
                }
                "compact" => {
                    let strategy = parsed.get("strategy").and_then(|v| v.as_str()).map(|s| s.to_string());
                    memory::MemoryCommand::Compact { strategy }
                }
                _ => {
                    return ToolResult {
                        tool_use_id,
                        tool_name: tool_name.to_string(),
                        output: format!("Unknown memory subcommand: {}. Available: read, write, append, ls, status, init, delete, compact", command),
                        is_error: true,
                    };
                }
            };

            match memory::execute_memory_command(&cmd).await {
                Ok(output) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        "read" | "Read" => {
            let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match read_file(path).await {
                Ok(content) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        "write" | "Write" => {
            let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = parsed.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match write_file(path, content).await {
                Ok(msg) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: msg,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        "edit" | "Edit" => {
            let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let old = parsed.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let new = parsed.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
            match edit_file(path, old, new).await {
                Ok(msg) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: msg,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        "bash" | "Bash" => {
            let cmd = parsed.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let timeout = parsed.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);
            match run_bash(cmd, timeout).await {
                Ok(output) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        "list_dir" | "ListDir" | "ls" => {
            let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            match list_dir(path).await {
                Ok(output) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    output: format!("Error: {e}"),
                    is_error: true,
                },
            }
        }
        _ => ToolResult {
            tool_use_id,
            tool_name: tool_name.to_string(),
            output: format!("Unknown tool: {tool_name}. Available: read, write, edit, bash, list_dir"),
            is_error: true,
        },
    }
}

/// Get the standard tool definitions to send to the model
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        memory::memory_tool_definition(),
        ToolDefinition {
            name: "read".to_string(),
            description: "Read the contents of a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write".to_string(),
            description: "Write content to a file (creates parent dirs)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to write to" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "edit".to_string(),
            description: "Edit a file by replacing exact string matches".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "old_string": { "type": "string", "description": "Text to replace" },
                    "new_string": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolDefinition {
            name: "bash".to_string(),
            description: "Run a shell command".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to run" },
                    "timeout": { "type": "number", "description": "Timeout in seconds", "default": 30 }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "list_dir".to_string(),
            description: "List contents of a directory".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path", "default": "." }
                },
                "required": []
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_and_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_string_lossy().to_string();

        write_file(&path_str, "hello world").await.unwrap();
        let content = read_file(&path_str).await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_edit_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_string_lossy().to_string();

        write_file(&path_str, "hello world").await.unwrap();
        edit_file(&path_str, "world", "there").await.unwrap();
        let content = read_file(&path_str).await.unwrap();
        assert_eq!(content, "hello there");
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let defs = tool_definitions();
        assert!(defs.iter().any(|t| t.name == "read"));
        assert!(defs.iter().any(|t| t.name == "write"));
        assert!(defs.iter().any(|t| t.name == "edit"));
        assert!(defs.iter().any(|t| t.name == "bash"));
        assert!(defs.iter().any(|t| t.name == "list_dir"));
    }
}
