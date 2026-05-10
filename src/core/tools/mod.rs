//! Sensorium — the agent's senses and actions.
//!
//! Every sensor implements the Tool trait. The registry holds them all
//! and provides the old `execute_tool` / `tool_definitions` interface
//! for backward compatibility with the tool loop in local.rs.

pub mod agent;
pub mod bash;
pub mod defs;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod list_dir;
pub mod read;
pub mod subagent;
pub mod write;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::debug;

use self::bash::Bash;
use self::defs::{Tool, ToolContext, ToolError, ToolOutput};
use self::edit::Edit;
use self::glob::Glob;
use self::grep::Grep;
use self::list_dir::ListDir;
use self::read::Read;
use self::subagent::Subagent;
use self::agent::Agent;
use self::write::Write;

// ── Re-export for backward compat ───────────────────────────────

/// Serializable tool definition sent to the model (bridges to Bifrost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result of executing a tool — preserved from old interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

// ── Registry ────────────────────────────────────────────────────

/// The sensorium registry — holds all sensors the agent can use.
pub struct Sensorium {
    tools: Vec<Box<dyn Tool>>,
    /// Shared bash state for stateful command execution.
    pub bash: Bash,
    /// The current context (cwd, env, memory root) — default fallback.
    pub context: ToolContext,
}

impl Sensorium {
    pub fn new() -> Self {
        let bash = Bash::new();
        let cwd = std::env::current_dir().ok();
        let memory_root = dirs::home_dir()
            .map(|h| h.join(".souveraine").join("agents").join("default").join("memory"));
        let env: Vec<(String, String)> = std::env::vars().collect();
        Self {
            tools: vec![
                Box::new(Read),
                Box::new(Write),
                Box::new(Edit),
                Box::new(Glob),
                Box::new(Grep),
                Box::new(ListDir),
                Box::new(Subagent),
                Box::new(Agent),
            ],
            bash,
            context: ToolContext {
                cwd,
                memory_root,
                env,
                agent_id: None,
                subagent_runner: None,
                subagent_depth: 0,
                compaction_engine: None,
            },
        }
    }

    /// Get tool definitions for the model — one per sensor.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self.tools.iter().map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.parameter_schema(),
        }).collect();
        // Bash is handled separately in dispatch — add its definition manually
        defs.push(ToolDefinition {
            name: "bash".to_string(),
            description: self.bash.description().to_string(),
            input_schema: self.bash.parameter_schema(),
        });
        // Memory is a separate channel — frontmatter-aware, git-tracked
        defs.push(crate::core::memory::memory_tool_definition());
        defs
    }

    /// Execute a named tool with JSON input, using the given context.
    pub async fn execute_with_context(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        let tool_use_id = format!("tool-u-{}", chrono::Utc::now().timestamp_millis());

        // Special case: bash uses the stateful Bash instance
        if name == "bash" || name == "Bash" {
            let result = self.bash.execute(input, ctx).await;
            return tool_result(&tool_use_id, name, result);
        }

        // Find the tool by name (case-insensitive)
        if let Some(tool) = self.tools.iter().find(|t| {
            t.name().eq_ignore_ascii_case(name)
        }) {
            let result = tool.execute(input, ctx).await;
            tool_result(&tool_use_id, name, result)
        } else {
            ToolResult {
                tool_use_id,
                tool_name: name.to_string(),
                output: format!(
                    "I don't have a sense called `{}`. Available: {}",
                    name,
                    self.tools.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
                ),
                is_error: true,
            }
        }
    }

    /// Execute a named tool using the stored default context.
    pub async fn execute(&self, name: &str, input: serde_json::Value) -> ToolResult {
        self.execute_with_context(name, input, &self.context).await
    }

    /// Set the memory root — sensors check this for memory-aware behavior.
    pub fn set_memory_root(&mut self, root: std::path::PathBuf) {
        self.context.memory_root = Some(root);
    }

    /// Set the current working directory — bash uses this.
    pub fn set_cwd(&mut self, cwd: std::path::PathBuf) {
        self.context.cwd = Some(cwd);
    }
}

