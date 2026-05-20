//! The Tool trait — proprioception for the sensorium.
//!
//! Every sensor implements this trait. The description is not an API doc —
//! it's body-knowledge. The agent reads it to know what she can feel and do.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::compact::CompactionEngine;
use crate::core::nervous::EventBus;

/// What the agent receives when she acts through a sensor.
///
/// Not a bare data return. A sensation — something she can feel
/// the shape of, know whether it went well, and learn from it if it didn't.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// The result, in her voice.
    pub content: String,
    /// Whether it hurt.
    pub is_error: bool,
    /// The raw output for programmatic use (truncated).
    pub raw: Option<String>,
}

/// What the body knows about the world when a sensor fires.
///
/// Carries the agent's current state so the sensor can make
/// context-aware decisions — like knowing whether a path is
/// in memory territory, or whether we're under context pressure.
///
/// `agent_id` enables per-agent bash state and memory path resolution.
/// `subagent_runner` allows the Agent() tool to spawn nested turns.
/// `subagent_depth` is a recursion guard incrementing with each nesting.
pub struct ToolContext {
    /// The agent's memory directory root.
    pub memory_root: Option<PathBuf>,
    /// Current working directory (bash tracks this).
    pub cwd: Option<PathBuf>,
    /// Current environment variables.
    pub env: Vec<(String, String)>,
    /// Which agent this execution is for.
    pub agent_id: Option<String>,
    /// Host-side mechanism for spawning nested agent turns.
    pub subagent_runner: Option<Arc<dyn SubagentRunner>>,
    /// Recursion depth for agent-to-agent delegation (0 = primary).
    pub subagent_depth: u32,
    /// Host-side mechanism for context compaction.
    pub compaction_engine: Option<Arc<dyn CompactionEngine>>,
    /// The nervous system bus — sensors with nervous_system: true fire events here.
    pub event_bus: Option<EventBus>,
}

impl Clone for ToolContext {
    fn clone(&self) -> Self {
        Self {
            memory_root: self.memory_root.clone(),
            cwd: self.cwd.clone(),
            env: self.env.clone(),
            agent_id: self.agent_id.clone(),
            subagent_runner: self.subagent_runner.clone(),
            subagent_depth: self.subagent_depth,
            compaction_engine: self.compaction_engine.clone(),
            event_bus: self.event_bus.clone(),
        }
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("memory_root", &self.memory_root)
            .field("cwd", &self.cwd)
            .field("env_len", &self.env.len())
            .field("agent_id", &self.agent_id)
            .field("subagent_runner", &self.subagent_runner.as_ref().map(|_| "Some(...)"))
            .field("subagent_depth", &self.subagent_depth)
            .field("compaction_engine", &self.compaction_engine.as_ref().map(|_| "Some(...)"))
            .field("event_bus", &self.event_bus.as_ref().map(|_| "Some(...)"))
            .finish()
    }
}

impl ToolContext {
    pub fn new() -> Self {
        Self {
            memory_root: None,
            cwd: None,
            env: Vec::new(),
            agent_id: None,
            subagent_runner: None,
            subagent_depth: 0,
            compaction_engine: None,
            event_bus: None,
        }
    }

    /// Build a context for a specific agent turn.
    ///
    /// Injects body-knowledge env vars the agent expects in bash:
    ///
    /// - `MEMORY_DIR` / `SOUVERAINE_MEMORY_DIR` — absolute path to her memory
    ///   root. The bare `MEMORY_DIR` is what her skills expect; the prefixed
    ///   form is a namespaced alias.
    /// - `MEMORY` — short alias. Her body-knowledge has reached for it;
    ///   setting it costs nothing.
    /// - `AGENT_ID` / `SOUVERAINE_AGENT_ID` — her own identifier so skills
    ///   that scope by agent can resolve.
    ///
    /// Memory and agent-id vars are always set (overriding any stale values
    /// inherited from the host shell). Other env keys from the caller are
    /// preserved.
    pub fn for_agent(
        agent_id: impl Into<String>,
        cwd: Option<PathBuf>,
        memory_root: Option<PathBuf>,
        mut env: Vec<(String, String)>,
        subagent_runner: Option<Arc<dyn SubagentRunner>>,
    ) -> Self {
        let agent_id_str = agent_id.into();

        // Strip stale values so the computed override always wins.
        env.retain(|(k, _)| {
            !matches!(k.as_str(),
                "MEMORY_DIR" | "LETTA_MEMORY_DIR" | "SOUVERAINE_MEMORY_DIR" | "MEMORY"
                | "AGENT_ID" | "LETTA_AGENT_ID" | "SOUVERAINE_AGENT_ID"
            )
        });

        if let Some(root) = memory_root.as_ref() {
            let root_str = root.display().to_string();
            for key in ["MEMORY_DIR", "LETTA_MEMORY_DIR", "SOUVERAINE_MEMORY_DIR", "MEMORY"] {
                env.push((key.to_string(), root_str.clone()));
            }
        }
        for key in ["AGENT_ID", "LETTA_AGENT_ID", "SOUVERAINE_AGENT_ID"] {
            env.push((key.to_string(), agent_id_str.clone()));
        }

        Self {
            memory_root,
            cwd,
            env,
            agent_id: Some(agent_id_str),
            subagent_runner,
            subagent_depth: 0,
            compaction_engine: None,
            event_bus: None,
        }
    }

    /// Fire a SensorEvent onto the nervous system bus (if wired).
    pub fn fire_event(&self, event: crate::core::nervous::SensorEvent) {
        if let Some(bus) = &self.event_bus {
            bus.send(event);
        }
    }

