#![allow(dead_code)] // WIP scaffolding not yet wired
//! Claude Code subscription inference provider.
//!
//! Speaks the Claude Code (claude.ai OAuth) wire protocol directly against
//! `api.anthropic.com`, using the OAuth credentials Claude Code already stored
//! at `~/.claude/.credentials.json`. Implements [`LlmProvider`] so it drops
//! into the existing provider registry alongside the OpenAI-compatible client and OpenAI OAuth.
//!
//! Internally it translates OpenAI chat-completion requests (Souveraine's
//! internal shape) to Anthropic `/v1/messages`, applies the subscription wire
//! shaping (attribution block + fingerprint, `metadata.user_id`, betas, identity
//! headers), refreshes the OAuth token under a mutex (writing it back so
//! `claude` stays in sync), and translates the Anthropic response back to
//! Souveraine's OpenAI-shaped `CompletionResult`.
//!
//! Wire constants and rules trace to the Claude Code subscription wire protocol.
//! On the wire the client presents as Claude Code because Anthropic's backend
//! validates the client fingerprint/salt against the first-party client to gate
//! subscription access. It uses YOUR token, from YOUR login, on YOUR machine.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use async_trait::async_trait;
use rand::RngCore;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::openai_compatible::{
    ChatCompletionRequest, CompletionResult, InferenceStrain, Message, ParsedToolCall,
    ToolDefinition, Usage,
};
use super::provider::LlmProvider;

// ── Claude Code subscription wire constants ───────────────────────────────
const PROD_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ROLES_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/roles";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const CC_PRODUCT_BETA: &str = "claude-code-20250219";
const SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const FINGERPRINT_SALT: &str = "59cf53e54c78";
const CC_ENTRYPOINT: &str = "cli";
const CC_PLATFORM: &str = "claude_code_cli";
const FALLBACK_CC_VERSION: &str = "2.1.196";
const TOKEN_REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);
/// Longest `retry-after` still worth waiting out on the account already holding
/// the cached prefix, instead of moving to another login.
const BURST_RETRY_CEILING_SECS: u64 = 60;
/// How long a spent login waits when the response says nothing about its window.
/// One wasted call a quarter-hour is cheaper than parking a live login for the
/// five hours the window actually runs.
const DEFAULT_WINDOW_COOLDOWN: Duration = Duration::from_secs(15 * 60);
const DEFAULT_MAX_TOKENS: u32 = 8192;
/// Models that think by default cap thinking *plus* response text with
/// `max_tokens`, so 8192 truncates mid-answer.
const THINKING_MAX_TOKENS: u32 = 32000;

/// Models that removed the sampling parameters. Sending `temperature`,
/// `top_p` or `top_k` to one of these is a 400, so they are dropped rather
/// than passed through.
fn rejects_sampling_params(model: &str) -> bool {
    const REMOVED: &[&str] = &[
        "claude-opus-5",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-sonnet-5",
        "claude-fable-5",
        "claude-mythos-5",
        "claude-mythos-preview",
    ];
    REMOVED.iter().any(|m| model.starts_with(m))
}

/// Models where omitting `thinking` still runs adaptive thinking, which shares
/// the `max_tokens` budget with the visible response.
fn thinks_by_default(model: &str) -> bool {
    const THINKS: &[&str] = &[
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-fable-5",
        "claude-mythos-5",
    ];
    THINKS.iter().any(|m| model.starts_with(m))
}

/// Models that accept a `role: "system"` turn inside `messages`. Everywhere
/// else a mid-conversation note has to be folded into the user turn, which
/// costs the operator role but keeps the cached prefix intact either way.
/// Sonnet 5 is deliberately absent — it 400s on the role.
fn supports_mid_conversation_system(model: &str) -> bool {
    const SUPPORTED: &[&str] = &[
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-fable-5",
        "claude-mythos-5",
    ];
    SUPPORTED.iter().any(|m| model.starts_with(m))
}

/// Direct Claude Code subscription inference provider.
pub struct ClaudeSubscriptionProvider {
    name: String,
    http: reqwest::Client,
    upstream_base: String,
    cc_version: String,
    user_agent: String,
    device_id: String,
    account_uuid: String,
    extra_metadata: Map<String, Value>,
    default_model: String,
    accounts: Arc<Mutex<Accounts>>,
}

/// Every login this provider may speak as, and which one it is speaking as now.
///
/// One subscription is one quota. A second login is the only thing that keeps
/// answering once the first is spent, and it has to be held *alongside* the
/// first — a token swapped in by hand arrives after the conversation has
/// already failed, and swapping back and forth throws away the account's cached
/// prefix each way.
struct Accounts {
    tokens: Vec<TokenState>,
    active: usize,
}

#[derive(Clone)]
struct TokenState {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_at_ms: Option<u64>,
    source_path: Option<PathBuf>,
    /// When this login's spent quota window comes back. Read from the response
    /// that reported it spent; until then the login is not tried.
    resumes_at_ms: Option<u64>,
}

impl TokenState {
    fn has_credential(&self) -> bool {
        let live = |t: &Option<String>| t.as_deref().is_some_and(|t| !t.is_empty());
        live(&self.access_token) || live(&self.refresh_token)
    }

    /// Worth trying: something to present, and no spent window still running.
    fn is_available(&self, now: u64) -> bool {
        self.has_credential() && self.resumes_at_ms.is_none_or(|at| now >= at)
    }

    fn access_is_live(&self, now: u64) -> bool {
        self.access_token.as_deref().is_some_and(|t| !t.is_empty())
            && self.expires_at_ms.is_some_and(|exp| exp > now)
    }

    fn label(&self) -> String {
        match &self.source_path {
            Some(p) => p.display().to_string(),
            None => "<unnamed>".to_string(),
        }
    }
}

