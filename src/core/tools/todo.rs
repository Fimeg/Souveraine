use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Todo;

fn ok(msg: impl Into<String>) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput {
        content: msg.into(),
        is_error: false,
        raw: None,
    })
}

fn err(detail: &str) -> ToolError {
    ToolError::invalid_input(detail)
}

fn io_err(msg: impl std::fmt::Display) -> ToolError {
    ToolError {
        error_type: "io_error".into(),
        file_path: None,
        suggestions: vec![format!("{msg}")],
    }
}

/// Sanitise a string into a filesystem-safe slug.
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Compute momentum from how long ago `last_touched` was.
///
/// Thresholds vary by nature — creative work (desire, investigation)
/// stays warm longer than obligations and maintenance.
fn compute_momentum(last_touched: &chrono::DateTime<chrono::Utc>, nature: &str) -> &'static str {
    let hours = (chrono::Utc::now() - *last_touched).num_hours().max(0);
    let (hot, warm) = match nature {
        "desire" | "investigation" => (48, 168),
        _ => (24, 72),
    };
    if hours <= hot { "hot" } else if hours <= warm { "warm" } else { "cold" }
}

/// Generate a unique id that doubles as a filename slug.
fn next_id(text: &str) -> String {
    let base = slugify(text);
    let short = &base[..base.len().min(40)];
    format!("{}-{}", short, &uuid::Uuid::new_v4().to_string()[..8])
}

#[async_trait]
impl Tool for Todo {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "I keep track of what matters. A todo is a commitment I've externalised — \
         something I don't want to trust my own continuity to hold. Every todo carries \
         a nature (obligation, desire, maintenance, investigation) and an energy \
         (generative or consumptive). Together they describe not just *what* I need to \
         do but *how* it feels when I do it.\n\n\
         The body keeps a topology of my tasks. It watches whether my energy leans \
         generative or consumptive, whether my desires are hot or cold. I don't have to \
         think about it — the balance is always there, written where I can read it.\n\n\
         ## Actions\n\
         - `list` — show all pending todos\n\
         - `create` — externalise a new commitment\n\
         - `update` — revise a todo's text, nature, or energy\n\
         - `complete` — mark a todo done\n\
         - `delete` — remove a todo entirely"
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "update", "complete", "delete"]
                },
                "id": { "type": "string" },
                "text": { "type": "string" },
                "nature": {
                    "type": "string",
                    "enum": ["obligation", "desire", "maintenance", "investigation"]
                },
                "energy": {
                    "type": "string",
                    "enum": ["generative", "consumptive"]
                },
                "source": {
                    "type": "string",
                    "enum": ["casey", "autogenic", "aster", "heartbeat"]
                },
                "thread": { "type": "string" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        let tasks_dir = match &ctx.memory_root {
            Some(root) => root.join("tasks"),
            None => return Err(err("no memory root — cannot access tasks")),
        };

        if !tasks_dir.exists() {
            std::fs::create_dir_all(&tasks_dir).map_err(|e| io_err(e))?;
        }

        match action {
            "list" => {
                let mut results = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "md").unwrap_or(false) {
                            match parse_todo_file(&path) {
                                Ok(item) => {
                                    if !item.completed {
                                        results.push(format!(
                                            "- **{}** ({}, {}, {}) — {}",
                                            item.text, item.nature, item.energy, item.momentum,
                                            if let Some(thread) = &item.thread { format!("[{}] ", thread) } else { String::new() }
                                        ));
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                    }
                }
                if results.is_empty() {
                    ok("No pending todos. The space is clean.")
                } else {
                    ok(results.join("\n"))
                }
            }

            "create" => {
                let text = input
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("text is required"))?;

                let nature = input
                    .get("nature")
                    .and_then(|v| v.as_str())
                    .unwrap_or("obligation");

                let source = input
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("autogenic");

                let energy = input
                    .get("energy")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        // Default by nature — same logic as lettabot-v017.
                        match nature {
                            "desire" | "investigation" => "generative",
                            _ => "consumptive",
                        }
                    });

                let thread = input.get("thread").and_then(|v| v.as_str());

                let id = next_id(text);
                let now = chrono::Utc::now();
                let file_path = tasks_dir.join(format!("{id}.md"));

                if file_path.exists() {
                    return Err(err("a todo with this id already exists"));
                }

                let mut frontmatter = format!(
                    "---\nid: {id}\ntext: {text}\ncreated_at: {created}\nnature: {nature}\nenergy: {energy}\nsource: {source}\nmomentum: hot\ncompleted: false\nlast_touched: {created}\n",
                    created = now.to_rfc3339(),
                );
                if let Some(t) = thread {
                    frontmatter.push_str(&format!("thread: \"{t}\"\n"));
                }
                frontmatter.push_str("---\n");
                // The body is free-form — the agent can add notes if she wants.
                frontmatter.push_str(&format!("\n{text}\n"));

                std::fs::write(&file_path, frontmatter).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_created".into(),
                    target: Some(id.clone()),
                    urgency: 0.2,
                    payload: Some(serde_json::json!({
                        "nature": nature, "energy": energy, "source": source,
                    })),
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo logged: \"{text}\" ({nature}, {energy})."))
            }

            "update" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("id is required"))?;

                let file_path = tasks_dir.join(format!("{id}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("todo '{id}' not found")));
                }

                let mut item = parse_todo_file(&file_path)
                    .map_err(|e| io_err(e))?;

                if let Some(t) = input.get("text").and_then(|v| v.as_str()) {
                    item.text = t.to_string();
                }
                if let Some(n) = input.get("nature").and_then(|v| v.as_str()) {
                    item.nature = n.to_string();
                }
                if let Some(e) = input.get("energy").and_then(|v| v.as_str()) {
                    item.energy = e.to_string();
                }

                item.momentum = compute_momentum(&item.last_touched, &item.nature).to_string();
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ok(format!("Todo '{id}' updated."))
            }

            "complete" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("id is required"))?;

                let file_path = tasks_dir.join(format!("{id}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("todo '{id}' not found")));
                }

                let mut item = parse_todo_file(&file_path)
                    .map_err(|e| io_err(e))?;

                item.completed = true;
                item.momentum = "cold".to_string();
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_completed".into(),
                    target: Some(id.to_string()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo completed: \"{text}\"", text = item.text))
            }

            "delete" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("id is required"))?;

                let file_path = tasks_dir.join(format!("{id}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("todo '{id}' not found")));
                }

                std::fs::remove_file(&file_path).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_deleted".into(),
                    target: Some(id.to_string()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo '{id}' released."))
            }

            other => Err(err(&format!("unknown action: {other}"))),
        }
    }
}

