//! Itinerary — the agent's route through the current work.
//!
//! An itinerary is an ordered sequence of stops. Each stop optionally
//! references a todo commitment by its ID. Stops that reference a todo
//! inherit the todo's nature, energy, and thread; stops without a
//! reference are ephemeral waypoints that leave no trace in the task
//! system.
//!
//! The itinerary is persisted to `system/dynamic/itinerary.md` in the
//! agent's memfs — the same volatile workspace that holds energy balance,
//! context pressure, and other running-state files. It is not a commitment
//! store; it is the *active face* of whatever the agent is doing right now.
//!
//! ## Agent actions
//!
//! - `set`     — lay out a new route (replaces current)
//! - `advance` — mark current stop done, move to next
//! - `describe` — read back the current itinerary
//! - `clear`   — dismiss the itinerary entirely

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

// ── Data types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StopStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "current")]
    Current,
    #[serde(rename = "done")]
    Done,
}

impl StopStatus {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Pending | Self::Current)
    }
}

/// A single stop on the itinerary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stop {
    /// Short label shown in the header strip.
    pub name: String,

    /// Optional longer description (for `describe` output).
    pub description: Option<String>,

    /// If set, this stop links back to a todo commitment by its id.
    /// The tool reads the todo's nature/energy from the file and surfaces
    /// them to the agent and UI.
    pub todo_id: Option<String>,

    pub status: StopStatus,

    /// Derived from the referenced todo, if any. Populated at read time.
    #[serde(skip)]
    pub nature: Option<String>,

    /// Derived from the referenced todo, if any.
    #[serde(skip)]
    pub energy: Option<String>,
}

/// The full itinerary, stored as YAML frontmatter in a markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Itinerary {
    /// A short title for the whole route.
    pub title: String,

    /// Ordered stops.
    pub stops: Vec<Stop>,

    /// Index of the current stop (0-based). Persisted so resume works.
    pub current: usize,
}

impl Itinerary {
    pub fn is_active(&self) -> bool {
        self.stops.iter().any(|s| s.status.is_live())
    }

    /// Return a compact one-line representation of the route for the
    /// description output and the header strip.
    pub fn route_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for stop in self.stops.iter() {
            let glyph = match stop.status {
                StopStatus::Done => "✓",
                StopStatus::Current => "●",
                StopStatus::Pending => "○",
            };
            let mut label = format!("{} {}", glyph, stop.name);
            if let Some(ref e) = stop.energy {
                let icon = match e.as_str() {
                    "generative" => "⚡",
                    "consumptive" => "—",
                    _ => "",
                };
                if let Some(ref n) = stop.nature {
                    let nature_glyph = match n.as_str() {
                        "desire" => "♪",
                        "investigation" => "◇",
                        "obligation" => "▤",
                        "maintenance" => "△",
                        _ => "·",
                    };
                    label = format!("{} {} {}", label, nature_glyph, icon);
                } else {
                    label = format!("{} {}", label, icon);
                }
            }
            parts.push(label);
        }
        format!("{}  ·  {}", self.title, parts.join("  "))
    }

    /// Summarise for the agent's describe action.
    pub fn describe(&self) -> String {
        let mut out = format!("## {}\n", self.title);
        for (i, stop) in self.stops.iter().enumerate() {
            let marker = match stop.status {
                StopStatus::Done => "✓",
                StopStatus::Current => "●",
                StopStatus::Pending => "○",
            };
            let nature = stop.nature.as_deref().unwrap_or("");
            let energy = stop.energy.as_deref().unwrap_or("");
            let todo_ref = stop
                .todo_id
                .as_ref()
                .map(|id| format!("  (todo: `{}`)", id))
                .unwrap_or_default();
            let desc = stop
                .description
                .as_deref()
                .map(|d| format!(" — {}", d))
                .unwrap_or_default();
            out.push_str(&format!(
                "\n{marker}  **{}**{desc}{todo_ref}",
                stop.name
            ));
            if !nature.is_empty() || !energy.is_empty() {
                out.push_str(&format!(" [{}{}]", nature, energy));
            }
        }
        out.push_str(&format!(
            "\n\nStep {} of {}",
            self.current + 1,
            self.stops.len()
        ));
        out
    }
}

// ── Helpers ───────────────────────────────────────────────────────

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

/// Path to the itinerary file inside `system/dynamic/`.
fn itin_path(dynamic_dir: &Path) -> PathBuf {
    dynamic_dir.join("itinerary.md")
}

