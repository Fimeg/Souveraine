//! Reading and writing the Codex CLI's ChatGPT OAuth login (`~/.codex/auth.json`).

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::Value;

/// The ChatGPT OAuth credential, as minted by `codex login`.
#[derive(Debug, Clone)]
pub struct CodexCredentials {
    pub access_token: String,
    pub refresh_token: String,
    /// `ChatGPT-Account-Id` header value.
    pub account_id: String,
    /// Unix epoch milliseconds when the access token expires.
    pub expires_at: i64,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `$CODEX_HOME` if set, else `~/.codex`.
fn codex_home() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(shellexpand::tilde(&dir).into_owned());
        }
    }
    dirs::home_dir().unwrap_or_default().join(".codex")
}

fn auth_path() -> PathBuf {
    codex_home().join("auth.json")
}

/// Decode a JWT's `exp` claim (seconds) into epoch milliseconds.
fn jwt_exp_ms(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    Some(json.get("exp")?.as_i64()? * 1000)
}

/// Read the Codex CLI login from disk.
pub fn read() -> Result<CodexCredentials> {
    let path = auth_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading codex auth at {}", path.display()))?;
    let json: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing codex auth at {}", path.display()))?;
    let tokens = json
        .get("tokens")
        .ok_or_else(|| anyhow!("codex auth.json has no `tokens` (run `codex login`)"))?;

    let access = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("codex auth.json missing tokens.access_token"))?
        .to_string();
    let refresh = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("codex auth.json missing tokens.refresh_token"))?
        .to_string();
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // Prefer the JWT's own expiry; fall back to file mtime + 1h.
    let expires_at = jwt_exp_ms(&access).unwrap_or_else(|| {
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        mtime.unwrap_or_else(now_ms) + 3_600_000
    });

    Ok(CodexCredentials {
        access_token: access,
        refresh_token: refresh,
        account_id,
        expires_at,
    })
}

/// Write rotated tokens back into `auth.json`, preserving every other field.
/// Best-effort: the Codex CLI may own the file, so callers ignore failure.
pub fn write_back(creds: &CodexCredentials) -> Result<()> {
    let path = auth_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading codex auth at {}", path.display()))?;
    let mut json: Value = serde_json::from_str(&raw)?;
    let tokens = json
        .get_mut("tokens")
        .ok_or_else(|| anyhow!("codex auth.json has no `tokens`"))?;
    tokens["access_token"] = Value::String(creds.access_token.clone());
    tokens["refresh_token"] = Value::String(creds.refresh_token.clone());
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("writing codex auth at {}", path.display()))?;
    Ok(())
}