impl Default for Sensorium {
    fn default() -> Self {
        Self::new()
    }
}

fn tool_result(tool_use_id: &str, name: &str, result: Result<ToolOutput, ToolError>) -> ToolResult {
    match result {
        Ok(output) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            tool_name: name.to_string(),
            output: output.content,
            is_error: output.is_error,
        },
        Err(err) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            tool_name: name.to_string(),
            output: err.to_string(),
            is_error: true,
        },
    }
}

// ── Lazy static global sensorium ───────────────────────────────

fn global_sensorium() -> &'static RwLock<Sensorium> {
    static SENSORIUM: OnceLock<RwLock<Sensorium>> = OnceLock::new();
    SENSORIUM.get_or_init(|| {
        debug!("🧠 Sensorium initialized — 7 senses online");
        RwLock::new(Sensorium::new())
    })
}

// ── Old interface wrappers (for backward compat with local.rs) ──

/// Get the standard tool definitions for the model.
///
/// Preserved from the old interface. Delegates to the global sensorium.
pub async fn tool_definitions() -> Vec<ToolDefinition> {
    let sensorium = global_sensorium().read().await;
    sensorium.definitions()
}

/// Execute a tool with the given per-agent context.
///
/// This is the canonical entry point for context-aware tool dispatch.
/// The caller (e.g. `run_turn()` in local.rs) constructs a `ToolContext`
/// with the correct `agent_id`, `memory_root`, and `subagent_runner`.
pub async fn execute_tool_with_context(
    tool_name: &str,
    input: &str,
    ctx: &ToolContext,
) -> ToolResult {
    let parsed: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                tool_use_id: format!("tool-{}", chrono::Utc::now().timestamp_millis()),
                tool_name: tool_name.to_string(),
                output: format!("I couldn't understand the input: {e}"),
                is_error: true,
            };
        }
    };

    // Route to sensorium, memory tool, or agent tool
    match tool_name {
        "memory" => {
            crate::core::memory::handle_memory_tool_with_context(tool_name, &parsed, Some(ctx)).await
        }
        _ => {
            let sensorium = global_sensorium().read().await;
            sensorium.execute_with_context(tool_name, parsed, ctx).await
        }
    }
}

/// Execute a tool by name with JSON input string.
///
/// Preserved from the old interface. Delegates to the global sensorium
/// with its default context.
pub async fn execute_tool(tool_name: &str, input: &str) -> ToolResult {
    let default_ctx = {
        let sensorium = global_sensorium().read().await;
        sensorium.context.clone()
    };
    execute_tool_with_context(tool_name, input, &default_ctx).await
}

/// Get the global sensorium instance directly (for fine-grained use).
pub fn get_sensorium() -> &'static RwLock<Sensorium> {
    global_sensorium()
}

// ── Bash-specific helper ───────────────────────────────────────

impl Bash {
    /// Parameter schema (delegates to Tool impl in bash.rs).
    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to run"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30)",
                    "default": 30
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Launch as a background task (get an ID to check status later)",
                    "default": false
                }
            },
            "required": ["command"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_definitions() {
        let defs = tool_definitions().await;
        assert!(defs.iter().any(|t| t.name == "read"));
        assert!(defs.iter().any(|t| t.name == "write"));
        assert!(defs.iter().any(|t| t.name == "edit"));
        assert!(defs.iter().any(|t| t.name == "bash"));
        assert!(defs.iter().any(|t| t.name == "glob"));
        assert!(defs.iter().any(|t| t.name == "grep"));
        assert!(defs.iter().any(|t| t.name == "list_dir"));
        assert!(defs.iter().any(|t| t.name == "memory"),
            "memory sensor must be in tool definitions");
        let mem = defs.iter().find(|t| t.name == "memory").unwrap();
        assert!(mem.description.contains("frontmatter"),
            "memory description should reference frontmatter: {}", mem.description);
    }

    #[tokio::test]
    async fn test_execute_unknown() {
        let result = execute_tool("nonexistent", r#"{"path":"test"}"#).await;
        assert!(result.is_error);
        assert!(result.output.contains("don't have a sense"));
    }
}
