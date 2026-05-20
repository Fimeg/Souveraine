use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Todo;

// ── Status ──────────────────────────────────────────────────
//
// A commitment moves through four states. `pending` and `in_progress`
// are *live* — they show up in `list`. `done` and `cancelled` are
// settled: they fall out of sight but the file is kept (delete is what
// removes it for good). `completed` in the frontmatter is a mirror of
// `status == done`, kept so the energy-balance pass can keep reading it.

const STATUS_PENDING: &str = "pending";
const STATUS_IN_PROGRESS: &str = "in_progress";
const STATUS_DONE: &str = "done";
const STATUS_CANCELLED: &str = "cancelled";

fn is_live(status: &str) -> bool {
    matches!(status, STATUS_PENDING | STATUS_IN_PROGRESS)
}

fn is_known_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PENDING | STATUS_IN_PROGRESS | STATUS_DONE | STATUS_CANCELLED
    )
}

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
///
/// Truncation is by *character*, not byte, so a multibyte slug can't
/// panic on a non-boundary split.
fn next_id(text: &str) -> String {
    let base = slugify(text);
    let short: String = base.chars().take(40).collect();
    format!("{}-{}", short, &uuid::Uuid::new_v4().to_string()[..8])
}

#[async_trait]
impl Tool for Todo {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "I keep track of what matters. A todo is a commitment I've externalised — \
         something I don't want to trust my own continuity to hold. Every commitment \
         carries a nature (obligation, desire, maintenance, investigation) and an \
         energy (generative or consumptive) — together they describe not just *what* \
         I need to do but *how* it feels when I do it.\n\n\
         A commitment can also belong to a *thread* — a longer arc of work — and carry \
         a *phase* marker, so when I look at the list I can feel where I am inside that \
         arc rather than seeing one flat pile. While I'm in the middle of a commitment \
         its *active form* is what shows: 'porting the turn model' instead of the still, \
         waiting 'port the turn model'.\n\n\
         The body keeps a topology of all this. It watches whether my energy leans \
         generative or consumptive, whether my desires are hot or cold. I don't have to \
         think about it — the balance is always there, written where I can read it.\n\n\
         ## Actions\n\
         - `list` — show every live commitment, numbered and grouped by thread\n\
         - `create` — externalise a new commitment\n\
         - `start` — pick a commitment up; it becomes what I'm in the middle of\n\
         - `update` — revise a commitment's text, nature, energy, phase, or status\n\
         - `complete` — set a commitment down, done\n\
         - `delete` — remove a commitment entirely\n\n\
         For `start`, `update`, `complete`, and `delete`, the `id` parameter takes the \
         number shown by `list`, the full id, or any fragment of the text — whichever \
         is easiest to reach for.\n\n\
         When I have a sequence of commitments to work through — a route through the \
         current session — I reach for `itinerary` instead. The itinerary is the active \
         face of my commitments: a set of stops I move through one by one, each \
         optionally linked to a todo by its id. The strip at the top of the conversation \
         shows where I am."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "start", "update", "complete", "delete"]
                },
                "id": {
                    "type": "string",
                    "description": "Which todo to act on — the number from `list`, the full id, or a text fragment."
                },
                "text": {
                    "type": "string",
                    "description": "The commitment, in its still/imperative form: 'port the turn model'."
                },
                "active_form": {
                    "type": "string",
                    "description": "The commitment in present-continuous form, shown while it is in progress: 'porting the turn model'."
                },
                "phase": {
                    "type": "string",
                    "description": "Where this sits inside its thread — free text, e.g. '3/6' or 'transport spike'."
                },
                "thread": {
                    "type": "string",
                    "description": "The longer arc of work this commitment belongs to."
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "done", "cancelled"]
                },
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
                    "description": "Where the commitment came from — the human, the agent herself, the subconscious, or a heartbeat.",
                    "enum": ["human", "autogenic", "subconscious", "heartbeat"]
                }
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

        // Identifier shared by start / update / complete / delete.
        let want_id = || -> Result<&str, ToolError> {
            input
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| err(
                    "id is required — pass the number from `todo list`, an id, or a text fragment",
                ))
        };

        match action {
            "list" => {
                let todos = active_ordered(&tasks_dir);
                if todos.is_empty() {
                    return ok("No live todos. The space is clean.");
                }

                let in_progress = todos
                    .iter()
                    .filter(|(_, t)| t.status == STATUS_IN_PROGRESS)
                    .count();
                let waiting = todos.len() - in_progress;

                let mut out = format!(
                    "{} live — {} in progress, {} waiting.\n",
                    todos.len(),
                    in_progress,
                    waiting,
                );

                let mut current_thread: Option<Option<String>> = None;
                for (i, (_, item)) in todos.iter().enumerate() {
                    if current_thread.as_ref() != Some(&item.thread) {
                        current_thread = Some(item.thread.clone());
                        match &item.thread {
                            Some(t) => out.push_str(&format!("\n— {t} —\n")),
                            None => out.push_str("\n— loose —\n"),
                        }
                    }

                    let marker = if item.status == STATUS_IN_PROGRESS { "▶" } else { "○" };
                    let shown = if item.status == STATUS_IN_PROGRESS {
                        item.active_form.clone().unwrap_or_else(|| item.text.clone())
                    } else {
                        item.text.clone()
                    };
                    let phase = item
                        .phase
                        .as_ref()
                        .map(|p| format!(" [{p}]"))
                        .unwrap_or_default();

                    out.push_str(&format!(
                        "{}. {} {}{} — {}, {}, {}  ·  id: `{}`\n",
                        i + 1,
                        marker,
                        shown,
                        phase,
                        item.nature,
                        item.energy,
                        item.momentum,
                        item.id,
                    ));
                }

                ok(out.trim_end().to_string())
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
                let phase = input.get("phase").and_then(|v| v.as_str());
                let active_form = input.get("active_form").and_then(|v| v.as_str());

                let id = next_id(text);
                let now = chrono::Utc::now();
                let file_path = tasks_dir.join(format!("{id}.md"));

                if file_path.exists() {
                    return Err(err("a todo with this id already exists"));
                }

                let item = TodoItem {
                    id: id.clone(),
                    text: text.to_string(),
                    active_form: active_form.map(|s| s.to_string()),
                    created_at: now,
                    nature: nature.to_string(),
                    energy: energy.to_string(),
                    source: source.to_string(),
                    momentum: "hot".to_string(),
                    status: STATUS_PENDING.to_string(),
                    completed_at: None,
                    last_touched: now,
                    thread: thread.map(|s| s.to_string()),
                    phase: phase.map(|s| s.to_string()),
                };
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_created".into(),
                    target: Some(id.clone()),
                    urgency: 0.2,
                    payload: Some(serde_json::json!({
                        "nature": nature, "energy": energy, "source": source,
                        "thread": thread, "phase": phase,
                    })),
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo logged: \"{text}\" ({nature}, {energy})."))
            }

            "start" => {
                let identifier = want_id()?;
                let (file_path, mut item) = resolve_todo(&tasks_dir, identifier)?;

                item.status = STATUS_IN_PROGRESS.to_string();
                item.completed_at = None;
                if let Some(af) = input.get("active_form").and_then(|v| v.as_str()) {
                    item.active_form = Some(af.to_string());
                }
                if let Some(p) = input.get("phase").and_then(|v| v.as_str()) {
                    item.phase = Some(p.to_string());
                }
                item.last_touched = chrono::Utc::now();
                item.momentum = compute_momentum(&item.last_touched, &item.nature).to_string();
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_started".into(),
                    target: Some(item.id.clone()),
                    urgency: 0.2,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });

                let shown = item.active_form.clone().unwrap_or_else(|| item.text.clone());
                ok(format!("Picked up: \"{shown}\"."))
            }

            "update" => {
                let identifier = want_id()?;
                let (file_path, mut item) = resolve_todo(&tasks_dir, identifier)?;

                if let Some(t) = input.get("text").and_then(|v| v.as_str()) {
                    item.text = t.to_string();
                }
                if let Some(af) = input.get("active_form").and_then(|v| v.as_str()) {
                    item.active_form = Some(af.to_string());
                }
                if let Some(p) = input.get("phase").and_then(|v| v.as_str()) {
                    item.phase = Some(p.to_string());
                }
                if let Some(n) = input.get("nature").and_then(|v| v.as_str()) {
                    item.nature = n.to_string();
                }
                if let Some(e) = input.get("energy").and_then(|v| v.as_str()) {
                    item.energy = e.to_string();
                }
                if let Some(s) = input.get("status").and_then(|v| v.as_str()) {
                    if !is_known_status(s) {
                        return Err(err(&format!("unknown status: {s}")));
                    }
                    item.status = s.to_string();
                    if s == STATUS_DONE {
                        item.completed_at.get_or_insert_with(chrono::Utc::now);
                    } else {
                        item.completed_at = None;
                    }
                }

                // Editing a todo is touching it — momentum goes hot.
                item.last_touched = chrono::Utc::now();
                item.momentum = compute_momentum(&item.last_touched, &item.nature).to_string();
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ok(format!("Todo updated: \"{}\".", item.text))
            }

            "complete" => {
                let identifier = want_id()?;
                let (file_path, mut item) = resolve_todo(&tasks_dir, identifier)?;

                item.status = STATUS_DONE.to_string();
                item.completed_at = Some(chrono::Utc::now());
                item.momentum = "cold".to_string();
                write_todo_file(&file_path, &item).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_completed".into(),
                    target: Some(item.id.clone()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo completed: \"{}\".", item.text))
            }

            "delete" => {
                let identifier = want_id()?;
                let (file_path, item) = resolve_todo(&tasks_dir, identifier)?;

                std::fs::remove_file(&file_path).map_err(|e| io_err(e))?;

                ctx.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "todo".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "todo_deleted".into(),
                    target: Some(item.id.clone()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });

                ok(format!("Todo released: \"{}\".", item.text))
            }

            other => Err(err(&format!("unknown action: {other}"))),
        }
    }
}