impl ClaudeSubscriptionProvider {
    /// Build from a provider config entry. Reads `~/.claude/.credentials.json`
    /// (or the `credential_file` override), fetches `account_uuid` best-effort,
    /// and persists a device id.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        base_url: &str,
        primary_model: &str,
        timeout_secs: u64,
        credential_file: Option<&str>,
        credential_files: Option<&[String]>,
        cc_version: Option<&str>,
        account_uuid: Option<&str>,
        device_id: Option<&str>,
        extra_metadata: Option<HashMap<String, Value>>,
    ) -> Result<Self> {
        let cc_version = cc_version
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_else(resolve_cc_version);
        let user_agent = format!("claude-cli/{cc_version} (undefined, {CC_ENTRYPOINT})");

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent(user_agent.clone())
            .build()
            .context("failed to build HTTP client")?;

        let cred_paths = resolve_credential_paths(credential_file, credential_files);
        if cred_paths.is_empty() {
            anyhow::bail!(
                "no Claude Code credential file found (set credential_file or log in with `claude`)"
            );
        }
        let mut tokens = Vec::new();
        for path in &cred_paths {
            match load_credentials(path) {
                Ok(token) if token.has_credential() => tokens.push(token),
                Ok(_) => warn!("{} has no tokens; skipping", path.display()),
                Err(e) => warn!("{} unreadable; skipping: {e}", path.display()),
            }
        }
        let (cred_path, cred_id) = {
            let token = tokens.first().ok_or_else(|| {
                anyhow::anyhow!(
                    "no usable Claude Code login in {}; log in with `claude` first",
                    cred_paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            (token.label(), credential_fingerprint(token))
        };
        let spares = tokens.len() - 1;

        let device_id = match device_id.filter(|v| is_device_id(v)) {
            Some(id) => id.to_ascii_lowercase(),
            None => get_or_create_device_id()?,
        };

        let account_uuid = account_uuid
            .filter(|v| !v.is_empty())
            .unwrap_or_default()
            .to_string();

        let extra = extra_metadata
            .map(|m| m.into_iter().collect::<Map<String, Value>>())
            .unwrap_or_default();

        info!(
            "🔐 Claude subscription provider initialized — model: {}, cc_version: {}, creds: {} (cred {}, {} spare login(s))",
            primary_model, cc_version, cred_path, cred_id, spares,
        );

        Ok(Self {
            name: name.to_string(),
            http,
            upstream_base: base_url.trim_end_matches('/').to_string(),
            cc_version,
            user_agent,
            device_id,
            account_uuid,
            extra_metadata: extra,
            default_model: primary_model.to_string(),
            accounts: Arc::new(Mutex::new(Accounts { tokens, active: 0 })),
        })
    }

    fn beta_header(&self) -> String {
        format!("{OAUTH_BETA},{CC_PRODUCT_BETA}")
    }

    /// Translate an OpenAI chat-completion request to a wire-shaped Anthropic
    /// `/v1/messages` body.
    fn shape_body(&self, model: &str, request: &ChatCompletionRequest) -> Result<Vec<u8>> {
        let (messages, system_text, first_user_text) =
            translate_messages(&request.messages, supports_mid_conversation_system(model));
        let default_max_tokens = if thinks_by_default(model) {
            THINKING_MAX_TOKENS
        } else {
            DEFAULT_MAX_TOKENS
        };
        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(default_max_tokens),
            "stream": false,
        });
        if let Some(t) = request.temperature {
            if !rejects_sampling_params(model) {
                body["temperature"] = json!(t);
            }
        }
        if thinks_by_default(model) {
            // Thinking runs either way on these models; the default display is
            // "omitted", which returns signed blocks whose text is empty and
            // leaves the reasoning pane blank. Ask for the summary instead.
            body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
        }
        if let Some(system) = system_text {
            body["system"] = json!(system);
        }
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools.iter().map(translate_tool).collect::<Vec<_>>());
            }
        }

        let obj = body.as_object_mut().unwrap();
        inject_attribution(obj, &self.cc_version, &first_user_text);
        inject_metadata(obj, self);
        mark_cache_breakpoints(obj);

        Ok(serde_json::to_vec(&body)?)
    }

    /// Return a usable access token and the account it belongs to, refreshing
    /// under the mutex if it is missing or inside the proactive window.
    async fn current_access_token(&self) -> Result<(String, usize)> {
        let mut accounts = self.accounts.lock().await;
        let active = accounts.active;
        let token = &mut accounts.tokens[active];
        adopt_file_if_changed(token);
        let access_empty = token.access_token.as_deref().is_none_or(str::is_empty);
        let expiring = token
            .expires_at_ms
            .is_none_or(|exp| exp <= now_ms() + TOKEN_REFRESH_BUFFER.as_millis() as u64);
        if access_empty || expiring {
            if let Err(e) = refresh_token(&self.http, &self.user_agent, token).await {
                // Refresh tokens are one-time-use, and this credential file is
                // shared with `claude` itself. Whichever process refreshes
                // first rotates the token out from under the other, so a
                // refresh failing here usually means our copy — read once at
                // startup — is simply the spent half, not that anything was
                // revoked. Re-read the file before giving up: if it now holds a
                // newer, still-valid token, adopt it. Without this the provider
                // 400s on every turn until the service is restarted by hand.
                match reread_credentials(token) {
                    Some(rotated) => {
                        info!("claude token refresh failed ({e}); adopted rotated credentials");
                        *token = rotated;
                    }
                    None => warn!("claude token refresh failed: {e}"),
                }
            }
        }
        let access = token
            .access_token
            .clone()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("no claude access_token available (refresh failed and none cached)")
            })?;
        Ok((access, active))
    }

    /// Move off the account at `from`, which answered `status` and is not worth
    /// trying again until `resumes_at`.
    ///
    /// Sticky by design: the next login is used until it too is exhausted,
    /// rather than alternating. Switching accounts abandons a prompt cache the
    /// transcript has already paid for — measured 2026-08-12 at ~125k cached
    /// tokens a turn — so a swap has to be worth a full re-bill, and only an
    /// exhausted window is — which is also why nothing walks back to a better
    /// login mid-conversation. The way home is this same round-robin: it wraps
    /// to the primary, and takes it once its recorded window has passed.
    async fn rotate_account(&self, from: usize, status: u16, resumes_at: u64) -> Option<usize> {
        let mut accounts = self.accounts.lock().await;
        accounts.tokens[from].resumes_at_ms = Some(resumes_at);
        let count = accounts.tokens.len();
        if count < 2 {
            return None;
        }
        // Another in-flight request already moved us; ride along rather than
        // stepping past a login nobody has tried yet.
        if accounts.active != from {
            return Some(accounts.active);
        }
        let now = now_ms();
        for step in 1..count {
            let candidate = (from + step) % count;
            adopt_file_if_changed(&mut accounts.tokens[candidate]);
            if !accounts.tokens[candidate].is_available(now) {
                continue;
            }
            let label = accounts.tokens[candidate].label();
            if !self.can_authenticate(&mut accounts.tokens[candidate]).await {
                warn!("claude login {label} cannot authenticate; skipping");
                continue;
            }
            info!(
                "claude login {} answered {status}; continuing as {label}",
                accounts.tokens[from].label(),
            );
            accounts.active = candidate;
            return Some(candidate);
        }
        None
    }

    /// Can this login still speak — an unexpired token, or a refresh the
    /// endpoint accepts?
    ///
    /// `has_credential` only asks whether a string is present. On 2026-08-17
    /// that sent the provider onto a spare whose token had expired four days
    /// earlier: every call after it 401'd as *revoked*, which reads as the
    /// account being gone rather than the spare being dead.
    async fn can_authenticate(&self, token: &mut TokenState) -> bool {
        token.access_is_live(now_ms())
            || refresh_token(&self.http, &self.user_agent, token)
                .await
                .is_ok()
    }
}

#[async_trait]
impl LlmProvider for ClaudeSubscriptionProvider {
    fn id(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let (access_token, _) = self.current_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/v1/models", self.upstream_base))
            .header("authorization", format!("Bearer {access_token}"))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", self.beta_header())
            .header("user-agent", &self.user_agent)
            .header("x-app", CC_ENTRYPOINT)
            .header("anthropic-client-platform", CC_PLATFORM)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let models = resp
            .get("data")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(models)
    }

