//! ChatGPT OAuth token refresh.
//!
//! Self-refresh against OpenAI's token endpoint using the Codex CLI's public
//! client id, then write the rotated token back to `~/.codex/auth.json`.
//! Refresh tokens are one-time-use, so two processes refreshing the same token
//! race; the caller serializes us behind a mutex (single-flight), and if the
//! refresh token was already spent we re-read the file — the CLI may have
//! rotated it out from under us.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use super::codex_creds::{self, now_ms, CodexCredentials};

const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Public client id of the Codex CLI OAuth app — the refresh must be attributed
/// to the same client that minted the token.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Refresh this many ms before the token actually expires.
const REFRESH_BUFFER_MS: i64 = 300_000;

fn is_expired(creds: &CodexCredentials) -> bool {
    now_ms() >= creds.expires_at - REFRESH_BUFFER_MS
}

/// Ensure `creds` holds a fresh access token, refreshing in place if needed.
pub async fn ensure_fresh(http: &reqwest::Client, creds: &mut CodexCredentials) -> Result<()> {
    if !is_expired(creds) {
        return Ok(());
    }

    match refresh(http, creds).await {
        Ok(fresh) => {
            *creds = fresh;
            // Best-effort: keep the CLI's file in sync. The CLI may own it.
            let _ = codex_creds::write_back(creds);
            Ok(())
        }
        Err(e) => {
            // Refresh failed — perhaps the CLI already rotated the token. If the
            // file now holds a newer, still-valid token, adopt it.
            if let Ok(reread) = codex_creds::read() {
                if reread.access_token != creds.access_token && now_ms() < reread.expires_at {
                    *creds = reread;
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}

async fn refresh(http: &reqwest::Client, creds: &CodexCredentials) -> Result<CodexCredentials> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", creds.refresh_token.as_str()),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await
        .context("posting ChatGPT OAuth token refresh")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("ChatGPT token refresh failed ({}): {}", status, body));
    }

    let json: Value = resp.json().await.context("parsing token refresh response")?;
    let access = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("token refresh response missing access_token"))?
        .to_string();
    // OpenAI may or may not rotate the refresh token; keep the old one if not.
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| creds.refresh_token.clone());
    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600);

    Ok(CodexCredentials {
        access_token: access,
        refresh_token,
        account_id: creds.account_id.clone(),
        expires_at: now_ms() + expires_in * 1000,
    })
}
