//! In-process Backend impl. Same engine as the HTTP server, no socket.
//!
//! Constructed once with a `ConsciousnessConfig`; spins up an `AgentInventory`
//! (SQLite under `~/.souveraine/server/`), `SessionManager`, `BifrostClient`,
//! and `ConsciousnessEngine`. `send` mirrors the server's `stream_messages`
//! handler, but emits `BackendEvent`s directly instead of SSE frames.
//!
//! This is the "harness still works when the server is gone" path
//! (`souveraine chat --local`, or auto-fallback when the remote is down).

pub(crate) mod energy;
mod turn;
mod consciousness;
mod subagent;

pub use subagent::LocalSubagentRunner;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::bridge::bifrost::{ChatCompletionRequest, Message as BifrostMessage};
use crate::bridge::model_router::TokenCounter;
use crate::core::compact::CompactionEngine;
use crate::core::config::ConsciousnessConfig;
use crate::core::identity::SeedId;
use crate::core::nervous::EventBus;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::core::tools::defs::{SubagentParams, SubagentRunner, ToolContext};
use crate::server::{ConsciousnessEvent, SouveraineServer};

use super::{AgentInfo, Backend, BackendEvent, ConversationInfo};

/// Below 95%: no cap. At 95%+: scale max_tokens so context + output
/// stays under the model's limit. The agent feels the room shrink.
fn pressure_to_max_tokens(pressure: f32, output_limit: u32) -> Option<u32> {
    if pressure <= 0.95 {
        return None;
    }
    let remaining = (1.0 - pressure) / 0.05;
    let ratio = remaining.max(0.0).min(1.0);
    Some((output_limit as f32 * ratio) as u32)
}

/// Mirror of `ConsciousnessEngine::calculate_pressure` for the in-loop
/// BifrostMessage shape, so we can recompute pressure as tool results
/// accumulate inside a single turn. `context_limit` comes from the
/// agent's `llm_config.context_window` (Constitution V.3 — per-model
/// physics, no hardcoded 128K).
fn bifrost_pressure(counter: &TokenCounter, messages: &[BifrostMessage], context_limit: usize) -> f32 {
    let tokens: usize = messages.iter().map(|m| counter.count(&m.content)).sum();
    let limit = context_limit.max(1);
    (tokens as f32 / limit as f32).min(1.0)
}

/// Helper: bump adaptive delay when we hit a 429. No decay — once bumped,
/// the delay stays at that level until the app restarts.
fn bump_on_strain(delay: &AtomicU64, status: u16) {
    if status == 429 {
        let current = delay.load(Ordering::Relaxed);
        let bumped = (current + 200).min(3000);
        if bumped > current {
            delay.store(bumped, Ordering::Relaxed);
            tracing::info!("rate delay bumped to {}ms (429)", bumped);
        }
    }
}

#[derive(Clone)]
pub struct LocalBackend {
    server: Arc<SouveraineServer>,
    event_bus: EventBus,
    seed_id: Arc<SeedId>,
    active_sessions: Arc<AtomicU32>,
    sensorium: Arc<tokio::sync::Mutex<crate::core::sensorium::SensoriumCoordinator>>,
    surface_conversations: Arc<std::sync::Mutex<HashMap<String, String>>>,
}

