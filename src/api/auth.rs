//! Per-agent bearer-token auth for the memfs HTTP write path.
//!
//! See `docs/CRON_API_AUTH.md` for the design rationale.
//!
//! Tokens live at `~/.souveraine/server/agents/<agent-id>/api_token`
//! (file mode 0600, single line, format `souv_<uuid-v4>`). Compared
//! constant-time against the `Authorization: Bearer ...` header.
//!
//! Loopback bypass is opt-in via `[server.auth].allow_loopback`.

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
    body::Body,
};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use crate::server::SouveraineServer;

/// Generate a fresh API token. Format: `souv_<uuid-v4>`, prefix is searchable
/// in logs / pastebins so leaks are easier to spot.
pub fn generate_token() -> String {
    format!("souv_{}", Uuid::new_v4())
}

/// Path to the token file for an agent.
pub fn token_path(server_data_dir: &std::path::Path, agent_id: &str) -> PathBuf {
    server_data_dir.join("agents").join(agent_id).join("api_token")
}

/// Read the token from disk. Errors if the file is missing or unreadable.
/// Validates the file is mode 0600 on Unix; on other platforms skips that check.
pub async fn read_token(server_data_dir: &std::path::Path, agent_id: &str) -> Result<String> {
    let path = token_path(server_data_dir, agent_id);
    let contents = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading token at {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = tokio::fs::metadata(&path).await?;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "token file mode is {:o}, must be 0600 (refusing to use): {}",
                mode,
                path.display()
            );
        }
    }

    Ok(contents.trim().to_string())
}

/// Write a new token, ensuring 0600 permissions on Unix. Idempotent for rotation.
pub async fn write_token(
    server_data_dir: &std::path::Path,
    agent_id: &str,
    token: &str,
) -> Result<()> {
    let path = token_path(server_data_dir, agent_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, token).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

/// Constant-time bearer-token comparison.
fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract `Authorization: Bearer <token>` from headers, lowercase-insensitive.
fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get("authorization")?.to_str().ok()?;
    let prefix = "Bearer ";
    let lower = v.to_ascii_lowercase();
    if !lower.starts_with(&prefix.to_ascii_lowercase()) {
        return None;
    }
    Some(v[prefix.len()..].trim().to_string())
}

/// True if the connecting peer is loopback (127.0.0.1 / ::1).
fn is_loopback(remote: Option<IpAddr>) -> bool {
    remote.map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// Auth middleware applied to `/v1/agents/:id/memory/...` routes.
///
/// Order of checks (per CRON_API_AUTH.md):
/// 1. If auth is not required at all (config off) → pass.
/// 2. If loopback bypass is on AND remote is loopback → pass.
/// 3. Read the agent's token from disk and constant-time compare.
/// 4. On any failure → 401 with a generic body (no agent-id enumeration).
pub async fn require_token(
    State(server): State<Arc<SouveraineServer>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    remote: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    req: axum::http::Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // 1. Config: auth off entirely.
    let cfg = server.app_config.read().await;
    let auth_required = cfg.server.auth.required;
    let allow_loopback = cfg.server.auth.allow_loopback;
    drop(cfg);

    if !auth_required {
        return Ok(next.run(req).await);
    }

    // 2. Loopback bypass.
    if allow_loopback && is_loopback(remote.map(|ci| ci.0.ip())) {
        return Ok(next.run(req).await);
    }

    // 3. Compare bearer token.
    let presented = extract_bearer(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let server_data_dir = {
        let cfg = server.config.read().await;
        cfg.data_dir.clone()
    };
    let expected = match read_token(&server_data_dir, &agent_id).await {
        Ok(t) => t,
        Err(_) => return Err(StatusCode::UNAUTHORIZED),
    };
    if !ct_eq(&presented, &expected) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

/// Lightweight in-memory cache for tokens that have been verified recently.
/// Optional optimization; the on-disk read is fast enough for now, but this
/// is the seam if/when we need it.
#[allow(dead_code)]
pub struct TokenCache {
    inner: tokio::sync::RwLock<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_has_prefix() {
        let t = generate_token();
        assert!(t.starts_with("souv_"));
        assert_eq!(t.len(), "souv_".len() + 36); // uuid v4 = 36 chars
    }

    #[test]
    fn ct_eq_basic() {
        assert!(ct_eq("souv_abc", "souv_abc"));
        assert!(!ct_eq("souv_abc", "souv_abd"));
        assert!(!ct_eq("short", "longer-string"));
    }

    #[test]
    fn extract_bearer_basic() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer souv_xyz".parse().unwrap());
        assert_eq!(extract_bearer(&h).as_deref(), Some("souv_xyz"));
    }

    #[test]
    fn extract_bearer_missing() {
        let h = HeaderMap::new();
        assert!(extract_bearer(&h).is_none());
    }

    #[test]
    fn extract_bearer_wrong_scheme() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Basic abc==".parse().unwrap());
        assert!(extract_bearer(&h).is_none());
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let agent_id = "test-agent";
        write_token(dir.path(), agent_id, "souv_abc123").await.unwrap();
        let got = read_token(dir.path(), agent_id).await.unwrap();
        assert_eq!(got, "souv_abc123");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_world_readable_token() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let agent_id = "test-agent";
        write_token(dir.path(), agent_id, "souv_abc").await.unwrap();
        // Manually loosen permissions to 0644.
        let path = token_path(dir.path(), agent_id);
        let perms = std::fs::Permissions::from_mode(0o644);
        std::fs::set_permissions(&path, perms).unwrap();
        let result = read_token(dir.path(), agent_id).await;
        assert!(result.is_err());
    }
}