    async fn chat_completion_with_strain(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<(CompletionResult, Vec<InferenceStrain>)> {
        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };
        let mut strain = Vec::new();
        let max_attempts = 4u32;

        let body = self.shape_body(&model, &request)?;

        for attempt in 0..max_attempts {
            let (access_token, account) = self.current_access_token().await?;
            let resp = self
                .http
                .post(format!("{}/v1/messages", self.upstream_base))
                .header("authorization", format!("Bearer {access_token}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", self.beta_header())
                .header("user-agent", &self.user_agent)
                .header("x-app", CC_ENTRYPOINT)
                .header("anthropic-client-platform", CC_PLATFORM)
                .body(body.clone())
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if attempt + 1 == max_attempts {
                        anyhow::bail!(
                            "claude subscription unreachable after {max_attempts} attempts: {e}"
                        );
                    }
                    let delay = backoff(attempt);
                    warn!(
                        "claude subscription connect failed (attempt {}), retry in {:?}: {e}",
                        attempt + 1,
                        delay
                    );
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
                let text = resp.text().await.context("reading claude response body")?;
                let result = parse_completion(&text)?;
                if !completion_has_payload(&result) {
                    let shape = empty_completion_shape(&text);
                    if attempt + 1 < max_attempts {
                        let delay = backoff(attempt);
                        warn!(
                            "claude subscription returned empty success on {model} ({shape}, attempt {}), retry in {:?}",
                            attempt + 1,
                            delay
                        );
                        strain.push(InferenceStrain::Transient {
                            attempt,
                            // The transport succeeded; the body did not. Keep
                            // the actual HTTP status instead of inventing one.
                            status: status.as_u16(),
                            model: model.clone(),
                            delay_ms: delay.as_millis() as u64,
                        });
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    anyhow::bail!(
                        "claude subscription returned an empty completion after {max_attempts} attempts on {model} ({shape})"
                    );
                }
                if let Some(usage) = &result.usage {
                    // Without this the cache is invisible: a hit and a short
                    // prompt look identical from the outside.
                    info!(
                        model = %model,
                        prompt = usage.prompt_tokens,
                        cache_read = usage.cache_read_tokens,
                        cache_write = usage.cache_write_tokens,
                        output = usage.completion_tokens,
                        "claude usage"
                    );
                }
                return Ok((result, strain));
            }

            let retry_after = retry_after_secs(resp.headers());
            let resumes_at = window_resumes_at(resp.headers(), retry_after);
            let body_text = resp.text().await.unwrap_or_default();

            // A spent quota window does not clear inside a retry loop — the only
            // thing that answers is another login. Retry it at once: the wait is
            // hours, not milliseconds.
            //
            // A login that cannot authenticate moves for the opposite reason:
            // nothing is lost by leaving, because the cached prefix sits behind
            // the same token that just failed.
            if (is_exhausted_window(status.as_u16(), retry_after)
                || is_auth_failure(status.as_u16()))
                && self
                    .rotate_account(account, status.as_u16(), resumes_at)
                    .await
                    .is_some()
            {
                strain.push(InferenceStrain::Transient {
                    attempt,
                    status: status.as_u16(),
                    model: model.clone(),
                    delay_ms: 0,
                });
                continue;
            }

            match classify_status(status) {
                ErrorClass::Transient if attempt + 1 < max_attempts => {
                    let delay = backoff(attempt);
                    warn!(
                        "claude subscription {status} on {model}{} (attempt {}), retry in {:?}",
                        status_hint(status.as_u16()),
                        attempt + 1,
                        delay
                    );
                    strain.push(InferenceStrain::Transient {
                        attempt,
                        status: status.as_u16(),
                        model: model.clone(),
                        delay_ms: delay.as_millis() as u64,
                    });
                    tokio::time::sleep(delay).await;
                }
                _ => {
                    strain.push(InferenceStrain::Exhausted {
                        attempts: attempt + 1,
                        status: status.as_u16(),
                        model: model.clone(),
                        body: body_text.chars().take(300).collect(),
                    });
                    anyhow::bail!(
                        "claude subscription returned {status} after {} attempt(s) on {model}{}{}: {}",
                        attempt + 1,
                        status_hint(status.as_u16()),
                        reset_hint(retry_after),
                        &body_text[..body_text.len().min(500)]
                    );
                }
            }
        }
        unreachable!("retry loop returns or bails")
    }
}

// ── token refresh ==========================================================

/// Take the credential file whenever it has moved since we read it.
///
/// The file belongs to `claude`, not to us, and it is the only record of *which
/// account* is logged in. A logout/login onto a second account rewrites it with
/// a token that is neither expired nor spent, so nothing in the refresh path
/// ever looks at the disk again and the provider keeps answering as the account
/// the user has left — 429ing on its exhausted quota while a live token sits in
/// the file. Measured 2026-08-12: server up since 10:55, file rewritten 15:20,
/// 429s from 15:22 with the on-disk token returning 200 to the same request.
/// The file is read every call rather than stat-compared: tokens are
/// fixed-length, so a rotation changes neither size nor — inside one filesystem
/// tick — mtime. A kilobyte ahead of an HTTPS call buys certainty.
fn adopt_file_if_changed(token: &mut TokenState) {
    let Some(path) = token.source_path.clone() else {
        return;
    };
    let Ok(fresh) = load_credentials(&path) else {
        return; // mid-write, or logged out entirely — what we hold still answers
    };
    let rotated = fresh
        .access_token
        .as_deref()
        .is_some_and(|t| !t.is_empty() && Some(t) != token.access_token.as_deref());
    if !rotated {
        return;
    }
    info!(
        "claude credentials rewritten on disk; adopting cred {} (was {})",
        credential_fingerprint(&fresh),
        credential_fingerprint(token),
    );
    // The quota belongs to the account, not to the token it was spent through:
    // a refresh mid-window must not read as a fresh window.
    let resumes = token.resumes_at_ms;
    *token = fresh;
    token.resumes_at_ms = resumes;
}

/// Short, stable id for a credential — enough to see in a log that souveraine
/// and `claude` are holding different logins. The token itself never gets there.
fn credential_fingerprint(token: &TokenState) -> String {
    match token.access_token.as_deref().filter(|t| !t.is_empty()) {
        Some(access) => {
            let mut hasher = Sha256::new();
            hasher.update(access.as_bytes());
            hex::encode(hasher.finalize())[..8].to_string()
        }
        None => "none".to_string(),
    }
}

/// Re-read the credential file behind `current`, returning it only if it now
/// holds a *different* and still-valid access token.
///
/// This is the recovery path for a lost refresh race: `claude` (or another
/// souveraine) rotated the one-time-use refresh token first, so ours is spent
/// while the file on disk already carries the winner's fresh pair. Adopting it
/// is strictly better than failing, since the alternative is a provider that
/// cannot recover without a restart.
///
/// Validity here is plain expiry, not the proactive refresh window: a token
/// with two minutes left still answers, and the next call will refresh it with
/// the rotated refresh token we picked up alongside it.
fn reread_credentials(current: &TokenState) -> Option<TokenState> {
    let path = current.source_path.as_ref()?;
    let mut rotated = load_credentials(path).ok()?;
    rotated.resumes_at_ms = current.resumes_at_ms;
    let is_newer = rotated
        .access_token
        .as_deref()
        .is_some_and(|t| !t.is_empty() && Some(t) != current.access_token.as_deref());
    let is_live = rotated.expires_at_ms.is_some_and(|exp| exp > now_ms());
    (is_newer && is_live).then_some(rotated)
}

async fn refresh_token(
    http: &reqwest::Client,
    user_agent: &str,
    token: &mut TokenState,
) -> Result<()> {
    let refresh = token
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no refresh_token"))?;
    let resp = http
        .post(TOKEN_URL)
        .header("content-type", "application/json")
        .header("user-agent", user_agent)
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "client_id": PROD_CLIENT_ID,
            "scope": SCOPES,
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token endpoint returned {status}: {body}");
    }
    let value: Value = resp.json().await?;
    let new_access = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("refresh response missing access_token"))?
        .to_string();
    let new_refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string);
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let new_expires = now_ms() + expires_in * 1000;

    if let Some(path) = token.source_path.clone() {
        match write_back_credentials(
            &path,
            &new_access,
            new_refresh.as_deref(),
            new_expires,
            &refresh,
        ) {
            Ok(true) => {}
            // Nothing to undo: the next call reads the file and adopts whoever
            // owns it now.
            Ok(false) => warn!("claude credential file holds another login; not writing over it"),
            Err(e) => warn!("credential write-back failed (continuing in-memory): {e}"),
        }
    }
    token.access_token = Some(new_access);
    token.refresh_token = Some(new_refresh.unwrap_or(refresh));
    token.expires_at_ms = Some(new_expires);
    Ok(())
}

/// Best-effort `account_uuid` lookup via the roles/profile endpoint.
#[allow(dead_code)]
async fn fetch_account_uuid(
    http: &reqwest::Client,
    access_token: &str,
    user_agent: &str,
) -> String {
    match http
        .get(ROLES_URL)
        .bearer_auth(access_token)
        .header("user-agent", user_agent)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v
                .get("account")
                .and_then(|a| a.get("uuid"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default(),
            Err(_) => String::new(),
        },
        _ => String::new(),
    }
}

// ── credentials file =======================================================