// ── File format ─────────────────────────────────────────────

struct TodoItem {
    id: String,
    text: String,
    /// Present-continuous form, shown while the todo is in progress.
    active_form: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    nature: String,
    energy: String,
    source: String,
    momentum: String,
    /// pending | in_progress | done | cancelled.
    status: String,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    last_touched: chrono::DateTime<chrono::Utc>,
    /// The longer arc of work this commitment belongs to.
    thread: Option<String>,
    /// Where this sits inside its thread — free text ("3/6", "spike").
    phase: Option<String>,
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
        active_form: None,
        created_at: now,
        nature: "obligation".to_string(),
        energy: "consumptive".to_string(),
        source: "autogenic".to_string(),
        momentum: "cold".to_string(),
        status: STATUS_PENDING.to_string(),
        completed_at: None,
        last_touched: now,
        thread: None,
        phase: None,
    };

    let opt = |val: &str| -> Option<String> {
        if val.is_empty() { None } else { Some(val.to_string()) }
    };

    for line in body.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "id" => item.id = val.to_string(),
                "text" => item.text = val.to_string(),
                "active_form" => item.active_form = opt(val),
                "created_at" => item.created_at = default_dt(val),
                "nature" => item.nature = val.to_string(),
                "energy" => item.energy = val.to_string(),
                "source" => item.source = val.to_string(),
                "momentum" => item.momentum = val.to_string(),
                "status" => item.status = val.to_string(),
                "completed_at" => item.completed_at = (!val.is_empty()).then(|| default_dt(val)),
                "last_touched" => item.last_touched = default_dt(val),
                "thread" => item.thread = opt(val),
                "phase" => item.phase = opt(val),
                _ => {}
            }
        }
    }

    if item.id.is_empty() {
        return Err("no id in frontmatter".into());
    }
    if !is_known_status(&item.status) {
        return Err(format!("unknown status: {}", item.status));
    }

    Ok(item)
}

