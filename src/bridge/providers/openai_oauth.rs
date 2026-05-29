//! OpenAI as an OAuth-riding provider.
//!
//! Drives ChatGPT Plus/Pro/Team models through
//! `chatgpt.com/backend-api/codex/responses`, authenticated by the Codex CLI's
//! OAuth token (no API key). Translates the engine's OpenAI-chat request into
//! the Responses API, accumulates the SSE stream back into a `CompletionResult`,
//! and synthesizes `InferenceStrain` from retries so the body still feels a
//! hoarse provider.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::warn;

use crate::bridge::bifrost::{ChatCompletionRequest, CompletionResult, InferenceStrain, RetryPolicy};
use crate::bridge::oauth::codex_creds::{self, CodexCredentials};
use crate::bridge::oauth::{catalog, refresh};
use crate::bridge::provider::LlmProvider;

use super::responses;

const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

pub struct OpenAiOAuthProvider {
    http: reqwest::Client,
    /// The mutex serializes refreshes — a one-time-use refresh token must not be
    /// spent by two concurrent calls (single-flight).
    creds: Arc<Mutex<CodexCredentials>>,
    default_model: String,
    retry: RetryPolicy,
}

impl OpenAiOAuthProvider {
    /// Build by riding the Codex CLI's existing ChatGPT login.
    pub fn from_codex_login(default_model: String, timeout_secs: u64) -> Result<Self> {
        let creds = codex_creds::read()
            .context("reading Codex CLI ChatGPT login (run `codex login` first)")?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("reqwest Client::builder() should never fail with static config");
        // The configured `primary_model` may be a Bifrost-namespaced id
        // (`openai/…`); pin the provider default to a model this backend serves.
        let default_model = catalog::resolve(&default_model, catalog::DEFAULT_MODEL);
        Ok(Self {
            http,
            creds: Arc::new(Mutex::new(creds)),
            default_model,
            retry: RetryPolicy::default(),
        })
    }

    /// Refresh if needed and return a usable `(access_token, account_id)`.
    async fn auth(&self) -> Result<(String, String)> {
        let mut creds = self.creds.lock().await;
        refresh::ensure_fresh(&self.http, &mut creds).await?;
        Ok((creds.access_token.clone(), creds.account_id.clone()))
    }
}

#[async_trait]
impl LlmProvider for OpenAiOAuthProvider {
    fn id(&self) -> &str {
        "openai-oauth"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(catalog::model_ids())
    }

    async fn chat_completion_with_strain(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<(CompletionResult, Vec<InferenceStrain>)> {
        // Translate the engine's (possibly Bifrost-namespaced) model id onto a
        // model the codex backend actually serves before building the payload.
        let mut request = request;
        request.model = catalog::resolve(&request.model, &self.default_model);
        let payload = responses::build_payload(&request);
        let model = request.model.clone();
        let mut strain: Vec<InferenceStrain> = Vec::new();

        for attempt in 0..=self.retry.max_retries {
            let (access, account) = self.auth().await?;

            let resp = self
                .http
                .post(ENDPOINT)
                .header("Authorization", format!("Bearer {}", access))
                .header("ChatGPT-Account-Id", account)
                .header("OpenAI-Beta", "responses=v1")
                .header("OpenAI-Originator", "codex")
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if attempt == self.retry.max_retries {
                        return Err(anyhow!(
                            "ChatGPT backend unreachable after {} attempts: {}",
                            attempt + 1,
                            e
                        ));
                    }
                    let delay = backoff(attempt, &self.retry);
                    warn!("ChatGPT backend connect failed (attempt {}), retrying in {:?}: {}", attempt, delay, e);
                    strain.push(InferenceStrain::Transient {
                        attempt,
                        status: 0,
                        model: model.clone(),
                        delay_ms: delay.as_millis() as u64,
                    });
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();
            if status.is_success() {
                let body = resp.text().await.context("reading ChatGPT responses stream")?;
                let result = responses::accumulate_sse(&body)?;
                return Ok((result, strain));
            }

            let body = resp.text().await.unwrap_or_default();
            let transient = matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504);
            if transient && attempt < self.retry.max_retries {
                let delay = backoff(attempt, &self.retry);
                warn!("ChatGPT backend {} (attempt {}), retrying in {:?}", status.as_u16(), attempt, delay);
                strain.push(InferenceStrain::Transient {
                    attempt,
                    status: status.as_u16(),
                    model: model.clone(),
                    delay_ms: delay.as_millis() as u64,
                });
                tokio::time::sleep(delay).await;
                continue;
            }

            strain.push(InferenceStrain::Exhausted {
                attempts: attempt + 1,
                status: status.as_u16(),
                model: model.clone(),
                body: body.chars().take(300).collect(),
            });
            return Err(anyhow!(
                "ChatGPT backend returned {} after {} attempt(s): {}",
                status,
                attempt + 1,
                body.chars().take(500).collect::<String>()
            ));
        }

        unreachable!("retry loop returns or bails")
    }
}

fn backoff(attempt: u32, policy: &RetryPolicy) -> Duration {
    let base = policy.base_delay_ms.saturating_mul(2u64.saturating_pow(attempt));
    Duration::from_millis(base.min(policy.max_delay_ms))
}
