use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Bifrost Inference Client
///
/// Bifrost is an OpenAI-compatible API gateway: http://127.0.0.1:3360/v1
#[derive(Debug, Clone)]
pub struct BifrostClient {
    /// Base URL including /v1 (e.g. "http://127.0.0.1:3360/v1")
    base_url: String,
    /// Bearer token for auth
    api_key: String,
    /// Optional virtual key for x-bf-vk header
    virtual_key: String,
    /// Reqwest HTTP client
    client: reqwest::Client,
    /// Default model for chat
    default_model: String,
    /// Retry policy — configurable, eventually agent-adjustable.
    retry_policy: RetryPolicy,
}

/// A single part in the OpenAI content array (for multimodal messages).
/// When content is an array, each element has a `type` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlSource },
}

/// Source for an image URL content part — always a data URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlSource {
    pub url: String,
}

/// Content value that can be either a plain string or a multimodal part array.
/// Uses `#[serde(untagged)]` so both wire shapes deserialize correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentValue {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Default for ContentValue {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl ContentValue {
    /// Extract the text content: for `Text` returns the string directly;
    /// for `Parts`, joins all text parts together.
    pub fn as_text(&self) -> String {
        match self {
            ContentValue::Text(t) => t.clone(),
            ContentValue::Parts(parts) => {
                let mut text = String::new();
                for part in parts {
                    if let ContentPart::Text { text: t } = part {
                        text.push_str(t);
                    }
                }
                text
            }
        }
    }

    /// True if the content is empty (no text and no parts).
    pub fn is_empty(&self) -> bool {
        match self {
            ContentValue::Text(t) => t.is_empty(),
            ContentValue::Parts(parts) => parts.is_empty(),
        }
    }
}

/// A message in OpenAI chat format.
///
/// For plain user/assistant/system turns, only `role` and `content` are set
/// and the wire shape matches `{role, content}`. For tool-calling turns the
/// optional fields engage:
///
/// - assistant calling tools: `tool_calls = Some(...)`, `content` usually `""`
/// - tool result: `role = "tool"`, `tool_call_id = Some(id)`, `name = Some(fn)`
///
/// Skipping the empty optionals on the wire keeps unrelated providers happy.
///
/// For multimodal messages, `content` may be `ContentValue::Parts` — an array of
/// `ContentPart` variants (text + image_url parts) in the OpenAI format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: ContentValue,
    /// Tool calls emitted by the assistant (OpenAI tool-use schema).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<MessageToolCall>>,
    /// Links a `role: "tool"` message back to the assistant's call id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
    /// Function name for `role: "tool"` messages (some providers require it).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

impl Message {
    /// Plain text message — system / user / assistant without tool use.
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: ContentValue::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Multimodal user message with text + image content parts.
    pub fn multimodal_user(text: impl Into<String>, parts: Vec<ContentPart>) -> Self {
        Self {
            role: "user".to_string(),
            content: ContentValue::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Assistant message that called tools. `content` may be empty.
    pub fn assistant_tool_calls(content: impl Into<String>, calls: Vec<MessageToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: ContentValue::Text(content.into()),
            tool_calls: Some(calls),
            tool_call_id: None,
            name: None,
        }
    }

    /// Tool-result message bound to a prior assistant tool_call by id.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".to_string(),
            content: ContentValue::Text(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }
}

/// Tool call emitted by the assistant — serializable in OpenAI shape:
/// `{id, type: "function", function: {name, arguments: "<json-string>"}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: MessageToolCallFunction,
}