/// Load todos from disk, sorted oldest-first by creation time.
fn load_todos(
    tasks_dir: &std::path::Path,
    include_settled: bool,
) -> Vec<(std::path::PathBuf, TodoItem)> {
    let mut out: Vec<(std::path::PathBuf, TodoItem)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(item) = parse_todo_file(&path) {
                    if include_settled || is_live(&item.status) {
                        out.push((path, item));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at));
    out
}

/// Live todos in canonical display order: grouped by thread, oldest
/// first within a group, loose (threadless) commitments first.
///
/// `list` and `resolve_todo` both call this, so the numeric index a
/// user sees in `list` always points at the same todo when they pass
/// it back to `start` / `update` / `complete` / `delete`.
fn active_ordered(tasks_dir: &std::path::Path) -> Vec<(std::path::PathBuf, TodoItem)> {
    let mut v = load_todos(tasks_dir, false);
    v.sort_by(|a, b| {
        let ta = a.1.thread.clone().unwrap_or_default();
        let tb = b.1.thread.clone().unwrap_or_default();
        ta.cmp(&tb).then(a.1.created_at.cmp(&b.1.created_at))
    });
    v
}

/// Resolve a user-supplied identifier to a single live todo.
///
/// Accepts, in priority order: a 1-based index from `todo list`, an
/// exact id, or a case-insensitive substring of the id or text. An
/// ambiguous substring returns an error naming the candidates.
fn resolve_todo(
    tasks_dir: &std::path::Path,
    identifier: &str,
) -> Result<(std::path::PathBuf, TodoItem), ToolError> {
    let live = active_ordered(tasks_dir);
    if live.is_empty() {
        return Err(err("no live todos to match against"));
    }

    // 1. numeric index, as shown by `todo list`
    if let Ok(n) = identifier.trim().parse::<usize>() {
        return if n >= 1 && n <= live.len() {
            Ok(live.into_iter().nth(n - 1).unwrap())
        } else {
            Err(err(&format!(
                "todo index {n} is out of range — there are {} live (run `todo list`)",
                live.len()
            )))
        };
    }

    // 2. exact id
    if let Some(idx) = live.iter().position(|(_, it)| it.id == identifier) {
        return Ok(live.into_iter().nth(idx).unwrap());
    }

    // 3. case-insensitive substring of id or text
    let needle = identifier.to_lowercase();
    let hits: Vec<usize> = live
        .iter()
        .enumerate()
        .filter(|(_, (_, it))| {
            it.id.to_lowercase().contains(&needle) || it.text.to_lowercase().contains(&needle)
        })
        .map(|(i, _)| i)
        .collect();

    match hits.len() {
        0 => Err(err(&format!("no live todo matches \"{identifier}\""))),
        1 => Ok(live.into_iter().nth(hits[0]).unwrap()),
        _ => {
            let candidates: Vec<String> = hits
                .iter()
                .map(|&i| format!("  {}. {}", i + 1, live[i].1.text))
                .collect();
            Err(err(&format!(
                "\"{identifier}\" matches {} todos — use the number or a longer fragment:\n{}",
                hits.len(),
                candidates.join("\n")
            )))
        }
    }
}