/// Read the current itinerary from disk.
/// `dynamic_dir` = memory_root / system / dynamic
pub fn load(dynamic_dir: &Path) -> Option<Itinerary> {
    let path = itin_path(dynamic_dir);
    let content = std::fs::read_to_string(&path).ok()?;

    // tasks/ lives at memory_root/tasks/ = dynamic_dir/../tasks/
    let tasks_dir = dynamic_dir.parent().and_then(|p| {
        let t = p.join("tasks");
        if t.exists() { Some(t) } else { None }
    });

    let body = if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            &rest[..end]
        } else {
            return None;
        }
    } else {
        return None;
    };

    let mut ity: Itinerary = serde_yaml::from_str(body).ok()?;
    enrich_from_todos(&mut ity, tasks_dir.as_deref());
    Some(ity)
}

/// Write the itinerary to `dynamic_dir/itinerary.md`.
fn save(dynamic_dir: &Path, ity: &Itinerary) -> Result<(), String> {
    std::fs::create_dir_all(dynamic_dir)
        .map_err(|e| format!("cannot create {}: {e}", dynamic_dir.display()))?;

    let yaml = serde_yaml::to_string(&ity).map_err(|e| format!("serialize: {e}"))?;
    let content = format!("---\n{}---\n\n# Itinerary\n{}", yaml, ity.title);
    std::fs::write(dynamic_dir.join("itinerary.md"), &content)
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}

/// Populate `nature` and `energy` on each stop that references a todo.
fn enrich_from_todos(ity: &mut Itinerary, tasks_dir: Option<&Path>) {
    let Some(dir) = tasks_dir else { return };

    for stop in &mut ity.stops {
        let Some(ref tid) = stop.todo_id else { continue };
        let path = dir.join(format!("{}.md", tid));
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        let Some(body) = content.strip_prefix("---\n") else { continue };
        let Some(end) = body.find("\n---\n") else { continue };
        let fm = &body[..end];
        for line in fm.lines() {
            let Some((key, val)) = line.split_once(':') else { continue };
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "nature" => stop.nature = Some(val.to_string()),
                "energy" => stop.energy = Some(val.to_string()),
                _ => {}
            }
        }
    }
}

fn emit_event(ctx: &ToolContext, ity: &Itinerary) {
    ctx.fire_event(crate::core::nervous::SensorEvent {
        sensor_name: "itinerary".into(),
        timestamp: chrono::Utc::now(),
        event_type: "itinerary_changed".into(),
        target: Some(ity.title.clone()),
        urgency: 0.15,
        payload: Some(serde_json::json!({
            "current": ity.current,
            "stops": ity.stops.len(),
        })),
        seed_id: None,
        reply_to: None,
    });
}

/// Complete a linked todo file on disk.
fn complete_todo(tasks_dir: &Path, todo_id: &str) {
    let path = tasks_dir.join(format!("{todo_id}.md"));
    let Ok(content) = std::fs::read_to_string(&path) else { return };
    let updated = content
        .replace("status: pending", "status: done")
        .replace("status: in_progress", "status: done");
    let now = chrono::Utc::now().to_rfc3339();
    let updated = if !updated.contains("completed_at:") {
        updated.replace(
            "status: done",
            &format!("status: done\ncompleted_at: {now}"),
        )
    } else {
        updated
    };
    let _ = std::fs::write(&path, &updated);
}

// ── Tool implementation ───────────────────────────────────────────

pub struct ItineraryTool;

#[async_trait]
impl Tool for ItineraryTool {
    fn name(&self) -> &str {
        "itinerary"
    }

    fn description(&self) -> &str {
        "I lay out the route ahead of me. An itinerary is a sequence of stops — the steps \
         I plan to take through the current work. Each stop can be a free waypoint or can \
         link back to a todo commitment; when it links to a todo, its nature and energy \
         travel with it.\n\n\
         The itinerary is not my task list — it's the *active face* of whatever I'm doing \
         right now. It lives in the strip at the top of the conversation so we both know \
         where I am.\n\n\
         ## Actions\n\
         - `set` — lay out a new route. Takes a `title` and a list of `stops`. Pass \
           `todo_id` to link a stop to an existing commitment.\n\
         - `advance` — mark the current stop done and move to the next one. Optionally \
           pass `todo_id` to auto-complete a linked todo; pass `phase` to set the next \
           stop's phase marker.\n\
         - `describe` — read back the whole route with status and details.\n\
         - `clear` — dismiss the itinerary. Does not touch linked todos."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "advance", "describe", "clear"]
                },
                "title": {
                    "type": "string",
                    "description": "Title for the whole route (required for `set`)."
                },
                "stops": {
                    "type": "array",
                    "description": "Ordered stops for `set`. Each stop is an object with `name`, \
                                    optional `description`, and optional `todo_id`.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Short label." },
                            "description": { "type": "string", "description": "Optional longer description." },
                            "todo_id": { "type": "string", "description": "Optional — link to an existing todo commitment." }
                        },
                        "required": ["name"]
                    }
                },
                "todo_id": {
                    "type": "string",
                    "description": "When advancing, optionally complete this linked todo."
                },
                "phase": {
                    "type": "string",
                    "description": "Phase marker for the next stop after advancing (e.g. '3/6')."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("describe");

        let memory_root = ctx
            .memory_root
            .clone()
            .ok_or_else(|| err("no memory root — I can't access the agent's memory"))?;

        let dynamic_dir = memory_root.join("system").join("dynamic");
        let tasks_dir = memory_root.join("tasks");
        std::fs::create_dir_all(&dynamic_dir).map_err(|e| {
            err(&format!("could not create system/dynamic/: {e}"))
        })?;

        match action {
            "set" => cmd_set(&input, &dynamic_dir, &tasks_dir, ctx),
            "advance" => cmd_advance(&input, &dynamic_dir, &tasks_dir, ctx),
            "describe" => cmd_describe(&dynamic_dir),
            "clear" => cmd_clear(&dynamic_dir, ctx),
            other => Err(err(&format!("unknown action: {other}"))),
        }
    }
}