impl LocalBackend {
    pub async fn new(config: ConsciousnessConfig) -> Result<Self> {
        let server = SouveraineServer::new(config.clone())
            .await
            .context("LocalBackend: SouveraineServer init")?;
        let event_bus = server.event_bus.clone();

        let base = config
            .memory
            .base_path
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".souveraine"));

        // Load or generate the seed identity (trust root)
        let seed_dir = SeedId::default_dir(&base);
        let seed_id = Arc::new(
            SeedId::load_or_generate(&seed_dir)
                .context("SeedId init")?,
        );
        tracing::info!(
            pubkey = %seed_id.public_key_hex(),
            "seed identity loaded"
        );

        // Spawn the persistent event log (firehose to disk)
        let events_dir = base.join("events");
        let mut event_log =
            crate::core::nervous::event_log::EventLog::new(events_dir, event_bus.subscribe());
        tokio::spawn(async move { event_log.run().await });

        let active_sessions = Arc::new(AtomicU32::new(0));
        let backend = Self {
            server: Arc::new(server),
            event_bus: event_bus.clone(),
            seed_id,
            active_sessions: active_sessions.clone(),
            sensorium: Arc::new(tokio::sync::Mutex::new(
                crate::core::sensorium::SensoriumCoordinator::new(),
            )),
            surface_conversations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        };

        // Spawn one CronSensor per agent (each agent owns its own schedules
        // directory), and one HeartbeatHandler on the bus that injects turns
        // when a schedule fires. The handler holds an Arc<dyn TurnInjector>
        // pointing back at us — clean dep direction, no LocalBackend leak
        // into the nervous module.
        let agents_dir = base.join("agents");
        match backend.server.agents.list(None).await {
            Ok(summaries) => {
                for summary in summaries {
                    let schedules_dir = agents_dir.join(&summary.id).join("schedules");
                    if let Err(e) = std::fs::create_dir_all(&schedules_dir) {
                        tracing::warn!(
                            agent = %summary.id,
                            error = %e,
                            "could not create schedules dir; skipping cron sensor"
                        );
                        continue;
                    }
                    let sensor = crate::core::nervous::cron::CronSensor::new(
                        summary.id.clone(),
                        schedules_dir,
                        event_bus.clone(),
                        active_sessions.clone(),
                    );
                    tokio::spawn(async move { sensor.run().await });
                    tracing::info!(agent = %summary.id, "cron sensor spawned");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "agent listing failed; no cron sensors spawned");
            }
        }

        let injector: Arc<dyn crate::core::nervous::handler::TurnInjector> =
            Arc::new(backend.clone());
        // Hand the same injector to the summon handler so an inbound
        // federation request can auto-wake the agent (gated on auto_wake).
        if let Some(sh) = &backend.server.summon_handler {
            sh.set_injector(injector.clone());
        }
        let mut handler =
            crate::core::nervous::handler::HeartbeatHandler::new(event_bus.subscribe(), injector.clone());
        tokio::spawn(async move { handler.run().await });
        tracing::info!("heartbeat handler spawned");

        // Spawn the sensorium input handler — subscribes to
        // `sensorium:input` events from non-terminal surfaces (Matrix,
        // email, federation) and injects turns on their behalf.
        // Same pattern as HeartbeatHandler; identical wiring.
        let mut input_handler =
            crate::core::nervous::handler::SensoriumInputHandler::new(
                event_bus.subscribe(),
                injector,
            );
        tokio::spawn(async move { input_handler.run().await });
        tracing::info!("sensorium input handler spawned");

        Ok(backend)
    }

    pub fn from_server(server: Arc<SouveraineServer>) -> Self {
        let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
        let seed_id = Arc::new(
            SeedId::load_or_generate(&SeedId::default_dir(&base))
                .unwrap_or_else(|_| SeedId::generate()),
        );
        let event_bus = server.event_bus.clone();
        Self {
            event_bus,
            server,
            seed_id,
            active_sessions: Arc::new(AtomicU32::new(0)),
            sensorium: Arc::new(tokio::sync::Mutex::new(
                crate::core::sensorium::SensoriumCoordinator::new(),
            )),
            surface_conversations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    pub fn seed_id(&self) -> &SeedId {
        &self.seed_id
    }

    /// Underlying agent inventory — used by the TUI dashboard to pull a
    /// `MemoryRepo` for live git-stat readouts.
    pub fn server_agents(&self) -> Arc<crate::server::AgentInventory> {
        self.server.agents.clone()
    }

    /// Underlying server. CLI subcommands (e.g. `souveraine reflect`)
    /// reach in here for the consciousness engine and session manager.
    pub fn server(&self) -> Arc<crate::server::SouveraineServer> {
        self.server.clone()
    }

    /// Register a sensorium on the coordinator and spawn its run loop.
    ///
    /// Each sensorium gets its own task, a shared EventBus subscription,
    /// and a child CancellationToken. `shutdown_sensoria` cancels all of
    /// them. Can be called at any time — the coordinator drains registered
    /// sensoria on `run_all` and accepts new ones afterward.
    pub async fn register_sensorium(
        &self,
        sensorium: Box<dyn crate::core::sensorium::Sensorium>,
    ) {
        let mut coord = self.sensorium.lock().await;
        coord.register(sensorium);
        coord.run_all(self.event_bus.clone());
    }

    /// Shut down all running sensorium tasks.
    pub async fn shutdown_sensoria(&self) {
        let coord = self.sensorium.lock().await;
        coord.shutdown();
    }

    /// Build the greeting line describing the agent's current visual state
    /// (atmosphere and outfit). Returns `None` when no atmosphere is set in
    /// config (fresh init, no state to report).
    async fn build_visual_greeting(&self) -> Option<String> {
        let config = self.server.app_config.read().await;
        let atm = config.presence.atmosphere.as_deref()?;
        if atm.is_empty() {
            return None;
        }
        let display = atm.replace('_', " ");
        let outfit = config.presence.outfit.as_deref().unwrap_or("default");
        Some(format!(
            "\n\nYour current atmosphere is {}, wearing the \"{}\" outfit.",
            display, outfit
        ))
    }
}

#[async_trait]
impl Backend for LocalBackend {
    async fn health(&self) -> bool {
        true
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let agents = self.server.agents.list(None).await?;
        Ok(agents
            .into_iter()
            .map(|a| AgentInfo {
                id: a.id,
                name: a.name,
                description: a.description,
            })
            .collect())
    }

    async fn new_conversation(&self, agent_id: &str) -> Result<String> {
        let _ = self.server.agents.get(agent_id).await?;
        let conv_id = self.server.sessions.create(agent_id);

        if let Err(e) = self.server.agents.register_instance(agent_id, &self.server.instance_id).await {
            tracing::warn!(agent = %agent_id, "instance registration failed: {}", e);
        }

        let memory_root = self.server.agents.memory_root(agent_id);
        let subconscious_root = self.server.agents.subconscious_memory_root(agent_id);
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

        let platform_prompt = self.server.app_config.read().await
            .agent.system_prompt.clone();
        let system_prompt = crate::core::prompt::build_system_prompt_full(
            &memory_root,
            Some(&subconscious_root),
            platform_prompt.as_deref(),
            Some(&skills),
        )
        .await;

        // Append visual state greeting.
        let greeting_extra = self.build_visual_greeting().await;
        let system_prompt = if let Some(extra) = greeting_extra {
            format!("{}{}", system_prompt, extra)
        } else {
            system_prompt
        };

        self.server.sessions.add_message(
            &conv_id,
            ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: system_prompt,
                }],
                usage: None,
                timestamp: Some(chrono::Utc::now()),
            },
        )?;

        Ok(conv_id)
    }

    async fn fork_conversation(&self, _agent_id: &str, source_conversation_id: &str) -> Result<String> {
        let forked_id = self.server.sessions.fork(source_conversation_id)?;
        Ok(forked_id)
    }

    async fn list_conversations(&self, agent_id: &str) -> Result<Vec<ConversationInfo>> {
        let store = match self.server.sessions.conversation_store_for(agent_id) {
            Some(s) => s,
            None => {
                let conv_ids = self.server.sessions.list_for_agent(agent_id);
                return Ok(conv_ids
                    .into_iter()
                    .filter_map(|id| {
                        let session = self.server.sessions.get(&id)?;
                        Some(ConversationInfo {
                            id: session.conversation_id.clone(),
                            agent_id: session.agent_id.clone(),
                            summary: None,
                            message_count: session.messages.len() as u32,
                            updated_at: session.updated_at.to_rfc3339(),
                        })
                    })
                    .collect());
            }
        };

        let records = store.list_active().await?;
        Ok(records
            .into_iter()
            .map(|r| ConversationInfo {
                id: r.id,
                agent_id: r.agent_id,
                summary: r.summary,
                message_count: r.message_count,
                updated_at: r.updated_at.to_rfc3339(),
            })
            .collect())
    }

    async fn load_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::core::session::ConversationMessage>> {
        if let Some(session) = self.server.sessions.get(conversation_id) {
            return Ok(session.messages.clone());
        }

        // Not in memory — try loading from disk. We need the agent_id to find the store.
        // Search all known agents.
        let agents = self.server.agents.list(None).await?;
        for agent in agents {
            if let Some(store) = self.server.sessions.conversation_store_for(&agent.id) {
                if let Ok(Some(_record)) = store.load_metadata(conversation_id).await {
                    let messages = store.load_messages(conversation_id).await?;
                    self.server.sessions.create_with_messages(
                        &agent.id,
                        conversation_id.to_string(),
                        messages.clone(),
                    );
                    return Ok(messages);
                }
            }
        }

        anyhow::bail!("Conversation not found: {}", conversation_id)
    }

    async fn ensure_conversation(&self, agent_id: &str) -> Result<String> {
        let _ = self.server.agents.get(agent_id).await?;
        let conv_id = self.server.sessions.create(agent_id);

        // Build system prompt from the agent's memfs and inject as first message
        let memory_root = self.server.agents.memory_root(agent_id);
        let subconscious_root = self.server.agents.subconscious_memory_root(agent_id);

        // Discover skills from all 4 tiers
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

        let platform_prompt = self.server.app_config.read().await
            .agent.system_prompt.clone();
        let system_prompt = crate::core::prompt::build_system_prompt_full(
            &memory_root,
            Some(&subconscious_root),
            platform_prompt.as_deref(),
            Some(&skills),
        )
        .await;

        // Append visual state greeting.
        let greeting_extra = self.build_visual_greeting().await;
        let system_prompt = if let Some(extra) = greeting_extra {
            format!("{}{}", system_prompt, extra)
        } else {
            system_prompt
        };

        self.server.sessions.add_message(
            &conv_id,
            ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: system_prompt,
                }],
                usage: None,
                timestamp: Some(chrono::Utc::now()),
            },
        )?;

        Ok(conv_id)
    }

    async fn send(
        &self,
        conversation_id: &str,
        text: &str,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        // No cancel signal — heartbeat path, drain path. Use a token that
        // never fires.
        self.send_with_cancel(conversation_id, text, CancellationToken::new()).await
    }

    async fn send_with_cancel(
        &self,
        conversation_id: &str,
        text: &str,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        // No queue passed in — use an empty queue. Equivalent to the old behavior.
        let empty: crate::backend::InterjectionQueue = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        self.send_with_signals(conversation_id, text, cancel, empty).await
    }

    async fn send_with_signals(
        &self,
        conversation_id: &str,
        text: &str,
        cancel: CancellationToken,
        interject: crate::backend::InterjectionQueue,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        // Resolve the agent for this conversation, then drain her
        // subconscious's intrusive box. Anything subconscious queued after the
        // last turn rides in on Casey's next message as `[ surfacing: ... ]`
        // lines — the lettabot-v017 pattern, ported. This is the channel
        // by which a Critical observation can interrupt mid-conversation
        // without forcing a halt: she sees it before she reads Casey.
        let session_agent_id = self
            .server
            .sessions
            .get(conversation_id)
            .map(|s| s.agent_id.clone());

        // Ambient sense rides in front of every turn — the date/time and who
        // is present — so she is never guessing what year it is.
        let ambient = crate::core::sensorium::ambient_line();

        let user_text = if let Some(agent_id) = session_agent_id {
            let surfacings = consciousness::drain_intrusive_surfacings(&self.server, &agent_id).await;
            if surfacings.is_empty() {
                format!("{}\n{}", ambient, text)
            } else {
                let prelude = surfacings
                    .iter()
                    .map(|line| line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}\n{}", ambient, prelude, text)
            }
        } else {
            format!("{}\n{}", ambient, text)
        };

        self.server.sessions.add_message(
            conversation_id,
            ConversationMessage::user_text(&user_text),
        )?;

        let (tx, rx) = mpsc::channel::<Result<BackendEvent>>(64);
        let server = self.server.clone();
        let conv_id = conversation_id.to_string();
        let event_bus = self.event_bus.clone();
        let active = self.active_sessions.clone();

        active.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            if let Err(e) = turn::run_turn(server, conv_id, &tx, event_bus, cancel, interject).await {
                let _ = tx.send(Err(e)).await;
            }
            let _ = tx.send(Ok(BackendEvent::Done)).await;
            active.fetch_sub(1, Ordering::Relaxed);
        });

        Ok(ReceiverStream::new(rx).boxed())
    }

    async fn update_agent_model(&self, agent_id: &str, model: &str) -> Result<()> {
        // Load current llm_config so we only change the model field —
        // context_window, temperature, tool rounds stay as they were.
        let current = self.server.agents.get(agent_id).await?;
        let update = crate::api::models::UpdateAgentRequest {
            name: None,
            description: None,
            llm_config: Some(crate::api::models::LlmConfig {
                model: model.to_string(),
                context_window: current.llm_config.context_window,
                temperature: current.llm_config.temperature,
                max_tool_rounds: current.llm_config.max_tool_rounds,
                inter_round_delay_ms: current.llm_config.inter_round_delay_ms,
            }),
            memory_blocks: None,
            tools: None,
        };
        self.server.agents.update(agent_id, update).await?;
        tracing::info!(agent = %agent_id, model = %model, "agent llm_config model updated via settings");
        Ok(())
    }

    async fn take_pending_surfacings(
        &self,
        agent_id: &str,
    ) -> Vec<crate::core::nervous::pending::PendingSurfacing> {
        let dir = self.server.agents.agent_data_dir(agent_id);
        crate::core::nervous::pending::take(&dir).await
    }
}
