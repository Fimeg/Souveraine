//! Storage-key resolution — how the daemon derives its AES-256 store key
//! from the *machine* identity without ever holding the machine private key.
//!
//! System tier first: ask souveraine-machined to sign the fixed key-derivation
//! payload over its socket. Ed25519 is deterministic, so the same machine key
//! yields the same signature — and therefore the same storage key — on every
//! boot. The private key never enters this process.
//!
//! Legacy fallback: machines from before the system tier carry their seed at
//! `~/.souveraine/seed-id`. Load it, sign the *identical* framed bytes
//! (`machined_protocol::signing_bytes`), and drop the key immediately after
//! derivation. Loud at resolve time, byte-identical output — a later
//! `souveraine machine init --migrate-from` keeps the store decryptable.
//!
//! Neither path generates identity. A machine with no seed is an error with
//! provisioning instructions, not a fresh key.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::identity::SeedId;
use crate::machined_protocol as protocol;

/// Domain under which machined signs the key-derivation payload. Domain
/// separation means this signature can never be replayed as a federation
/// envelope or any other machined-signed artifact.
const KEY_DOMAIN: &str = "secrets-store-key";
/// Versioned payload — bump only with a deliberate store migration.
const KEY_PAYLOAD: &[u8] = b"v1";

/// Which tier answered — callers report this so an audit can tell whether the
/// box has moved to the system tier yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Machined,
    LegacyUserSeed,
}

/// Resolve the machine key material — the deterministic Ed25519 signature
/// over the fixed derivation payload. The store HKDF-expands this into its
/// KEKs; the machine private key itself never reaches the caller. Machined
/// socket first, legacy user-tier seed as a loud transitional fallback.
pub fn resolve(base: &Path) -> Result<(Vec<u8>, KeySource)> {
    match machined_sign(KEY_DOMAIN, KEY_PAYLOAD) {
        Ok(signature) => {
            info!("storage key rooted in the system-tier machine identity (souveraine-machined)");
            return Ok((signature, KeySource::Machined));
        }
        Err(e) => {
            warn!(
                "souveraine-machined unavailable ({e:#}); falling back to the legacy \
                 user-tier machine seed — provision the system tier with \
                 `sudo souveraine machine init`"
            );
        }
    }

    let legacy_dir = SeedId::default_dir(base);
    let seed = SeedId::load(&legacy_dir).with_context(|| {
        format!(
            "no machine identity: souveraine-machined is not running and no legacy seed \
             exists at {} — provision one with `sudo souveraine machine init --fresh` \
             (or `--migrate-from <dir>` to carry an existing identity over)",
            legacy_dir.display()
        )
    })?;
    let framed = protocol::signing_bytes(KEY_DOMAIN, KEY_PAYLOAD);
    let signature = seed.sign(&framed).to_bytes().to_vec();
    Ok((signature, KeySource::LegacyUserSeed))
}

/// One `Sign` round-trip against the machined socket. Mirrors
/// `machined::client` (which the bin cannot reuse — its module paths assume
/// the main crate root); the wire format is the shared `machined_protocol`.
fn machined_sign(domain: &str, payload: &[u8]) -> Result<Vec<u8>> {
    let path = std::env::var_os("SOUVERAINE_MACHINED_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(protocol::DEFAULT_SOCKET_PATH));

    let stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting to souveraine-machined at {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = protocol::Request::Sign {
        domain: domain.to_string(),
        payload_hex: hex::encode(payload),
    };
    let mut writer = stream.try_clone()?;
    writer.write_all(serde_json::to_string(&request)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("reading souveraine-machined response")?;
    let value: serde_json::Value =
        serde_json::from_str(line.trim()).context("parsing souveraine-machined response")?;

    if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unspecified refusal");
        anyhow::bail!("souveraine-machined refused: {reason}");
    }
    let signature_hex = value
        .get("signature")
        .and_then(|v| v.as_str())
        .context("response missing signature")?;
    hex::decode(signature_hex).context("decoding signature hex")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The legacy path must frame exactly like machined does, or the same
    /// machine key would produce two different key materials across the tier
    /// migration and silently orphan every stored secret.
    #[test]
    fn legacy_derivation_uses_machined_framing() {
        let seed = SeedId::generate();
        let framed = protocol::signing_bytes(KEY_DOMAIN, KEY_PAYLOAD);
        assert_eq!(
            framed,
            b"souveraine-machined:v1:secrets-store-key:v1".to_vec()
        );
        // Ed25519 is deterministic — same seed, same payload, same material.
        assert_eq!(seed.sign(&framed).to_bytes(), seed.sign(&framed).to_bytes());
    }
}
