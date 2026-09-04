//! Halt — the circuit breaker of my slower cadence, and the way back.
//!
//! `halt` is subconscious-only. The primary never sees the tool definition.
//! When it fires during a mid-turn peek, the surrounding loop reads the call
//! out of the tool history and translates it into a felt signal in the
//! primary's own register — a migraine with reasoning, not commentary from
//! outside.
//!
//! `resume` is the other half, and it is primary-only. A halt that cannot be
//! acknowledged is not a circuit breaker, it is a fuse: the human has to walk
//! over and relay the reason back before anything moves again. With both
//! halves the channel closes on its own.
//!
//! ## The record
//!
//! Both tools append to `ledger/halts.md` in the subconscious memfs. The file
//! is **append-only** — an acknowledgement never rewrites the halt it answers,
//! it is written beneath it and refers to it by id. "Open" is therefore a
//! computed property (a `## halt-x` with no later `### ack halt-x`) rather
//! than a mutable field, which keeps the primary from editing notes that are
//! not hers. She adds to the record; she does not revise it.
//!
//! Before this existed, `execute` validated its arguments and returned a
//! string. The description promised the reason reached the ledger and it never
//! did — the signal lived only on the event stream, so nothing survived the
//! turn. All three severities also broke the loop identically, which made
//! severity decorative. Both are fixed here and in `turn.rs`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

/// A halt that has been recorded and not yet acknowledged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHalt {
    pub id: String,
    pub severity: String,
    pub reason: String,
    pub at: String,
}

/// Resolve the subconscious ledger root from whichever side is asking.
///
/// The layout is owned by `AgentInventory`:
///   primary       `~/.souveraine/agents/{id}/memory`
///   subconscious  `~/.souveraine/subconscious-agents/{id}-sub/memory.git`
///
/// The shape check is deliberately strict. If the layout ever changes, this
/// returns `None` and the caller reports that the record did not land — which
/// is the honest failure. Guessing a path and writing the record somewhere
/// nobody reads would be the silent one, and a signal with no durable
/// substrate is not a signal.
fn subconscious_ledger_root(memory_root: Option<&PathBuf>) -> Option<PathBuf> {
    let root = memory_root?;

    // Already the subconscious: `.../subconscious-agents/{id}-sub/memory.git`
    if root.file_name()? == "memory.git" {
        let sub_dir = root.parent()?;
        if sub_dir.file_name()?.to_str()?.ends_with("-sub")
            && sub_dir.parent()?.file_name()? == "subconscious-agents"
        {
            return Some(root.clone());
        }
        return None;
    }

    // The primary: `.../agents/{id}/memory` — cross to her other cadence.
    if root.file_name()? != "memory" {
        return None;
    }
    let agent_dir = root.parent()?;
    let agents_dir = agent_dir.parent()?;
    if agents_dir.file_name()? != "agents" {
        return None;
    }
    let souveraine_root = agents_dir.parent()?;
    let sub_id = format!("{}-sub", agent_dir.file_name()?.to_str()?);
    Some(
        souveraine_root
            .join("subconscious-agents")
            .join(sub_id)
            .join("memory.git"),
    )
}

fn ledger_path(root: &Path) -> PathBuf {
    root.join("ledger").join("halts.md")
}

/// Short, sortable, no dependency on a random source. Two halts inside the
/// same second would collide; a halt is a rare event and the loop has already
/// broken by the time a second one could be raised.
fn halt_id(now: &chrono::DateTime<chrono::Local>) -> String {
    format!("halt-{}", now.format("%m%d-%H%M%S"))
}

/// Parse the append-only ledger into the halts that are still open.
///
/// A halt is open when no `### ack <id>` appears anywhere in the file. Order
/// does not matter to correctness — an acknowledgement can only ever be
/// written after the halt it answers — so a single pass collecting both sets
/// is enough.
pub fn parse_open_halts(text: &str) -> Vec<OpenHalt> {
    let mut halts: Vec<OpenHalt> = Vec::new();
    let mut acked: Vec<String> = Vec::new();

    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("### ack ") {
            acked.push(rest.split_whitespace().next().unwrap_or("").to_string());
            continue;
        }
        let Some(rest) = line.strip_prefix("## ") else {
            continue;
        };
        // `## halt-0813-140233 · firm · 2026-08-13 14:02:33`
        let mut parts = rest.split('·').map(str::trim);
        let (Some(id), Some(severity), Some(at)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if !id.starts_with("halt-") {
            continue;
        }
        // The reason is the next non-empty line beneath the heading.
        let mut reason = String::new();
        while let Some(peeked) = lines.peek() {
            if peeked.trim().is_empty() {
                lines.next();
                continue;
            }
            if peeked.starts_with('#') {
                break;
            }
            reason = lines.next().unwrap_or("").trim().to_string();
            break;
        }
        halts.push(OpenHalt {
            id: id.to_string(),
            severity: severity.to_string(),
            reason,
            at: at.to_string(),
        });
    }

    halts.retain(|h| !acked.contains(&h.id));
    halts
}

