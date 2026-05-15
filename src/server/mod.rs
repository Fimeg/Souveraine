use crate::bridge::BifrostClient;
use crate::core::compact::{CompactionEngine, CompactionConfig, DefaultCompactionEngine, UtcClock};
use crate::core::config::ConsciousnessConfig;
use crate::core::identity::SeedId;
use crate::server::gitea_memory::GiteaMemory;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

pub mod agent_inventory;
pub mod consciousness_engine;
pub mod conversation;
pub mod db;
pub mod device_registry;
pub mod federation;
pub mod gitea_client;
pub mod gitea_memory;
pub mod listener;
pub mod session_manager;
pub mod summon_handler;

pub use agent_inventory::AgentInventory;
pub use consciousness_engine::{ConsciousnessEngine, ConsciousnessEvent};
pub use device_registry::DeviceRegistry;
pub use session_manager::SessionManager;

// Server-side memory backend (Send-safe, HTTP-only via Gitea API).
// The richer consciousness layer (PersonaRouter / SubconsciousN1 / Archivist)
// is rebuilt in Stage 5 against the `Memory` trait from `crates/memory`.
pub type ServerMemory = GiteaMemory;

#[derive(Clone)]
pub struct SouveraineServer {
    pub agents: Arc<AgentInventory>,
    pub sessions: Arc<SessionManager>,
    pub consciousness: Arc<ConsciousnessEngine>,
    pub compaction_engine: Arc<dyn CompactionEngine>,
    pub bifrost: Arc<BifrostClient>,
    pub config: Arc<RwLock<ServerConfig>>,
    pub data_dir: PathBuf,
    pub memory: Option<Arc<ServerMemory>>,
    pub app_config: Arc<RwLock<ConsciousnessConfig>>,
    /// Adaptive inter-round delay — starts at 500ms, bumps +200ms on 429.
    /// Shared across primary loop and Aster so both respect the same ceiling.
    pub rate_delay: Arc<AtomicU64>,
    /// Stable identifier for this process — used to register/heartbeat
    /// agent instances so the manager card shows running counts.
    pub instance_id: String,
    /// Nervous system bus. Every sensor event flows through here — cron,
    /// todo, energy, posture. Firehose subscribers (EventLog, WebSocket
    /// bridge, desktop overlay) listen on this bus.
    pub event_bus: crate::core::nervous::EventBus,
    /// Tracks known federated peers. Updated by `device_announce`/`device_leave`
    /// events on the bus. Persisted to disk for CLI access.
    pub device_registry: Option<Arc<DeviceRegistry>>,
    /// Manages cross-instance Reach & Consult requests. Present when
    /// federation is enabled and a seed identity is available.
    pub summon_handler: Option<Arc<summon_handler::SummonHandler>>,
    /// This instance's Ed25519 public key hex — used to filter self-announcements
    /// from the device registry. Loaded at construction; None if seed unavailable.
    pub local_seed_id: Option<String>,
}

pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub gitea_url: Option<String>,
}