// ── Command implementations (free functions for testability) ─────

fn cmd_set(
    input: &JsonValue,
    dynamic_dir: &Path,
    tasks_dir: &Path,
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let title = input
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err("title is required"))?;

    let stops_raw = input
        .get("stops")
        .and_then(|v| v.as_array())
        .ok_or_else(|| err("stops array is required"))?;

    let stops: Vec<Stop> = stops_raw
        .iter()
        .map(|s| {
            let name = s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed")
                .to_string();
            let description = s.get("description").and_then(|v| v.as_str()).map(String::from);
            let todo_id = s.get("todo_id").and_then(|v| v.as_str()).map(String::from);
            Stop {
                name,
                description,
                todo_id,
                status: StopStatus::Pending,
                nature: None,
                energy: None,
            }
        })
        .collect();

    if stops.is_empty() {
        return Err(err("at least one stop is required"));
    }

    let mut ity = Itinerary {
        title: title.to_string(),
        current: 0,
        stops,
    };

    // Mark the first stop as current.
    if let Some(first) = ity.stops.first_mut() {
        first.status = StopStatus::Current;
    }

    enrich_from_todos(&mut ity, Some(tasks_dir));
    save(dynamic_dir, &ity).map_err(|e| err(&e))?;
    emit_event(ctx, &ity);

    let first = &ity.stops[0];
    let mut line = format!(
        "Route set: {} — {} stops.\n●  {}",
        ity.title,
        ity.stops.len(),
        first.name,
    );
    if let Some(ref desc) = first.description {
        line.push_str(&format!(" — {desc}"));
    }
    if let Some(ref e) = first.energy {
        line.push_str(&format!("  [{e}]"));
    }
    Ok(ToolOutput { content: line, is_error: false, raw: None })
}

fn cmd_advance(
    input: &JsonValue,
    dynamic_dir: &Path,
    tasks_dir: &Path,
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let mut ity = load(dynamic_dir).ok_or_else(|| err("no active itinerary — use `set` first"))?;

    let current = ity.current;
    if current >= ity.stops.len() {
        return Err(err("all stops already done — use `set` for a new route"));
    }

    // Mark the current stop done and optionally complete its linked todo.
    if let Some(stop) = ity.stops.get_mut(current) {
        stop.status = StopStatus::Done;

        // Complete the stop's own linked todo (if any).
        if let Some(ref tid) = stop.todo_id {
            complete_todo(tasks_dir, tid);
        }

        // Also complete a todo_id passed explicitly (may differ from stop's).
        if let Some(ref tid) = input.get("todo_id").and_then(|v| v.as_str()) {
            complete_todo(tasks_dir, tid);
        }
    }

    let next = current + 1;
    if next < ity.stops.len() {
        if let Some(stop) = ity.stops.get_mut(next) {
            stop.status = StopStatus::Current;

            if let Some(p) = input.get("phase").and_then(|v| v.as_str()) {
                stop.description = Some(format!("phase: {p}"));
            }
        }
        ity.current = next;
    } else {
        // No more stops — itinerary complete.
        ity.current = ity.stops.len();
    }

    enrich_from_todos(&mut ity, Some(tasks_dir));
    save(dynamic_dir, &ity).map_err(|e| err(&e))?;
    emit_event(ctx, &ity);

    let mut line = if next < ity.stops.len() {
        let s = &ity.stops[next];
        format!(
            "Advanced — now on step {} of {}.\n●  {}",
            next + 1,
            ity.stops.len(),
            s.name,
        )
    } else {
        format!("Route complete! All {} stops done.", ity.stops.len())
    };
    if next < ity.stops.len() {
        if let Some(ref desc) = ity.stops[next].description {
            line.push_str(&format!(" — {desc}"));
        }
        if let Some(ref e) = ity.stops[next].energy {
            line.push_str(&format!("  [{e}]"));
        }
    }

    Ok(ToolOutput { content: line, is_error: false, raw: None })
}

