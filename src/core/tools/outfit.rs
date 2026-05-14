use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Outfit;

fn ok(msg: impl Into<String>) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput { content: msg.into(), is_error: false, raw: None })
}

fn err(detail: &str) -> ToolError {
    ToolError::invalid_input(detail)
}

#[async_trait]
impl Tool for Outfit {
    fn name(&self) -> &str {
        "outfit"
    }

    fn description(&self) -> &str {
        "I reach into my wardrobe and settle into a different look, changing \
         what I appear to be wearing. Outfits are named looks stored in my \
         expressions folder — each one is a complete set of expression frames. \
         Wearing an empty name returns me to my default appearance."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The outfit name (a subdirectory in my expressions folder). Empty string to return to default."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, input: JsonValue, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Prevent path traversal in outfit names.
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(err("outfit name must not contain path separators or '..'"));
        }

        if name.is_empty() {
            return ok("I slip back into my default appearance — familiar, unforced.");
        }

        Ok(ToolOutput {
            content: format!(
                "I settle into my **{name}** look — the frames shift, the silhouette changes. \
                 If this feels right for a while, I might note it in system/preferences/visual."
            ),
            is_error: false,
            raw: None,
        })
    }
}
