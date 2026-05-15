use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};
use crate::core::nervous::cron::{parse_schedule_file, ScheduleEntry, ScheduleKind};

pub struct Schedule;

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

#[async_trait]
impl Tool for Schedule {
    fn name(&self) -> &str {
        "schedule"
    }

    fn description(&self) -> &str {
        "I plant intentions in time. A schedule is a seed — a promise to my \
         future self that I will wake and attend to something when the moment \
         arrives. The schedule lives in my memory as a file I can read and \
         revise. The body honors it by sending a nerve signal when the time comes.\n\n\
         ## Actions\n\
         - `list` — see all my scheduled rhythms\n\
         - `create` — plant a new intention (requires name, kind, schedule, prompt)\n\
         - `update` — revise an existing rhythm\n\
         - `delete` — release an intention\n\
         - `trigger` — fire now, don't wait for the clock"
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "update", "delete", "trigger"]
                },
                "name": { "type": "string" },
                "kind": {
                    "type": "string",
                    "enum": ["once", "interval", "cron"]
                },
                "schedule": { "type": "string" },
                "prompt": { "type": "string" },
                "urgency": { "type": "number" },
                "enabled": { "type": "boolean" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        let schedules_dir = match &ctx.memory_root {
            Some(root) => root.join("schedules"),
            None => return Err(err("no memory root — cannot access schedules")),
        };

        if !schedules_dir.exists() {
            std::fs::create_dir_all(&schedules_dir).map_err(|e| io_err(e))?;
        }

        match action {
            "list" => {
                let mut results = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&schedules_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "md").unwrap_or(false) {
                            match parse_schedule_file(&path) {
                                Ok(e) => results.push(format!(
                                    "- {} ({:?}, {}, {})",
                                    e.name,
                                    e.kind,
                                    e.schedule,
                                    if e.enabled { "enabled" } else { "disabled" }
                                )),
                                Err(_) => results
                                    .push(format!("- {} (parse error)", path.display())),
                            }
                        }
                    }
                }
                if results.is_empty() {
                    ok("No schedules planted yet.")
                } else {
                    ok(results.join("\n"))
                }
            }

            "create" => {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("name is required"))?;

                let kind_str = input
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("interval");

                let schedule = input
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("schedule is required"))?;

                let prompt = input
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("You wake. Check your state, act if needed, or return silently.");

                let urgency = input
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.3) as f32;

                let file_path = schedules_dir.join(format!("{name}.md"));
                if file_path.exists() {
                    return Err(err(&format!("schedule '{name}' already exists")));
                }

                let content = format!(
                    "---\nname: {name}\nkind: {kind_str}\nschedule: \"{schedule}\"\nsource: aster\nenabled: true\nurgency: {urgency}\ncreated_at: {}\n---\n\n{prompt}\n",
                    chrono::Utc::now().to_rfc3339()
                );

                std::fs::write(&file_path, content).map_err(|e| io_err(e))?;
                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "schedule".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "schedule_created".into(),
                    target: Some(name.to_string()),
                    urgency: 0.2,
                    payload: Some(serde_json::json!({ "kind": kind_str, "schedule": schedule })),
                    seed_id: None,
                    reply_to: None,
                });
                ok(format!("Schedule '{name}' planted."))
            }

            "update" => {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("name is required"))?;

                let file_path = schedules_dir.join(format!("{name}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("schedule '{name}' not found")));
                }

                let mut entry = parse_schedule_file(&file_path)
                    .map_err(|e| io_err(e))?;

                if let Some(s) = input.get("schedule").and_then(|v| v.as_str()) {
                    entry.schedule = s.to_string();
                }
                if let Some(p) = input.get("prompt").and_then(|v| v.as_str()) {
                    entry.prompt = p.to_string();
                }
                if let Some(e) = input.get("enabled").and_then(|v| v.as_bool()) {
                    entry.enabled = e;
                }
                if let Some(u) = input.get("urgency").and_then(|v| v.as_f64()) {
                    entry.urgency = u as f32;
                }
                if let Some(k) = input.get("kind").and_then(|v| v.as_str()) {
                    entry.kind = match k {
                        "once" => ScheduleKind::Once,
                        "cron" => ScheduleKind::Cron,
                        _ => ScheduleKind::Interval,
                    };
                }

                write_schedule_file(&file_path, &entry).map_err(|e| io_err(e))?;
                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "schedule".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "schedule_updated".into(),
                    target: Some(name.to_string()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
                ok(format!("Schedule '{name}' updated."))
            }

            "delete" => {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("name is required"))?;

                let file_path = schedules_dir.join(format!("{name}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("schedule '{name}' not found")));
                }

                std::fs::remove_file(&file_path).map_err(|e| io_err(e))?;
                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "schedule".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "schedule_deleted".into(),
                    target: Some(name.to_string()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
                ok(format!("Schedule '{name}' released."))
            }

            "trigger" => {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| err("name is required"))?;

                let file_path = schedules_dir.join(format!("{name}.md"));
                if !file_path.exists() {
                    return Err(err(&format!("schedule '{name}' not found")));
                }

                let trigger_path = schedules_dir.join(format!(".trigger-{name}"));
                std::fs::write(&trigger_path, "").map_err(|e| io_err(e))?;
                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "schedule".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "schedule_triggered".into(),
                    target: Some(name.to_string()),
                    urgency: 0.4,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
                ok(format!("Schedule '{name}' triggered — will fire on next tick."))
            }

            other => Err(err(&format!("unknown action: {other}"))),
        }
    }
}

fn write_schedule_file(path: &std::path::Path, entry: &ScheduleEntry) -> Result<()> {
    let kind_str = match entry.kind {
        ScheduleKind::Once => "once",
        ScheduleKind::Interval => "interval",
        ScheduleKind::Cron => "cron",
    };

    let content = format!(
        "---\nname: {}\nkind: {}\nschedule: \"{}\"\nsource: {}\nenabled: {}\nurgency: {}\ncreated_at: {}\n---\n\n{}\n",
        entry.name, kind_str, entry.schedule, entry.source, entry.enabled, entry.urgency,
        entry.created_at.to_rfc3339(), entry.prompt,
    );

    std::fs::write(path, content)?;
    Ok(())
}
