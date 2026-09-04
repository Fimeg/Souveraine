//! Agent-identity signing for federation summons.
//!
//! A `reach`/`consult` request is signed by the *agent* seed — the keypair
//! kept at `agents/{id}/seed/`, which travels with the memfs and is therefore
//! identical across every machine one agent runs on. The receiver verifies
//! this signature against its *own* agent pubkey:
//!
//! - match  → genuinely the same being → `reach`, no summoner-consent gate
//! - differ → a separate being         → `consult`, consent-gated
//!
//! This is distinct from the transport `SignedEvent` envelope, which the
//! *machine* seed signs ("this event came from that box"). Device identity
//! authenticates the wire; agent identity authenticates the being. Both are
//! needed, and they are not the same key.

use ed25519_dalek::Signature;

use super::SeedId;

/// The canonical bytes an agent signature covers — the fields that pin a
/// summon to one request, one declared intent, one target machine, one
/// payload. Caller (signing) and receiver (verifying) must build these
/// byte-identically or every verification fails.
pub fn summon_signing_bytes(request_id: &str, tool: &str, target: &str, prompt: &str) -> Vec<u8> {
    format!("{request_id}\n{tool}\n{target}\n{prompt}").into_bytes()
}

/// Sign a summon with the agent seed. Returns the hex-encoded signature.
pub fn sign_summon(
    agent_seed: &SeedId,
    request_id: &str,
    tool: &str,
    target: &str,
    prompt: &str,
) -> String {
    let bytes = summon_signing_bytes(request_id, tool, target, prompt);
    hex::encode(agent_seed.sign(&bytes).to_bytes())
}

/// Verify a summon's agent signature against the claimed agent pubkey (hex).
/// Returns true only if the signature is valid over those exact fields — a
/// malformed pubkey, malformed signature, or any tampered field fails closed.
pub fn verify_summon(
    agent_pubkey_hex: &str,
    agent_sig_hex: &str,
    request_id: &str,
    tool: &str,
    target: &str,
    prompt: &str,
) -> bool {
    let pubkey: [u8; 32] = match hex::decode(agent_pubkey_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
    {
        Some(p) => p,
        None => return false,
    };
    let sig_bytes: [u8; 64] = match hex::decode(agent_sig_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
    {
        Some(s) => s,
        None => return false,
    };
    let signature = Signature::from_bytes(&sig_bytes);
    let bytes = summon_signing_bytes(request_id, tool, target, prompt);
    SeedId::verify_with_pubkey(&pubkey, &bytes, &signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_verifies() {
        let seed = SeedId::generate();
        let sig = sign_summon(&seed, "req-1", "reach", "machineX", "carry this");
        assert!(verify_summon(
            &seed.public_key_hex(),
            &sig,
            "req-1",
            "reach",
            "machineX",
            "carry this",
        ));
    }

    #[test]
    fn tampered_field_fails() {
        let seed = SeedId::generate();
        let sig = sign_summon(&seed, "req-1", "reach", "machineX", "carry this");
        // Prompt changed after signing.
        assert!(!verify_summon(
            &seed.public_key_hex(),
            &sig,
            "req-1",
            "reach",
            "machineX",
            "carry something else",
        ));
    }

    #[test]
    fn wrong_pubkey_fails() {
        let signer = SeedId::generate();
        let other = SeedId::generate();
        let sig = sign_summon(&signer, "req-1", "consult", "machineX", "a question");
        assert!(!verify_summon(
            &other.public_key_hex(),
            &sig,
            "req-1",
            "consult",
            "machineX",
            "a question",
        ));
    }
}
