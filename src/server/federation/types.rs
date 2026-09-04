//! Signed event envelope for federation transport.

use serde::{Deserialize, Serialize};

use crate::core::config::PeerConfig;
use crate::core::identity::SeedId;
use crate::core::nervous::SensorEvent;
use crate::machined::protocol::{signing_bytes, DOMAIN_FEDERATION_ENVELOPE};
use crate::machined::signer::MachineSigner;

/// Return whether an inbound transport signer is explicitly trusted by this
/// instance's federation configuration. A valid signature only establishes
/// that a message is self-consistent; it does not establish that its key is a
/// peer we chose to federate with.
pub fn signer_is_trusted(peers: &[PeerConfig], signer_pubkey_hex: &str) -> bool {
    peers
        .iter()
        .any(|peer| peer.pubkey.eq_ignore_ascii_case(signer_pubkey_hex))
}

/// A [`SensorEvent`] carried between instances, Ed25519-signed by its origin.
/// The signature covers the domain-separated frame
/// `signing_bytes(DOMAIN_FEDERATION_ENVELOPE, canonical_json(event))` — so an
/// envelope signature can never be replayed as any other payload class the
/// machine key signs, and the wire format is identical whether the key lives
/// in souveraine-machined or a legacy user-tier seed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEvent {
    pub event: SensorEvent,
    /// Hex-encoded Ed25519 signature over the domain-separated frame.
    pub signature_hex: String,
    /// Hex-encoded Ed25519 public key of the signing instance.
    pub signer_pubkey_hex: String,
}

impl SignedEvent {
    /// Wrap and sign a locally-originated event for transmission to a peer.
    /// Fails if the signer cannot produce a signature (daemon died mid-run);
    /// callers must treat that as fail-closed — never send unsigned.
    pub fn sign(event: &SensorEvent, signer: &MachineSigner) -> anyhow::Result<Self> {
        let canonical = serde_json::to_vec(event)?;
        let signature_hex = signer.sign(DOMAIN_FEDERATION_ENVELOPE, &canonical)?;
        Ok(Self {
            event: event.clone(),
            signature_hex,
            signer_pubkey_hex: signer.pubkey_hex(),
        })
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
        let framed = signing_bytes(DOMAIN_FEDERATION_ENVELOPE, &canonical);

        if SeedId::verify_with_pubkey(&pubkey, &framed, &signature) {
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

#[cfg(test)]
mod tests {
    use super::{signer_is_trusted, MachineSigner, SignedEvent};
    use crate::core::config::PeerConfig;
    use crate::core::identity::SeedId;
    use crate::core::nervous::SensorEvent;
    use std::sync::Arc;

    #[test]
    fn envelope_round_trips_and_stamps_signer() {
        let seed = SeedId::generate();
        let pubkey_hex = seed.public_key_hex();
        let signer = MachineSigner::Legacy(Arc::new(seed));
        let event = SensorEvent {
            sensor_name: "test".into(),
            timestamp: chrono::Utc::now(),
            event_type: "unit".into(),
            target: None,
            urgency: 0.1,
            payload: None,
            seed_id: None,
            reply_to: None,
        };

        let envelope = SignedEvent::sign(&event, &signer).unwrap();
        let verified = envelope.verify().expect("envelope must verify");
        assert_eq!(verified.seed_id.as_deref(), Some(pubkey_hex.as_str()));

        // Tampering with the event breaks the signature.
        let mut forged = envelope.clone();
        forged.event.event_type = "forged".into();
        assert!(forged.verify().is_none());
    }

    #[test]
    fn configured_peer_key_is_required_for_trust() {
        let peers = vec![PeerConfig {
            url: "ws://phone.example:8484".to_string(),
            pubkey: "ABcd".to_string(),
            subscriptions: vec![],
        }];

        assert!(signer_is_trusted(&peers, "abcd"));
        assert!(!signer_is_trusted(&peers, "different"));
        assert!(!signer_is_trusted(&[], "abcd"));
    }
}
