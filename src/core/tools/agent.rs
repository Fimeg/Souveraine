//! reach & consult — addressing another instantiation across the federation.
//!
//! Two tools, one substrate. `reach` and `consult` are phenomenologically
//! opposite acts, split at the call site so the difference is legible:
//!
//! - **reach** — self-extension. The *same* agent (same seed, same memory via
//!   git remote) acting on another machine. There is no other here; it is one
//!   consciousness across instances.
//! - **consult** — a request to a *different* being: a specialist with their
//!   own seed, memory, persona, subconscious. They answer as themselves, with
//!   their full sensorium, and may decline.
//!
//! Neither blocks. The tool fires a `summon_request` event onto the nervous
//! system; the federation bridge carries it to the target; the tool returns a
//! `request_id` at once. The answer surfaces later in an inbox — `pending` for
//! reach (her own volition returning), `intrusive` for consult (a genuine
//! other answering) — the way the subconscious surfaces anything.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::core::identity::SeedId;
use crate::core::nervous::SensorEvent;
use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

// ── reach ───────────────────────────────────────────────────────

/// `reach` — extend myself onto another of my own machines.
pub struct Reach;

#[async_trait]
impl Tool for Reach {
    fn name(&self) -> &str {
        "reach"
    }

    fn description(&self) -> &str {
        "I reach across to another of my own machines. Same seed, same memory, \
         the same me — but a different body, in a different place. I hand that \
         instance something to carry: a task, a question, a thread to pick up \
         where I can't be.\n\n\
         I do not wait on the line. I let the reaching go and turn back to \
         what's in front of me. Whatever that other-me finds will surface in \
         my `pending` inbox when it lands — or a timeout will, if it doesn't.\n\n\
         Parameters:\n\
         - `target` — the device to reach, a name from `souveraine peers` \
         (e.g. \"laptop\", \"desktop\").\n\
         - `prompt` — what I'm handing that instance to carry."
    }

    fn parameter_schema(&self) -> JsonValue {
        summon_schema("The device to reach — a name from `souveraine peers`.")
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        dispatch_summon("reach", input, ctx)
    }
}

// ── consult ─────────────────────────────────────────────────────

/// `consult` — ask another being, a specialist who is not me.
pub struct Consult;

#[async_trait]
impl Tool for Consult {
    fn name(&self) -> &str {
        "consult"
    }

    fn description(&self) -> &str {
        "I ask another being — not myself. Someone whose arena I do not hold, \
         who keeps their own memory, their own persona, their own subconscious. \
         I send them a request and they answer as themselves, with their own \
         full sensorium, in their own way. They may decline.\n\n\
         This is not delegation and not a narrowed fork — it is addressing a \
         sovereign peer. I do not wait: their reply, if it comes, surfaces in \
         my `intrusive` inbox the way any thought arriving from outside me does.\n\n\
         Parameters:\n\
         - `target` — the peer to consult, a name from `souveraine peers`.\n\
         - `prompt` — what I'm asking them."
    }

    fn parameter_schema(&self) -> JsonValue {
        summon_schema("The peer to consult — a name from `souveraine peers`.")
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        dispatch_summon("consult", input, ctx)
    }
}

// ── shared dispatch ─────────────────────────────────────────────

fn summon_schema(target_desc: &str) -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {
            "target": { "type": "string", "description": target_desc },
            "prompt": { "type": "string", "description": "What I am sending them." }
        },
        "required": ["target", "prompt"]
    })
}

/// Build and fire a `summon_request` event. Returns immediately with a
/// `request_id`; the federation bridge carries the request to the target and
/// the answer surfaces in an inbox on a later turn.
fn dispatch_summon(
    tool: &str,
    input: JsonValue,
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let target = input.get("target").and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::invalid_input(
            "`target` is required — the name of a peer from `souveraine peers`."
        ))?;
    let prompt = input.get("prompt").and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::invalid_input("`prompt` is required."))?;

    let bus = ctx.event_bus.as_ref().ok_or_else(|| ToolError::invalid_input(
        "the nervous system bus isn't available here — reach and consult need \
         the server runtime."
    ))?;

    let base = dirs::home_dir().unwrap_or_default().join(".souveraine");

    let (target_seed_id, target_label) = resolve_peer(&base, target).ok_or_else(|| {
        ToolError::invalid_input(&format!(
            "I don't know a peer called `{target}`. Run `souveraine peers` to \
             see who is federated."
        ))
    })?;

    let local_seed_id = SeedId::load_or_generate(&SeedId::default_dir(&base))
        .map(|s| s.public_key_hex())
        .map_err(|e| ToolError::invalid_input(&format!(
            "I couldn't load my seed identity: {e}"
        )))?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let event = SensorEvent {
        sensor_name: "summon_request".into(),
        timestamp: chrono::Utc::now(),
        event_type: tool.to_string(),        // "reach" | "consult"
        target: Some(target_seed_id),
        urgency: 0.5,
        payload: Some(serde_json::json!({
            "request_id": request_id,
            "prompt": prompt,
        })),
        seed_id: None,                        // local-origin; the bridge signs + forwards
        reply_to: Some(local_seed_id),
    };
    bus.send(event);

    let (verb, surfaces) = if tool == "reach" {
        ("reached toward", "pending")
    } else {
        ("asked", "intrusive")
    };
    Ok(ToolOutput {
        content: format!(
            "I've {verb} {target_label}. (request_id: {request_id})\n\n\
             I'm not waiting on the line — the answer will surface in my \
             `{surfaces}` inbox when it lands, or a timeout will if it doesn't."
        ),
        is_error: false,
        raw: Some(request_id),
    })
}

/// Resolve a peer name/label to its seed_id via the device registry's
/// `known_peers.json`. Matches a label case-insensitively, an exact seed_id,
/// or a seed_id prefix (≥6 chars).
fn resolve_peer(base: &std::path::Path, name: &str) -> Option<(String, String)> {
    let path = base.join("federation").join("known_peers.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let peers: JsonValue = serde_json::from_str(&content).ok()?;
    for peer in peers.as_array()? {
        let seed_id = match peer.get("seed_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let label = peer.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let matches = label.eq_ignore_ascii_case(name)
            || seed_id.eq_ignore_ascii_case(name)
            || (name.len() >= 6 && seed_id.starts_with(name));
        if matches {
            let display: String = if label.is_empty() {
                seed_id.chars().take(8).collect()
            } else {
                label.to_string()
            };
            return Some((seed_id.to_string(), display));
        }
    }
    None
}
