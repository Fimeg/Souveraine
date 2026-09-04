//! Wire protocol for souveraine-machined — the system-tier machine identity
//! daemon.
//!
//! One JSON object per line over a Unix socket. Responses follow the guarded
//! `ok`/`reason` pattern used by the session IPC surface: refusals always
//! carry a reason, never a silent failure.
//!
//! Signatures are domain-separated: the daemon never signs caller-supplied
//! bytes raw. A machined signature can therefore never be confused with (or
//! replayed as) a federation envelope, a node commission, or any other
//! payload signed by the same key outside this protocol.

// Shared with the souveraine-machined bin via #[path]; the main crate uses
// only a subset of these items.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Where the daemon listens. `RuntimeDirectory=souveraine` in the unit owns
/// the parent; the socket itself is group-rw so members of the `souveraine`
/// group may connect.
pub const DEFAULT_SOCKET_PATH: &str = "/run/souveraine/machined.sock";

/// Where the machine seed lives on the system tier. Root-of-trust for this
/// box; provisioned deliberately via `souveraine machine init`, never by the
/// daemon itself.
pub const DEFAULT_SEED_DIR: &str = "/var/lib/souveraine/seed-id";

/// Upper bound on one request line. Anything longer is refused, not read.
pub const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Version tag baked into every signature's domain separation.
pub const SIGNING_CONTEXT: &str = "souveraine-machined:v1";

/// Domain for federation transport envelopes (`SignedEvent`).
pub const DOMAIN_FEDERATION_ENVELOPE: &str = "federation-envelope";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Daemon liveness + identity summary.
    Status,
    /// Public key + glyph only.
    Pubkey,
    /// Sign `payload_hex` under `domain`. The signed bytes are
    /// `{SIGNING_CONTEXT}:{domain}:{payload}` — see [`signing_bytes`].
    Sign { domain: String, payload_hex: String },
}

/// The exact bytes a machined signature covers.
pub fn signing_bytes(domain: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SIGNING_CONTEXT.len() + domain.len() + payload.len() + 2);
    bytes.extend_from_slice(SIGNING_CONTEXT.as_bytes());
    bytes.push(b':');
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(b':');
    bytes.extend_from_slice(payload);
    bytes
}

/// Domains are short ASCII labels. Excluding `:` keeps [`signing_bytes`]
/// framing unambiguous.
pub fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 64
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_bytes_are_framed() {
        let bytes = signing_bytes("federation-envelope", b"payload");
        assert_eq!(
            bytes,
            b"souveraine-machined:v1:federation-envelope:payload".to_vec()
        );
    }

    #[test]
    fn domain_charset_is_enforced() {
        assert!(valid_domain("federation-envelope"));
        assert!(valid_domain("node.commission_v1"));
        assert!(!valid_domain(""));
        assert!(!valid_domain("has:colon"));
        assert!(!valid_domain("has space"));
        assert!(!valid_domain(&"x".repeat(65)));
    }

    #[test]
    fn request_round_trips() {
        let req: Request =
            serde_json::from_str(r#"{"op":"sign","domain":"d","payload_hex":"00ff"}"#).unwrap();
        match req {
            Request::Sign {
                domain,
                payload_hex,
            } => {
                assert_eq!(domain, "d");
                assert_eq!(payload_hex, "00ff");
            }
            _ => panic!("wrong variant"),
        }
    }
}