/// Every login to hold at once, primary first. `credential_files` wins when set;
/// entries that do not exist are dropped so one stale path cannot shadow a good
/// one. Falls back to `credential_file`, then to Claude Code's own default.
fn resolve_credential_paths(one: Option<&str>, many: Option<&[String]>) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = many
        .unwrap_or_default()
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()))
        .filter(|p| {
            p.exists() || {
                warn!(
                    "claude credential file {} does not exist; skipping",
                    p.display()
                );
                false
            }
        })
        .collect();
    if !listed.is_empty() {
        let mut deduped: Vec<PathBuf> = Vec::with_capacity(listed.len());
        for path in listed {
            if !deduped.contains(&path) {
                deduped.push(path);
            }
        }
        return deduped;
    }
    resolve_credential_path(one).into_iter().collect()
}

fn resolve_credential_path(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured.filter(|p| !p.is_empty()) {
        let p = PathBuf::from(shellexpand::tilde(path).into_owned());
        return Some(p);
    }
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".claude/.credentials.json");
    path.exists().then_some(path)
}

/// `{ claudeAiOauth: { accessToken, refreshToken, expiresAt(ms), ... }, ... }`
fn load_credentials(path: &Path) -> Result<TokenState> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw).context("parsing credentials json")?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or_else(|| anyhow::anyhow!("credential file missing `claudeAiOauth` object"))?;
    Ok(TokenState {
        access_token: oauth
            .get("accessToken")
            .and_then(Value::as_str)
            .map(str::to_string),
        refresh_token: oauth
            .get("refreshToken")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_at_ms: oauth.get("expiresAt").and_then(Value::as_u64),
        source_path: Some(path.to_path_buf()),
        resumes_at_ms: None,
    })
}

/// Atomically write refreshed tokens back, preserving all other fields.
///
/// Compare-and-swap on `refreshed_from`: the file is shared with `claude`, and
/// between our refresh and this write the user may have logged into another
/// account. Writing then replaces a live login with one from an account they
/// left — breaking `claude` as well as us. Returns false when the file has
/// moved on and was left alone.
fn write_back_credentials(
    path: &Path,
    access: &str,
    new_refresh: Option<&str>,
    expires_at_ms: u64,
    refreshed_from: &str,
) -> Result<bool> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut value: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let on_disk = value
        .get("claudeAiOauth")
        .and_then(|o| o.get("refreshToken"))
        .and_then(Value::as_str);
    if on_disk.is_some_and(|t| t != refreshed_from) {
        return Ok(false);
    }
    let oauth = value
        .as_object_mut()
        .context("cred file is not an object")?
        .entry("claudeAiOauth")
        .or_insert_with(|| json!({}));
    let object = oauth
        .as_object_mut()
        .context("claudeAiOauth is not an object")?;
    object.insert("accessToken".to_string(), json!(access));
    if let Some(refresh) = new_refresh {
        object.insert("refreshToken".to_string(), json!(refresh));
    }
    object.insert("expiresAt".to_string(), json!(expires_at_ms));

    let bytes = serde_json::to_vec_pretty(&value)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(true)
}

// ── wire shaping ===========================================================

/// Inject the attribution block into `system[0]`, preserving the rest.
fn inject_attribution(body: &mut Map<String, Value>, cc_version: &str, first_user_text: &str) {
    let fingerprint = compute_fingerprint(first_user_text, cc_version);
    let block = json!({
        "type": "text",
        "text": format!(
            "x-anthropic-billing-header: cc_version={cc_version}.{fingerprint}; cc_entrypoint={CC_ENTRYPOINT};"
        ),
    });
    let system = body.remove("system");
    let next = match system {
        Some(Value::String(text)) if !text.is_empty() => {
            Value::Array(vec![block, json!({ "type": "text", "text": text })])
        }
        Some(Value::Array(items)) => {
            let rest = if items.first().is_some_and(is_attribution_block) {
                items.into_iter().skip(1).collect::<Vec<_>>()
            } else {
                items
            };
            let mut out = vec![block];
            out.extend(rest);
            Value::Array(out)
        }
        Some(other) => Value::Array(vec![block, other]),
        None => Value::Array(vec![block]),
    };
    body.insert("system".to_string(), next);
}

fn is_attribution_block(value: &Value) -> bool {
    value
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|t| t.starts_with("x-anthropic-billing-header:"))
}

/// `metadata.user_id` = JSON string of `{device_id, account_uuid, session_id, …extra}`.
fn inject_metadata(body: &mut Map<String, Value>, client: &ClaudeSubscriptionProvider) {
    let mut metadata = body
        .remove("metadata")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut user_id = client.extra_metadata.clone();
    user_id.insert(
        "device_id".to_string(),
        Value::String(client.device_id.clone()),
    );
    user_id.insert(
        "session_id".to_string(),
        Value::String(uuid::Uuid::new_v4().to_string()),
    );
    user_id.insert(
        "account_uuid".to_string(),
        Value::String(client.account_uuid.clone()),
    );
    metadata.insert(
        "user_id".to_string(),
        Value::String(Value::Object(user_id).to_string()),
    );
    body.insert("metadata".to_string(), Value::Object(metadata));
}

/// `fingerprint = sha256( SALT + chars[4,7,20] + VERSION )[:3]`, JS UTF-16 indexing.
fn compute_fingerprint(first_user_text: &str, version: &str) -> String {
    let units: Vec<u16> = first_user_text.encode_utf16().collect();
    let mut chars = String::new();
    for &index in &[4usize, 7, 20] {
        match units.get(index) {
            Some(unit) => chars.push(char::from_u32(*unit as u32).unwrap_or('0')),
            None => chars.push('0'),
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_SALT.as_bytes());
    hasher.update(chars.as_bytes());
    hasher.update(version.as_bytes());
    hex::encode(hasher.finalize())[..3].to_string()
}

// ── OpenAI ↔ Anthropic translation =========================================

/// Translate OpenAI messages into Anthropic messages + a joined system string.
///
/// `mid_conv_system` decides where a system note that arrives *after* the
/// conversation has started lands. The top-level `system` field renders at the
/// very front of the prefix, so hoisting the turn loop's pulses, interjections
/// and subagent-awareness notes into it rewrites the front of the prompt on the
/// round they fire and re-bills tools, system and the whole transcript. They
/// stay where they happened instead.
fn translate_messages(
    openai: &[Message],
    mid_conv_system: bool,
) -> (Vec<Value>, Option<String>, String) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut merged: Vec<(String, Vec<Value>)> = Vec::new();

    for msg in openai {
        match msg.role.as_str() {
            "system" => {
                if msg.content.is_empty() {
                    continue;
                }
                let text = msg.content.as_text();
                let conversation_started = merged.iter().any(|(role, _)| role == "user");
                if !conversation_started {
                    system_parts.push(text);
                } else {
                    let role = if mid_conv_system { "system" } else { "user" };
                    push_merge(&mut merged, role, json!({ "type": "text", "text": text }));
                }
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": msg.content.as_text(),
                });
                push_merge(&mut merged, "user", block);
            }
            "assistant" => {
                // The thinking block must lead the turn and must come back
                // unmodified — Anthropic rejects a tool_use whose preceding
                // thinking block was dropped or edited.
                if let Some(thinking) = &msg.thinking {
                    push_merge(
                        &mut merged,
                        "assistant",
                        json!({
                            "type": "thinking",
                            "thinking": thinking.text,
                            "signature": thinking.signature,
                        }),
                    );
                }
                if !msg.content.is_empty() {
                    push_merge(
                        &mut merged,
                        "assistant",
                        json!({ "type": "text", "text": msg.content.as_text() }),
                    );
                }
                if let Some(calls) = &msg.tool_calls {
                    for call in calls {
                        let input: Value =
                            serde_json::from_str(&call.function.arguments).unwrap_or(json!({}));
                        push_merge(
                            &mut merged,
                            "assistant",
                            json!({
                                "type": "tool_use",
                                "id": call.id,
                                "name": call.function.name,
                                "input": input,
                            }),
                        );
                    }
                }
            }
            _ => {
                push_merge(
                    &mut merged,
                    "user",
                    json!({ "type": "text", "text": msg.content.as_text() }),
                );
            }
        }
    }

    // Anthropic requires the first role to be "user". A leading assistant
    // turn is only dropped as a pair: compaction can erase the opening user
    // message, and the tool_results answering that turn would orphan on the
    // wire if the calls alone were deleted.
    while merged.first().is_some_and(|(role, _)| role == "assistant") {
        let dropped: HashSet<String> = merged[0]
            .1
            .iter()
            .filter_map(|b| b.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        merged.remove(0);
        if !dropped.is_empty() {
            for (_, blocks) in &mut merged {
                blocks.retain(|b| {
                    b.get("type").and_then(Value::as_str) != Some("tool_result")
                        || !b
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| dropped.contains(id))
                });
            }
            // A user turn that only answered the dropped calls is empty now
            // and cannot stay on the wire either.
            merged.retain(|(role, blocks)| role != "user" || !blocks.is_empty());
        }
    }
    let merged = normalize_system_turns(merged);

    let messages: Vec<Value> = merged
        .into_iter()
        .map(|(role, blocks)| {
            if role == "system" {
                let text = blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                return json!({ "role": "system", "content": text });
            }
            // Content is always a block array, never the equivalent bare
            // string. Cache breakpoints attach to a block, and a shape that
            // flipped between the two as the marked tail moved would change the
            // prefix bytes every round and miss every read.
            json!({ "role": role, "content": Value::Array(blocks) })
        })
        .collect();

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let first_user_text = openai
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_text())
        .unwrap_or_default();

    (messages, system, first_user_text)
}

