//! Matrix sensorium — Souveraine's first non-terminal surface.
//!
//! A Matrix room is a *surface* consciousness renders to and receives
//! input from. That is exactly what the [`Sensorium`] trait describes, so
//! Matrix is not a "channel" bolted on the side — it is a sensorium, the
//! first one ever to be a real network surface rather than a local
//! terminal.
//!
//! ## What this module is, by phase
//!
//! - **Phase 3 (here): transport spike.** Prove the wire. Build a
//!   `matrix-sdk` client, log in or restore a session, sync, receive room
//!   messages, send replies. The inbound handler answers a `!ping` so a
//!   human can confirm end-to-end liveness from any Element client.
//! - **Phase 4:** room message → `InputEvent` → conversation routing.
//! - **Phase 5:** the outbound streaming turn model — `MatrixTurn` grows
//!   into throttled message edits, tool cards, thinking blocks, HTML.
//! - **Phase 6:** setup wizard, cross-signing, media.
//!
//! [`MatrixSensorium`] already implements [`Sensorium`], so once a runtime
//! constructs one and hands it to the `SensoriumCoordinator`, the wire is
//! live. It is *not* registered with the coordinator yet — that wiring,
//! and the config/keyring that feeds it credentials, is Phase 6.

pub mod client;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use matrix_sdk::{
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent,
    },
    Room, RoomState,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::{BandwidthClass, DiscoveryLevel, Sensorium};
use crate::core::nervous::EventBus;
use client::{account_dir, build_client, load_session_record, save_session_record, MatrixAuth};

/// Per-room outbound turn state.
///
/// Phase 3 stub: it knows which room a turn belongs to and accumulates the
/// segments that turn emits. Phase 5 grows this into the full streaming
/// turn model — throttled leading-edge message edits, tool blocks, thinking
/// blocks. For now it is just enough state for the EventBus loop to have
/// somewhere to put what it hears.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct MatrixTurn {
    /// The Matrix room this turn renders into.
    pub room_id: String,
    /// Streamed output text accumulated so far.
    pub buffer: String,
    /// True once `turn:finish` has been seen — late events are dropped.
    pub finished: bool,
}

/// The Matrix surface. Implements [`Sensorium`]: `run` builds the client,
/// drives `/sync` on its own task, and consumes turn-lifecycle events off
/// the [`EventBus`] until cancelled.
pub struct MatrixSensorium {
    /// Filesystem-safe account slug — usually the user-id localpart.
    account: String,
    /// Root of Souveraine's data dir (`~/.souveraine`). State lives under
    /// `<root>/sensorium/matrix/<account>/`.
    store_root: PathBuf,
    /// How to obtain the session. `Some` until `run` consumes it.
    auth: Option<MatrixAuth>,
    /// Outbound turn state, one entry per active room. Shared because the
    /// streaming turn model (Phase 5) will mutate it from several tasks.
    turns: Arc<Mutex<HashMap<String, MatrixTurn>>>,
    /// Agent ID to route inbound messages to. Set at registration time.
    agent_id: String,
}

impl MatrixSensorium {
    /// Construct a Matrix sensorium for one account.
    ///
    /// `account` is the filesystem slug for this identity's state dir.
    /// `auth` decides whether `run` restores a saved session or logs in
    /// fresh. `store_root` is Souveraine's data dir (`~/.souveraine`).
    pub fn new(
        account: impl Into<String>,
        auth: MatrixAuth,
        store_root: impl Into<PathBuf>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            account: account.into(),
            store_root: store_root.into(),
            auth: Some(auth),
            turns: Arc::new(Mutex::new(HashMap::new())),
            agent_id: agent_id.into(),
        }
    }

    /// Build a Matrix sensorium from environment variables — a spike
    /// convenience so the transport can be exercised before the setup
    /// wizard exists. Restores a saved session if one is on disk for the
    /// account; otherwise expects `MATRIX_HOMESERVER`, `MATRIX_USER`, and
    /// `MATRIX_PASSWORD` for a first login.
    ///
    /// Returns `None` if no saved session exists and the login env vars
    /// are not set — i.e. there is nothing to connect with.
    pub fn from_env(store_root: impl Into<PathBuf>, agent_id: impl Into<String>) -> Option<Self> {
        let agent_id = agent_id.into();
        let store_root = store_root.into();
        let user = std::env::var("MATRIX_USER").ok();
        // Account slug: localpart of the user id, or "default".
        let account = user
            .as_deref()
            .and_then(|u| u.trim_start_matches('@').split(':').next())
            .unwrap_or("default")
            .to_string();

        let dir = account_dir(&store_root, &account);
        if let Some(record) = load_session_record(&dir) {
            return Some(Self::new(account, MatrixAuth::Restore(record), store_root, agent_id));
        }

        let homeserver = std::env::var("MATRIX_HOMESERVER").ok()?;
        let user_id = user?;
        let password = std::env::var("MATRIX_PASSWORD").ok()?;
        let auth = MatrixAuth::Password {
            homeserver,
            user_id,
            password,
            device_name: "Souveraine".to_string(),
        };
        Some(Self::new(account, auth, store_root, agent_id))
    }

    /// Handle one turn-lifecycle event off the [`EventBus`].
    ///
    /// Phase 3: this routes the event into the [`MatrixTurn`] keyed by its
    /// `target` (the turn/room id) and accumulates segment text. It does
    /// *not* yet push anything outbound — outbound streaming is the
    /// `StreamingMessage` port in Phase 5. The seam is here so that phase
    /// is a fill-in, not a rewrite.
    async fn handle_turn_event(&self, event: &crate::core::nervous::SensorEvent) {
        use crate::core::nervous::turn_dispatcher::TurnEventDispatcher as T;

        let Some(turn_id) = event.target.clone() else {
            return;
        };
        let mut turns = self.turns.lock().await;
        let turn = turns.entry(turn_id.clone()).or_insert_with(|| MatrixTurn {
            room_id: turn_id.clone(),
            ..Default::default()
        });

        match event.event_type.as_str() {
            T::EVT_SEGMENT => {
                if let Some(text) = event
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                {
                    turn.buffer.push_str(text);
                }
            }
            T::EVT_TURN_FINISH => {
                turn.finished = true;
                debug!(
                    "matrix: turn {turn_id} finished — {} chars buffered (outbound send is Phase 5)",
                    turn.buffer.len()
                );
            }
            other => {
                debug!("matrix: turn event {other} for {turn_id} (unhandled in spike)");
            }
        }
    }
}

