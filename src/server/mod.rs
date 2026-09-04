#![allow(dead_code, clippy::type_complexity)] // WIP scaffolding; Arc<dyn Fn> callback plumbing
use crate::bridge::{build_registry, ProviderRegistry};
use crate::core::compact::{CompactionEngine, DefaultCompactionEngine, UtcClock};
use crate::core::config::ConsciousnessConfig;
use crate::server::gitea_memory::GiteaMemory;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, RwLock};

pub mod agent_inventory;
pub mod consciousness_engine;
pub mod db;
pub mod device_registry;
pub mod energy;
pub mod federation;
pub mod gitea_client;
pub mod gitea_memory;
pub mod listener;
pub mod restart;
pub mod session_manager;
pub mod subagent;
pub mod summon_handler;
pub mod turn;

pub use agent_inventory::AgentInventory;
pub use consciousness_engine::ConsciousnessEngine;
pub use device_registry::DeviceRegistry;
pub use session_manager::SessionManager;

// Server-side memory backend (Send-safe, HTTP-only via Gitea API).
// The richer consciousness layer (PersonaRouter / SubconsciousN1 / Archivist)
// is rebuilt in Stage 5 against the `Memory` trait from `crates/memory`.
pub type ServerMemory = GiteaMemory;

/// How this process came up. `resumed` carries the reason from an
/// intentional restart (POST /v1/server/restart); `started` is a cold boot.
/// Surfaces read this so a restarted server says why it is here instead of
/// pretending nothing happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootInfo {
    pub instance_id: String,
    pub event_type: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub reason: Option<String>,
    pub by: Option<String>,
}

#[derive(Clone)]
pub struct SouveraineServer {
    pub agents: Arc<AgentInventory>,
    pub sessions: Arc<SessionManager>,
    pub consciousness: Arc<ConsciousnessEngine>,
    pub compaction_engine: Arc<dyn CompactionEngine>,
    pub providers: Arc<ProviderRegistry>,
    pub config: Arc<RwLock<ServerConfig>>,
    pub data_dir: PathBuf,
    pub memory: Option<Arc<ServerMemory>>,
    pub app_config: Arc<RwLock<ConsciousnessConfig>>,
    /// Adaptive inter-round delay — starts at 500ms, bumps +200ms on 429.
    /// Shared across primary loop and subconscious so both respect the same ceiling.
    pub rate_delay: Arc<AtomicU64>,
    /// Stable identifier for this process — used to register/heartbeat
    /// agent instances so the manager card shows running counts.
    pub instance_id: String,
    /// Nervous system bus. Every sensor event flows through here — cron,
    /// todo, energy, posture. Firehose subscribers (EventLog, WebSocket
    /// bridge, desktop overlay) listen on this bus.
    pub event_bus: crate::core::nervous::EventBus,
    /// How this process came up — set at construction from the restart
    /// marker, and read by surfaces to announce a resumption.
    pub boot: BootInfo,
    /// Tracks known federated peers. Updated by `device_announce`/`device_leave`
    /// events on the bus. Persisted to disk for CLI access.
    pub device_registry: Option<Arc<DeviceRegistry>>,
    /// Manages cross-instance Reach & Consult requests. Present when
    /// federation is enabled and a seed identity is available.
    pub summon_handler: Option<Arc<summon_handler::SummonHandler>>,
    /// This instance's Ed25519 public key hex — used to filter self-announcements
    /// from the device registry. Loaded at construction; None if seed unavailable.
    pub local_seed_id: Option<String>,
    /// The machine signer behind `local_seed_id` — machined daemon or legacy
    /// seed. Resolved once at construction; the federation bridge signs
    /// envelopes through it.
    pub machine_signer: Option<Arc<crate::machined::signer::MachineSigner>>,
    /// Per-conversation turn backchannel. Holds the cancel token for the
    /// turn in flight and a persistent interjection queue, so any surface
    /// can interrupt (`POST /v1/conversations/:id/cancel`) or slip a note
    /// mid-turn (`POST .../interject`). The queue survives between turns —
    /// an interjection landed while she's idle is read at the next turn start.
    pub turn_signals: Arc<dashmap::DashMap<String, TurnSignals>>,
    /// Manages every active surface (TUI, Matrix, mobile, etc.).
    /// Each sensorium runs in its own task, sharing the EventBus.
    /// Register via `register_sensorium()`; `shutdown()` cancels all.
    pub sensorium: Arc<Mutex<crate::core::sensorium::SensoriumCoordinator>>,
    /// Maps surface chat/room IDs (Matrix room IDs, etc.) to Souveraine
    /// conversation IDs. Lives beside the SensoriumCoordinator because
    /// inbound surface messages need conversation resolution before they
    /// can be turned into turns.
    pub surface_conversations: Arc<StdMutex<HashMap<String, String>>>,
}

pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub gitea_url: Option<String>,
}

/// Backchannel signals for one conversation's turns. The cancel token is
/// replaced at each turn start (fire-once semantics); the interjection
/// queue is shared with `run_turn`, which drains it between LLM rounds.
pub struct TurnSignals {
    pub cancel: StdMutex<tokio_util::sync::CancellationToken>,
    pub interject: crate::backend::InterjectionQueue,
}

impl Default for TurnSignals {
    fn default() -> Self {
        Self {
            cancel: StdMutex::new(tokio_util::sync::CancellationToken::new()),
            interject: Arc::new(StdMutex::new(Vec::new())),
        }
    }
}

impl SouveraineServer {
    pub async fn new(mut config: ConsciousnessConfig) -> anyhow::Result<Self> {
        config.normalize_providers();
        let data_dir = dirs::home_dir().unwrap().join(".souveraine").join("server");

        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(data_dir.join("agents")).await?;

        let db_path = data_dir.join("database.sqlite3");
        let db = db::init_database(&db_path).await?;

        let agents_dir = data_dir.join("agents");
        let agents = Arc::new(AgentInventory::new(agents_dir, db).await?);

        // Reconcile every primary's cadences. The seven agents on these
        // machines predate reflection and the archivist having bodies, so this
        // is what gives an existing agent her longer wavelengths rather than
        // leaving them as functions inside the server.
        if let Ok(existing) = agents.list(None).await {
            let mut tokenless = 0usize;
            for summary in &existing {
                if let Err(e) = agents.create_subconscious_for(&summary.id).await {
                    tracing::warn!("Subconscious reconcile failed for {}: {}", summary.id, e);
                }
                agents.ensure_cadences(&summary.id).await;
                if !crate::api::auth::has_token(&data_dir, &summary.id).await {
                    tokenless += 1;
                }
            }
            // Agents older than token issuance have no credential and reach the
            // API only through the loopback bypass. CRON_API_AUTH.md § Migration
            // refuses to mint for them at boot — a missing token can equally mean
            // a corrupted agent dir — so this counts them and names the repair
            // rather than papering over it.
            if tokenless > 0 {
                tracing::warn!(
                    tokenless,
                    total = existing.len(),
                    "agents have no API token: authenticated only by the loopback \
                     bypass. Run `souveraine agents token <id> --rotate` for each."
                );
            }
        }

        // Instance registry: rows are registered per-agent when a session
        // starts (not blanket for all agents). The heartbeat loop keeps
        // registered rows alive and accumulates lifetime_active_seconds.
        let instance_id = uuid::Uuid::new_v4().to_string();
        {
            const TICK_SECONDS: i64 = 30;
            let agents_for_tick = agents.clone();
            let id_for_tick = instance_id.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(TICK_SECONDS as u64));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(e) = agents_for_tick
                        .heartbeat_instance(&id_for_tick, TICK_SECONDS)
                        .await
                    {
                        tracing::warn!("instance heartbeat failed: {}", e);
                    }
                }
            });
        }

        let event_bus = crate::core::nervous::EventBus::default();
        let sessions = Arc::new(SessionManager::with_persistence(data_dir.join("agents")));

        // ── Boot: resume with history, and say why we are here ──
        // Hydrate every agent's persisted conversations before serving: the
        // surfaces hold conversation ids, not a map, so a restart that answers
        // a held id with 404 until someone lists reads as amnesia. An
        // intentional restart (POST /v1/server/restart) leaves a marker; boot
        // announces `resumed` with its reason on the event bus and clears it.
        // A boot with no marker announces `started`.
        if let Ok(existing) = agents.list(None).await {
            for summary in &existing {
                if let Err(e) = sessions.load_persisted(&summary.id).await {
                    tracing::warn!("boot hydration failed for {}: {}", summary.id, e);
                }
            }
        }
        let boot_marker = crate::server::restart::read(&data_dir);
        let boot_event_type = if boot_marker.is_some() { "resumed" } else { "started" };
        let boot = BootInfo {
            instance_id: instance_id.clone(),
            event_type: boot_event_type.to_string(),
            at: boot_marker
                .as_ref()
                .map(|m| m.at)
                .unwrap_or_else(chrono::Utc::now),
            reason: boot_marker.as_ref().map(|m| m.reason.clone()),
            by: boot_marker.as_ref().map(|m| m.by.clone()),
        };
        event_bus.send(crate::core::nervous::SensorEvent {
            sensor_name: "server".to_string(),
            timestamp: boot.at,
            event_type: boot.event_type.clone(),
            target: None,
            urgency: 1.0,
            payload: boot_marker
                .as_ref()
                .and_then(|m| serde_json::to_value(m).ok()),
            seed_id: None,
            reply_to: None,
        });
        tracing::info!(
            event_type = boot.event_type,
            reason = boot.reason,
            "server boot"
        );
        crate::server::restart::clear(&data_dir);

        // Build the per-agent provider registry from config. z.ai, bifrost, etc.
        let providers = Arc::new(build_registry(&config)?);

        let rate_delay = Arc::new(AtomicU64::new(1000));
        tracing::info!("rate delay initialized at 1000ms");

        // Build compaction engine with closure-based session access (before
        // consciousness engine so it can be shared with the subconscious).
        let comp_session = sessions.clone();
        let comp_agents = agents.clone();
        let app_cfg = Arc::new(RwLock::new(config.clone()));
        let get_messages: Arc<dyn Fn(&str) -> Option<Vec<_>> + Send + Sync> = {
            let s = comp_session.clone();
            Arc::new(move |agent_id| {
                let conv_id = s.latest_for_agent(agent_id)?;
                s.get(&conv_id).map(|session| session.messages.clone())
            })
        };
        let replace_messages: Arc<dyn Fn(&str, Vec<_>) -> anyhow::Result<()> + Send + Sync> = {
            let s = comp_session.clone();
            Arc::new(move |agent_id, messages| {
                let conv_id = s
                    .latest_for_agent(agent_id)
                    .ok_or_else(|| anyhow::anyhow!("No session for {}", agent_id))?;
                let mut session = s
                    .get_mut(&conv_id)
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                session.messages = messages;
                Ok(())
            })
        };
        let get_repo: Arc<dyn Fn(&str) -> Option<crate::core::memory::MemoryRepo> + Send + Sync> = {
            let agents = comp_agents.clone();
            Arc::new(move |id| {
                let (primary_id, cadence) = crate::core::cadence::split(id);
                Some(agents.cadence_memory_repo(primary_id, cadence))
            })
        };
        let get_agent_type: Arc<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Arc::new(|id| Some(crate::core::cadence::of(id).type_name().to_string()));
        let get_context_limit: Arc<dyn Fn(&str) -> Option<usize> + Send + Sync> = {
            // Cloned out of config so the closure stays sync — it is called
            // from the `memory status` tool path, which has no runtime handle.
            let models = config.models.clone();
            let global_sub_model = config.subconscious.model.clone();
            Arc::new(move |id| {
                let (primary_id, cadence) = crate::core::cadence::split(id);
                let is_sub = cadence != crate::core::cadence::Cadence::Primary;
                let home = dirs::home_dir()?;
                let path = home
                    .join(".souveraine/server/agents")
                    .join(primary_id)
                    .join("agent.json");
                let parsed: serde_json::Value = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or(serde_json::Value::Null);

                if is_sub {
                    // Her ceiling follows *her* model, not her primary's. The
                    // per-agent override wins over the global, matching the
                    // resolution order in `subconscious_tool_loop`.
                    let model = parsed
                        .get("_souveraine")
                        .and_then(|s| s.get("subconscious_model"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| global_sub_model.clone())?;
                    models.get(&model).map(|m| m.context_limit)
                } else {
                    // The agent's own declared window first; the model
                    // registry is the fallback when it is unset.
                    parsed
                        .get("llm_config")
                        .and_then(|l| l.get("context_window"))
                        .and_then(|w| w.as_u64())
                        .map(|w| w as usize)
                        .filter(|w| *w > 0)
                        .or_else(|| {
                            let model = parsed
                                .get("llm_config")
                                .and_then(|l| l.get("model"))
                                .and_then(|m| m.as_str())?;
                            models.get(model).map(|m| m.context_limit)
                        })
                }
            })
        };

        let get_agent_provider: Arc<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Arc::new(|id| {
                let primary_id = if id.ends_with("-sub") {
                    id.trim_end_matches("-sub")
                } else {
                    id
                };
                let home = dirs::home_dir()?;
                let path = home
                    .join(".souveraine/server/agents")
                    .join(primary_id)
                    .join("agent.json");
                let content = std::fs::read_to_string(&path).ok()?;
                let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
                parsed
                    .get("_souveraine")?
                    .get("provider")?
                    .as_str()
                    .map(|s| s.to_string())
            });

        let compaction_engine: Arc<dyn CompactionEngine> = Arc::new(DefaultCompactionEngine {
            config: app_cfg,
            counter: crate::bridge::model_router::TokenCounter::new(),
            providers: Some(providers.clone()),
            model: config
                .compaction
                .model
                .clone()
                .or_else(|| config.subconscious.model.clone()),
            clock: Arc::new(UtcClock),
            get_messages,
            replace_messages,
            get_repo,
            get_agent_type,
            get_agent_provider,
            get_context_limit,
        });

        let consciousness = Arc::new(ConsciousnessEngine::new(
            agents.clone(),
            sessions.clone(),
            providers.clone(),
            config.subconscious.model.clone(),
            config.reflection.model.clone(),
            config.subconscious.max_tokens,
            rate_delay.clone(),
            config.archivist.clone(),
            config.reflection.clone(),
            config.models.clone(),
            compaction_engine.clone(),
            config.subconscious.system_prompt.clone(),
        ));

        // Gitea-backed memory is opt-in for the server: it requires a reachable
        // Gitea instance + token. If those aren't configured, the server still
        // runs (agent CRUD, sessions, conversation pass-through) without memfs.
        let memory = match ServerMemory::new(Arc::new(RwLock::new(config.clone()))).await {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                tracing::warn!("Gitea memory disabled: {}", e);
                None
            }
        };

        let server_config = ServerConfig {
            bind: config.server.bind.clone(),
            port: config.server.port,
            data_dir: data_dir.clone(),
            gitea_url: std::env::var("SOUVERAINE_GITEA_URL").ok(),
        };

        // ── Machine identity ──
        // Resolved once: souveraine-machined (system tier) first, legacy
        // user-tier seed as a loud fallback. Never generates — a box with no
        // identity runs federation-less with a warning, it does not silently
        // mint a key.
        let souveraine_base = dirs::home_dir().unwrap_or_default().join(".souveraine");
        let machine_signer = match crate::machined::signer::MachineSigner::resolve(&souveraine_base)
        {
            Ok(signer) => Some(Arc::new(signer)),
            Err(e) => {
                tracing::warn!(
                    "no machine identity — device registry, summon handling, and \
                     federation are disabled: {e:#}"
                );
                None
            }
        };
        let local_seed_id = machine_signer.as_ref().map(|s| s.pubkey_hex());
        let sb_for_device_reg = souveraine_base.clone();
        let local_role = config.federation.role;
        let device_registry = local_seed_id.clone().map(|seed_id| {
            let reg = Arc::new(DeviceRegistry::new(sb_for_device_reg, seed_id, local_role));
            // Subscribe the registry to the event bus for live updates.
            let reg_clone = reg.clone();
            let mut rx = event_bus.subscribe();
            tokio::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    reg_clone.handle_event(&event);
                }
            });
            // Prune peers unheard-from for 3 minutes.
            let reg_prune = reg.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
                tick.tick().await;
                loop {
                    tick.tick().await;
                    reg_prune.prune_stale(180);
                }
            });
            reg
        });

        // ── Summon handler ──
        let summon_handler = match (&local_seed_id, &machine_signer) {
            (Some(seed_id), Some(signer)) => {
                let handler = Arc::new(summon_handler::SummonHandler::new(
                    seed_id.clone(),
                    event_bus.clone(),
                    signer.clone(),
                    souveraine_base.clone(),
                    config.federation.auto_wake,
                ));
                handler.spawn_listener();
                Some(handler)
            }
            _ => None,
        };

        Ok(Self {
            agents,
            sessions,
            consciousness,
            compaction_engine,
            providers,
            config: Arc::new(RwLock::new(server_config)),
            data_dir,
            memory,
            app_config: Arc::new(RwLock::new(config)),
            rate_delay,
            instance_id,
            event_bus,
            boot,
            device_registry,
            summon_handler,
            local_seed_id,
            machine_signer,
            turn_signals: Arc::new(dashmap::DashMap::new()),
            sensorium: Arc::new(Mutex::new(
                crate::core::sensorium::SensoriumCoordinator::new(),
            )),
            surface_conversations: Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    /// Seed a freshly created conversation with the agent's full system
    /// prompt — constitution/platform prompt, memfs base memories,
    /// subconscious window, skills, visual state. Every conversation
    /// entry point (TUI backend, HTTP API, sensoria) must pass through
    /// here: a session without this message boots the agent amnesiac.
    pub async fn seed_conversation_system_prompt(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<()> {
        let memory_root = self.agents.memory_root(agent_id);
        let subconscious_root = self.agents.subconscious_memory_root(agent_id);

        let (bundled, user, agent_memfs, project) =
            crate::core::skills::default_discovery_paths(Some(memory_root.clone()));
        let skills = crate::core::skills::discover(
            bundled.as_deref(),
            user.as_deref(),
            agent_memfs.as_deref(),
            project.as_deref(),
        )
        .await
        .unwrap_or_default();

        let config = self.app_config.read().await;
        let platform_prompt = config.agent.system_prompt.clone();
        // Visual state greeting (atmosphere + outfit), when presence is set.
        let greeting_extra = config
            .presence
            .atmosphere
            .as_deref()
            .filter(|a| !a.is_empty())
            .map(|atm| {
                let display = atm.replace('_', " ");
                let outfit = config.presence.outfit.as_deref().unwrap_or("default");
                format!(
                    "\n\nYour current atmosphere is {}, wearing the \"{}\" outfit.",
                    display, outfit
                )
            });
        drop(config);

        let mut system_prompt = crate::core::prompt::build_system_prompt_full(
            &memory_root,
            Some(&subconscious_root),
            platform_prompt.as_deref(),
            Some(&skills),
        )
        .await;
        if let Some(extra) = greeting_extra {
            system_prompt.push_str(&extra);
        }

        self.sessions.add_message(
            conversation_id,
            crate::core::session::ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: system_prompt,
                }],
                usage: None,
                timestamp: Some(chrono::Utc::now()),
            },
        )?;
        Ok(())
    }

    /// Register a sensorium on the coordinator and spawn its run loop.
    ///
    /// Each sensorium gets its own task, a shared EventBus subscription,
    /// and a child CancellationToken. `shutdown_sensoria` cancels all of
    /// them. Can be called at any time — the coordinator drains registered
    /// sensoria on `run_all` and accepts new ones afterward.
    pub async fn register_sensorium(&self, sensorium: Box<dyn crate::core::sensorium::Sensorium>) {
        let mut coord = self.sensorium.lock().await;
        coord.register(sensorium);
        coord.run_all(self.event_bus.clone());
    }

    /// Shut down all running sensorium tasks.
    pub async fn shutdown_sensoria(&self) {
        let coord = self.sensorium.lock().await;
        coord.shutdown();
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let this = self.clone();
        let app = crate::api::create_routes(Arc::new(this))
            .layer(tower_http::cors::CorsLayer::permissive());

        let config = self.config.read().await;
        let addr = format!("{}:{}", config.bind, config.port);
        drop(config);

        // ── Federation bridge ──
        // When federation is enabled, spawn an outbound signed-event stream
        // to each configured peer. Inbound events arrive symmetrically on
        // this server's own /v1/federation/events handler.
        {
            let fed = self.app_config.read().await.federation.clone();
            if fed.enabled && !fed.peers.is_empty() {
                let peer_count = fed.peers.len();
                match &self.machine_signer {
                    Some(signer) => {
                        let mut bridge = federation::FederationBridge::new(
                            self.event_bus.clone(),
                            signer.clone(),
                            fed.role,
                            dirs::home_dir().unwrap_or_default().join(".souveraine"),
                        );
                        for peer in fed.peers {
                            bridge.add_peer(peer);
                        }
                        bridge.run();
                        tracing::info!("federation bridge started ({peer_count} peer(s))");
                    }
                    None => {
                        tracing::error!(
                            "federation: enabled in config but no machine identity is \
                             available — bridge not started"
                        );
                    }
                }
            }
        }

        // ── Drain parked summons ──
        // A lite listener parks summons it couldn't answer to
        // ~/.souveraine/.summon-pending/. Re-fire them onto the bus so the
        // full engine's SummonHandler picks them up, then clear the files.
        {
            let pending_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".souveraine")
                .join(".summon-pending");
            if let Ok(entries) = std::fs::read_dir(&pending_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(event) =
                            serde_json::from_str::<crate::core::nervous::SensorEvent>(&content)
                        {
                            self.event_bus.send(event);
                            let _ = std::fs::remove_file(&path);
                            tracing::info!(file = ?path, "drained parked summon");
                        }
                    }
                }
            }
        }

        println!("Souveraine server listening on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;

        Ok(())
    }
}
