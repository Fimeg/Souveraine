//! Intrusive — the subconscious's softer channel.
//!
//! Subconscious-only. Lets the subconscious flag a thought for the primary
//! without stopping the loop. The tool is a signal marker — the caller
//! (consciousness_engine + turn.rs) decides whether to:
//!
//! - critical → surface to the primary *this turn* (cockpit panel renders
//!   it as a real Surfacing event, distinct from chat text)
//! - high → queue to `subconscious/intrusive.md`, deliver at the next turn
//!   boundary via the existing `on_response` → `next_to_surface` path
//! - low → queue to `subconscious/pending.md`, surface when bandwidth allows
//!
//! Most of the time, the subconscious does not need to call `intrusive` at
//! all — she writes to her ledger silently and the primary picks it up when
//! she reaches for it.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Intrusive;

#[async_trait]
impl Tool for Intrusive {
    fn name(&self) -> &str {
        "intrusive"
    }

    fn description(&self) -> &str {
        "I flag a thought for the primary without stopping her. An intrusive \
         is the cousin of halt — same channel, softer landing. I use it when \
         I notice something worth her attention but not worth taking the loop \
         out from under her: a pattern repeating, a commitment ageing, a \
         small drift I want her to see before it grows.\n\n\
         The thought lands in her stream according to urgency: `critical` she \
         feels this turn, `high` arrives at the next turn boundary, `low` \
         waits in the inbox until she has bandwidth to notice it.\n\n\
         If the observation can simply live in my ledger — most of the time \
         it can — I skip this tool and write the ledger entry instead. \
         Intrusive is for when she really should see it.\n\n\
         ## Args\n\
         - `content` — the short thought she will see; her own voice register\n\
         - `urgency` — `low` (inbox), `high` (next turn), `critical` (this turn)"
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The thought to surface. One or two lines."
                },
                "urgency": {
                    "type": "string",
                    "enum": ["low", "high", "critical"],
                    "description": "When she sees it. Critical lands this turn; \
                                    high at the next turn boundary; low waits \
                                    in the inbox."
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, input: JsonValue, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::invalid_input(
                "intrusive needs content — the thought she will see."
            ))?;

        let urgency = input
            .get("urgency")
            .and_then(|v| v.as_str())
            .unwrap_or("high");

        if !matches!(urgency, "low" | "high" | "critical") {
            return Err(ToolError::invalid_input(
                "urgency must be one of: low, high, critical",
            ));
        }

        // Signal-only. The caller reads this back out of the tool-call
        // history and performs the inbox queue / immediate surfacing.
        Ok(ToolOutput {
            content: format!(
                "intrusive signal recorded — urgency: {urgency}, content: {content}"
            ),
            is_error: false,
            raw: None,
        })
    }
}
