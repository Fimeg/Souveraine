//! Per-agent bearer-token auth for the memfs HTTP write path.
//!
//! Design: `SouveraineOS/docs/substrate/CRON_API_AUTH.md`. The old citation
//! pointed at this repo's `docs/`, which is gitignored and empty on a clone.
//!
//! Tokens live at `~/.souveraine/server/agents/<agent-id>/api_token`
//! (file mode 0600, single line, format `souv_<uuid-v4>`). Compared
//! constant-time against the `Authorization: Bearer ...` header.
//!
//! Loopback bypass is opt-in via `[server.auth].allow_loopback`.

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
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
    server_data_dir
        .join("agents")
        .join(agent_id)
        .join("api_token")
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

    let token = contents.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("token file is empty: {}", path.display());
    }
    Ok(token)
}

/// Return this agent's token, minting one if the file is absent or empty.
///
/// Called at agent creation, per CRON_API_AUTH.md § "Where tokens live". An
/// existing token is never silently replaced: a file that is present but
/// unreadable (wrong mode, tampered) returns its own error rather than a fresh
/// credential, because that is a signal, not a reason to mint. Rotation is
/// `rotate_token`, and it is always deliberate.
pub async fn ensure_token(server_data_dir: &std::path::Path, agent_id: &str) -> Result<String> {
    let path = token_path(server_data_dir, agent_id);
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        let existing = tokio::fs::read_to_string(&path).await?;
        if !existing.trim().is_empty() {
            return read_token(server_data_dir, agent_id).await;
        }
    }
    let token = generate_token();
    write_token(server_data_dir, agent_id, &token).await?;
    tracing::info!(agent_id, "issued api token");
    Ok(token)
}

/// True when this agent has a token it can actually authenticate with. An
/// unreadable or wrong-mode file counts as absent — it cannot pass a check.
pub async fn has_token(server_data_dir: &std::path::Path, agent_id: &str) -> bool {
    read_token(server_data_dir, agent_id).await.is_ok()
}

/// Mint a new token, replacing any existing one. Invalidates immediately —
/// there is no grace period, so every holder must re-read the file.
pub async fn rotate_token(server_data_dir: &std::path::Path, agent_id: &str) -> Result<String> {
    let token = generate_token();
    write_token(server_data_dir, agent_id, &token).await?;
    tracing::info!(agent_id, "rotated api token");
    Ok(token)
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

/// A refused request.
///
/// `MissingToken` is the one denial that names its own repair: an agent
/// created before tokens were issued has no file, and the caller cannot fix
/// that by guessing. It is safe to say so because the answer is identical for
/// an agent that does not exist — nothing is enumerable through it. Every
/// other denial stays generic.
pub enum AuthDenial {
    MissingToken,
    Unauthorized,
    ConversationNotFound,
}

impl IntoResponse for AuthDenial {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            AuthDenial::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "missing_token",
                "Run 'souveraine agents token <id> --rotate' to generate one.",
            ),
            AuthDenial::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Invalid or missing bearer token.",
            ),
            AuthDenial::ConversationNotFound => (
                StatusCode::NOT_FOUND,
                "conversation_not_found",
                "No such conversation.",
            ),
        };
        (
            status,
            Json(serde_json::json!({ "error": error, "message": message })),
        )
            .into_response()
    }
}

/// Auth middleware applied to `/v1/agents/:id/memory/...` routes.
pub async fn require_token(
    State(server): State<Arc<SouveraineServer>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    remote: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    req: axum::http::Request<Body>,
    next: Next,
) -> Result<Response, AuthDenial> {
    verify_token(&server, &agent_id, &headers, remote.map(|ci| ci.0.ip())).await?;
    Ok(next.run(req).await)
}

