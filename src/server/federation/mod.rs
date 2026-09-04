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

/// Read the agent pubkeys this device hosts from
/// `{base}/agents/*/seed/public.key`. Cheap — a handful of 32-byte
/// files, no DB, no memfs load. Announced to peers so the federation knows
/// which agents live where (instance awareness, FEDERATION.md).
///
/// Note the tree: `{base}/agents/<id>/` holds memory + seed + schedules
/// (the live layout); `{base}/server/agents/<id>/` holds agent.json +
/// conversations and has NO seeds. Entries without a 32-byte
/// `seed/public.key` (e.g. the `schedules`/`system` dirs) are skipped.
pub fn load_hosted_agent_pubkeys(base: &std::path::Path) -> Vec<String> {
    let agents_dir = base.join("agents");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let pk = entry.path().join("seed").join("public.key");
            if let Ok(bytes) = std::fs::read(&pk) {
                if bytes.len() == 32 {
                    out.push(hex::encode(bytes));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_agent_pubkeys_read_the_live_agents_tree() {
        let base = tempfile::tempdir().unwrap();
        // Live layout: {base}/agents/<id>/seed/public.key
        let seed = base.path().join("agents/agent-a/seed");
        std::fs::create_dir_all(&seed).unwrap();
        std::fs::write(seed.join("public.key"), [7u8; 32]).unwrap();
        // Non-agent dirs in the same tree must be skipped.
        std::fs::create_dir_all(base.path().join("agents/schedules")).unwrap();
        // A seed in the server/agents tree (conversations layout) must NOT count.
        let wrong = base.path().join("server/agents/agent-b/seed");
        std::fs::create_dir_all(&wrong).unwrap();
        std::fs::write(wrong.join("public.key"), [9u8; 32]).unwrap();
        // Truncated keys are ignored.
        let short = base.path().join("agents/agent-c/seed");
        std::fs::create_dir_all(&short).unwrap();
        std::fs::write(short.join("public.key"), [1u8; 16]).unwrap();

        let keys = load_hosted_agent_pubkeys(base.path());
        assert_eq!(keys, vec![hex::encode([7u8; 32])]);
    }
}