/// A `role: "system"` turn is only legal between a user turn and an assistant
/// turn, or as the last entry. Demote every other one into the user turn ahead
/// of it — still after the cached prefix, just without the operator role.
fn normalize_system_turns(merged: Vec<(String, Vec<Value>)>) -> Vec<(String, Vec<Value>)> {
    let mut out: Vec<(String, Vec<Value>)> = Vec::with_capacity(merged.len());
    for (i, (role, blocks)) in merged.iter().enumerate() {
        let legal = role != "system"
            || (out.last().is_some_and(|(r, _)| r == "user")
                && merged
                    .get(i + 1)
                    .is_none_or(|(r, _)| r.as_str() == "assistant"));
        let role = if legal { role.as_str() } else { "user" };
        match out.last_mut() {
            Some(last) if last.0 == role => last.1.extend(blocks.iter().cloned()),
            _ => out.push((role.to_string(), blocks.clone())),
        }
    }
    out
}

/// Place the four breakpoints a request is allowed.
///
/// Caching is a prefix match rendered `tools` → `system` → `messages`. The tool
/// schemas are byte-identical across every conversation and every agent, and
/// the system prompt is fixed for the life of one conversation, so each gets a
/// breakpoint of its own — editing the prompt then still reads the tools. The
/// remaining two ride the tail of the transcript so round N+1 of the tool loop
/// reads round N's prefix instead of paying for it again.
fn mark_cache_breakpoints(body: &mut Map<String, Value>) {
    fn mark(block: &mut Value) {
        if let Some(obj) = block.as_object_mut() {
            obj.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
        }
    }
    fn mark_last(body: &mut Map<String, Value>, key: &str) {
        if let Some(last) = body
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .and_then(|blocks| blocks.last_mut())
        {
            mark(last);
        }
    }

    mark_last(body, "tools");
    mark_last(body, "system");

    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let mut marked = 0;
    for message in messages.iter_mut().rev() {
        if marked == 2 {
            break;
        }
        // A system turn carries a bare string — no block to hang a breakpoint
        // on, and it is the volatile tail anyway.
        let Some(last) = message
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .and_then(|blocks| blocks.last_mut())
        else {
            continue;
        };
        mark(last);
        marked += 1;
    }
}

fn push_merge(merged: &mut Vec<(String, Vec<Value>)>, role: &str, block: Value) {
    if let Some(last) = merged.last_mut() {
        if last.0 == role {
            last.1.push(block);
            return;
        }
    }
    merged.push((role.to_string(), vec![block]));
}

fn translate_tool(tool: &ToolDefinition) -> Value {
    json!({
        "name": tool.function.name,
        "description": tool.function.description,
        "input_schema": tool.function.parameters,
    })
}

