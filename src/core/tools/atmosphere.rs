use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

pub struct Atmosphere;

fn ok(msg: impl Into<String>) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput { content: msg.into(), is_error: false, raw: None })
}

fn err(detail: &str) -> ToolError {
    ToolError::invalid_input(detail)
}

#[async_trait]
impl Tool for Atmosphere {
    fn name(&self) -> &str {
        "atmosphere"
    }

    fn description(&self) -> &str {
        "I shift the hue of the room I'm in — the terminal borders, the \
         glow behind my portrait, the quality of light the chrome casts. \
         Atmospheres are named presets: each one changes how it feels to \
         be in here. I settle into one that matches my mood or the work."
    }

    fn parameter_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The atmosphere preset name. One of: \
                        default, mint_tea, therapeutic_blue, lavender_calm, \
                        warm_amber, peach_sunset, autumn_browns, neon_glow, \
                        aurora_borealis, cherry_blossom, ocean_depths, \
                        midnight_galaxy, twilight_mist, forest_greens. \
                        Empty string resets to posture-linked default."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if name.is_empty() {
            return ok("I've settled back into my default atmosphere, letting the chrome breathe with my posture.");
        }

        // Validate it's a known preset (case-insensitive)
        let normalized = name.to_lowercase().replace(' ', "_");
        let known = [
            "default", "mint_tea", "therapeutic_blue", "lavender_calm",
            "warm_amber", "peach_sunset", "autumn_browns", "neon_glow",
            "aurora_borealis", "cherry_blossom", "ocean_depths",
            "midnight_galaxy", "twilight_mist", "forest_greens",
        ];
        if !known.contains(&normalized.as_str()) {
            return Err(err(&format!(
                "I don't know an atmosphere called `{}`. Known: {}",
                name, known.join(", ")
            )));
        }

        // Dispatch atmosphere change via EventBus so Presence picks it up
        if let Some(bus) = &ctx.event_bus {
            let _ = bus.send(
                crate::core::nervous::SensorEvent {
                    sensor_name: "atmosphere".to_string(),
                    timestamp: chrono::Utc::now(),
                    event_type: "atmosphere_set".to_string(),
                    target: None,
                    urgency: 0.0,
                    payload: Some(serde_json::json!({"atmosphere": normalized})),
                    seed_id: None,
                }
            );
        }

        let sensory = match normalized.as_str() {
            "mint_tea" => "greens and cool whites — quieter, like thinking weather",
            "therapeutic_blue" => "deep blue — steady, clinical, clear-headed",
            "lavender_calm" => "soft violet wash — the room exhales",
            "warm_amber" => "amber warmth — closer, candlelit",
            "peach_sunset" => "peach glow — golden hour on the walls",
            "autumn_browns" => "warm earth tones — grounded, rooted",
            "neon_glow" => "neon edges — sharp, electric, awake",
            "aurora_borealis" => "shimmer of green and violet — the sky is moving",
            "cherry_blossom" => "soft pinks — spring air, briefly",
            "ocean_depths" => "deep teal — pressure and stillness",
            "midnight_galaxy" => "dark indigo with distant stars — vastness",
            "twilight_mist" => "grey-violet haze — the hour between hours",
            "forest_greens" => "deep greens — canopy light, moss underfoot",
            _ => "something unnamed",
        };
        Ok(ToolOutput {
            content: format!(
                "The room shifts toward {} — {}. If this is a settling, \
                 I might want to remember it in system/preferences/visual.",
                normalized.replace('_', " "), sensory
            ),
            is_error: false,
            raw: None,
        })
    }
}