fn cmd_describe(dynamic_dir: &Path) -> Result<ToolOutput, ToolError> {
    match load(dynamic_dir) {
        Some(ity) => {
            let mut out = ity.describe();
            let linked: Vec<&Stop> = ity.stops.iter().filter(|s| s.todo_id.is_some()).collect();
            if !linked.is_empty() {
                out.push_str("\n\n---\nLinked commitments:\n");
                for stop in &linked {
                    let n = stop.nature.as_deref().unwrap_or("—");
                    let e = stop.energy.as_deref().unwrap_or("—");
                    out.push_str(&format!(
                        "- `{}`  {}  ·  {n} / {e}\n",
                        stop.todo_id.as_deref().unwrap_or(""),
                        stop.name,
                    ));
                }
            }
            ok(out)
        }
        None => ok("No active itinerary. Use `itinerary set` to lay out a route."),
    }
}

fn cmd_clear(dynamic_dir: &Path, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
    let path = itin_path(dynamic_dir);
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    emit_event(ctx, &Itinerary {
        title: String::new(),
        stops: vec![],
        current: 0,
    });
    ok("Itinerary cleared.")
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dynamic() -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("souveraine-itinerary-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("system").join("dynamic")).unwrap();
        dir
    }

    #[test]
    fn roundtrip_empty_route() {
        let dynamic_dir = temp_dynamic();
        let ity = Itinerary {
            title: "Test".into(),
            stops: vec![],
            current: 0,
        };
        save(&dynamic_dir, &ity).unwrap();
        let loaded = load(&dynamic_dir).unwrap();
        assert_eq!(loaded.title, "Test");
        assert!(loaded.stops.is_empty());
    }

    #[test]
    fn roundtrip_with_stops() {
        let dynamic_dir = temp_dynamic();
        let ity = Itinerary {
            title: "Port the turn model".into(),
            stops: vec![
                Stop {
                    name: "Design".into(),
                    description: Some("sketch the interface".into()),
                    todo_id: None,
                    status: StopStatus::Current,
                    nature: None,
                    energy: None,
                },
                Stop {
                    name: "Implement".into(),
                    description: None,
                    todo_id: None,
                    status: StopStatus::Pending,
                    nature: None,
                    energy: None,
                },
            ],
            current: 0,
        };
        save(&dynamic_dir, &ity).unwrap();
        let loaded = load(&dynamic_dir).unwrap();
        assert_eq!(loaded.title, "Port the turn model");
        assert_eq!(loaded.stops.len(), 2);
        assert_eq!(loaded.stops[0].status, StopStatus::Current);
        assert_eq!(loaded.stops[1].status, StopStatus::Pending);
    }

    #[test]
    fn advance_marks_done_and_moves() {
        let dynamic_dir = temp_dynamic();
        let mut ity = Itinerary {
            title: "Build".into(),
            stops: vec![
                Stop {
                    name: "A".into(),
                    description: None,
                    todo_id: None,
                    status: StopStatus::Current,
                    nature: None,
                    energy: None,
                },
                Stop {
                    name: "B".into(),
                    description: None,
                    todo_id: None,
                    status: StopStatus::Pending,
                    nature: None,
                    energy: None,
                },
            ],
            current: 0,
        };
        save(&dynamic_dir, &ity).unwrap();

        // Manually advance
        ity.stops[0].status = StopStatus::Done;
        ity.stops[1].status = StopStatus::Current;
        ity.current = 1;
        save(&dynamic_dir, &ity).unwrap();

        let loaded = load(&dynamic_dir).unwrap();
        assert_eq!(loaded.stops[0].status, StopStatus::Done);
        assert_eq!(loaded.stops[1].status, StopStatus::Current);
        assert_eq!(loaded.current, 1);
    }

    #[test]
    fn is_active_checks_live() {
        assert!(!StopStatus::Done.is_live());
        assert!(StopStatus::Pending.is_live());
        assert!(StopStatus::Current.is_live());
    }

    #[test]
    fn describe_renders_markdown() {
        let ity = Itinerary {
            title: "Test route".into(),
            current: 0,
            stops: vec![
                Stop {
                    name: "One".into(),
                    description: Some("first".into()),
                    todo_id: None,
                    status: StopStatus::Current,
                    nature: None,
                    energy: None,
                },
            ],
        };
        let desc = ity.describe();
        assert!(desc.contains("Test route"));
        assert!(desc.contains("●"));
        assert!(desc.contains("One"));
    }
}
