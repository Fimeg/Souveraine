//! MachineSigner — how server code signs as the machine without knowing
//! which tier holds the key.
//!
//! Daemon mode asks souveraine-machined over its socket; the private key
//! never enters this process. Legacy mode holds the old user-tier seed
//! directly — transitional, loud at resolve time, and it signs the exact
//! same domain-separated bytes the daemon does, so the wire format is
//! identical either way. Verifiers reconstruct
//! `protocol::signing_bytes(domain, payload)` and never care where the key
//! lived.
//!
//! Neither mode generates identity. A machine with no seed is an error with
//! provisioning instructions, not a fresh key.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use super::{client, protocol};
use crate::core::identity::SeedId;

pub enum MachineSigner {
    /// souveraine-machined answers on its socket; we hold only the pubkey.
    Daemon { pubkey_hex: String },
    /// Transitional: the legacy user-tier seed, held in-process.
    Legacy(Arc<SeedId>),
}

impl MachineSigner {
    /// Resolve the machine identity: system tier first, legacy user seed as
    /// a loud fallback. Blocking (one local socket round-trip / file read) —
    /// call at startup, not per-event.
    pub fn resolve(base: &Path) -> Result<Self> {
        match client::pubkey() {
            Ok((pubkey_hex, glyph)) => {
                info!("machine identity via souveraine-machined: {glyph} ({pubkey_hex})");
                return Ok(Self::Daemon { pubkey_hex });
            }
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
                 seed exists at {} — provision one with `sudo souveraine machine init --fresh`",
                legacy_dir.display()
            )
        })?;
        Ok(Self::Legacy(Arc::new(seed)))
    }

    pub fn pubkey_hex(&self) -> String {
        match self {
            Self::Daemon { pubkey_hex } => pubkey_hex.clone(),
            Self::Legacy(seed) => seed.public_key_hex(),
        }
    }

    /// Hex Ed25519 signature over `protocol::signing_bytes(domain, payload)`.
    /// Daemon mode is a blocking local socket round-trip (microseconds); a
    /// dead daemon mid-run is an error the caller must treat as fail-closed —
    /// never send unsigned.
    pub fn sign(&self, domain: &str, payload: &[u8]) -> Result<String> {
        match self {
            Self::Daemon { .. } => client::sign(domain, payload),
            Self::Legacy(seed) => {
                let bytes = protocol::signing_bytes(domain, payload);
                Ok(hex::encode(seed.sign(&bytes).to_bytes()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_signature_covers_domain_separated_bytes() {
        let seed = SeedId::generate();
        let pubkey = seed.public_key_bytes();
        let signer = MachineSigner::Legacy(Arc::new(seed));

        let sig_hex = signer.sign("test-domain", b"payload").unwrap();
        let sig_bytes: [u8; 64] = hex::decode(sig_hex).unwrap().try_into().unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);

        let framed = protocol::signing_bytes("test-domain", b"payload");
        assert!(SeedId::verify_with_pubkey(&pubkey, &framed, &signature));
        // Raw payload must NOT verify — domain separation is load-bearing.
        assert!(!SeedId::verify_with_pubkey(&pubkey, b"payload", &signature));
    }
}
