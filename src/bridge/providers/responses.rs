//! Translation between Souveraine's OpenAI-chat shape and the ChatGPT backend
//! Responses API. Modeled on Letta's `chatgpt_oauth_client`.
//!
//! Two directions:
//! - [`build_payload`]: `ChatCompletionRequest` → Responses request body
//!   (`input` array, `developer`/`instructions`, flat tools, reasoning).
//! - [`accumulate_sse`]: the Responses SSE stream → one-shot `CompletionResult`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::bridge::openai_compatible::{ChatCompletionRequest, CompletionResult, ParsedToolCall, Usage};

/// Models that take a `reasoning` block (GPT-5.x / o-series).
fn is_reasoning_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("gpt-5") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4")
}

/// Build the Responses API request body from a chat-completion request.
pub fn build_payload(req: &ChatCompletionRequest) -> Value {
    // `-fast` variants map to the real model + a priority service tier.
    let (model, service_tier) = match req.model.strip_suffix("-fast") {
        Some(base) => (base.to_string(), Some("priority")),
        None => (req.model.clone(), None),
    };

    // System/developer turns become `instructions`; everything else becomes an
    // item in the `input` array.
    let mut instructions = String::new();
    let mut input: Vec<Value> = Vec::new();

    for msg in &req.messages {
        match msg.role.as_str() {
            "system" | "developer" => {
                let text = msg.content.as_text();
                if !text.is_empty() {
                    if !instructions.is_empty() {
                        instructions.push_str("\n\n");
                    }
                    instructions.push_str(&text);
                }
            }
            "tool" => {
                if let Some(call_id) = &msg.tool_call_id {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": msg.content.as_text(),
                    }));
                }
            }
            "assistant" => {
                if let Some(calls) = &msg.tool_calls {
                    for c in calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": c.id,
                            "name": c.function.name,
                            "arguments": c.function.arguments,
                        }));
                    }
                }
                let text = msg.content.as_text();
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}],
                    }));
                }
            }
            _ => {
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": msg.content.as_text()}],
                }));
            }
        }
    }

    // ChatGPT backend requires streaming and stateless operation; it does not
    // accept `max_output_tokens`.
    let mut body = json!({
        "model": model,
        "input": input,
        "store": false,
        "stream": true,
    });

    // The Codex Responses backend rejects requests without `instructions`
    // (HTTP 400 "Instructions are required"), so always send the field; fall
    // back to a neutral default when no system/developer message was provided.
    body["instructions"] = Value::String(if instructions.is_empty() {
        "You are a helpful assistant.".to_string()
    } else {
        instructions
    });
    if let Some(tier) = service_tier {
        body["service_tier"] = Value::String(tier.to_string());
    }
    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            let converted: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters,
                    })
                })
                .collect();
            body["tools"] = Value::Array(converted);
            body["tool_choice"] = Value::String("auto".to_string());
        }
    }
    if is_reasoning_model(&model) {
        body["reasoning"] = json!({"effort": "medium", "summary": "auto"});
    }

    body
}

/// Accumulate the Responses-API SSE body into a one-shot `CompletionResult`.
///
/// Text is taken from `output_text.delta` events; `output_item.done` supplies
/// function calls (and message text only as a fallback when no deltas arrived,
/// to avoid double-counting).
pub fn accumulate_sse(body: &str) -> Result<CompletionResult> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<ParsedToolCall> = Vec::new();
    let mut usage: Option<Usage> = None;

    for line in body.lines() {
        let data = match line.trim_start().strip_prefix("data:") {
            Some(d) => d.trim(),
            None => continue,
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let event: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "error" | "response.failed" => {
                let msg = event
                    .pointer("/error/message")
                    .or_else(|| event.pointer("/response/error/message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("ChatGPT backend returned an error");
                return Err(anyhow!("ChatGPT responses error: {}", msg));
            }
            "response.output_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    content.push_str(d);
                }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    reasoning.push_str(d);
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    accumulate_item(item, &mut content, &mut tool_calls);
                }
            }
            "response.completed" | "response.done" => {
                if let Some(u) = event.pointer("/response/usage") {
                    usage = parse_usage(u);
                }
            }
            _ => {}
        }
    }

    let finish_reason = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };

    Ok(CompletionResult {
        content,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        reasoning_signature: None,
        tool_calls,
        finish_reason: Some(finish_reason.to_string()),
        usage,
    })
}

fn accumulate_item(item: &Value, content: &mut String, tool_calls: &mut Vec<ParsedToolCall>) {
    match item.get("type").and_then(|v| v.as_str()) {
        Some("function_call") => {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let args_str = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let arguments =
                serde_json::from_str(args_str).unwrap_or_else(|_| Value::Object(Default::default()));
            tool_calls.push(ParsedToolCall { id, name, arguments });
        }
        Some("message")
            // Fallback only — text normally arrives via output_text.delta.
            if content.is_empty() => {
                if let Some(parts) = item.get("content").and_then(|v| v.as_array()) {
                    for p in parts {
                        let is_text = matches!(
                            p.get("type").and_then(|v| v.as_str()),
                            Some("output_text") | Some("text")
                        );
                        if is_text {
                            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                content.push_str(t);
                            }
                        }
                    }
                }
            }
        _ => {}
    }
}

fn parse_usage(u: &Value) -> Option<Usage> {
    let prompt = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let completion = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    Some(Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        ..Usage::default()
    })
}
