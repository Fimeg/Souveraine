//! Versioned identity records for a commissioned federation node.
//!
//! These types are intentionally transport- and storage-neutral. They define
//! what a future `node invite` / `node join` ceremony signs without yet
//! selecting a commissioning authority, a RedFlag integration, or a token
//! format.

// Commissioning records are intentionally staged ahead of the enrolment CLI
// and federation transport. Keep this public model lint-clean while those
// callers land in the next implementation phase.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use ed25519_dalek::Signature;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::SeedId;

/// First version of the node commission payload.
pub const NODE_COMMISSION_VERSION: u16 = 1;

/// An immutable, randomly minted identifier for one physical federation node.
///
/// This is deliberately distinct from a hostname or OS machine ID. Those may
/// change; a node ID names the branch and the commission for its lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(Uuid);

impl NodeId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A node-generated request to join one agent's federation.
///
/// The node private key stays on the device. `hardware_key_fingerprint` is an
/// opaque RedFlag-facing reference for now; its concrete attestation format is
/// a later integration decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeJoinRequest {
    pub version: u16,
    pub node_id: NodeId,
    pub agent_root_pubkey_hex: String,
    pub node_pubkey_hex: String,
    pub hardware_key_fingerprint: String,
    pub label: String,
    pub requested_at: DateTime<Utc>,
}

impl NodeJoinRequest {
    pub fn new(
        agent_root_pubkey_hex: String,
        node_pubkey_hex: String,
        hardware_key_fingerprint: String,
        label: String,
    ) -> Self {
        Self {
            version: NODE_COMMISSION_VERSION,
            node_id: NodeId::generate(),
            agent_root_pubkey_hex,
            node_pubkey_hex,
            hardware_key_fingerprint,
            label,
            requested_at: Utc::now(),
        }
    }

    pub fn validate(&self) -> Result<(), NodeIdentityError> {
        if self.version != NODE_COMMISSION_VERSION {
            return Err(NodeIdentityError::UnsupportedVersion(self.version));
        }
        validate_pubkey("agent root", &self.agent_root_pubkey_hex)?;
        validate_pubkey("node", &self.node_pubkey_hex)?;
        if self.hardware_key_fingerprint.trim().is_empty() {
            return Err(NodeIdentityError::MissingHardwareFingerprint);
        }
        if self.label.trim().is_empty() || self.label.chars().count() > 64 {
            return Err(NodeIdentityError::InvalidLabel);
        }
        Ok(())
    }
}

/// A commission issued by an authority trusted by the agent root.
///
/// The commissioner may initially be the root key. Keeping its public key
/// explicit permits a later RedFlag-backed delegated authority without
/// changing the signed request shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCommission {
    pub request: NodeJoinRequest,
    pub commissioner_pubkey_hex: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub signature_hex: String,
}

impl NodeCommission {
    pub fn issue(
        commissioner: &SeedId,
        request: NodeJoinRequest,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Self, NodeIdentityError> {
        request.validate()?;
        let commissioner_pubkey_hex = commissioner.public_key_hex();
        let issued_at = Utc::now();
        let bytes =
            commission_signing_bytes(&request, &commissioner_pubkey_hex, issued_at, expires_at);
        let signature_hex = hex::encode(commissioner.sign(&bytes).to_bytes());

        Ok(Self {
            request,
            commissioner_pubkey_hex,
            issued_at,
            expires_at,
            signature_hex,
        })
    }

    /// Verify payload structure, expiry, and the commissioner's signature.
    /// Trusting that commissioner for this agent is deliberately the caller's
    /// policy decision; this method only establishes cryptographic validity.
    pub fn verify(&self, now: DateTime<Utc>) -> Result<(), NodeIdentityError> {
        self.request.validate()?;
        validate_pubkey("commissioner", &self.commissioner_pubkey_hex)?;
        if self.expires_at.is_some_and(|expiry| expiry <= now) {
            return Err(NodeIdentityError::Expired);
        }

        let pubkey: [u8; 32] = hex::decode(&self.commissioner_pubkey_hex)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(NodeIdentityError::InvalidPublicKey("commissioner"))?;
        let signature_bytes: [u8; 64] = hex::decode(&self.signature_hex)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(NodeIdentityError::InvalidSignature)?;
        let signature = Signature::from_bytes(&signature_bytes);
        let bytes = commission_signing_bytes(
            &self.request,
            &self.commissioner_pubkey_hex,
            self.issued_at,
            self.expires_at,
        );

        if SeedId::verify_with_pubkey(&pubkey, &bytes, &signature) {
            Ok(())
        } else {
            Err(NodeIdentityError::InvalidSignature)
        }
    }
}

/// The canonical bytes covered by a node commission.
pub fn commission_signing_bytes(
    request: &NodeJoinRequest,
    commissioner_pubkey_hex: &str,
    issued_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
) -> Vec<u8> {
    serde_json::to_vec(&(
        NODE_COMMISSION_VERSION,
        request,
        commissioner_pubkey_hex,
        issued_at,
        expires_at,
    ))
    .expect("node commission payload must serialize")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeIdentityError {
    UnsupportedVersion(u16),
    InvalidPublicKey(&'static str),
    MissingHardwareFingerprint,
    InvalidLabel,
    Expired,
    InvalidSignature,
}

impl std::fmt::Display for NodeIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported node commission version {version}")
            }
            Self::InvalidPublicKey(kind) => write!(f, "invalid {kind} public key"),
            Self::MissingHardwareFingerprint => write!(f, "missing hardware key fingerprint"),
            Self::InvalidLabel => {
                write!(f, "node label must be non-empty and at most 64 characters")
            }
            Self::Expired => write!(f, "node commission has expired"),
            Self::InvalidSignature => write!(f, "invalid node commission signature"),
        }
    }
}

impl std::error::Error for NodeIdentityError {}

fn validate_pubkey(kind: &'static str, public_key_hex: &str) -> Result<(), NodeIdentityError> {
    match hex::decode(public_key_hex)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    {
        Some(_) => Ok(()),
        None => Err(NodeIdentityError::InvalidPublicKey(kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(agent: &SeedId, node: &SeedId) -> NodeJoinRequest {
        NodeJoinRequest::new(
            agent.public_key_hex(),
            node.public_key_hex(),
            "redflag:device-public-key:abc123".to_string(),
            "phone".to_string(),
        )
    }

    #[test]
    fn commission_round_trip_verifies() {
        let authority = SeedId::generate();
        let node = SeedId::generate();
        let commission =
            NodeCommission::issue(&authority, request(&authority, &node), None).unwrap();

        assert!(commission.verify(Utc::now()).is_ok());
        assert_eq!(commission.request.label, "phone");
    }

    #[test]
    fn changing_a_commissioned_field_invalidates_the_signature() {
        let authority = SeedId::generate();
        let node = SeedId::generate();
        let mut commission =
            NodeCommission::issue(&authority, request(&authority, &node), None).unwrap();
        commission.request.label = "hearth".to_string();

        assert_eq!(
            commission.verify(Utc::now()),
            Err(NodeIdentityError::InvalidSignature)
        );
    }

    #[test]
    fn expired_commission_fails_closed() {
        let authority = SeedId::generate();
        let node = SeedId::generate();
        let commission = NodeCommission::issue(
            &authority,
            request(&authority, &node),
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .unwrap();

        assert_eq!(
            commission.verify(Utc::now()),
            Err(NodeIdentityError::Expired)
        );
    }
}