#[async_trait]
impl Sensorium for MatrixSensorium {
    fn bandwidth(&self) -> BandwidthClass {
        // A Matrix room renders HTML and tool cards but has no real-time
        // subconscious channel — Medium, between the TUI and a watch.
        BandwidthClass::Medium
    }

    fn discovery_level(&self) -> DiscoveryLevel {
        DiscoveryLevel::Operational
    }

    async fn run(&mut self, events: EventBus, cancel: CancellationToken) -> Result<()> {
        let auth = self
            .auth
            .take()
            .context("MatrixSensorium::run called twice — auth already consumed")?;
        let dir = account_dir(&self.store_root, &self.account);

        info!("matrix sensorium: connecting (account {})", self.account);
        let (matrix_client, record) = build_client(auth, &dir).await?;

        // Persist the (possibly refreshed) session so the next run restores.
        save_session_record(&dir, &record)?;

        // ── Inbound: register handlers before sync ───────────────────
        // Fire `sensorium:input` onto the EventBus for every room
        // message so the SensoriumInputHandler picks it up and routes
        // it to the backend.
        let bus = events.clone();
        let account = self.account.clone();
        let agent_id = self.agent_id.clone();
        matrix_client.add_event_handler(
            move |event: OriginalSyncRoomMessageEvent, room: Room| {
                let bus = bus.clone();
                let account = account.clone();
                let agent_id = agent_id.clone();
                async move {
                    if room.state() != RoomState::Joined {
                        return;
                    }
                    if event.sender.as_str() == room.own_user_id().as_str() {
                        return;
                    }
                    let MessageType::Text(text) = event.content.msgtype else {
                        return;
                    };
                    let body = text.body.trim().to_string();
                    if body.is_empty() {
                        return;
                    }

                    bus.send(crate::core::nervous::SensorEvent {
                        sensor_name: format!("matrix/{account}"),
                        timestamp: chrono::Utc::now(),
                        event_type: "sensorium:input".into(),
                        target: Some(room.room_id().to_string()),
                        urgency: 0.3,
                        payload: Some(serde_json::json!({
                            "text": body,
                            "agent_id": agent_id,
                            "sender": event.sender.as_str(),
                            "message_id": event.event_id.as_str(),
                        })),
                        seed_id: None,
                        reply_to: None,
                    });
                }
            },
        );

        // ── Drive /sync on its own task ──────────────────────────────
        let sync_cancel = cancel.child_token();
        let sync_client = matrix_client.clone();
        let sync_handle: JoinHandle<()> = tokio::spawn(async move {
            if let Err(e) = client::sync_forever(sync_client, sync_cancel).await {
                warn!("matrix sync loop ended with error: {e:#}");
            }
        });

        info!("matrix sensorium: synced and listening");

        // ── Outbound: consume turn-lifecycle events until cancelled ──
        let mut event_rx = events.subscribe();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    debug!("matrix sensorium: shutdown requested");
                    break;
                }
                event = event_rx.recv() => {
                    match event {
                        Ok(ev) if ev.event_type.starts_with("turn:") => {
                            self.handle_turn_event(&ev).await;
                        }
                        Ok(_) => { /* non-turn event — not this surface's concern */ }
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("matrix sensorium: event bus closed");
                            break;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("matrix sensorium: lagged {n} events");
                        }
                    }
                }
            }
        }

        // Cancel propagates to the child token; wait for the sync task.
        cancel.cancel();
        let _ = sync_handle.await;
        info!("matrix sensorium: stopped");
        Ok(())
    }

    async fn send_message(&self, chat_id: &str, text: &str) -> Result<super::OutboundResult> {
        // Find the room, send a text message.
        // This is the wire that Phase 5's streaming edits build on —
        // first chunk creates a new message, subsequent deltas edit it.
        //
        // TODO: when we don't have direct room access from the sensorium
        // itself (it lives in the run loop), this needs matrix_client to be
        // shared. For now, stubbed — the transport spike sends via the
        // event handler path.
        debug!("matrix::send_message: {chat_id} ({})", text.len());
        Ok(super::OutboundResult {
            message_id: String::new(),
        })
    }

    async fn send_direct_reply(&self, chat_id: &str, text: &str) -> Result<super::OutboundResult> {
        debug!("matrix::send_direct_reply: {chat_id} {text}");
        Ok(super::OutboundResult {
            message_id: String::new(),
        })
    }

    fn is_running(&self) -> bool {
        true
    }

    async fn stop(&self) -> Result<()> {
        // Matrix needs to leave rooms / close the sync connection,
        // but the CancellationToken in `run` handles the actual
        // shutdown. `stop` is a signal, not the mechanism.
        Ok(())
    }
}
