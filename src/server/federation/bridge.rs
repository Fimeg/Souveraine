//! The federation bridge — outbound signed-event streams to peers.

use std::sync::Arc;
use std::time::Duration;

use futures::SinkExt;
use tokio_tungstenite::tungstenite::Message;

use crate::core::config::{FederationRole, PeerConfig};
use crate::core::identity::SeedId;
use crate::core::nervous::{EventBus, SensorEvent};

use super::types::SignedEvent;

/// Runs one outbound task per configured peer. Each task maintains a
/// WebSocket client connection to the peer's `/v1/federation/events`,
/// forwarding every locally-originated event that matches the peer's
/// subscriptions, Ed25519-signed. Inbound events are *not* handled here —
/// they arrive on this instance's own `/v1/federation/events` endpoint.
pub struct FederationBridge {
    event_bus: EventBus,
    seed: Arc<SeedId>,
    role: FederationRole,
    peers: Vec<PeerConfig>,
}

impl FederationBridge {
    pub fn new(event_bus: EventBus, seed: Arc<SeedId>, role: FederationRole) -> Self {
        Self {
            event_bus,
            seed,
            role,
            peers: Vec::new(),
        }
    }

    /// Register a peer to connect to. Call before [`run`](Self::run).
    pub fn add_peer(&mut self, peer: PeerConfig) {
        self.peers.push(peer);
    }

    /// Spawn the outbound task for every registered peer. Each task owns its
    /// own reconnect loop and runs for the life of the process.
    pub fn run(self) {
        for peer in self.peers {
            tokio::spawn(peer_outbound_task(
                peer,
                self.event_bus.clone(),
                self.seed.clone(),
                self.role,
            ));
        }
    }
}

/// Connect to one peer and forward signed events, reconnecting with
/// exponential backoff whenever the link drops.
async fn peer_outbound_task(
    peer: PeerConfig,
    event_bus: EventBus,
    seed: Arc<SeedId>,
    role: FederationRole,
) {
    let endpoint = federation_endpoint(&peer.url);
    let mut retry: u32 = 0;

    loop {
        match tokio_tungstenite::connect_async(endpoint.as_str()).await {
            Ok((mut ws, _resp)) => {
                retry = 0;
                tracing::info!(peer = %endpoint, "federation: outbound connected");

                // Announce our presence to the peer.
                let announce = SensorEvent {
                    sensor_name: "federation".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "device_announce".into(),
                    target: None,
                    urgency: 0.0,
                    payload: Some(serde_json::json!({
                        "federation_url": endpoint,
                        "label": None::<String>,
                        "pubkey": seed.public_key_hex(),
                        "role": role.as_str(),
                    })),
                    seed_id: None,
                    reply_to: None,
                };
                let signed = SignedEvent::sign(&announce, &seed);
                if let Ok(json) = serde_json::to_string(&signed) {
                    let _ = ws.send(Message::Text(json)).await;
                }

                let mut rx = event_bus.subscribe();

                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            // Only forward locally-originated events. An event
                            // carrying a seed_id came from a peer — forwarding
                            // it would echo the federation into a loop.
                            if event.seed_id.is_some() {
                                continue;
                            }
                            // Control-plane events (discovery, summons) always
                            // cross; data events respect the peer's subscriptions.
                            if !is_control_event(&event.sensor_name)
                                && !subscription_matches(&peer.subscriptions, &event.sensor_name)
                            {
                                continue;
                            }
                            let signed = SignedEvent::sign(&event, &seed);
                            let json = match serde_json::to_string(&signed) {
                                Ok(j) => j,
                                Err(_) => continue,
                            };
                            if ws.send(Message::Text(json)).await.is_err() {
                                tracing::warn!(peer = %endpoint, "federation: send failed — reconnecting");
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(peer = %endpoint, dropped = n, "federation: outbound lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::info!(peer = %endpoint, "federation: local bus closed — outbound task ending");
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(peer = %endpoint, error = %e, "federation: connect failed");
            }
        }

        retry = retry.saturating_add(1);
        let delay = backoff_delay(retry);
        tracing::debug!(peer = %endpoint, ?delay, "federation: backing off before reconnect");
        tokio::time::sleep(delay).await;
    }
}

/// Resolve a peer's base URL to its federation endpoint.
fn federation_endpoint(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1/federation/events") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/federation/events")
    }
}

/// Subscription matching: `*` matches everything, `prefix*` is a prefix
/// match, a bare name is exact. No subscriptions = nothing forwarded.
fn subscription_matches(subscriptions: &[String], sensor_name: &str) -> bool {
    subscriptions.iter().any(|s| {
        if s == "*" {
            true
        } else if let Some(prefix) = s.strip_suffix('*') {
            sensor_name.starts_with(prefix)
        } else {
            s == sensor_name
        }
    })
}

/// Control-plane events always cross the federation regardless of a peer's
/// data subscriptions — discovery and directed summons must arrive. The
/// receiving side filters by `target`, so a broadcast is safe.
fn is_control_event(sensor_name: &str) -> bool {
    matches!(sensor_name, "federation" | "summon_request" | "summon_response")
}

/// Exponential backoff — 1s, 2s, 4s … capped at 60s, plus up to 1s jitter.
fn backoff_delay(retry: u32) -> Duration {
    let shift = retry.saturating_sub(1).min(6);
    let secs = (1u64 << shift).min(60);
    let jitter_ms = rand::random::<u64>() % 1000;
    Duration::from_millis(secs * 1000 + jitter_ms)
}