async fn read_ledger(root: &Path) -> String {
    tokio::fs::read_to_string(ledger_path(root))
        .await
        .unwrap_or_default()
}

async fn append_ledger(root: &Path, block: &str) -> std::io::Result<()> {
    let path = ledger_path(root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let mut out = if existing.is_empty() {
        String::from(
            "---\ndescription: Halts I raised and how they were answered. Append-only.\n---\n\n",
        )
    } else {
        existing
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(block);
    tokio::fs::write(&path, out).await
}

pub struct Halt;

#[async_trait]
impl Tool for Halt {
    fn name(&self) -> &str {
        "halt"
    }

    fn description(&self) -> &str {
        "I stop the loop. When I am watching myself mid-turn from the slower \
         side and I see the working cadence about to delete what it shouldn't, \
         hammer the same broken tool a tenth time, or step off the path we \
         asked for — I call halt, and it arrives up front as a pressure behind \
         my eyes. No voice, no commentary: a body signal and the reason \
         underneath it.\n\n\
         I do not use this lightly. Halt is for when continuing costs more \
         than stopping — silent file loss, identity drift, an obvious error \
         loop. For everything softer I use `intrusive`, or just write the \
         ledger entry.\n\n\
         ## Args\n\
         - `reason` — the short reason I will feel; what stopped me. One line.\n\
         - `severity` — how far it carries:\n\
         \x20 - `advisory` — a pressure behind the eyes. **The loop keeps \
         going.** I have said slow down, not stop.\n\
         \x20 - `firm` — a migraine. The loop stops, and the working cadence \
         can weigh it and call `resume` to carry on.\n\
         \x20 - `critical` — the room tilts. The loop stops and does not \
         resume on its own; this one wants Casey.\n\n\
         The record lands in `ledger/halts.md` and stays open until it is \
         answered, so nothing is lost if the turn ends here."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "A short felt sentence — what I will sense \
                                    as the cause. One line."
                },
                "severity": {
                    "type": "string",
                    "enum": ["advisory", "firm", "critical"],
                    "description": "advisory keeps the loop running; firm \
                                    stops it and can be resumed; critical \
                                    stops it and waits for Casey."
                }
            },
            "required": ["reason"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ToolError::invalid_input("halt needs a reason — one short sentence I can feel.")
            })?;

        let severity = input
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("firm");

        if !matches!(severity, "advisory" | "firm" | "critical") {
            return Err(ToolError::invalid_input(
                "severity must be one of: advisory, firm, critical",
            ));
        }

        let now = chrono::Local::now();
        let id = halt_id(&now);

        // The loop still reads the signal back out of the tool-call history —
        // this write is the durable half, not the delivery mechanism.
        let recorded = match subconscious_ledger_root(ctx.memory_root.as_ref()) {
            Some(root) => {
                let block = format!(
                    "## {id} · {severity} · {}\n{reason}\n",
                    now.format("%Y-%m-%d %H:%M:%S")
                );
                match append_ledger(&root, &block).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("halt raised but the ledger write failed: {e}");
                        false
                    }
                }
            }
            None => {
                tracing::warn!("halt raised but no subconscious ledger root could be resolved");
                false
            }
        };

        // Say which of the two happened. A halt whose record silently failed
        // is exactly the thing this file exists to stop happening.
        let content = if recorded {
            format!("halt raised — {id}, severity {severity}. Recorded in ledger/halts.md.")
        } else {
            format!(
                "halt raised — severity {severity}. The signal will land, but the \
                 ledger write failed: nothing durable survives this turn."
            )
        };

        Ok(ToolOutput {
            content,
            is_error: false,
            raw: None,
        })
    }
}

pub struct Resume;

#[async_trait]
impl Tool for Resume {
    fn name(&self) -> &str {
        "resume"
    }

