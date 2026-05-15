//! Lite listener — a minimal Souveraine presence.
//!
//! `souveraine listen` runs just enough to be reachable: the federation
//! transport, the inbound `/v1/federation/events` endpoint, the firehose, and
//! a summon-wake watcher. No agents, no database, no inference engine are
//! loaded — the process stays small and starts fast.
//!
//! When a summon targeted at this instance arrives from an authorized peer,
//! the listener parks it in `~/.souveraine/.summon-pending/{request_id}.json`.
//! With `[federation].auto_wake`, it then spawns the full `souveraine server`
//! to pick the request up and answer it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocket, State, WebSocketUpgrade},
    routing::get,
    Router,
};

use crate::core::config::FederationConfig;
use crate::core::identity::{verify_summon, SeedId};
use crate::core::nervous::{EventBus, SensorEvent};
use crate::server::federation::{FederationBridge, SignedEvent};

/// Shared state for the lite route set — only what the minimal handlers and
/// the wake watcher need.
pub struct LiteListener {
    pub event_bus: EventBus,
    pub local_seed_id: String,
    /// Agent pubkeys (hex) permitted to `consult` here — the consent floor.
    pub authorized_summoners: Vec<String>,
    /// Agent pubkeys (hex) this device hosts. A summon signed by one of these
    /// is a genuine self-extension (`reach`) and bypasses the consent floor.
    /// Read from each agent's `seed/public.key` — no engine, no memfs load.
    pub hosted_agent_pubkeys: Vec<String>,
    pub auto_wake: bool,
}

/// Read the agent pubkeys this device hosts from `server/agents/*/seed/public.key`.
/// Cheap enough for the lite path — a handful of 32-byte files, no DB.
fn load_hosted_agent_pubkeys(base: &Path) -> Vec<String> {
    let agents_dir = base.join("server").join("agents");
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
    out
}

impl LiteListener {
    /// Build the listener: load the seed identity, start the federation
    /// bridge to configured peers, and spawn the summon-wake watcher.
    pub fn new(config: &FederationConfig, base: PathBuf) -> anyhow::Result<Arc<Self>> {
        let event_bus = EventBus::default();
        let seed = SeedId::load_or_generate(&SeedId::default_dir(&base))?;
        let local_seed_id = seed.public_key_hex();

        let listener = Arc::new(Self {
            event_bus: event_bus.clone(),
            local_seed_id,
            authorized_summoners: config.authorized_summoners.clone(),
            hosted_agent_pubkeys: load_hosted_agent_pubkeys(&base),
            auto_wake: config.auto_wake,
        });

        // Outbound federation bridge — so this instance can also reach peers.
        if !config.peers.is_empty() {
            let mut bridge = FederationBridge::new(event_bus.clone(), Arc::new(seed), config.role);
            for peer in &config.peers {
                bridge.add_peer(peer.clone());
            }
            bridge.run();
        }

        listener.clone().spawn_wake_watcher(base);
        Ok(listener)
    }

    /// Watch the bus for summon requests targeted at this instance. An
    /// authorized summon is parked for the full engine; with `auto_wake`,
    /// the full server is then spawned to answer it.
    fn spawn_wake_watcher(self: Arc<Self>, base: PathBuf) {
        let mut rx = self.event_bus.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if event.sensor_name != "summon_request" {
                    continue;
                }
                if event.target.as_deref() != Some(&self.local_seed_id) {
                    continue;
                }
                // Authenticate by the agent signature — never by trusting the
                // event's `event_type`. A summon signed by an agent we host is
                // a genuine self-extension (reach) and bypasses the consent
                // floor; anything else must be an authorized summoner (consult).
                let field = |k: &str| event.payload.as_ref()
                    .and_then(|p| p.get(k))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let request_id = field("request_id");
                let prompt = field("prompt");
                let agent_pubkey = field("agent_pubkey");
                let agent_sig = field("agent_sig");
                let target = event.target.clone().unwrap_or_default();

                if !verify_summon(
                    &agent_pubkey, &agent_sig, &request_id,
                    &event.event_type, &target, &prompt,
                ) {
                    tracing::warn!(
                        request_id = %request_id,
                        "lite listener: invalid agent signature — summon ignored"
                    );
                    continue;
                }

                let is_self = self.hosted_agent_pubkeys.iter().any(|p| p == &agent_pubkey);
                let authorized = is_self
                    || self.authorized_summoners.iter().any(|s| s == &agent_pubkey);
                if !authorized {
                    tracing::warn!(
                        summoner = %agent_pubkey,
                        "lite listener: unauthorized summon ignored"
                    );
                    continue;
                }
                if let Err(e) = park_summon(&base, &event) {
                    tracing::warn!(error = %e, "lite listener: failed to park summon");
                    continue;
                }
                if self.auto_wake {
                    wake_full_server();
                }
            }
        });
    }
}

/// Persist an incoming summon so the full engine can pick it up.
fn park_summon(base: &Path, event: &SensorEvent) -> anyhow::Result<()> {
    let request_id = event
        .payload
        .as_ref()
        .and_then(|p| p.get("request_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let dir = base.join(".summon-pending");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(event)?;
    std::fs::write(dir.join(format!("{request_id}.json")), json)?;
    tracing::info!(request_id, "lite listener: summon parked");
    Ok(())
}

/// Spawn the full `souveraine server` to process parked summons.
///
/// NOTE: the spawned server binds the configured HTTP port — if the lite
/// listener holds that port, run the full server on a distinct port or stop
/// the listener first. A dedicated one-shot `--resume-summon` mode is the
/// proper finish for this path.
fn wake_full_server() {
    match std::env::current_exe() {
        Ok(exe) => match std::process::Command::new(exe).arg("server").spawn() {
            Ok(_) => tracing::info!("lite listener: woke full server"),
            Err(e) => tracing::warn!(error = %e, "lite listener: failed to spawn full server"),
        },
        Err(e) => tracing::warn!(error = %e, "lite listener: cannot locate own binary"),
    }
}

/// The minimal route set: health, firehose, the federation inbound endpoint.
pub fn create_routes_lite(state: Arc<LiteListener>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/firehose", get(firehose_lite))
        .route("/v1/federation/events", get(federation_events_lite))
        .with_state(state)
}

async fn firehose_lite(
    State(listener): State<Arc<LiteListener>>,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| firehose_lite_stream(listener, socket))
}

async fn firehose_lite_stream(listener: Arc<LiteListener>, mut socket: WebSocket) {
    use axum::extract::ws::Message;
    let mut rx = listener.event_bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                let json = match serde_json::to_string(&event) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn federation_events_lite(
    State(listener): State<Arc<LiteListener>>,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| federation_events_lite_stream(listener, socket))
}

async fn federation_events_lite_stream(listener: Arc<LiteListener>, mut socket: WebSocket) {
    use axum::extract::ws::Message;
    while let Some(msg) = socket.recv().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        let signed: SignedEvent = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Some(event) = signed.verify() {
            listener.event_bus.send(event);
        }
    }
}