/// Verify a bearer token against a given agent_id, respecting config and loopback.
///
/// Order of checks (per CRON_API_AUTH.md):
/// 1. If auth is not required at all (config off) → pass.
/// 2. If loopback bypass is on AND remote is loopback → pass.
/// 3. Read the agent's token from disk and constant-time compare.
///
/// Shared by all middleware variants. Never logs the presented credential —
/// only the agent id and the outcome (operational rule 1).
pub async fn verify_token(
    server: &SouveraineServer,
    agent_id: &str,
    headers: &HeaderMap,
    remote: Option<IpAddr>,
) -> Result<(), AuthDenial> {
    let cfg = server.app_config.read().await;
    let auth_required = cfg.server.auth.required;
    let allow_loopback = cfg.server.auth.allow_loopback;
    drop(cfg);

    if !auth_required {
        return Ok(());
    }

    if allow_loopback && is_loopback(remote) {
        return Ok(());
    }

    let server_data_dir = {
        let cfg = server.config.read().await;
        cfg.data_dir.clone()
    };

    // An agent with no token file predates issuance; say which failure this is
    // so the operator has somewhere to go.
    let path = token_path(&server_data_dir, agent_id);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tracing::warn!(agent_id, outcome = "denied", reason = "missing_token");
        return Err(AuthDenial::MissingToken);
    }

    let presented = extract_bearer(headers).ok_or_else(|| {
        tracing::debug!(agent_id, outcome = "denied", reason = "no_bearer");
        AuthDenial::Unauthorized
    })?;

    let expected = match read_token(&server_data_dir, agent_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(agent_id, outcome = "denied", reason = %e);
            return Err(AuthDenial::Unauthorized);
        }
    };
    if !ct_eq(&presented, &expected) {
        tracing::debug!(agent_id, outcome = "denied", reason = "token_mismatch");
        return Err(AuthDenial::Unauthorized);
    }
    Ok(())
}

/// Auth middleware for agent CRUD routes (PATCH/DELETE /v1/agents/:id).
///
/// Uses the same per-agent token as the memory routes. Applied as a
/// route_layer on agent update/delete so only the agent owner can modify
/// or remove an agent.
pub async fn require_agent_token(
    State(server): State<Arc<SouveraineServer>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    remote: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    req: axum::http::Request<Body>,
    next: Next,
) -> Result<Response, AuthDenial> {
    verify_token(&server, &agent_id, &headers, remote.map(|ci| ci.0.ip())).await?;
    Ok(next.run(req).await)
}

/// Auth middleware for conversation routes (GET /v1/conversations/:id,
/// POST /v1/conversations/:id/messages).
///
/// Resolves the agent_id from the conversation session, then applies
/// the same per-agent token check.
pub async fn require_conversation_token(
    State(server): State<Arc<SouveraineServer>>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    remote: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    req: axum::http::Request<Body>,
    next: Next,
) -> Result<Response, AuthDenial> {
    let agent_id = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or(AuthDenial::ConversationNotFound)?;
        session.agent_id.clone()
    };
    verify_token(&server, &agent_id, &headers, remote.map(|ci| ci.0.ip())).await?;
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
        write_token(dir.path(), agent_id, "souv_abc123")
            .await
            .unwrap();
        let got = read_token(dir.path(), agent_id).await.unwrap();
        assert_eq!(got, "souv_abc123");
    }

    #[tokio::test]
    async fn read_rejects_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        write_token(dir.path(), "a", "").await.unwrap();
        assert!(read_token(dir.path(), "a").await.is_err());
    }

    #[tokio::test]
    async fn ensure_mints_once_then_returns_the_same_token() {
        let dir = tempfile::tempdir().unwrap();
        let first = ensure_token(dir.path(), "a").await.unwrap();
        assert!(first.starts_with("souv_"));
        assert_eq!(ensure_token(dir.path(), "a").await.unwrap(), first);
    }

    #[tokio::test]
    async fn ensure_replaces_an_empty_token_file() {
        let dir = tempfile::tempdir().unwrap();
        write_token(dir.path(), "a", "  \n").await.unwrap();
        let minted = ensure_token(dir.path(), "a").await.unwrap();
        assert!(minted.starts_with("souv_"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_refuses_to_mint_over_a_tampered_token() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write_token(dir.path(), "a", "souv_original").await.unwrap();
        let path = token_path(dir.path(), "a");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(ensure_token(dir.path(), "a").await.is_err());
        // The credential on disk is untouched — a wrong mode is a signal.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "souv_original"
        );
    }

    #[tokio::test]
    async fn rotate_replaces_an_existing_token() {
        let dir = tempfile::tempdir().unwrap();
        let first = ensure_token(dir.path(), "a").await.unwrap();
        let second = rotate_token(dir.path(), "a").await.unwrap();
        assert_ne!(first, second);
        assert_eq!(read_token(dir.path(), "a").await.unwrap(), second);
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