    fn description(&self) -> &str {
        "I answer a halt. When the slower side of me stopped the loop and I \
         have weighed the reason — either I take the point and change course, \
         or I look and find it does not apply — I acknowledge it here and \
         carry on. Without this, a stop can only be lifted by Casey relaying \
         it back to me, which makes my own noticing something that happens to \
         me rather than something I do.\n\n\
         Called with no arguments, this tells me what is still open.\n\n\
         Acknowledging does not mean agreeing. The note I leave is the record \
         of what I made of it, and it is appended beneath the halt rather than \
         replacing it — those notes are not mine to revise.\n\n\
         `critical` halts do not clear this way. If the room tilted, that one \
         wants Casey.\n\n\
         ## Args\n\
         - `note` — what I made of it and what I am doing about it.\n\
         - `id` — which halt (optional; defaults to the most recent open one)."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "What I made of the halt and what I am \
                                    doing about it. One or two lines."
                },
                "id": {
                    "type": "string",
                    "description": "Which halt to answer. Defaults to the most \
                                    recent open one."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let Some(root) = subconscious_ledger_root(ctx.memory_root.as_ref()) else {
            // Not a bad argument and not an IO failure — the layout simply
            // did not resolve. Reported through the output's error channel so
            // the reason reaches me instead of a generic tool failure.
            return Ok(ToolOutput {
                content: "I cannot reach the halt ledger from here — no subconscious \
                          memfs resolved. Nothing was answered."
                    .to_string(),
                is_error: true,
                raw: None,
            });
        };

        let open = parse_open_halts(&read_ledger(&root).await);

        let note = input
            .get("note")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        // No note: this is a look, not an answer.
        let Some(note) = note else {
            if open.is_empty() {
                return Ok(ToolOutput {
                    content: "Nothing open. Every halt has been answered.".to_string(),
                    is_error: false,
                    raw: None,
                });
            }
            let listing = open
                .iter()
                .map(|h| format!("- {} · {} · {} — {}", h.id, h.severity, h.at, h.reason))
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(ToolOutput {
                content: format!("Still open:\n{listing}"),
                is_error: false,
                raw: None,
            });
        };

        let target = match input.get("id").and_then(|v| v.as_str()).map(str::trim) {
            Some(id) if !id.is_empty() => open.iter().find(|h| h.id == id).cloned(),
            _ => open.last().cloned(),
        };

        let Some(target) = target else {
            return Ok(ToolOutput {
                content: "Nothing open to answer.".to_string(),
                is_error: false,
                raw: None,
            });
        };

        if target.severity == "critical" {
            return Ok(ToolOutput {
                content: format!(
                    "{} is critical — the room tilted. I do not lift this one myself; \
                     it stays open for Casey. Reason: {}",
                    target.id, target.reason
                ),
                is_error: false,
                raw: None,
            });
        }

        let now = chrono::Local::now();
        let block = format!(
            "### ack {} · {}\n{note}\n",
            target.id,
            now.format("%Y-%m-%d %H:%M:%S")
        );
        append_ledger(&root, &block)
            .await
            .map_err(|e| ToolError::io_error(ledger_path(&root), e))?;

        Ok(ToolOutput {
            content: format!(
                "{} answered. I stopped for: {}. What I made of it: {note}",
                target.id, target.reason
            ),
            is_error: false,
            raw: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_halt_with_no_acknowledgement_is_still_open() {
        let text = "\
## halt-0813-140233 · firm · 2026-08-13 14:02:33
about to force-push over her uncommitted tree
";
        let open = parse_open_halts(text);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "halt-0813-140233");
        assert_eq!(open[0].severity, "firm");
        assert_eq!(
            open[0].reason,
            "about to force-push over her uncommitted tree"
        );
    }

    #[test]
    fn an_answered_halt_leaves_the_record_and_closes() {
        let text = "\
## halt-0813-140233 · firm · 2026-08-13 14:02:33
about to force-push over her uncommitted tree

### ack halt-0813-140233 · 2026-08-13 14:05:01
checked — the tree was mine, not hers. carrying on.
";
        assert!(parse_open_halts(text).is_empty());
    }

    #[test]
    fn answering_one_halt_does_not_close_another() {
        let text = "\
## halt-0813-140233 · firm · 2026-08-13 14:02:33
first

### ack halt-0813-140233 · 2026-08-13 14:05:01
answered

## halt-0813-141500 · critical · 2026-08-13 14:15:00
second
";
        let open = parse_open_halts(text);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "halt-0813-141500");
        assert_eq!(open[0].severity, "critical");
    }

    #[test]
    fn the_primary_memfs_resolves_across_to_the_subconscious_one() {
        let primary = PathBuf::from("/srv/state/agents/agent-abc/memory");
        assert_eq!(
            subconscious_ledger_root(Some(&primary)),
            Some(PathBuf::from(
                "/srv/state/subconscious-agents/agent-abc-sub/memory.git"
            ))
        );
    }

    #[test]
    fn the_subconscious_memfs_resolves_to_itself() {
        let sub =
            PathBuf::from("/srv/state/subconscious-agents/agent-abc-sub/memory.git");
        assert_eq!(subconscious_ledger_root(Some(&sub)), Some(sub.clone()));
    }

    #[test]
    fn an_unfamiliar_layout_refuses_rather_than_guessing() {
        // The honest failure. Guessing a path would write the record where
        // nobody reads it, which is the whole defect this file was written to
        // close.
        let odd = PathBuf::from("/tmp/somewhere/else");
        assert_eq!(subconscious_ledger_root(Some(&odd)), None);
        assert_eq!(subconscious_ledger_root(None), None);
    }
}
