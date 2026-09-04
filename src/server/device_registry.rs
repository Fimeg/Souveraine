use crate::core::config::FederationRole;
use crate::core::nervous::SensorEvent;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_role() -> String {
    "hearth".to_string()
}

/// A peer device known to this federation. Updated on every `device_announce`
/// and `device_leave` event from the peer's bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// The peer's Ed25519 public key (hex) — also its seed_id.
    pub seed_id: String,
    /// Instance label from the peer's FederationConfig.
    pub label: Option<String>,
    /// The peer's federation endpoint (ws://host:port).
    pub url: String,
    /// The peer's federation role — "hearth" or "limb".
    #[serde(default = "default_role")]
    pub role: String,
    /// First time we saw this peer's announce.
    pub first_seen: DateTime<Utc>,
    /// Most recent announce.
    pub last_seen: DateTime<Utc>,
    /// Is the peer considered alive?
    pub alive: bool,
    /// Agent pubkeys (hex) the peer hosts, from its announce — the live half
    /// of instance awareness ("which agents live where").
    #[serde(default)]
    pub hosted_agents: Vec<String>,
}

/// Tracks known federated peers for the local instance. Written to a JSON file
/// so the CLI (`souveraine peers list`) can query without needing the server.
pub struct DeviceRegistry {
    peers: DashMap<String, PeerEntry>,
    known_peers_path: PathBuf,
    /// This instance's own seed_id (pubkey hex) — filters self-announcements.
    local_seed_id: String,
    /// This machine's role — used to detect a hearth/hearth split-brain.
    local_role: FederationRole,
}

impl DeviceRegistry {
    /// `base_dir` = `~/.souveraine/`. The registry writes to
    /// `{base_dir}/federation/known_peers.json`.
    pub fn new(base_dir: PathBuf, local_seed_id: String, local_role: FederationRole) -> Self {
        let fed_dir = base_dir.join("federation");
        let known_peers_path = fed_dir.join("known_peers.json");
        let peers = DashMap::new();

        // Seed from disk if the file exists (CLI path where server isn't running).
        if let Ok(content) = std::fs::read_to_string(&known_peers_path) {
            if let Ok(disk_peers) = serde_json::from_str::<Vec<PeerEntry>>(&content) {
                for entry in disk_peers {
                    peers.insert(entry.seed_id.clone(), entry);
                }
            }
        }

        Self {
            peers,
            known_peers_path,
            local_seed_id,
            local_role,
        }
    }

    /// Handle a `device_announce` or `device_leave` SensorEvent from the bus.
    /// Returns true if the registry changed.
    pub fn handle_event(&self, event: &SensorEvent) -> bool {
        if event.seed_id.as_deref() == Some(&self.local_seed_id) {
            return false; // Ignore our own announcements.
        }
        match event.event_type.as_str() {
            "device_announce" => {
                let seed_id = match &event.seed_id {
                    Some(id) => id.clone(),
                    None => return false,
                };
                let url = event
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("federation_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let label = event
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("label"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let role = event
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("role"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("hearth")
                    .to_string();
                let hosted_agents = event
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("hosted_agents"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                // Split-brain guard: two hearths for one agent diverge memory.
                // We refuse to auto-resolve — surface it loudly for the human.
                if role == "hearth" && self.local_role == FederationRole::Hearth {
                    tracing::error!(
                        peer = %seed_id,
                        "federation: HEARTH CONFLICT — this machine and {} both \
                         claim hearth. Only one machine should be the hearth; \
                         set [federation].role = \"limb\" on one of them.",
                        seed_id,
                    );
                }
                let now = Utc::now();
                // Preserve the original first_seen across re-announces.
                let first_seen = self
                    .peers
                    .get(&seed_id)
                    .map(|e| e.first_seen)
                    .unwrap_or(now);
                let was_new = !self.peers.contains_key(&seed_id);
                self.peers.insert(
                    seed_id.clone(),
                    PeerEntry {
                        seed_id: seed_id.clone(),
                        label,
                        url,
                        role,
                        first_seen,
                        last_seen: now,
                        alive: true,
                        hosted_agents,
                    },
                );
                self.persist();
                was_new
            }
            "device_leave" => {
                let seed_id = match &event.seed_id {
                    Some(id) => id.clone(),
                    None => return false,
                };
                if let Some(mut entry) = self.peers.get_mut(&seed_id) {
                    entry.alive = false;
                    entry.last_seen = Utc::now();
                    drop(entry);
                    self.persist();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// List all known peers.
    pub fn list(&self) -> Vec<PeerEntry> {
        let mut entries: Vec<PeerEntry> = self.peers.iter().map(|e| e.value().clone()).collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.last_seen));
        entries
    }

    /// Mark peers not heard from within `max_age_secs` as offline.
    pub fn prune_stale(&self, max_age_secs: i64) {
        let cutoff = Utc::now() - chrono::Duration::seconds(max_age_secs);
        let mut changed = false;
        for mut entry in self.peers.iter_mut() {
            if entry.alive && entry.last_seen < cutoff {
                entry.alive = false;
                changed = true;
            }
        }
        if changed {
            self.persist();
        }
    }

    /// Persist known peers to disk (for CLI access).
    fn persist(&self) {
        if let Some(dir) = self.known_peers_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let entries: Vec<PeerEntry> = self.list();
        if let Ok(json) = serde_json::to_string(&entries) {
            let _ = std::fs::write(&self.known_peers_path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announce_records_hosted_agents() {
        let dir = tempfile::tempdir().unwrap();
        let registry = DeviceRegistry::new(
            dir.path().to_path_buf(),
            "local-seed".into(),
            FederationRole::Hearth,
        );
        let event = SensorEvent {
            sensor_name: "federation".into(),
            timestamp: Utc::now(),
            event_type: "device_announce".into(),
            target: None,
            urgency: 0.0,
            payload: Some(serde_json::json!({
                "federation_url": "ws://peer:7777",
                "role": "limb",
                "hosted_agents": ["aabbcc", "ddeeff"],
            })),
            seed_id: Some("peer-seed".into()),
            reply_to: None,
        };
        assert!(registry.handle_event(&event));
        let peers = registry.list();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].hosted_agents, vec!["aabbcc", "ddeeff"]);

        // Old announces without the field must still parse — empty roster.
        let bare = SensorEvent {
            payload: Some(serde_json::json!({ "federation_url": "ws://old:7777" })),
            seed_id: Some("old-peer".into()),
            ..event
        };
        registry.handle_event(&bare);
        let peers = registry.list();
        let old = peers.iter().find(|p| p.seed_id == "old-peer").unwrap();
        assert!(old.hosted_agents.is_empty());
    }
}
