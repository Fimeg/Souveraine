#![allow(dead_code)] // WIP scaffolding not yet wired
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::bridge::provider::LlmProvider;

/// Client for any endpoint speaking the OpenAI `/chat/completions` shape —
/// DeepSeek, z.ai, Google's OpenAI-compat root, a local llama-swap, and the
/// Bifrost gateway among them. It is not specific to Bifrost, and its logs
/// must not imply a request traversed that service.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleClient {
    /// Base URL including its version path (e.g. "http://127.0.0.1:3360/v1"
    /// or "https://api.z.ai/api/coding/paas/v4").
    base_url: String,
    /// Configured `[providers.<id>]` name — what logs and errors call this
    /// endpoint, so a failure names the provider the operator configured.
    id: String,
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
    Text {
        text: String,
    },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: ImageUrlSource,
    },
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
    /// Signed thinking block from the assistant turn, carried so it can be
    /// replayed. Anthropic requires the thinking block that preceded a
    /// `tool_use` to come back with it, byte-identical and with its signature,
    /// or the follow-up request is rejected. Only providers that return a
    /// signed block populate this; it is skipped on the wire when absent, so
    /// OpenAI-shaped gateways never see it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thinking: Option<ThinkingBlock>,
}

/// An Anthropic thinking block preserved verbatim for replay. Both halves are
/// required — text without its signature cannot be replayed, so this is only
/// ever constructed when the provider returned both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingBlock {
    pub text: String,
    pub signature: String,
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
            thinking: None,
        }
    }

    /// Multimodal message with text + image content parts, under any role.
    ///
    /// The role must be carried: an assistant message holding an image
    /// replayed as `user` reads to the model as though the human said it.
    pub fn multimodal(role: impl Into<String>, parts: Vec<ContentPart>) -> Self {
        Self {
            role: role.into(),
            content: ContentValue::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            thinking: None,
        }
    }

    /// Multimodal user message with text + image content parts.
    ///
    /// The text argument is ignored — the parts array is the whole content.
    pub fn multimodal_user(_text: impl Into<String>, parts: Vec<ContentPart>) -> Self {
        Self::multimodal("user", parts)
    }

    /// Assistant message that called tools. `content` may be empty.
    pub fn assistant_tool_calls(content: impl Into<String>, calls: Vec<MessageToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: ContentValue::Text(content.into()),
            tool_calls: Some(calls),
            tool_call_id: None,
            name: None,
            thinking: None,
        }
    }

    /// Attach a signed thinking block for replay. No-op unless both the text
    /// and the signature are present.
    pub fn with_thinking(
        mut self,
        text: Option<impl Into<String>>,
        signature: Option<impl Into<String>>,
    ) -> Self {
        if let (Some(text), Some(signature)) = (text, signature) {
            self.thinking = Some(ThinkingBlock {
                text: text.into(),
                signature: signature.into(),
            });
        }
        self
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
            thinking: None,
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
    pub fn function(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
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
    // OpenRouter says "reasoning"; DeepSeek and z.ai say "reasoning_content".
    #[serde(default, alias = "reasoning_content")]
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    /// Prefix served from cache. Anthropic reports the uncached remainder as
    /// `input_tokens`, so these two are the only way to tell a cache hit from a
    /// short prompt — without them there is no way to know caching works.
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_write_tokens: u32,
}

/// Stream chunk in OpenAI SSE format
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
    // OpenRouter says "reasoning"; DeepSeek and z.ai say "reasoning_content".
    #[serde(default, alias = "reasoning_content")]
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
    /// Opaque signature for the thinking block in `reasoning`. Anthropic
    /// returns it and requires it back verbatim when the same assistant turn
    /// carried a `tool_use`. `None` for providers that don't sign thinking.
    pub reasoning_signature: Option<String>,
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

/// An upstream timeout (`504` with `request_timed_out` / `"type":"timeout"`, as
/// the Bifrost gateway reports it) is *deterministic* — the same slow model on
/// the same request will time out
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

/// Longest `retry-after` still worth sleeping on rather than failing the call.
///
/// A window this long is a spent quota, not a burst limit, and it does not
/// clear inside a retry loop. Measured 2026-08-16: an `11997` header put the
/// subconscious pass to sleep for 3h20m with no cancel path and nothing on the
/// wire to say it was waiting rather than dead.
///
/// `claude_subscription.rs` learned this first and answers it by rotating to
/// another login, so there an *absent* `retry-after` also counts as exhausted.
/// This client has nothing to rotate to, so an absent header keeps the ordinary
/// jittered backoff — only an explicitly long one fails fast.
const BURST_RETRY_CEILING_SECS: u64 = 60;

fn is_exhausted_window(status: reqwest::StatusCode, retry_after: Option<Duration>) -> bool {
    status.as_u16() == 429
        && retry_after.is_some_and(|d| d.as_secs() > BURST_RETRY_CEILING_SECS)
}

/// Name the window in the error, so a failure says how long the provider
/// wanted rather than only that it refused.
fn reset_hint(retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(d) => format!(" (provider asked for {}s)", d.as_secs()),
        None => String::new(),
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

/// True if the base URL carries a path past the authority. Such a base is the
/// complete API root; only a bare host gets the OpenAI default `/v1`.
///
/// Measured 2026-08-18: testing only the last segment for `v<digits>` appended
/// `/v1` to Google's `/v1beta/openai`, and a POST to the doubled path hangs
/// (40s, zero bytes) while `GET /models` returns 200 on both — so the mangled
/// base looks healthy on a listing.
fn has_own_path(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| u.path() != "/")
}

impl OpenAiCompatibleClient {
    pub fn new(
        id: &str,
        base_url: &str,
        api_key: &str,
        virtual_key: &str,
        default_model: &str,
        timeout_secs: u64,
    ) -> Result<Self> {
        let base = base_url.trim_end_matches('/').to_string();
        let base_url = if has_own_path(&base) {
            base
        } else {
            format!("{}/v1", base)
        };

        info!(
            "provider {} ready — model: {}, endpoint: {}, timeout: {}s",
            id, default_model, base_url, timeout_secs
        );
        Ok(Self {
            base_url,
            id: id.to_string(),
            api_key: api_key.to_string(),
            virtual_key: virtual_key.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .with_context(|| format!("building reqwest client for provider {id}"))?,
            default_model: default_model.to_string(),
            retry_policy: RetryPolicy::default(),
        })
    }

    pub fn with_fallbacks(mut self, fallbacks: Vec<String>) -> Self {
        self.retry_policy.fallback_models = fallbacks;
        self
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if !self.api_key.is_empty() {
            let auth_val = format!("Bearer {}", self.api_key);
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&auth_val).unwrap(),
            );
        }
        if !self.virtual_key.is_empty() {
            headers.insert(
                "x-bf-vk",
                reqwest::header::HeaderValue::from_str(&self.virtual_key).unwrap(),
            );
        }
        headers
    }

    /// List the models this provider advertises at `/models`.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .with_context(|| format!("provider {} — failed to fetch models", self.id))?;

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
    pub async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<CompletionResult> {
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

        match self
            .try_model_with_retries(&request, &request.model, &mut strain_events)
            .await
        {
            Ok(result) => return Ok((result, strain_events)),
            Err(primary_err) => {
                if fallbacks.is_empty() {
                    return Err(primary_err);
                }
                warn!(
                    "Primary model {} exhausted (error: {}), trying {} fallback(s)",
                    request.model,
                    primary_err,
                    fallbacks.len()
                );
            }
        }

        // Try each fallback model
        for fallback in &fallbacks {
            info!("Falling back to model: {}", fallback);
            match self
                .try_model_with_retries(&request, fallback, &mut strain_events)
                .await
            {
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

            let resp = self
                .client
                .post(&url)
                .headers(self.auth_headers())
                .json(&req_with_model)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if attempt == policy.max_retries {
                        anyhow::bail!(
                            "provider {} unreachable at {} after {} attempts: {}",
                            self.id,
                            url,
                            attempt + 1,
                            e
                        );
                    }
                    let delay = jittered_delay(attempt, policy);
                    warn!(
                        "provider {} connection failed at {} (attempt {}), retrying in {:?}: {}",
                        self.id, url, attempt, delay, e
                    );
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
                let body_text = resp
                    .text()
                    .await
                    .with_context(|| format!("provider {} — unreadable response body", self.id))?;
                return self.parse_completion_response(&body_text);
            }

            let body_text = resp
                .text()
                .await
                .with_context(|| format!("provider {} — unreadable error body", self.id))?;

            match classify_status(status, &body_text) {
                ErrorClass::Transient
                    if attempt < retry_cap(&body_text, policy.max_retries)
                        && !is_exhausted_window(status, retry_after) =>
                {
                    let delay = retry_after.unwrap_or_else(|| jittered_delay(attempt, policy));
                    warn!(
                        "provider {} returned {} on {} (attempt {}), retrying in {:?}",
                        self.id,
                        status.as_u16(),
                        model,
                        attempt,
                        delay
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
                        "provider {} returned {} after {} attempt(s) on {}{}: {}",
                        self.id,
                        status,
                        attempt + 1,
                        model,
                        reset_hint(retry_after),
                        &body_text[..body_text.len().min(500)]
                    );
                }
            }
        }

        anyhow::bail!(
            "provider {} exhausted its retry loop without returning a result",
            self.id
        )
    }

    fn parse_completion_response(&self, body_text: &str) -> Result<CompletionResult> {
        let parsed: ChatCompletionResponse =
            serde_json::from_str(body_text).with_context(|| {
                let preview = &body_text[..body_text.len().min(200)];
                format!("provider {} — unparseable response: {preview}", self.id)
            })?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .with_context(|| format!("provider {} returned empty choices", self.id))?;

        let content = choice.message.content.unwrap_or_default();
        let reasoning = choice.message.reasoning;
        let tool_calls = choice
            .message
            .tool_calls
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
            // OpenAI-shaped gateways return reasoning text unsigned.
            reasoning_signature: None,
            tool_calls,
            finish_reason: choice.finish_reason.clone(),
            usage: parsed.usage,
        })
    }
}

