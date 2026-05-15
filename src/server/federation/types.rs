//! Signed event envelope for federation transport.

use serde::{Deserialize, Serialize};

use crate::core::identity::SeedId;
use crate::core::nervous::SensorEvent;

/// A [`SensorEvent`] carried between instances, Ed25519-signed by its origin.
/// The signature covers the canonical JSON of `event` as it was at the
/// sender — verified before the event is allowed onto the local bus.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEvent {
    pub event: SensorEvent,
    /// Hex-encoded Ed25519 signature over `serde_json::to_vec(&event)`.
    pub signature_hex: String,
    /// Hex-encoded Ed25519 public key of the signing instance.
    pub signer_pubkey_hex: String,
}

impl SignedEvent {
    /// Wrap and sign a locally-originated event for transmission to a peer.
    pub fn sign(event: &SensorEvent, seed: &SeedId) -> Self {
        let bytes = serde_json::to_vec(event).unwrap_or_default();
        let signature = seed.sign(&bytes);
        Self {
            event: event.clone(),
            signature_hex: hex::encode(signature.to_bytes()),
            signer_pubkey_hex: seed.public_key_hex(),
        }
    }

    /// Verify the signature. On success, returns the inner event with its
    /// `seed_id` stamped to the signer — marking it peer-originated so the
    /// local bridge will not echo it back. Returns `None` if the payload is
    /// malformed or the signature does not verify.
    pub fn verify(&self) -> Option<SensorEvent> {
        let pubkey = decode_array::<32>(&self.signer_pubkey_hex)?;
        let sig_bytes = decode_array::<64>(&self.signature_hex)?;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        let canonical = serde_json::to_vec(&self.event).ok()?;

        if SeedId::verify_with_pubkey(&pubkey, &canonical, &signature) {
            let mut event = self.event.clone();
            event.seed_id = Some(self.signer_pubkey_hex.clone());
            Some(event)
        } else {
            None
        }
    }
}

/// Decode a hex string into a fixed-size byte array.
fn decode_array<const N: usize>(hex_str: &str) -> Option<[u8; N]> {
    let bytes = hex::decode(hex_str).ok()?;
    bytes.try_into().ok()
}
