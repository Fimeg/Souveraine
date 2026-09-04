//! Client for souveraine-machined — how user-tier processes reach the
//! system-tier machine identity.
//!
//! Synchronous by design: one tiny local round-trip, callable from both sync
//! tool code and async server code without ceremony.
//!
//! The legacy fallback exists because deployed machines still carry their
//! machine seed at `~/.souveraine/seed-id` from before the system tier
//! existed. It is transitional, loud, and strict — it will `load` an
//! existing legacy seed but never generate one. New identity comes only
//! from `souveraine machine init`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::warn;

use super::protocol::{Request, DEFAULT_SOCKET_PATH};
use crate::core::identity::SeedId;

/// Where the machine seed identity came from — callers that report or audit
/// should say which tier answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineIdentitySource {
    /// The system-tier daemon answered on its socket.
    Machined,
    /// Transitional: read from the legacy user-tier seed directory.
    LegacyUserSeed,
}

/// Test hook only — points the client at a scratch socket. Not a deployment
/// knob; production callers always use the default path.
pub fn socket_path() -> PathBuf {
    std::env::var_os("SOUVERAINE_MACHINED_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

fn request(req: &Request) -> Result<serde_json::Value> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting to souveraine-machined at {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut writer = stream.try_clone()?;
    writer.write_all(serde_json::to_string(req)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("reading souveraine-machined response")?;
    let value: serde_json::Value =
        serde_json::from_str(line.trim()).context("parsing souveraine-machined response")?;

    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(value)
    } else {
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unspecified refusal");
        anyhow::bail!("souveraine-machined refused: {reason}")
    }
}

/// Daemon status as reported by the daemon itself.
pub fn status() -> Result<serde_json::Value> {
    request(&Request::Status)
}

/// Machine public key + glyph from the daemon.
pub fn pubkey() -> Result<(String, String)> {
    let value = request(&Request::Pubkey)?;
    let pk = value
        .get("public_key")
        .and_then(|v| v.as_str())
        .context("response missing public_key")?
        .to_string();
    let glyph = value
        .get("glyph")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((pk, glyph))
}

/// Domain-separated signature over `payload`. Returns the signature hex;
/// the signed bytes are `protocol::signing_bytes(domain, payload)`.
#[allow(dead_code)] // phase 2: federation envelope signing routes through machined
pub fn sign(domain: &str, payload: &[u8]) -> Result<String> {
    let value = request(&Request::Sign {
        domain: domain.to_string(),
        payload_hex: hex::encode(payload),
    })?;
    value
        .get("signature")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("response missing signature")
}

/// Resolve this box's machine public key: system tier first, legacy user
/// seed as a loud transitional fallback. Never generates — a machine with
/// no identity is an error with provisioning instructions, not a fresh key.
pub fn machine_pubkey_with_fallback(base: &Path) -> Result<(String, MachineIdentitySource)> {
    match pubkey() {
        Ok((pk, _glyph)) => return Ok((pk, MachineIdentitySource::Machined)),
        Err(e) => {
            warn!(
                "souveraine-machined unavailable ({e:#}); falling back to legacy \
                 user-tier machine seed — provision the system tier with \
                 `sudo souveraine machine init`"
            );
        }
    }

    let legacy_dir = SeedId::default_dir(base);
    let seed = SeedId::load(&legacy_dir).with_context(|| {
        format!(
            "no machine identity: souveraine-machined is not running and no legacy \
             seed exists at {} — provision one with `sudo souveraine machine init --fresh` \
             (or `--migrate-from <dir>` to carry an existing identity over)",
            legacy_dir.display()
        )
    })?;
    Ok((seed.public_key_hex(), MachineIdentitySource::LegacyUserSeed))
}
