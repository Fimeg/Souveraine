//! Federation transport — signed event streams between Souveraine instances.
//!
//! Each instance runs a [`FederationBridge`]: for every configured peer it
//! opens an outbound WebSocket client to that peer's `/v1/federation/events`
//! and pushes every locally-originated
//! [`SensorEvent`](crate::core::nervous::SensorEvent) that matches the peer's
//! subscriptions, Ed25519-signed. Inbound events arrive symmetrically —
//! remote bridges connect to *this* instance's `/v1/federation/events`
//! handler, which verifies the signature before injecting onto the local
//! bus. The firehose is left untouched: it remains a plain observability
//! stream for humans and the TUI, not a federation transport.

pub mod bridge;
pub mod types;

pub use bridge::FederationBridge;
pub use types::SignedEvent;