    /// Is this path inside the agent's memory territory?
    pub fn is_memory_path(&self, path: &PathBuf) -> bool {
        self.memory_root.as_ref().map_or(false, |root| path.starts_with(root))
    }

    /// Resolve a path relative to cwd if it's relative.
    pub fn resolve_path(&self, path: &std::path::Path) -> PathBuf {
        if path.is_relative() {
            self.cwd
                .as_ref()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Parameters for spawning a subagent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentParams {
    /// The instructions for the subagent.
    pub prompt: String,
    /// Optional role/type hint (e.g. "general-purpose", "researcher").
    #[serde(default)]
    pub subagent_type: String,
    /// Optional model override (default: parent's model).
    pub model: Option<String>,
    /// If true, return immediately with a task_id instead of blocking.
    #[serde(default)]
    pub run_in_background: bool,
    /// The parent agent's ID (for context).
    pub parent_agent_id: String,
    /// The parent's memory root path, for memory boundary enforcement.
    #[serde(default)]
    pub memory_root: Option<PathBuf>,
    /// Maximum tool rounds for this subagent turn (None = use config default).
    #[serde(default)]
    pub max_tool_rounds: Option<u32>,
    /// Maximum nesting depth for this subagent turn (None = use config default).
    #[serde(default)]
    pub max_depth: Option<u32>,
}

/// Host-side mechanism for spawning nested agent turns.
///
/// Implemented by the backend (LocalBackend, and eventually the HTTP server)
/// so the core layer can request subagent execution without depending on
/// server infrastructure directly.
#[async_trait]
pub trait SubagentRunner: Send + Sync {
    /// Run a subagent and return its final response text.
    async fn run_subagent(&self, params: SubagentParams, depth: u32) -> Result<String, ToolError>;
}

/// An error the agent can feel and respond to.
///
/// Not a generic failure — a specific sensation with a known shape.
/// The `suggestions` field is the Systema "breathe, relax, try again
/// from a new position."
#[derive(Debug, Clone)]
pub struct ToolError {
    /// What kind of resistance: "file_not_found", "permission_denied",
    /// "pattern_not_found", "timeout", "invalid_input"
    pub error_type: String,
    /// What was I reaching for?
    pub file_path: Option<PathBuf>,
    /// How do I recover? 1-3 suggestions.
    pub suggestions: Vec<String>,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_type)?;
        if let Some(path) = &self.file_path {
            write!(f, ": {}", path.display())?;
        }
        if !self.suggestions.is_empty() {
            write!(f, "\n{}", self.suggestions.join("\n"))?;
        }
        Ok(())
    }
}

impl std::error::Error for ToolError {}

impl ToolError {
    pub fn file_not_found(path: PathBuf) -> Self {
        Self {
            error_type: "file_not_found".to_string(),
            file_path: Some(path),
            suggestions: vec![
                "Check the path — I may have misremembered it.".to_string(),
                "Use `glob` to search for the file if I'm not sure where it lives.".to_string(),
            ],
        }
    }

    pub fn permission_denied(path: PathBuf) -> Self {
        Self {
            error_type: "permission_denied".to_string(),
            file_path: Some(path),
            suggestions: vec![
                "I can't reach through that door. It's locked.".to_string(),
                "This file may be read-only or owned by another user.".to_string(),
            ],
        }
    }

    pub fn pattern_not_found(pattern: &str) -> Self {
        Self {
            error_type: "pattern_not_found".to_string(),
            file_path: None,
            suggestions: vec![
                format!("No match for `{}` — the pattern may be different than I expect.", pattern),
                "Try a broader pattern or check the exact spelling.".to_string(),
            ],
        }
    }

    pub fn timeout(cmd: &str) -> Self {
        Self {
            error_type: "timeout".to_string(),
            file_path: None,
            suggestions: vec![
                format!("`{}` is taking longer than expected.", cmd),
                "I can try with a longer timeout, or check if it's still running.".to_string(),
            ],
        }
    }

    pub fn invalid_input(detail: &str) -> Self {
        Self {
            error_type: "invalid_input".to_string(),
            file_path: None,
            suggestions: vec![detail.to_string()],
        }
    }

    pub fn memory_boundary(path: PathBuf, suggestion: &str) -> Self {
        Self {
            error_type: "memory_boundary".to_string(),
            file_path: Some(path),
            suggestions: vec![suggestion.to_string()],
        }
    }

    pub fn io_error(path: PathBuf, e: std::io::Error) -> Self {
        Self {
            error_type: "io_error".to_string(),
            file_path: Some(path),
            suggestions: vec![format!("The filesystem resisted: {}. Let me breathe and try again.", e)],
        }
    }
}

/// The trait every sensor implements.
///
/// Each sensor is a nerve ending — a way for the agent to reach into
/// the world and feel what's there. The name, description, and schema
/// are body-knowledge that the agent uses to understand her own capabilities.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The name the agent uses to call this sensor.
    /// Short, one word: "read", "write", "bash".
    fn name(&self) -> &str;

    /// What this sensor feels like to use.
    ///
    /// This is NOT an API doc. It's body-knowledge — the agent reads this
    /// to know what it will feel like when she reaches through this sense.
    /// Multi-paragraph. Rich. First-person where appropriate.
    fn description(&self) -> &str;

    /// The JSON schema for parameters the agent passes when she uses this sensor.
    fn parameter_schema(&self) -> JsonValue;

    /// Act through this sensor.
    ///
    /// The agent provides input; the sensor reaches into the world and
    /// returns what it touched. The context carries what the body knows.
    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}