impl SouveraineServer {
    pub async fn new(config: ConsciousnessConfig) -> anyhow::Result<Self> {
        let data_dir = dirs::home_dir()
            .unwrap()
            .join(".souveraine")
            .join("server");

        tokio::fs::create_dir_all(&data_dir).await?;
        tokio::fs::create_dir_all(data_dir.join("agents")).await?;

        let db_path = data_dir.join("database.sqlite3");
        let db = db::init_database(&db_path).await?;

        let agents_dir = data_dir.join("agents");
        let agents = Arc::new(AgentInventory::new(agents_dir, db).await?);

        // Reconcile subconscious agents for existing primaries
        if let Ok(existing) = agents.list(None).await {
            for summary in &existing {
                if let Err(e) = agents.create_subconscious_for(&summary.id).await {
                    tracing::warn!("Subconscious reconcile failed for {}: {}", summary.id, e);
                }
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
                let mut interval = tokio::time::interval(
                    std::time::Duration::from_secs(TICK_SECONDS as u64),
                );
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(e) = agents_for_tick.heartbeat_instance(&id_for_tick, TICK_SECONDS).await {
                        tracing::warn!("instance heartbeat failed: {}", e);
                    }
                }
            });
        }

        let event_bus = crate::core::nervous::EventBus::default();
        let sessions = Arc::new(SessionManager::with_persistence(data_dir.join("agents")));

        let primary = &config.bifrost.primary_model;
        let mut fallbacks = Vec::new();
        if !primary.ends_with("-precision") {
            fallbacks.push(format!("{}-precision", primary));
        }
        let bifrost = Arc::new(BifrostClient::new(
            &config.bifrost.base_url,
            &config.bifrost.api_key,
            &config.bifrost.virtual_key,
            primary,
            config.bifrost.timeout_secs,
        ).with_fallbacks(fallbacks));

        let rate_delay = Arc::new(AtomicU64::new(1000));
        tracing::info!("rate delay initialized at 1000ms");

        let consciousness = Arc::new(ConsciousnessEngine::new(
            agents.clone(),
            sessions.clone(),
            bifrost.clone(),
            config.subconscious.model.clone(),
            config.reflection.model.clone(),
            config.subconscious.max_tokens,
            rate_delay.clone(),
        ));

        // Build compaction engine with closure-based session access
        let comp_session = sessions.clone();
        let comp_agents = agents.clone();
        let app_cfg = Arc::new(RwLock::new(config.clone()));
        let get_messages: Arc<dyn Fn(&str) -> Option<Vec<_>> + Send + Sync> = {
            let s = comp_session.clone();
            Arc::new(move |agent_id| {
                let conv_ids = s.list_for_agent(agent_id);
                let conv_id = conv_ids.last()?.clone();
                s.get(&conv_id).map(|session| session.messages.clone())
            })
        };
        let replace_messages: Arc<dyn Fn(&str, Vec<_>) -> anyhow::Result<()> + Send + Sync> = {
            let s = comp_session.clone();
            Arc::new(move |agent_id, messages| {
                let conv_ids = s.list_for_agent(agent_id);
                let conv_id = conv_ids.last()
                    .ok_or_else(|| anyhow::anyhow!("No session for {}", agent_id))?;
                let mut session = s.get_mut(&conv_id)
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                session.messages = messages;
                Ok(())
            })
        };
        let get_repo: Arc<dyn Fn(&str) -> Option<crate::core::memory::MemoryRepo> + Send + Sync> = {
            let agents = comp_agents.clone();
            Arc::new(move |id| Some(agents.memory_repo(id)))
        };
        let get_agent_type: Arc<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Arc::new(|_| Some("primary".to_string()));

        let compaction_engine: Arc<dyn CompactionEngine> = Arc::new(DefaultCompactionEngine {
            config: app_cfg,
            counter: crate::bridge::model_router::TokenCounter::new(),
            bifrost: Some((*bifrost).clone()),
            model: config.compaction.model.clone().or_else(|| config.subconscious.model.clone()),
            clock: Arc::new(UtcClock),
            get_messages,
            replace_messages,
            get_repo,
            get_agent_type,
        });

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

        // ── Device registry ──
        let souveraine_base = dirs::home_dir()
            .unwrap_or_default()
            .join(".souveraine");
        let local_seed_id = match crate::core::identity::SeedId::load_or_generate(
            &crate::core::identity::SeedId::default_dir(&souveraine_base),
        ) {
            Ok(seed) => {
                let pubkey = seed.public_key_hex();
                Some(pubkey)
            }
            Err(_) => None,
        };
        let sb_for_device_reg = souveraine_base.clone();
        let device_registry = local_seed_id.clone().map(|seed_id| {
            let reg = Arc::new(DeviceRegistry::new(sb_for_device_reg, seed_id));
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
        let sb_for_seed = souveraine_base.clone();
        let local_seed: Option<Arc<SeedId>> = local_seed_id.as_ref().and_then(|_| {
            crate::core::identity::SeedId::load_or_generate(
                &crate::core::identity::SeedId::default_dir(&sb_for_seed),
            ).ok().map(Arc::new)
        });
        let summon_handler = match (&local_seed_id, &local_seed) {
            (Some(seed_id), Some(seed)) => {
                let handler = Arc::new(
                    summon_handler::SummonHandler::new(
                        seed_id.clone(),
                        event_bus.clone(),
                        seed.clone(),
                        souveraine_base.clone(),
                    ),
                );
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
            bifrost,
            config: Arc::new(RwLock::new(server_config)),
            data_dir,
            memory,
            app_config: Arc::new(RwLock::new(config)),
            rate_delay,
            instance_id,
            event_bus,
            device_registry,
            summon_handler,
            local_seed_id,
        })
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
                let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
                match crate::core::identity::SeedId::load_or_generate(
                    &crate::core::identity::SeedId::default_dir(&base),
                ) {
                    Ok(seed) => {
                        let mut bridge = federation::FederationBridge::new(
                            self.event_bus.clone(),
                            Arc::new(seed),
                        );
                        for peer in fed.peers {
                            bridge.add_peer(peer);
                        }
                        bridge.run();
                        tracing::info!("federation bridge started ({peer_count} peer(s))");
                    }
                    Err(e) => {
                        tracing::error!("federation: failed to load seed identity: {}", e);
                    }
                }
            }
        }

        // ── Drain parked summons ──
        // A lite listener parks summons it couldn't answer to
        // ~/.souveraine/.summon-pending/. Re-fire them onto the bus so the
        // full engine's SummonHandler picks them up, then clear the files.
        {
            let pending_dir = dirs::home_dir().unwrap_or_default()
                .join(".souveraine").join(".summon-pending");
            if let Ok(entries) = std::fs::read_dir(&pending_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(event) = serde_json::from_str::<crate::core::nervous::SensorEvent>(&content) {
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

pub use conversation::{ServerConversation, ServerTurnResult};
