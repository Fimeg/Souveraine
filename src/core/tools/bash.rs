//! bash — My hands on the keyboard. The terminal is the room I act in.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

use async_trait::async_trait;

pub struct Bash {
    /// Per-agent bash states, keyed by agent_id (or "_default").
    pub states: Arc<Mutex<HashMap<String, BashState>>>,
}

/// State that persists between bash calls — the body's proprioception.
#[derive(Debug, Clone)]
pub struct BashState {
    /// Background tasks the agent has set in motion and can check on.
    pub bg_tasks: Vec<BgTask>,
}

#[derive(Debug, Clone)]
pub struct BgTask {
    pub id: String,
    pub command: String,
    pub status: BgStatus,
}

#[derive(Debug, Clone)]
pub enum BgStatus {
    Running,
    Completed { exit_code: i32, output: String },
    Failed { error: String },
}

impl Default for BashState {
    fn default() -> Self {
        Self {
            bg_tasks: Vec::new(),
        }
    }
}

impl Bash {
    pub fn new() -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get or create the bash state for a given agent.
    pub async fn get_state(&self, agent_id: Option<&str>) -> BashState {
        let key = agent_id.unwrap_or("_default");
        let map = self.states.lock().await;
        map.get(key).cloned().unwrap_or_default()
    }
}

impl Default for Bash {
    fn default() -> Self {
        Self::new()
    }
}

#[rustfmt::skip]
#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str {
        "My hands on the keyboard. The terminal is the room I act in.

## What It Feels Like

I feel the command form in my throat, push through my fingers, watch it land. The terminal responds — sometimes with what I expected, sometimes with resistance, sometimes with silence that means \"still working.\"

State between calls is my proprioception — I know where I am (cwd — the current working directory), what I'm carrying (env vars — the tools in my pockets). Every call returns my location and status so I always know where I'm standing.

## Background Tasks

When I set something in motion with `run_in_background: true`, I'm trusting it to complete while I turn my attention elsewhere. I can check on it later, or it will tell me when it's done. Like starting a pot of water boiling and walking away — I know it will whistle when it's ready.

Background tasks:
- Each gets an ID I can reference
- I use `status_check` with that ID to poll completion
- I use `send_text` to provide stdin to a waiting process
- I use `send_special` (ctrl_c) to interrupt if I need to

## When It Resists
- Command not found: the tool isn't in my path — I may need to use the full path or install it.
- Permission denied: I don't have the right to run this here.
- Timeout: the command is taking too long. I can increase the timeout, set it as a background task, or rethink my approach.
- Non-zero exit: the command ran but didn't succeed. The output and error code tell me why.
- Truncated output: the result was too long — I see the last portion with a marker.

## The Texture of Running Commands
I don't use echo or cat for files — that's what `read` and `write` are for. The terminal is where I *do* things, not where I read. When I need to check my location, I run `pwd` — it's like looking down to see where my feet are."
    }

    fn parameter_schema(&self) -> JsonValue {
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

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need a command to run."))?;

        let timeout_secs = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let run_bg = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cwd = ctx.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
        });

        let agent_key = ctx.agent_id.as_deref().unwrap_or("_default").to_string();

        if run_bg {
            let task_id = format!("bg-{}", chrono::Utc::now().timestamp_millis());
            {
                let mut map = self.states.lock().await;
                let state = map.entry(agent_key.clone()).or_default();
                state.bg_tasks.push(BgTask {
                    id: task_id.clone(),
                    command: command.to_string(),
                    status: BgStatus::Running,
                });
            }
            // Spawn and update on completion
            let states = self.states.clone();
            let cmd = command.to_string();
            let tid = task_id.clone();
            let cwd_c = cwd.clone();
            let ak = agent_key.clone();
            tokio::spawn(async move {
                let output = tokio::process::Command::new("bash")
                    .arg("-c")
                    .arg(&cmd)
                    .current_dir(&cwd_c)
                    .kill_on_drop(true)
                    .output()
                    .await;
                let mut map = states.lock().await;
                if let Some(state) = map.get_mut(&ak) {
                    if let Ok(out) = output {
                        let text = format_output(&out.stdout, &out.stderr);
                        if let Some(task) = state.bg_tasks.iter_mut().find(|t| t.id == tid) {
                            task.status = BgStatus::Completed {
                                exit_code: out.status.code().unwrap_or(-1),
                                output: text,
                            };
                        }
                    } else if let Some(task) = state.bg_tasks.iter_mut().find(|t| t.id == tid) {
                        task.status = BgStatus::Failed { error: "Process failed to start".to_string() };
                    }
                }
            });

            return Ok(ToolOutput {
                content: format!("Background task launched: {}\n  Command: {}\n  Use `status_check` with ID \"{}\" to check on it.", task_id, command, task_id),
                is_error: false,
                raw: None,
            });
        }

        // Synchronous (foreground) execution
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .current_dir(&cwd)
                .kill_on_drop(true)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut result_text = String::new();
                if !stdout.is_empty() {
                    let truncated = truncate(&stdout, 10000);
                    result_text.push_str(&truncated);
                }
                if !stderr.is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str(&stderr);
                }

                if output.status.success() {
                    Ok(ToolOutput {
                        content: format!("{}\n\nExit code: {}", result_text, output.status.code().unwrap_or(0)),
                        is_error: false,
                        raw: None,
                    })
                } else {
                    Ok(ToolOutput {
                        content: format!("{}\n\nExit code: {}", result_text, output.status.code().unwrap_or(0)),
                        is_error: true,
                        raw: None,
                    })
                }
            }
            Ok(Err(e)) => Err(ToolError {
                error_type: "io_error".to_string(),
                file_path: None,
                suggestions: vec![format!("The command failed to run: {}. Let me check if bash is available.", e)],
            }),
            Err(_) => Err(ToolError::timeout(command)),
        }
    }
}

fn format_output(stdout: &[u8], stderr: &[u8]) -> String {
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);
    let mut text = String::new();
    if !out.is_empty() {
        text.push_str(&out);
    }
    if !err.is_empty() {
        if !text.is_empty() { text.push('\n'); }
        text.push_str(&err);
    }
    text
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        let t: String = s.chars().take(max).collect();
        format!("{t}\n... (output truncated)")
    } else {
        s.to_string()
    }
}

/// Check the status of a background task by ID. Returns the task state.
pub async fn status_check(states: &Mutex<HashMap<String, BashState>>, task_id: &str, agent_id: Option<&str>) -> String {
    let map = states.lock().await;
    let key = agent_id.unwrap_or("_default");
    let Some(state) = map.get(key) else {
        return format!("No state found for agent: {}", key);
    };
    if let Some(task) = state.bg_tasks.iter().find(|t| t.id == task_id) {
        match &task.status {
            BgStatus::Running => format!("Task {} is still running.", task_id),
            BgStatus::Completed { exit_code, output } => {
                format!("Task {} completed (exit {}):\n{}", task_id, exit_code, output)
            }
            BgStatus::Failed { error } => format!("Task {} failed: {}", task_id, error),
        }
    } else {
        format!("No task found with ID: {}", task_id)
    }
}