/// `OpenAiCompatibleClient` is the OpenAI-compatible gateway implementation of the
/// provider seam. The inherent methods do the work; the trait just exposes them
/// behind `dyn LlmProvider` so the engine can hold any provider uniformly.
#[async_trait]
impl LlmProvider for OpenAiCompatibleClient {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        OpenAiCompatibleClient::list_models(self).await
    }

    async fn chat_completion_with_strain(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<(CompletionResult, Vec<InferenceStrain>)> {
        OpenAiCompatibleClient::chat_completion_with_strain(self, request).await
    }

    async fn chat_completion(&self, request: ChatCompletionRequest) -> Result<CompletionResult> {
        OpenAiCompatibleClient::chat_completion(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_window_rejects_the_measured_11997s_header() {
        // The header that put the subconscious to sleep for 3h20m on
        // 2026-08-16. It must not be slept on.
        let long = Some(Duration::from_secs(11997));
        assert!(is_exhausted_window(reqwest::StatusCode::TOO_MANY_REQUESTS, long));
    }

    #[test]
    fn exhausted_window_still_sleeps_on_a_real_burst_limit() {
        let short = Some(Duration::from_secs(30));
        assert!(!is_exhausted_window(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            short
        ));
        // Exactly at the ceiling is still a burst, not a spent window.
        let at_ceiling = Some(Duration::from_secs(BURST_RETRY_CEILING_SECS));
        assert!(!is_exhausted_window(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            at_ceiling
        ));
    }

    #[test]
    fn exhausted_window_ignores_absent_headers_and_other_statuses() {
        // This client has no account to rotate to, so a 429 with no header keeps
        // the ordinary jittered backoff — this is where it diverges from
        // `claude_subscription::is_exhausted_window` on purpose.
        assert!(!is_exhausted_window(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            None
        ));
        assert!(!is_exhausted_window(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            Some(Duration::from_secs(11997))
        ));
    }

    #[test]
    fn reset_hint_names_the_window_or_says_nothing() {
        assert_eq!(
            reset_hint(Some(Duration::from_secs(11997))),
            " (provider asked for 11997s)"
        );
        assert_eq!(reset_hint(None), "");
    }

    /// Google's OpenAI-compatible root nests its version: `/v1beta/openai`.
    /// Appending `/v1` there produced a path that hangs on POST.
    #[test]
    fn google_nested_version_path_is_not_doubled() {
        let client = OpenAiCompatibleClient::new(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "",
            "",
            "gemini-3.6-flash",
            300,
        )
        .unwrap();
        assert_eq!(
            client.base_url,
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
    }

    /// Every `[providers.*]` base URL on disk 2026-08-18 and the API root the
    /// client must end up posting to.
    #[test]
    fn base_url_completion_across_configured_providers() {
        let cases = [
            // Paths of their own — taken as complete, nested version or not.
            (
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "https://generativelanguage.googleapis.com/v1beta/openai",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/openai/",
                "https://generativelanguage.googleapis.com/v1beta/openai",
            ),
            (
                "https://api.z.ai/api/coding/paas/v4",
                "https://api.z.ai/api/coding/paas/v4",
            ),
            (
                "https://token-plan-sgp.xiaomimimo.com/v1",
                "https://token-plan-sgp.xiaomimimo.com/v1",
            ),
            ("https://opencode.ai/zen/go/v1", "https://opencode.ai/zen/go/v1"),
            // Bare hosts — get the OpenAI default.
            ("https://api.deepseek.com", "https://api.deepseek.com/v1"),
            ("https://api.deepseek.com/", "https://api.deepseek.com/v1"),
            ("http://localhost:3360", "http://localhost:3360/v1"),
            ("http://localhost:8080", "http://localhost:8080/v1"),
        ];
        for (base, want) in cases {
            let client = OpenAiCompatibleClient::new("p", base, "", "", "m", 30).unwrap();
            assert_eq!(client.base_url, want, "base {base}");
        }
    }

    #[test]
    fn test_client_creation() {
        let client = OpenAiCompatibleClient::new(
            "bifrost",
            "http://127.0.0.1:3360",
            "sk-bf-test",
            "",
            "openai/deepseek-v4-pro",
            120,
        )
        .unwrap();
        assert!(client.base_url.ends_with("/v1"));
        assert_eq!(client.id, "bifrost");
    }

    #[test]
    fn test_chat_request_serialization() {
        let req = ChatCompletionRequest {
            model: "openai/deepseek-v4-pro".to_string(),
            messages: vec![Message::text("user", "Hello")],
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
        assert_eq!(
            resp.choices[0].message.reasoning.as_deref(),
            Some("The user greeted me.")
        );
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
