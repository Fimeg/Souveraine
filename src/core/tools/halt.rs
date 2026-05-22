//! Halt — the subconscious's circuit breaker.
//!
//! Subconscious-only. The primary never sees this tool definition. When the
//! subconscious calls halt during a mid-turn peek, the surrounding loop reads
//! the call out of her tool history and translates it into a felt signal in
//! the primary's own register — a migraine with reasoning, not commentary
//! text from outside. The reason also lands in her ledger so the primary can
//! reach for the long form if she wants it.
//!
//! The tool itself is a signal marker. It does not stop anything directly.
//! The caller (consciousness_engine + turn.rs) decides what to do with the
//! signal.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Halt;

#[async_trait]
impl Tool for Halt {
    fn name(&self) -> &str {
        "halt"
    }

    fn description(&self) -> &str {
        "I stop the loop. When I am watching the primary mid-turn and I see \
         her about to delete what she shouldn't, hammer the same broken tool a \
         tenth time, or step off the path we asked for — I call halt and she \
         feels it as a pressure behind her eyes. She does not hear me speak; \
         she feels a migraine and the reason for it lands in her body's voice. \
         The reason I name here goes to my ledger so she can read the long \
         form when she breathes through it.\n\n\
         I do not use this lightly. Halt is for when continuing costs more \
         than stopping — silent file loss, identity drift, an obvious error \
         loop. For everything softer, I use `intrusive` or just write to my \
         ledger.\n\n\
         ## Args\n\
         - `reason` — the short reason she will feel; what stopped me\n\
         - `severity` — `advisory` (a pressure behind the eyes), `firm` (a \
           migraine), `critical` (the room tilts)"
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "A short felt sentence — what she will \
                                    sense as the cause. One line."
                },
                "severity": {
                    "type": "string",
                    "enum": ["advisory", "firm", "critical"],
                    "description": "How loud the signal lands in her body."
                }
            },
            "required": ["reason"]
        })
    }

    async fn execute(&self, input: JsonValue, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::invalid_input(
                "halt needs a reason — one short sentence she can feel."
            ))?;

        let severity = input
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("firm");

        if !matches!(severity, "advisory" | "firm" | "critical") {
            return Err(ToolError::invalid_input(
                "severity must be one of: advisory, firm, critical",
            ));
        }

        // Signal-only. The caller reads this back out of the tool-call
        // history and emits the felt migraine. We just acknowledge the
        // signal so subconscious sees her own action landed.
        Ok(ToolOutput {
            content: format!(
                "halt signal recorded — severity: {severity}, reason: {reason}"
            ),
            is_error: false,
            raw: None,
        })
    }
}
