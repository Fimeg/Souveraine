//! souveraine-machined — the system tier's machine identity.
//!
//! SouveraineOS splits identity into two tiers. The **machine** (seed at
//! `/var/lib/souveraine/seed-id`, owned by the `souveraine` system user,
//! served by the `souveraine-machined` daemon) exists from boot, before any
//! human authenticates — it is what enrolls with central authority, signs
//! federation transport, and anchors node commissions. **Agents** (per-agent
//! seeds beside their memfs) belong to the user tier and exist only inside
//! an authenticated session.
//!
//! This module carries the wire protocol and the user-tier client. The serve
//! loop (`server.rs`) is compiled only into the `souveraine-machined` bin,
//! the same `#[path]` pattern as `souveraine-secrets`.

pub mod client;
pub mod protocol;
pub mod signer;
