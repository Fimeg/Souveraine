// Core engine modules.
//
// The consciousness types (memory, persona, conversation, archivist, subconscious)
// are being reconstructed in Stage 1 of the harness rebuild as a `Memory` trait
// + Gitea/LocalGit impls living in their own crate. For now this module only
// exposes the stubs that survived the cleanup.

pub mod chain;
pub mod bootstrap;
pub mod compact;
pub mod config;
pub mod credentials;
pub mod voice;
pub mod conversation;
pub mod identity;
pub mod nervous;
pub mod memory;
pub mod prompt;
pub mod reflection;
pub mod sensorium;
pub mod session;
pub mod skills;
pub mod subagent;
pub mod subconscious;
pub mod tools;