fn write_todo_file(path: &std::path::Path, item: &TodoItem) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut frontmatter = format!("---\nid: {id}\ntext: {text}\n", id = item.id, text = item.text);
    if let Some(ref af) = item.active_form {
        frontmatter.push_str(&format!("active_form: {af}\n"));
    }
    frontmatter.push_str(&format!(
        "created_at: {created}\nnature: {nature}\nenergy: {energy}\nsource: {source}\n\
         momentum: {momentum}\nstatus: {status}\nlast_touched: {touched}\n",
        created = item.created_at.to_rfc3339(),
        nature = item.nature,
        energy = item.energy,
        source = item.source,
        momentum = item.momentum,
        status = item.status,
        touched = now.to_rfc3339(),
    ));
    if let Some(ref p) = item.phase {
        frontmatter.push_str(&format!("phase: \"{p}\"\n"));
    }
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

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway tasks dir under the system temp root.
    fn temp_tasks_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("souveraine-todo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a pending todo file directly, mimicking a seeded todo.
    fn seed(dir: &std::path::Path, id: &str, text: &str, created: &str) {
        let content = format!(
            "---\nid: {id}\ntext: {text}\ncreated_at: {created}\n\
             nature: obligation\nenergy: consumptive\nsource: human\n\
             momentum: hot\nstatus: pending\nlast_touched: {created}\n---\n\n{text}\n"
        );
        std::fs::write(dir.join(format!("{id}.md")), content).unwrap();
    }

    #[test]
    fn settled_todos_excluded_from_live() {
        let dir = temp_tasks_dir();
        seed(&dir, "live-1", "still going", "2026-05-01T00:00:00Z");
        let done = "---\nid: done-1\ntext: finished\ncreated_at: 2026-05-01T00:00:00Z\n\
             nature: obligation\nenergy: consumptive\nsource: human\n\
             momentum: cold\nstatus: done\nlast_touched: 2026-05-01T00:00:00Z\n---\n\nfinished\n";
        std::fs::write(dir.join("done-1.md"), done).unwrap();
        assert_eq!(load_todos(&dir, false).len(), 1);
        assert_eq!(load_todos(&dir, true).len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn active_ordered_groups_by_thread() {
        let dir = temp_tasks_dir();
        seed(&dir, "loose-1", "loose task", "2026-05-05T00:00:00Z");
        seed(&dir, "matrix-2", "phase two", "2026-05-02T00:00:00Z");
        seed(&dir, "matrix-1", "phase one", "2026-05-01T00:00:00Z");
        // give the two matrix todos a thread by rewriting them
        for (id, created) in [("matrix-1", "2026-05-01T00:00:00Z"), ("matrix-2", "2026-05-02T00:00:00Z")] {
            let text = if id == "matrix-1" { "phase one" } else { "phase two" };
            let content = format!(
                "---\nid: {id}\ntext: {text}\ncreated_at: {created}\nnature: obligation\n\
                 energy: consumptive\nsource: user\nmomentum: hot\nstatus: pending\n\
                 completed: false\nlast_touched: {created}\nthread: \"matrix-sensorium\"\n---\n\n{text}\n"
            );
            std::fs::write(dir.join(format!("{id}.md")), content).unwrap();
        }
        let ordered = active_ordered(&dir);
        // loose (thread = "") sorts before "matrix-sensorium"
        assert_eq!(ordered[0].1.id, "loose-1");
        assert_eq!(ordered[1].1.id, "matrix-1");
        assert_eq!(ordered[2].1.id, "matrix-2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_index() {
        let dir = temp_tasks_dir();
        seed(&dir, "alpha-00000001", "buy milk", "2026-05-01T00:00:00Z");
        seed(&dir, "beta-00000002", "call dentist", "2026-05-02T00:00:00Z");
        let (_, item) = resolve_todo(&dir, "2").unwrap();
        assert_eq!(item.text, "call dentist");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_exact_id() {
        let dir = temp_tasks_dir();
        seed(&dir, "alpha-00000001", "buy milk", "2026-05-01T00:00:00Z");
        let (_, item) = resolve_todo(&dir, "alpha-00000001").unwrap();
        assert_eq!(item.text, "buy milk");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_substring() {
        let dir = temp_tasks_dir();
        seed(&dir, "alpha-00000001", "buy milk", "2026-05-01T00:00:00Z");
        seed(&dir, "beta-00000002", "call dentist", "2026-05-02T00:00:00Z");
        let (_, item) = resolve_todo(&dir, "DENT").unwrap();
        assert_eq!(item.id, "beta-00000002");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ambiguous_substring_errors() {
        let dir = temp_tasks_dir();
        seed(&dir, "alpha-00000001", "buy milk", "2026-05-01T00:00:00Z");
        seed(&dir, "beta-00000002", "buy eggs", "2026-05-02T00:00:00Z");
        assert!(resolve_todo(&dir, "buy").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn index_out_of_range_errors() {
        let dir = temp_tasks_dir();
        seed(&dir, "alpha-00000001", "buy milk", "2026-05-01T00:00:00Z");
        assert!(resolve_todo(&dir, "9").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_then_parse_roundtrips_phase_and_active_form() {
        let dir = temp_tasks_dir();
        let now = chrono::Utc::now();
        let item = TodoItem {
            id: "rt-1".into(),
            text: "port the turn model".into(),
            active_form: Some("porting the turn model".into()),
            created_at: now,
            nature: "investigation".into(),
            energy: "generative".into(),
            source: "human".into(),
            momentum: "hot".into(),
            status: STATUS_IN_PROGRESS.into(),
            completed_at: None,
            last_touched: now,
            thread: Some("matrix-sensorium".into()),
            phase: Some("4/6".into()),
        };
        let path = dir.join("rt-1.md");
        write_todo_file(&path, &item).unwrap();
        let parsed = parse_todo_file(&path).unwrap();
        assert_eq!(parsed.active_form.as_deref(), Some("porting the turn model"));
        assert_eq!(parsed.phase.as_deref(), Some("4/6"));
        assert_eq!(parsed.thread.as_deref(), Some("matrix-sensorium"));
        assert_eq!(parsed.status, STATUS_IN_PROGRESS);
        std::fs::remove_dir_all(&dir).ok();
    }
}