impl MessageToolCall {
    pub fn function(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tool_type: "function".to_string(),
            function: MessageToolCallFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolCallFunction {
    pub name: String,
    pub arguments: String,
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
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// A parsed tool call ready for execution
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Retry behavior for transient inference failures.
/// Defaults are conservative — the agent can request changes via
/// the memory system (e.g. writing to `system/dynamic/retry_policy.md`).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub fallback_models: Vec<String>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 6,
            base_delay_ms: 300,
            max_delay_ms: 12000,
            fallback_models: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum InferenceStrain {
    Transient {
        attempt: u32,
        status: u16,
        model: String,
        delay_ms: u64,
    },
    Exhausted {
        attempts: u32,
        status: u16,
        model: String,
        body: String,
    },
}

/// A Bifrost upstream timeout (`504` with `request_timed_out` / `"type":"timeout"`)
/// is *deterministic* — the same slow model on the same request will time out
/// again. Retrying it the full 6 times just multiplies one ~30s failure into a
/// multi-minute stall that was never going to succeed. Cap those at a single
/// retry (2 attempts total). Genuine transient blips — `503` overloaded,
/// connection resets, `429` — keep the full retry budget.
fn retry_cap(body: &str, max_retries: u32) -> u32 {
    let b = body.to_ascii_lowercase();
    if b.contains("request_timed_out") || b.contains("\"type\":\"timeout\"") {
        1
    } else {
        max_retries
    }
}

fn classify_status(status: reqwest::StatusCode, body: &str) -> ErrorClass {
    match status.as_u16() {
        429 => {
            if body.contains("quota") || body.contains("billing") || body.contains("exceeded") {
                ErrorClass::Permanent
            } else {
                ErrorClass::Transient
            }
        }
        500 | 502 | 503 | 504 => ErrorClass::Transient,
        408 => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

fn jittered_delay(attempt: u32, policy: &RetryPolicy) -> Duration {
    let base = policy.base_delay_ms * 2u64.pow(attempt);
    let capped = base.min(policy.max_delay_ms);
    let jitter = (capped as f64 * rand_jitter()) as u64;
    Duration::from_millis(capped.saturating_sub(jitter / 2) + jitter)
}

fn rand_jitter() -> f64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    (h.finish() % 1000) as f64 / 1000.0
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ErrorClass {
    Transient,
    Permanent,
}

impl BifrostClient {
    pub fn new(base_url: &str, api_key: &str, virtual_key: &str, default_model: &str, timeout_secs: u64) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let base_url = if base.ends_with("/v1") { base } else { format!("{}/v1", base) };

        info!(
            "🌉 Bifrost client initialized — model: {}, endpoint: {}, timeout: {}s",
            default_model, base_url, timeout_secs
        );
        Self {
            base_url,
            api_key: api_key.to_string(),
            virtual_key: virtual_key.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .expect("reqwest Client::builder() should never fail with static config"),
            default_model: default_model.to_string(),
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_fallbacks(mut self, fallbacks: Vec<String>) -> Self {
        self.retry_policy.fallback_models = fallbacks;
        self
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

    /// Send a non-streaming chat completion with retry on transient failures.
    ///
    /// Returns the completion result plus any strain events that occurred.
    /// Strain events are body-knowledge: the agent can feel when inference
    /// was difficult, correlate it over time, notice patterns.
    pub async fn chat_completion(&self, request: ChatCompletionRequest) -> Result<CompletionResult> {
        let (result, _strain) = self.chat_completion_with_strain(request).await?;
        Ok(result)
    }

    pub async fn chat_completion_with_strain(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<(CompletionResult, Vec<InferenceStrain>)> {
        let mut strain_events: Vec<InferenceStrain> = Vec::new();

        // Try primary model
        let fallbacks = self.retry_policy.fallback_models.clone();

        match self.try_model_with_retries(&request, &request.model, &mut strain_events).await {
            Ok(result) => return Ok((result, strain_events)),
            Err(primary_err) => {
                if fallbacks.is_empty() {
                    return Err(primary_err);
                }
                warn!(
                    "Primary model {} exhausted, trying {} fallback(s)",
                    request.model,
                    fallbacks.len()
                );
            }
        }

        // Try each fallback model
        for fallback in &fallbacks {
            info!("Falling back to model: {}", fallback);
            match self.try_model_with_retries(&request, fallback, &mut strain_events).await {
                Ok(result) => {
                    info!("Fallback to {} succeeded", fallback);
                    return Ok((result, strain_events));
                }
                Err(e) => {
                    warn!("Fallback model {} also failed: {}", fallback, e);
                }
            }
        }

        anyhow::bail!(
            "All models exhausted ({} + {} fallbacks). Last strain: {:?}",
            request.model,
            fallbacks.len(),
            strain_events.last()
        )
    }

    async fn try_model_with_retries(
        &self,
        request: &ChatCompletionRequest,
        model: &str,
        strain_events: &mut Vec<InferenceStrain>,
    ) -> Result<CompletionResult> {
        let url = format!("{}/chat/completions", self.base_url);
        let policy = &self.retry_policy;

        let mut req_with_model = request.clone();
        req_with_model.model = model.to_string();

        for attempt in 0..=policy.max_retries {
            debug!("POST {} — model: {} (attempt {})", url, model, attempt);

            let resp = self.client
                .post(&url)
                .headers(self.auth_headers())
                .json(&req_with_model)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if attempt == policy.max_retries {
                        anyhow::bail!("Bifrost unreachable after {} attempts: {}", attempt + 1, e);
                    }
                    let delay = jittered_delay(attempt, policy);
                    warn!("Bifrost connection failed (attempt {}), retrying in {:?}: {}", attempt, delay, e);
                    strain_events.push(InferenceStrain::Transient {
                        attempt,
                        status: 0,
                        model: model.to_string(),
                        delay_ms: delay.as_millis() as u64,
                    });
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();
            let retry_after = parse_retry_after(resp.headers());

            if status.is_success() {
                let body_text = resp.text().await
                    .context("Failed to read Bifrost response body")?;
                return Self::parse_completion_response(&body_text);
            }

            let body_text = resp.text().await
                .context("Failed to read Bifrost error body")?;

            match classify_status(status, &body_text) {
                ErrorClass::Transient if attempt < retry_cap(&body_text, policy.max_retries) => {
                    let delay = retry_after.unwrap_or_else(|| jittered_delay(attempt, policy));
                    warn!(
                        "Bifrost {} on {} (attempt {}), retrying in {:?}",
                        status.as_u16(), model, attempt, delay
                    );
                    strain_events.push(InferenceStrain::Transient {
                        attempt,
                        status: status.as_u16(),
                        model: model.to_string(),
                        delay_ms: delay.as_millis() as u64,
                    });
                    tokio::time::sleep(delay).await;
                }
                _ => {
                    strain_events.push(InferenceStrain::Exhausted {
                        attempts: attempt + 1,
                        status: status.as_u16(),
                        model: model.to_string(),
                        body: body_text[..body_text.len().min(300)].to_string(),
                    });
                    anyhow::bail!(
                        "Bifrost returned {} after {} attempt(s) on {}: {}",
                        status, attempt + 1, model, &body_text[..body_text.len().min(500)]
                    );
                }
            }
        }

        unreachable!("retry loop should have returned or bailed")
    }

    fn parse_completion_response(body_text: &str) -> Result<CompletionResult> {
        let parsed: ChatCompletionResponse = serde_json::from_str(body_text)
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
            finish_reason: choice.finish_reason.clone(),
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
            "http://127.0.0.1:3360",
            "sk-bf-test",
            "",
            "openai/deepseek-v4-pro",
            120,
        );
        assert!(client.base_url.ends_with("/v1"));
    }

    #[test]
    fn test_chat_request_serialization() {
        let req = ChatCompletionRequest {
            model: "openai/deepseek-v4-pro".to_string(),
            messages: vec![
                Message::text("user", "Hello"),
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