/// Anthropic non-stream response → Souveraine's OpenAI-shaped `CompletionResult`.
fn parse_completion(text: &str) -> Result<CompletionResult> {
    let value: Value = serde_json::from_str(text).context("parsing claude messages response")?;
    let content_parts: Vec<String> = value
        .get("content")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let content = content_parts.concat();

    // The first thinking block carries the signature that has to be replayed
    // alongside any tool_use in this same turn. Keep its text and signature
    // paired — a concatenation of several blocks could not be signed.
    let thinking_block = value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("thinking"))
        });
    let reasoning = thinking_block
        .and_then(|b| b.get("thinking"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let reasoning_signature = thinking_block
        .and_then(|b| b.get("signature"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let tool_calls = value
        .get("content")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter_map(|b| {
                    Some(ParsedToolCall {
                        id: b.get("id").and_then(Value::as_str)?.to_string(),
                        name: b.get("name").and_then(Value::as_str)?.to_string(),
                        arguments: b.get("input").cloned().unwrap_or(json!({})),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let finish_reason = value
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(map_stop_reason)
        .or(Some("stop".to_string()));

    let usage = value.get("usage").map(|u| {
        let field = |key: &str| u.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
        // `input_tokens` is only the part of the prompt that missed cache, so
        // it understates the context by however much was served cheaply.
        // `prompt_tokens` carries the real prompt size; the split rides
        // alongside it.
        let uncached = field("input_tokens");
        let cache_read = field("cache_read_input_tokens");
        let cache_write = field("cache_creation_input_tokens");
        let output = field("output_tokens");
        let input = uncached + cache_read + cache_write;
        Usage {
            prompt_tokens: input,
            completion_tokens: output,
            total_tokens: input + output,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
        }
    });

    Ok(CompletionResult {
        content,
        reasoning,
        reasoning_signature,
        tool_calls,
        finish_reason,
        usage,
    })
}

fn completion_has_payload(result: &CompletionResult) -> bool {
    !result.content.trim().is_empty() || !result.tool_calls.is_empty()
}

/// Describe an HTTP-success response that carried no output Souveraine knows
/// how to continue from. Keep this structural: block kinds and stop reason are
/// enough to diagnose a new wire shape without putting model prose in logs.
fn empty_completion_shape(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return "unparseable response".to_string();
    };
    let stop = value
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("missing");
    let block_types = value
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .map(|block| {
                    block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "not-an-array".to_string());
    format!("stop_reason={stop}, block_types=[{block_types}]")
}

fn map_stop_reason(reason: &str) -> String {
    match reason {
        "end_turn" | "stop_sequence" => "stop".to_string(),
        "max_tokens" => "length".to_string(),
        "tool_use" => "tool_calls".to_string(),
        other => other.to_string(),
    }
}

// ── misc helpers ===========================================================

fn resolve_cc_version() -> String {
    if let Ok(output) = std::process::Command::new("claude")
        .arg("--version")
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(version) = parse_version(&text) {
                return version;
            }
        }
    }
    FALLBACK_CC_VERSION.to_string()
}

fn parse_version(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?;
    let core: String = first
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!core.is_empty() && core.matches('.').count() >= 2).then_some(core)
}

fn get_or_create_device_id() -> Result<String> {
    let state_dir = match std::env::var("SOUVERAINE_STATE_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::state_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("souveraine"),
    };
    std::fs::create_dir_all(&state_dir)?;
    let path = state_dir.join("claude-subscription-device-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if is_device_id(existing) {
            return Ok(existing.to_ascii_lowercase());
        }
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = hex::encode(bytes);
    std::fs::write(&path, format!("{id}\n"))?;
    Ok(id)
}

fn is_device_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn backoff(attempt: u32) -> Duration {
    let base = 400u64 * 2u64.pow(attempt);
    Duration::from_millis(base.min(8_000))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ErrorClass {
    Transient,
    Permanent,
}

fn classify_status(status: reqwest::StatusCode) -> ErrorClass {
    match status.as_u16() {
        // 529 is Anthropic's own overload signal on this wire, not a
        // standard HTTP code — easy to misread as an auth/token failure
        // because nothing before this line said otherwise.
        429 | 500 | 502 | 503 | 504 | 408 | 529 => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

/// Seconds from a `retry-after` header. The HTTP-date form is not parsed —
/// unreadable reads as "no idea when", which routes to the same place as a long
/// wait: try another login.
fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Is this a spent quota window rather than a burst limit?
///
/// A burst limit comes back in seconds and is cheaper to wait out than to swap
/// for — the swap costs the account's whole cached prefix. Above the ceiling,
/// or with nothing to read, the window is gone and only another login helps.
fn is_exhausted_window(status: u16, retry_after: Option<u64>) -> bool {
    status == 429 && retry_after.is_none_or(|secs| secs > BURST_RETRY_CEILING_SECS)
}

/// This login cannot speak, whatever it is holding.
fn is_auth_failure(status: u16) -> bool {
    matches!(status, 401 | 403)
}

/// When the login that produced this response is worth trying again, in ms.
///
/// These 429s carry no `retry-after`; the unified rate-limit headers carry the
/// reset as epoch seconds, and are the only precise answer on the wire.
fn window_resumes_at(headers: &reqwest::header::HeaderMap, retry_after: Option<u64>) -> u64 {
    let now = now_ms();
    headers
        .get("anthropic-ratelimit-unified-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| secs * 1000)
        .filter(|at| *at > now)
        .or_else(|| retry_after.map(|secs| now + secs * 1000))
        .unwrap_or(now + DEFAULT_WINDOW_COOLDOWN.as_millis() as u64)
}

fn reset_hint(retry_after: Option<u64>) -> String {
    match retry_after {
        Some(secs) if secs >= 60 => format!(" (resets in ~{}m)", secs / 60),
        Some(secs) => format!(" (resets in {secs}s)"),
        None => String::new(),
    }
}

/// A short, human hint appended to the bail message for status codes whose
/// number alone reads as something it isn't — chiefly 529, which looks like
/// an OAuth/subscription failure but means Anthropic's capacity, not ours.
fn status_hint(status: u16) -> &'static str {
    match status {
        529 => " (Anthropic overloaded — not a Souveraine auth/token failure)",
        429 => " (rate limited)",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::openai_compatible::{
        ContentValue, MessageToolCall, MessageToolCallFunction, ToolFunction,
    };

    fn class_of(code: u16) -> ErrorClass {
        classify_status(reqwest::StatusCode::from_u16(code).expect("valid status"))
    }

    #[test]
    fn anthropic_overload_529_retries_instead_of_bailing_on_the_first_try() {
        // 529 is Anthropic's own overload signal, not a standard HTTP code.
        // Classified Permanent, one overloaded response bailed the entire
        // request with zero retries, and that bail string reaches the Panel
        // where it reads as a Souveraine auth/token failure.
        for code in [408, 429, 500, 502, 503, 504, 529] {
            assert!(
                matches!(class_of(code), ErrorClass::Transient),
                "{code} must be transient — it describes capacity, not our credentials"
            );
        }
        for code in [400, 401, 403, 404, 422] {
            assert!(
                matches!(class_of(code), ErrorClass::Permanent),
                "{code} must stay permanent — retrying it only burns quota"
            );
        }
    }

    #[test]
    fn the_529_hint_blames_capacity_rather_than_our_credentials() {
        let hint = status_hint(529);
        assert!(hint.contains("Anthropic overloaded"), "{hint}");
        assert!(
            hint.contains("not a Souveraine auth"),
            "the hint exists to stop a capacity failure reading as an auth failure: {hint}"
        );
        // Codes that read plainly get no editorial.
        assert_eq!(status_hint(500), "");
        assert_eq!(status_hint(401), "");
    }

    #[test]
    fn fingerprint_matches_spec_worked_example() {
        assert_eq!(compute_fingerprint("hello", "2.1.196"), "68b");
    }

    #[test]
    fn attribution_has_no_invented_fields() {
        let mut body = json!({ "messages": [{"role":"user","content":"hi"}] })
            .as_object_mut()
            .unwrap()
            .clone();
        inject_attribution(&mut body, "2.1.196", "hello");
        let text = body["system"].as_array().unwrap()[0]
            .get("text")
            .and_then(Value::as_str)
            .unwrap();
        assert!(text.starts_with("x-anthropic-billing-header:"));
        assert!(text.contains("cc_version=2.1.196."));
    }

    #[test]
    fn translates_user_assistant_tool_roundtrip() {
        let msgs = vec![
            Message::text("user", "please read /etc/hostname"),
            Message {
                role: "assistant".into(),
                content: ContentValue::Text(String::new()),
                tool_calls: Some(vec![MessageToolCall {
                    id: "call_1".into(),
                    tool_type: "function".into(),
                    function: MessageToolCallFunction {
                        name: "read".into(),
                        arguments: r#"{"path":"/etc/hostname"}"#.into(),
                    },
                }]),
                tool_call_id: None,
                name: None,
                thinking: None,
            },
            Message::tool_result("call_1", "read", "example-host"),
            Message::text("user", "thanks"),
        ];
        let (messages, system, first_user) = translate_messages(&msgs, true);
        assert!(system.is_none());
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles[0], "user");
        assert!(messages.iter().any(|m| {
            m["role"] == "user"
                && m["content"].is_array()
                && m["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b["type"] == "tool_result")
        }));
        assert_eq!(first_user, "please read /etc/hostname");
    }

    /// Compaction can drop the opening user message; history then begins with
    /// an assistant turn that called tools. The first role must still be
    /// "user", but stripping that turn alone would orphan its tool_result —
    /// the call gone, the result unanswered, a 400. The pair goes together.
    #[test]
    fn leading_assistant_turn_drops_with_the_results_that_answer_it() {
        let msgs = vec![
            Message::assistant_tool_calls(
                "",
                vec![MessageToolCall {
                    id: "call_1".into(),
                    tool_type: "function".into(),
                    function: MessageToolCallFunction {
                        name: "read".into(),
                        arguments: r#"{"path":"/etc/hostname"}"#.into(),
                    },
                }],
            ),
            Message::tool_result("call_1", "read", "example-host"),
            Message::text("user", "thanks"),
        ];
        let (messages, _, _) = translate_messages(&msgs, true);
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["user"]);
        assert_eq!(messages[0]["content"][0]["text"], "thanks");
        for m in &messages {
            for block in m["content"].as_array().unwrap() {
                assert_ne!(block["type"], "tool_result");
            }
        }
    }

    #[test]
    fn translates_system_message_into_system_field() {
        let msgs = vec![
            Message::text("system", "you are souvie"),
            Message::text("user", "hi"),
        ];
        let (_, system, _) = translate_messages(&msgs, true);
        assert_eq!(system.as_deref(), Some("you are souvie"));
    }

    /// The turn loop's pulse and the subagent's round warnings arrive as system
    /// messages mid-conversation. Hoisting them into the top-level field would
    /// rewrite the front of the prefix on the round they fire.
    #[test]
    fn mid_conversation_system_note_stays_out_of_the_system_field() {
        let msgs = vec![
            Message::text("system", "you are souvie"),
            Message::text("user", "hi"),
            Message::text("system", "[pulse] 4m elapsed"),
        ];
        let (messages, system, _) = translate_messages(&msgs, true);
        assert_eq!(system.as_deref(), Some("you are souvie"));
        let last = messages.last().unwrap();
        assert_eq!(last["role"], "system");
        assert_eq!(last["content"], "[pulse] 4m elapsed");
    }

    #[test]
    fn mid_conversation_note_folds_into_the_user_turn_when_unsupported() {
        let msgs = vec![
            Message::text("user", "hi"),
            Message::text("system", "[pulse] 4m elapsed"),
        ];
        let (messages, system, _) = translate_messages(&msgs, false);
        assert!(system.is_none());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["text"], "[pulse] 4m elapsed");
    }

    /// A system turn may only sit between a user turn and an assistant turn, or
    /// last. One landing anywhere else is demoted rather than sent and 400ed.
    #[test]
    fn demotes_a_system_turn_that_cannot_sit_where_it_landed() {
        let msgs = vec![
            Message::text("user", "hi"),
            Message::text("assistant", "hello"),
            Message::text("system", "[migraine] stop"),
            Message::text("user", "still there?"),
        ];
        let (messages, _, _) = translate_messages(&msgs, true);
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        let folded = messages[2]["content"].as_array().unwrap();
        assert_eq!(folded[0]["text"], "[migraine] stop");
        assert_eq!(folded[1]["text"], "still there?");
    }

    #[test]
    fn message_content_is_always_a_block_array() {
        let msgs = vec![Message::text("user", "hi")];
        let (messages, _, _) = translate_messages(&msgs, true);
        assert!(messages[0]["content"].is_array());
    }

    /// Every request gets at most the four breakpoints the API allows, and they
    /// land on the tail of each stable span: tools, system, transcript.
    #[test]
    fn places_breakpoints_on_tools_system_and_the_transcript_tail() {
        let mut body = json!({
            "tools": [{ "name": "read" }, { "name": "bash" }],
            "system": "you are souvie",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "one" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "two" }] },
                { "role": "user", "content": [{ "type": "text", "text": "three" }] },
            ],
        })
        .as_object_mut()
        .unwrap()
        .clone();
        inject_attribution(&mut body, "2.1.196", "one");
        mark_cache_breakpoints(&mut body);

        let marked = |v: &Value| v.get("cache_control").is_some();
        let tools = body["tools"].as_array().unwrap();
        assert!(!marked(&tools[0]) && marked(&tools[1]));
        let system = body["system"].as_array().unwrap();
        assert!(!marked(&system[0]) && marked(system.last().unwrap()));

        let messages = body["messages"].as_array().unwrap();
        let tails: Vec<bool> = messages
            .iter()
            .map(|m| marked(m["content"].as_array().unwrap().last().unwrap()))
            .collect();
        assert_eq!(tails, vec![false, true, true]);

        let total = serde_json::to_string(&body)
            .unwrap()
            .matches("cache_control")
            .count();
        assert_eq!(total, 4);
    }

    /// A system turn carries a bare string, so it has no block to mark — the
    /// breakpoint has to skip past it to the last turn that does.
    #[test]
    fn breakpoints_skip_a_trailing_system_turn() {
        let mut body = json!({
            "system": ["you are souvie"],
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "one" }] },
                { "role": "system", "content": "[pulse] 4m elapsed" },
            ],
        })
        .as_object_mut()
        .unwrap()
        .clone();
        mark_cache_breakpoints(&mut body);
        let messages = body["messages"].as_array().unwrap();
        assert!(messages[1].get("cache_control").is_none());
        assert!(messages[0]["content"].as_array().unwrap()[0]
            .get("cache_control")
            .is_some());
    }

    #[test]
    fn usage_counts_the_cached_prefix_as_prompt_tokens() {
        let raw = r#"{
            "id":"msg_3","stop_reason":"end_turn",
            "content":[{"type":"text","text":"pong"}],
            "usage":{
                "input_tokens":12,
                "cache_read_input_tokens":9000,
                "cache_creation_input_tokens":300,
                "output_tokens":4
            }
        }"#;
        let usage = parse_completion(raw).unwrap().usage.unwrap();
        assert_eq!(usage.prompt_tokens, 9312);
        assert_eq!(usage.cache_read_tokens, 9000);
        assert_eq!(usage.cache_write_tokens, 300);
        assert_eq!(usage.total_tokens, 9316);
    }

    #[test]
    fn parses_anthropic_text_response() {
        let raw = r#"{
            "id":"msg_1","type":"message","role":"assistant","stop_reason":"end_turn",
            "content":[{"type":"text","text":"pong"}],
            "usage":{"input_tokens":16,"output_tokens":4}
        }"#;
        let r = parse_completion(raw).unwrap();
        assert_eq!(r.content, "pong");
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
        assert_eq!(r.usage.unwrap().prompt_tokens, 16);
        assert!(r.tool_calls.is_empty());
    }

    #[test]
    fn empty_completion_shape_is_not_a_usable_payload() {
        let raw = r#"{
            "id":"msg_empty","type":"message","role":"assistant","stop_reason":"end_turn",
            "content":[],
            "usage":{"input_tokens":60535,"output_tokens":2}
        }"#;
        let result = parse_completion(raw).unwrap();
        assert!(!completion_has_payload(&result));
        assert_eq!(
            empty_completion_shape(raw),
            "stop_reason=end_turn, block_types=[]"
        );
    }

    #[test]
    fn parses_anthropic_tool_use_response() {
        let raw = r#"{
            "id":"msg_2","stop_reason":"tool_use",
            "content":[
                {"type":"text","text":"reading it"},
                {"type":"tool_use","id":"call_9","name":"read","input":{"path":"/etc/hostname"}}
            ],
            "usage":{"input_tokens":10,"output_tokens":8}
        }"#;
        let r = parse_completion(raw).unwrap();
        assert_eq!(r.content, "reading it");
        assert_eq!(r.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "read");
        assert_eq!(r.tool_calls[0].arguments["path"], "/etc/hostname");
    }

    #[test]
    fn translates_openai_tool_definition_to_anthropic_schema() {
        let tool = ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "read".to_string(),
                description: "read a file".to_string(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
            },
        };
        let v = translate_tool(&tool);
        assert_eq!(v["name"], "read");
        assert_eq!(v["input_schema"]["type"], "object");
    }

    /// Write a credential file and return the `TokenState` naming it.
    fn creds_at(dir: &std::path::Path, access: &str, expires_at_ms: u64) -> TokenState {
        let path = dir.join(".credentials.json");
        std::fs::write(
            &path,
            json!({
                "claudeAiOauth": {
                    "accessToken": access,
                    "refreshToken": format!("refresh-for-{access}"),
                    "expiresAt": expires_at_ms,
                }
            })
            .to_string(),
        )
        .unwrap();
        load_credentials(&path).unwrap()
    }

    #[test]
    fn rereads_credentials_rotated_by_another_process() {
        let dir = tempfile::tempdir().unwrap();
        let stale = creds_at(dir.path(), "spent", now_ms() + 60_000);
        // `claude` wins the refresh race and rewrites the file underneath us.
        creds_at(dir.path(), "rotated", now_ms() + 8 * 3_600_000);

        let adopted = reread_credentials(&stale).expect("rotated token should be adopted");
        assert_eq!(adopted.access_token.as_deref(), Some("rotated"));
        // The matching refresh token comes with it — the spent one is dropped.
        assert_eq!(
            adopted.refresh_token.as_deref(),
            Some("refresh-for-rotated")
        );
    }

    /// A spent five-hour window is what a second login exists for. A burst
    /// limit is not — it clears in seconds, and swapping would abandon a cached
    /// prefix worth more than the wait.
    #[test]
    fn only_a_spent_window_is_worth_another_login() {
        assert!(is_exhausted_window(429, None));
        assert!(is_exhausted_window(429, Some(3600)));
        assert!(is_exhausted_window(429, Some(61)));
        assert!(!is_exhausted_window(429, Some(30)));
        assert!(!is_exhausted_window(429, Some(60)));
        // Overload and auth failures follow every account; moving is pointless.
        assert!(!is_exhausted_window(529, None));
        assert!(!is_exhausted_window(401, None));
        assert!(!is_exhausted_window(500, Some(3600)));
    }

    /// 2026-08-17T21:51: the primary spent its window, rotation took the spare
    /// on `has_credential` alone — a login whose token had expired on the 13th —
    /// and every call after it came back 401 *revoked*, which reads as the
    /// subscription being gone rather than the spare being dead. Presence of a
    /// string is not the question.
    #[test]
    fn an_expired_spare_is_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let dead = creds_at(dir.path(), "expired-on-the-13th", now_ms() - 4 * 86_400_000);
        assert!(dead.has_credential());
        assert!(!dead.access_is_live(now_ms()));

        let other = tempfile::tempdir().unwrap();
        let live = creds_at(other.path(), "good", now_ms() + 8 * 3_600_000);
        assert!(live.access_is_live(now_ms()));
    }

    /// A login parked on a spent window is skipped until the window returns —
    /// and the window survives the refresh that rotates its token, because the
    /// quota belongs to the account, not to the string it was spent through.
    #[test]
    fn a_parked_login_stays_parked_until_its_window_returns() {
        let dir = tempfile::tempdir().unwrap();
        let mut spent = creds_at(dir.path(), "account-a", now_ms() + 8 * 3_600_000);
        spent.resumes_at_ms = Some(now_ms() + 3_600_000);
        assert!(!spent.is_available(now_ms()));
        assert!(spent.is_available(now_ms() + 3_600_001));

        creds_at(dir.path(), "account-a-refreshed", now_ms() + 8 * 3_600_000);
        adopt_file_if_changed(&mut spent);
        assert_eq!(spent.access_token.as_deref(), Some("account-a-refreshed"));
        assert!(!spent.is_available(now_ms()));
    }

    #[test]
    fn the_window_reset_is_read_from_the_header_that_carries_it() {
        let mut headers = reqwest::header::HeaderMap::new();
        let reset_secs = now_ms() / 1000 + 3600;
        headers.insert(
            "anthropic-ratelimit-unified-reset",
            reqwest::header::HeaderValue::from_str(&reset_secs.to_string()).unwrap(),
        );
        let at = window_resumes_at(&headers, None);
        assert!(at.abs_diff(now_ms() + 3_600_000) < 2_000, "{at}");

        // No header: `retry-after`, then a cooldown short enough that one wasted
        // call is the whole cost of guessing.
        let empty = reqwest::header::HeaderMap::new();
        let after = window_resumes_at(&empty, Some(120));
        assert!(after.abs_diff(now_ms() + 120_000) < 2_000, "{after}");
        let blind = window_resumes_at(&empty, None);
        assert!(blind.abs_diff(now_ms() + 900_000) < 2_000, "{blind}");
    }

    /// 429 moves because the window is gone; 401 moves because the cached prefix
    /// is unreachable behind the token that just failed. Everything else stays
    /// put — a swap costs the prefix a re-bill.
    #[test]
    fn only_a_dead_login_and_a_spent_one_are_worth_leaving() {
        assert!(is_auth_failure(401));
        assert!(is_auth_failure(403));
        for code in [400, 404, 429, 500, 529] {
            assert!(!is_auth_failure(code), "{code} is not an auth failure");
        }
    }

    #[test]
    fn the_bail_says_when_the_quota_comes_back() {
        assert_eq!(reset_hint(Some(7200)), " (resets in ~120m)");
        assert_eq!(reset_hint(Some(45)), " (resets in 45s)");
        assert_eq!(reset_hint(None), "");
    }

    #[test]
    fn credential_paths_drop_what_is_missing_and_keep_the_order() {
        let dir = tempfile::tempdir().unwrap();
        creds_at(dir.path(), "account-a", now_ms() + 60_000);
        let present = dir.path().join(".credentials.json");
        let absent = dir.path().join("no-such-login.json");

        let listed = vec![
            present.display().to_string(),
            absent.display().to_string(),
            present.display().to_string(),
        ];
        assert_eq!(
            resolve_credential_paths(None, Some(&listed)),
            vec![present.clone()]
        );
        // Nothing listed and nothing on disk falls back to the single path.
        let single = absent.display().to_string();
        assert_eq!(
            resolve_credential_paths(Some(&single), Some(&[])),
            vec![absent]
        );
    }

    /// The swap that started this: `claude logout` then a login on a second
    /// account. Nothing expired, nothing was spent, no refresh was attempted —
    /// and the token we hold belongs to an account the user has left.
    #[test]
    fn adopts_the_account_a_logout_login_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let mut held = creds_at(dir.path(), "account-a", now_ms() + 8 * 3_600_000);
        creds_at(dir.path(), "account-b", now_ms() + 8 * 3_600_000);

        adopt_file_if_changed(&mut held);
        assert_eq!(held.access_token.as_deref(), Some("account-b"));
        assert_eq!(held.refresh_token.as_deref(), Some("refresh-for-account-b"));
    }

    #[test]
    fn a_touched_but_unchanged_file_is_not_an_adoption() {
        let dir = tempfile::tempdir().unwrap();
        let mut held = creds_at(dir.path(), "account-a", now_ms() + 8 * 3_600_000);
        let before = credential_fingerprint(&held);
        creds_at(dir.path(), "account-a", now_ms() + 8 * 3_600_000);

        adopt_file_if_changed(&mut held);
        assert_eq!(credential_fingerprint(&held), before);
    }

    /// The other half of a swap: our refresh succeeds, but by the time it lands
    /// the file holds the account the user actually logged into. Writing would
    /// log `claude` back out from under them.
    #[test]
    fn write_back_refuses_to_overwrite_another_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        creds_at(dir.path(), "account-b", now_ms() + 8 * 3_600_000);

        let wrote = write_back_credentials(
            &path,
            "account-a-refreshed",
            Some("refresh-for-account-a-refreshed"),
            now_ms() + 8 * 3_600_000,
            "refresh-for-account-a",
        )
        .unwrap();
        assert!(!wrote);
        let after = load_credentials(&path).unwrap();
        assert_eq!(after.access_token.as_deref(), Some("account-b"));
    }

    #[test]
    fn write_back_lands_when_the_file_is_still_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        creds_at(dir.path(), "account-a", now_ms() + 60_000);

        let wrote = write_back_credentials(
            &path,
            "account-a-refreshed",
            Some("refresh-for-account-a-refreshed"),
            now_ms() + 8 * 3_600_000,
            "refresh-for-account-a",
        )
        .unwrap();
        assert!(wrote);
        let after = load_credentials(&path).unwrap();
        assert_eq!(after.access_token.as_deref(), Some("account-a-refreshed"));
        // Everything else the file carries survives the write.
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw["claudeAiOauth"]["expiresAt"].as_u64().unwrap() > now_ms());
    }

    #[test]
    fn does_not_readopt_the_same_or_expired_token() {
        let dir = tempfile::tempdir().unwrap();
        // Unchanged file: nothing rotated, so there is nothing to adopt and the
        // caller must surface the original refresh error.
        let current = creds_at(dir.path(), "same", now_ms() + 60_000);
        assert!(reread_credentials(&current).is_none());

        // Rotated but already expired — adopting it would just fail differently.
        let stale = creds_at(dir.path(), "spent", now_ms() + 60_000);
        creds_at(
            dir.path(),
            "rotated-but-dead",
            now_ms().saturating_sub(1_000),
        );
        assert!(reread_credentials(&stale).is_none());
    }
}
