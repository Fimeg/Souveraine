use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Bifrost Inference Client
///
/// Bifrost is an OpenAI-compatible API gateway: http://10.10.20.120:3360/v1
/// Uses Bearer token auth + optional x-bf-vk header for provider virtual keys.
/// OpenAI format for chat completions + tool calls.
#[derive(Debug, Clone)]
pub struct BifrostClient {
    /// Base URL including /v1 (e.g. "http://10.10.20.120:3360/v1")
    base_url: String,
    /// Bearer token for auth
    api_key: String,
    /// Optional virtual key for x-bf-vk header
    virtual_key: String,
    /// Reqwest HTTP client
    client: reqwest::Client,
    /// Default model for chat
    default_model: String,
}

/// A message in OpenAI chat format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// A tool definition in OpenAI format
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Chat completion request (OpenAI format)
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

/// Response from a non-streaming chat completion
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: u32,
    #[serde(default)]
    pub finish_reason: Option<String>,
    pub message: ResponseMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

/// Stream chunk from Bifrost (OpenAI SSE format)
#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunk {
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamToolCall {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub tool_type: Option<String>,
    pub function: Option<StreamToolCallFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// Parsed result from a chat completion (non-streaming)
#[derive(Debug, Clone)]
pub struct CompletionResult {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ParsedToolCall>,
    pub usage: Option<Usage>,
}

/// A parsed tool call ready for execution
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl BifrostClient {
    pub fn new(base_url: &str, api_key: &str, virtual_key: &str, default_model: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let base_url = if base.ends_with("/v1") { base } else { format!("{}/v1", base) };

        info!(
            "🌉 Bifrost client initialized — model: {}, endpoint: {}",
            default_model, base_url
        );
        Self {
            base_url,
            api_key: api_key.to_string(),
            virtual_key: virtual_key.to_string(),
            client: reqwest::Client::new(),
            default_model: default_model.to_string(),
        }
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if !self.api_key.is_empty() {
            let auth_val = format!("Bearer {}", self.api_key);
            headers.insert(reqwest::header::AUTHORIZATION, reqwest::header::HeaderValue::from_str(&auth_val).unwrap());
        }
        if !self.virtual_key.is_empty() {
            headers.insert("x-bf-vk", reqwest::header::HeaderValue::from_str(&self.virtual_key).unwrap());
        }
        headers
    }

    /// List available models from Bifrost
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.base_url);
        let resp = self.client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .with_context(|| "Failed to fetch Bifrost models")?;

        let body: serde_json::Value = resp.json().await?;
        let models = body["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(models)
    }

    /// Send a non-streaming chat completion
    pub async fn chat_completion(&self, request: ChatCompletionRequest) -> Result<CompletionResult> {
        let url = format!("{}/chat/completions", self.base_url);
        debug!("POST {} — model: {}", url, request.model);

        let resp = self.client
            .post(&url)
            .headers(self.auth_headers())
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Bifrost request failed: {}", url))?;

        let status = resp.status();
        let body_text = resp.text().await
            .context("Failed to read Bifrost response body")?;

        if !status.is_success() {
            anyhow::bail!("Bifrost returned {}: {}", status, &body_text[..body_text.len().min(500)]);
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&body_text)
            .with_context(|| {
                let preview = &body_text[..body_text.len().min(200)];
                format!("Failed to parse Bifrost response: {preview}")
            })?;

        let choice = parsed.choices.into_iter().next()
            .context("Bifrost returned empty choices")?;

        let content = choice.message.content.unwrap_or_default();
        let reasoning = choice.message.reasoning;
        let tool_calls = choice.message.tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments).ok()?;
                Some(ParsedToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: args,
                })
            })
            .collect();

        Ok(CompletionResult {
            content,
            reasoning,
            tool_calls,
            usage: parsed.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = BifrostClient::new(
            "http://10.10.20.120:3360",
            "sk-bf-test",
            "openai/deepseek-v4-pro",
        );
        assert!(client.base_url.ends_with("/v1"));
    }

    #[test]
    fn test_chat_request_serialization() {
        let req = ChatCompletionRequest {
            model: "openai/deepseek-v4-pro".to_string(),
            messages: vec![
                Message { role: "user".to_string(), content: "Hello".to_string() },
            ],
            stream: None,
            max_tokens: None,
            temperature: None,
            tools: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("openai/deepseek-v4-pro"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_chat_response_deserialize() {
        let json = r#"{
            "id": "test",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "Hello!",
                    "reasoning": "The user greeted me."
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            },
            "model": "deepseek-v4-pro"
        }"#;
        let resp: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
        assert_eq!(resp.choices[0].message.reasoning.as_deref(), Some("The user greeted me."));
    }

    #[test]
    fn test_tool_call_response_deserialize() {
        let json = r#"{
            "id": "test",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "index": 0,
                        "type": "function",
                        "id": "call_123",
                        "function": {
                            "name": "read",
                            "arguments": "{\"path\": \"/etc/hostname\"}"
                        }
                    }]
                }
            }],
            "usage": null,
            "model": "deepseek-v4-pro"
        }"#;
        let resp: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        let msg = &resp.choices[0].message;
        assert!(msg.tool_calls.is_some());
        let calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].function.name, "read");
    }
}