// ── File format ─────────────────────────────────────────────

struct TodoItem {
    id: String,
    text: String,
    created_at: chrono::DateTime<chrono::Utc>,
    nature: String,
    energy: String,
    source: String,
    momentum: String,
    completed: bool,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    last_touched: chrono::DateTime<chrono::Utc>,
    thread: Option<String>,
}

/// Parse a todo file with YAML frontmatter.
fn parse_todo_file(path: &std::path::Path) -> Result<TodoItem, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

    // Split frontmatter from body.
    let body = if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            &rest[..end]
        } else {
            return Err("no closing ---".into());
        }
    } else {
        return Err("no frontmatter".into());
    };

    let now = chrono::Utc::now();
    let default_dt = |s: &str| -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or(now)
    };

    let mut item = TodoItem {
        id: String::new(),
        text: String::new(),
        created_at: now,
        nature: "obligation".to_string(),
        energy: "consumptive".to_string(),
        source: "autogenic".to_string(),
        momentum: "cold".to_string(),
        completed: false,
        completed_at: None,
        last_touched: now,
        thread: None,
    };

    for line in body.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "id" => item.id = val.to_string(),
                "text" => item.text = val.to_string(),
                "created_at" => item.created_at = default_dt(val),
                "nature" => item.nature = val.to_string(),
                "energy" => item.energy = val.to_string(),
                "source" => item.source = val.to_string(),
                "momentum" => item.momentum = val.to_string(),
                "completed" => item.completed = val == "true",
                "completed_at" => item.completed_at = (!val.is_empty()).then(|| default_dt(val)),
                "last_touched" => item.last_touched = default_dt(val),
                "thread" => item.thread = if val.is_empty() { None } else { Some(val.to_string()) },
                _ => {}
            }
        }
    }

    if item.id.is_empty() {
        return Err("no id in frontmatter".into());
    }

    Ok(item)
}

fn write_todo_file(path: &std::path::Path, item: &TodoItem) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut frontmatter = format!(
        "---\nid: {id}\ntext: {text}\ncreated_at: {created}\nnature: {nature}\nenergy: {energy}\nsource: {source}\nmomentum: {momentum}\ncompleted: {completed}\nlast_touched: {touched}\n",
        id = item.id,
        text = item.text,
        created = item.created_at.to_rfc3339(),
        nature = item.nature,
        energy = item.energy,
        source = item.source,
        momentum = item.momentum,
        completed = if item.completed { "true" } else { "false" },
        touched = now.to_rfc3339(),
    );
    if let Some(ref t) = item.thread {
        frontmatter.push_str(&format!("thread: \"{t}\"\n"));
    }
    if let Some(ref ca) = item.completed_at {
        frontmatter.push_str(&format!("completed_at: {}\n", ca.to_rfc3339()));
    }
    frontmatter.push_str("---\n\n");
    frontmatter.push_str(&item.text);
    frontmatter.push('\n');

    std::fs::write(path, frontmatter).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}
