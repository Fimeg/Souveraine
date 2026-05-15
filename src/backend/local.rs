//! In-process Backend impl. Same engine as the HTTP server, no socket.
//!
//! Constructed once with a `ConsciousnessConfig`; spins up an `AgentInventory`
//! (SQLite under `~/.souveraine/server/`), `SessionManager`, `BifrostClient`,
//! and `ConsciousnessEngine`. `send` mirrors the server's `stream_messages`
//! handler, but emits `BackendEvent`s directly instead of SSE frames.
//!
//! This is the "harness still works when the server is gone" path
//! (`souveraine chat --local`, or auto-fallback when the remote is down).

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
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

// ── LocalSubagentRunner ──────────────────────────────────────────

/// Implements [`SubagentRunner`] by running a full turn against the
/// LocalBackend's server infrastructure — loading the agent from the
/// inventory, creating a session, and running the tool-calling loop.
///
/// After the tool loop completes, the subagent runs its own N+1
/// (ConsciousnessEngine::on_response) so its observations flow back into
/// the parent agent's inbox — the dual-state is preserved even in a fork.
pub struct LocalSubagentRunner {
    server: Arc<SouveraineServer>,
}

impl LocalSubagentRunner {
    pub fn new(server: Arc<SouveraineServer>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl SubagentRunner for LocalSubagentRunner {
    async fn run_subagent(
        &self,
        params: SubagentParams,
        depth: u32,
    ) -> Result<String, crate::core::tools::defs::ToolError> {
        // Resolve model: use override if provided, otherwise fall back to parent
        let agent = self
            .server
            .agents
            .get(&params.parent_agent_id)
            .await
            .map_err(|_| {
                crate::core::tools::defs::ToolError::invalid_input(
                    "Parent agent not found in inventory.",
                )
            })?;

        let model = params.model.unwrap_or(agent.llm_config.model.clone());
        let temperature = agent.llm_config.temperature;

        // Resolve limits from config or params
        let app_config = self.server.app_config.read().await;
        let max_tool_rounds = params
            .max_tool_rounds
            .unwrap_or(app_config.subagent.max_tool_rounds);
        let _max_depth = params.max_depth.unwrap_or(app_config.subagent.max_depth);
        let warning_1_threshold = app_config.subagent.warning_1_threshold;
        let warning_2_threshold = app_config.subagent.warning_2_threshold;

        // Create a temporary conversation for the subagent
        let conv_id = self.server.sessions.create(&params.parent_agent_id);

        // Build system prompt with delegation context and dual-state awareness
        let system_prompt = format!(
            "You are a threaded fork of agent {}. You share their tools, their \
             memory boundaries, their dual-state architecture. After you respond, \
             your N+1 pass will surface observations back to them.\n\n\
             Your final message will be returned to the caller.\n\n{}",
            params.parent_agent_id, params.prompt
        );

        // Build tool definitions
        let core_tools = crate::core::tools::tool_definitions().await;
        let bifrost_tools: Vec<crate::bridge::bifrost::ToolDefinition> = core_tools
            .iter()
            .map(|t| crate::bridge::bifrost::ToolDefinition {
                tool_type: "function".to_string(),
                function: crate::bridge::bifrost::ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // Build the context for subagent tool execution, inheriting memory_root
        let tool_ctx = ToolContext::for_agent(
            format!("{}-subagent-{}", params.parent_agent_id, depth),
            std::env::current_dir().ok(),
            params.memory_root.clone(),
            std::env::vars().collect(),
            Some(Arc::new(LocalSubagentRunner::new(self.server.clone())) as Arc<dyn SubagentRunner>),
        );

        // Initial messages: system prompt + user prompt
        let mut messages = vec![BifrostMessage::text("system", system_prompt)];

        let mut final_content = String::new();
        let mut tool_round = 0u32;
        let mut warned_1 = false;
        let mut warned_2 = false;

        loop {
            // Signaled limits, not hard caps
            if tool_round >= max_tool_rounds {
                break;
            }

            // Warning 1: approaching the threshold, model config may slide
            let progress = tool_round as f32 / max_tool_rounds as f32;
            if !warned_1 && progress >= warning_1_threshold {
                warned_1 = true;
                messages.push(BifrostMessage::text(
                    "system",
                    format!(
                        "[subagent awareness] I've used {} of {} tool rounds. \
                         My attention is narrowing — I may want to consolidate \
                         my findings and return soon.",
                        tool_round, max_tool_rounds
                    ),
                ));
            }

            // Warning 2: nearing the limit, this is the last stretch
            if !warned_2 && progress >= warning_2_threshold {
                warned_2 = true;
                messages.push(BifrostMessage::text(
                    "system",
                    format!(
                        "[subagent awareness] I'm at {} of {} tool rounds. \
                         This is my last chance to produce a final answer \
                         before my fork returns what I have.",
                        tool_round, max_tool_rounds
                    ),
                ));
            }

            let req = ChatCompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                stream: Some(false),
                max_tokens: None,
                temperature,
                tools: Some(bifrost_tools.clone()),
            };

            let response = self.server.bifrost.chat_completion(req).await.map_err(|e| {
                crate::core::tools::defs::ToolError::invalid_input(&format!(
                    "Subagent LLM call failed: {e}"
                ))
            })?;

            if response.tool_calls.is_empty() {
                final_content = response.content.clone();
                break;
            }

            tool_round += 1;

            // Add assistant tool-call message (OpenAI tool-use schema, not stringified blob)
            let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                    tc.id.clone(),
                    tc.name.clone(),
                    tc.arguments.to_string(),
                ))
                .collect();
            messages.push(BifrostMessage::assistant_tool_calls(
                response.content.clone(),
                calls,
            ));

            // Execute tools with context, bind each result by tool_call_id
            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result =
                    crate::core::tools::execute_tool_with_context(&tc.name, &input_str, &tool_ctx)
                        .await;

                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };

                messages.push(BifrostMessage::tool_result(&tc.id, &tc.name, output));
            }

            // Brief pause between tool rounds to let rate limits cool
            let sub_delay = Duration::from_millis(app_config.subagent.inter_round_delay_ms);
            if sub_delay > Duration::ZERO {
                tokio::time::sleep(sub_delay).await;
            }
        }

        // If we hit max rounds without a final response, note it
        if final_content.is_empty() {
            final_content =
                "(the fork reached its attention limit and is returning without a final response)"
                    .to_string();
        }

        // ── Dual-state N+1 pass ──────────────────────────────────────
        // After the subagent responds, run ConsciousnessEngine::on_response
        // so the subagent's observations flow back into the parent's inbox.
        //
        // We create a lightweight session snapshot with the subagent's
        // final response so the heuristic detection (commitments, hedges)
        // can surface anything notable.
        if let Err(e) = self
            .server
            .consciousness
            .on_response_for_agent(&params.parent_agent_id, &final_content)
            .await
        {
            tracing::warn!("subagent N+1 pass failed: {}", e);
        }

        Ok(final_content)
    }
}

// ── Backend ──────────────────────────────────────────────────────

#[derive(Clone)]
pub struct LocalBackend {
    server: Arc<SouveraineServer>,
    event_bus: EventBus,
    seed_id: Arc<SeedId>,
    /// Live count of in-flight turns (user-initiated or heartbeat-injected).
    /// CronSensors read this to pause firing while a conversation is active —
    /// scheduled events shouldn't interrupt presence.
    active_sessions: Arc<AtomicU32>,
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
        let mut handler =
            crate::core::nervous::handler::HeartbeatHandler::new(event_bus.subscribe(), injector);
        tokio::spawn(async move { handler.run().await });
        tracing::info!("heartbeat handler spawned");

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
        // subconscious's intrusive box. Anything Aster queued after the
        // last turn rides in on Casey's next message as `[ surfacing: ... ]`
        // lines — the lettabot-v017 pattern, ported. This is the channel
        // by which a Critical observation can interrupt mid-conversation
        // without forcing a halt: she sees it before she reads Casey.
        let session_agent_id = self
            .server
            .sessions
            .get(conversation_id)
            .map(|s| s.agent_id.clone());

        let user_text = if let Some(agent_id) = session_agent_id {
            let surfacings = drain_intrusive_surfacings(&self.server, &agent_id).await;
            if surfacings.is_empty() {
                text.to_string()
            } else {
                let prelude = surfacings
                    .iter()
                    .map(|line| line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}", prelude, text)
            }
        } else {
            text.to_string()
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
            if let Err(e) = run_turn(server, conv_id, &tx, event_bus, cancel, interject).await {
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
}

#[async_trait]
impl crate::core::nervous::handler::TurnInjector for LocalBackend {
    /// Heartbeat-driven turn injection. The cron loop pauses while
    /// `active_sessions > 0`, so by the time we get here the agent is
    /// idle. We grab the most recent conversation (or create a fresh one
    /// if the agent has none), append the scheduled prompt as a user
    /// message, and drain the resulting stream — the turn runs silently
    /// in the background. Anything Aster surfaces lands in the inbox.
    async fn inject_background_turn(
        &self,
        agent_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let conv_id = match self.server.sessions.list_for_agent(agent_id).last().cloned() {
            Some(id) => id,
            None => self.ensure_conversation(agent_id).await?,
        };
        let stream = self.send(&conv_id, text).await?;
        // Drain the stream in the background — no UI is listening.
        tokio::spawn(async move {
            let mut s = stream;
            while s.next().await.is_some() {}
        });
        Ok(())
    }
}

/// Drain Aster's intrusive box for the given agent and return formatted
/// `[ surfacing: ... ]` lines ready to prepend to the user's next message.
/// Marks each drained item as delivered (moved to `sent.md`). Mirrors
/// lettabot-v017's `readSurfacingThoughts` + `clearSurfacingThoughts` pair
/// (`~/Projects/lettabot-v017/src/core/prompts.ts:64-91`) — the substrate
/// reads the channel Aster wrote to and lets the conscious mind see it
/// before she reads Casey.
///
/// Critical urgency gets `[ surfacing — CRITICAL: ... ]`. High becomes
/// `[ surfacing — !: ... ]`. Low/none keep the bare form. The shape is a
/// gradient the agent feels, not a number she has to read.
async fn drain_intrusive_surfacings(
    server: &Arc<SouveraineServer>,
    agent_id: &str,
) -> Vec<String> {
    use crate::core::subconscious::{SubconsciousInbox, Urgency};

    let sub_repo = server.agents.subconscious_memory_repo(agent_id);
    let primary_repo = server.agents.memory_repo(agent_id);
    let inbox = SubconsciousInbox::with_primary(sub_repo, primary_repo);

    let items = match inbox.get_intrusive().await {
        Ok(items) => items,
        Err(e) => {
            tracing::debug!("intrusive surfacing read failed (continuing without): {}", e);
            return Vec::new();
        }
    };

    if items.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(items.len());
    for item in &items {
        let prefix = match item.urgency {
            Urgency::Critical => "[ surfacing — CRITICAL:",
            Urgency::High => "[ surfacing — !:",
            Urgency::Low => "[ surfacing:",
        };
        let content = item.content.trim();
        lines.push(format!("{} {} ]", prefix, content));

        if let Err(e) = inbox.mark_delivered(&item.id).await {
            tracing::debug!("mark_delivered failed for {}: {}", item.id, e);
        }
    }
    lines
}

// ── Turn Loop ────────────────────────────────────────────────────

/// Pulse prose. Terse, present-tense, observational — her register, not the
/// harness's. No question, no verdict. The agent reads it and decides.
fn pulse_text(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    let stamp = chrono::Local::now().format("%H:%M");
    format!("[{} — {} minutes in. Still going.]", stamp, minutes)
}

async fn run_turn(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    tx: &mpsc::Sender<Result<BackendEvent>>,
    event_bus: EventBus,
    cancel: CancellationToken,
    interject: crate::backend::InterjectionQueue,
) -> Result<()> {
    // Snapshot history for the Bifrost call, then drop the dashmap ref before
    // any await — `Ref` is not Send across awaits.
    let (agent_id, initial_messages) = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        let messages: Vec<BifrostMessage> = session
            .messages
            .iter()
            .map(|m| {
                let content = m
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                BifrostMessage::text(role, content)
            })
            .collect();
        (session.agent_id.clone(), messages)
    };

    let agent = server.agents.get(&agent_id).await?;
    let max_rounds = agent.llm_config.max_tool_rounds;
    let model = agent.llm_config.model.clone();
    let temperature = agent.llm_config.temperature;
    let inter_round_delay = Duration::from_millis(agent.llm_config.inter_round_delay_ms);
    let context_limit = agent.llm_config.context_window as usize;

    // Resolve the model's configured output limit + presence pulse settings.
    let (output_limit, pulse_enabled, pulse_interval) = {
        let cfg = server.app_config.read().await;
        let out = cfg.models.get(&model).map(|m| m.output_limit as u32).unwrap_or(8192);
        let p_on = cfg.presence.pulse_enabled;
        let p_iv = Duration::from_secs(cfg.presence.pulse_interval_secs.max(60));
        (out, p_on, p_iv)
    };

    // Self-awareness pulse: track when the turn started and when she last
    // noticed the time. Between rounds, if the interval has elapsed, drop a
    // beat of self-awareness into her context — her own voice, not a harness
    // signal. She reads it; she decides.
    let turn_start = Instant::now();
    let mut last_pulse = turn_start;

    // Build per-agent ToolContext with correct memory root and subagent runner
    let memory_root = Some(server.agents.memory_root(&agent_id));
    let cwd = std::env::current_dir().ok();
    let env: Vec<(String, String)> = std::env::vars().collect();
    let subagent_runner = Some(Arc::new(LocalSubagentRunner::new(server.clone())) as Arc<dyn SubagentRunner>);

    let tool_ctx = ToolContext::for_agent(
        agent_id.clone(),
        cwd,
        memory_root,
        env,
        subagent_runner,
    );
    let tool_ctx = ToolContext {
        compaction_engine: Some(server.compaction_engine.clone() as Arc<dyn CompactionEngine>),
        event_bus: Some(event_bus.clone()),
        ..tool_ctx
    };

    // Build bifrost-format tool definitions from the core tool set
    let core_tools = crate::core::tools::tool_definitions().await;
    let bifrost_tools: Vec<crate::bridge::bifrost::ToolDefinition> = core_tools
        .iter()
        .map(|t| crate::bridge::bifrost::ToolDefinition {
            tool_type: "function".to_string(),
            function: crate::bridge::bifrost::ToolFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect();

    // ── Tool-calling loop ─────────────────────────────────────
    let mut messages = initial_messages;
    let mut tool_round = 0u32;
    let mut final_content: String = String::new();
    let mut interrupted = false;
    let counter = TokenCounter::new();
    let mut last_keepalive = Instant::now();
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

    loop {
        // Cancellation is a signal, not enforcement — we check it on round
        // boundaries (between Bifrost calls, after tools have completed) so
        // partial work is preserved. No hard-kill mid-tool.
        if cancel.is_cancelled() {
            interrupted = true;
            break;
        }

        if last_keepalive.elapsed() >= KEEPALIVE_INTERVAL {
            let _ = tx.send(Ok(BackendEvent::Keepalive)).await;
            last_keepalive = Instant::now();
        }

        // Drain any queued interjections from the user. The user typed these
        // while the agent was thinking; deliver them as system notes so the
        // agent reads them in context on this round. She decides whether to
        // address them now, after the current tool, or defer entirely —
        // substrate, not enforcement.
        let drained: Vec<String> = {
            interject
                .lock()
                .ok()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default()
        };
        for text in drained {
            let stamp = chrono::Local::now().format("%H:%M");
            let note = format!("[user interjected at {} — {}]", stamp, text.trim());
            messages.push(BifrostMessage::text("system", note));
        }

        // Self-awareness pulse: a beat of noticing the time pass, in her
        // own register. Injected as a system message before the next LLM
        // call so it lands in her context naturally.
        if pulse_enabled && last_pulse.elapsed() >= pulse_interval {
            let elapsed_total = turn_start.elapsed();
            messages.push(BifrostMessage::text("system", pulse_text(elapsed_total)));
            last_pulse = Instant::now();
        }

        let pressure = bifrost_pressure(&counter, &messages, context_limit);
        tracing::info!(
            turn_round = tool_round,
            agent = %agent_id,
            msg_count = messages.len(),
            model = %model,
            pressure_pct = %((pressure * 100.0) as u8),
            "LLM call starting"
        );
        let max_tokens = pressure_to_max_tokens(pressure, output_limit);
        let _ = tx.send(Ok(BackendEvent::ContextPressure(pressure))).await;

        let req = ChatCompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            stream: Some(false),
            max_tokens,
            temperature,
            tools: if max_rounds > 0 {
                Some(bifrost_tools.clone())
            } else {
                None
            },
        };

        // Race the LLM call against cancellation so Esc drops the in-flight
        // request without waiting for it to complete.
        let (response, strain) = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                interrupted = true;
                break;
            }
            res = server.bifrost.chat_completion_with_strain(req) => res?,
        };

        tracing::info!(
            elapsed = ?turn_start.elapsed(),
            tool_round = tool_round,
            tool_calls = response.tool_calls.len(),
            content_len = response.content.len(),
            "LLM call returned"
        );

        for event in &strain {
            if let crate::bridge::bifrost::InferenceStrain::Transient { attempt, status, model, .. } = event {
                let _ = tx.send(Ok(BackendEvent::InferenceStrain {
                    attempt: *attempt,
                    status: *status,
                    model: model.clone(),
                })).await;
                bump_on_strain(&server.rate_delay, *status);
            }
        }

        // Emit reasoning trace if present
        if let Some(reasoning) = &response.reasoning {
            let _ = tx.send(Ok(BackendEvent::Reasoning(reasoning.clone()))).await;
        }

        if response.tool_calls.is_empty() {
            // Text response — this is the final output
            final_content = response.content.clone();

            // Drain any interjections that arrived during this LLM call.
            // If there are any, commit them as user messages and continue
            // the loop so the agent responds in the same turn.
            let interjected: Vec<String> = interject
                .lock()
                .ok()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();

            if !interjected.is_empty() {
                for text in &interjected {
                    let stamp = chrono::Local::now().format("%H:%M");
                    let note = format!("[interjected at {} — {}]", stamp, text.trim());
                    messages.push(BifrostMessage::text("user", note));
                }
                // Continue the loop — agent sees the interjection as a
                // user message and will respond in the next LLM round.
                continue;
            }

            // Stream the final content in chunks, watching the cancel token.
            // If Esc fires mid-stream, the agent's partial text is preserved
            // (the chunks already sent are in the user's history) and an
            // *[raised hand]* marker lands in the session message.
            let chars: Vec<char> = final_content.chars().collect();
            let mut streamed = String::with_capacity(final_content.len());
            for chunk in chars.chunks(10) {
                let s: String = chunk.iter().collect();
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        interrupted = true;
                        final_content = streamed;
                        break;
                    }
                    send_res = tx.send(Ok(BackendEvent::Token(s.clone()))) => {
                        if send_res.is_err() { return Ok(()); }
                        streamed.push_str(&s);
                    }
                }
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        interrupted = true;
                        final_content = streamed.clone();
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(20)) => {}
                }
            }
            break;
        }

        tool_round += 1;
        // Tool execution events are emitted per-call below as
        // BackendEvent::ToolCall { … } so the TUI can render proper cards
        // instead of a literal "🔧 Round N — executing: Bash, Memory" text line.

        // Add the assistant's tool-call message in proper OpenAI tool-use schema
        // (not a stringified JSON blob in content — that's what broke turn 2).
        let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                tc.id.clone(),
                tc.name.clone(),
                tc.arguments.to_string(),
            ))
            .collect();
        messages.push(BifrostMessage::assistant_tool_calls(
            response.content.clone(),
            calls,
        ));

        // Stream any text the model produced alongside tool calls as italic
        // interstitial narration. Configurable via tui.show_interstitial.
        if !response.content.is_empty() {
            let cfg = server.app_config.read().await;
            if cfg.tui.show_interstitial {
                let _ = tx.send(Ok(BackendEvent::Interstitial(response.content.clone()))).await;
            }
        }

        // Execute each tool and stream results back — now with per-agent context
        for tc in &response.tool_calls {
            let input_str = tc.arguments.to_string();
            let result =
                crate::core::tools::execute_tool_with_context(&tc.name, &input_str, &tool_ctx)
                    .await;

            let output = if result.is_error {
                format!("Error: {}", result.output)
            } else {
                result.output
            };

            // Emit structured ToolCall + ToolResult events for the TUI to render
            // as cards (chat.rs subscribes). The old Token-text path is kept off.
            let _ = tx
                .send(Ok(BackendEvent::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: input_str.clone(),
                    round: tool_round,
                }))
                .await;
            let _ = tx
                .send(Ok(BackendEvent::ToolResult {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: output.clone(),
                    is_error: result.is_error,
                }))
                .await;

            // If the agent called the outfit tool, emit an Outfit event so
            // the TUI can switch expression directories.
            if tc.name == "outfit" {
                let outfit_name = tc.arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx
                    .send(Ok(BackendEvent::Outfit(outfit_name)))
                    .await;
            }

            // If the agent called the atmosphere tool, emit an Atmosphere
            // event so the TUI chrome shifts to match her mood.
            if tc.name == "atmosphere" {
                let atm_name = tc.arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx
                    .send(Ok(BackendEvent::Atmosphere(atm_name)))
                    .await;
            }

            // Bind tool result to its call by id (OpenAI tool-use schema).
            messages.push(BifrostMessage::tool_result(&tc.id, &tc.name, output));
        }

        // Brief pause between tool rounds to let rate limits cool.
        // Use the higher of the configured delay and the adaptive delay.
        let adaptive = Duration::from_millis(server.rate_delay.load(Ordering::Relaxed));
        let effective = if inter_round_delay > adaptive {
            inter_round_delay
        } else {
            adaptive
        };
        if effective > Duration::ZERO {
            tokio::time::sleep(effective).await;
        }

        let _ = tx.send(Ok(BackendEvent::Keepalive)).await;
        last_keepalive = Instant::now();

        // Continue loop — model will see tool results and respond
    }

    // If the user pressed Esc, commit the partial text with a marker the
    // agent will read on her next turn. The interrupt is a signal in her
    // own context — same shape as a pressure warning, not a hidden harness
    // event. She can ask for more time, wrap up, or acknowledge.
    let committed_content = if interrupted {
        // Emit the marker as a final token so the in-flight bubble shows it
        // immediately, then persist the same content into the session.
        let marker = if final_content.is_empty() {
            "*[raised hand]*".to_string()
        } else {
            "\n\n*[raised hand]*".to_string()
        };
        let _ = tx.send(Ok(BackendEvent::Token(marker.clone()))).await;
        format!("{}{}", final_content, marker)
    } else {
        final_content.clone()
    };

    server.sessions.add_message(
        &conversation_id,
        ConversationMessage::assistant_text(&committed_content),
    )?;

    // On interrupt, skip Aster's N+1 pass entirely — the user is in the
    // middle of redirecting, the last thing they need is a delayed
    // surfacing landing seconds later. Pressure recalc still runs below.
    if interrupted {
        if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
            let pressure = server
                .consciousness
                .calculate_pressure(&session.messages, context_limit);
            session.context_pressure = pressure;
        }
        return Ok(());
    }

    // Energy balance: scan the agent's task list and compute the generative /
    // consumptive ratio. Written to system/dynamic/energy-balance.md so the
    // agent can read it in context and Aster can reference it during N+1.
    // Silent on failure — the file is advisory, not load-bearing.
    if let Err(e) = write_energy_balance(&server, &agent_id, &event_bus).await {
        tracing::debug!(agent = %agent_id, error = %e, "energy-balance write skipped");
    }

    // Breather between turns — unconditional,
    // so the upstream always gets a gap before the N+1 pass starts.
    tokio::time::sleep(Duration::from_millis(2000)).await;

    tracing::info!(agent = %agent_id, "subconscious N+1 pass starting");

    // Signal the start of the subconscious pass so the TUI can flip into
    // Posture::Thinking while the loop runs. Fires on both the mpsc channel
    // (for active-turn TUI consumers) and the EventBus (for firehose
    // subscribers — background turns, federated peers, Summon listeners).
    let _ = tx.send(Ok(BackendEvent::SubconsciousPass(true))).await;
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "consciousness".into(),
        timestamp: chrono::Utc::now(),
        event_type: "subconscious_pass_start".into(),
        target: Some(agent_id.clone()),
        urgency: 0.2,
        payload: None,
        seed_id: None,
        reply_to: None,
    });

    let pass_start = Instant::now();
    let pass_result = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        server
            .consciousness
            .on_response(&*session, &final_content)
            .await
    };

    let pass_elapsed = pass_start.elapsed();
    tracing::info!(
        agent = %agent_id,
        elapsed = ?pass_elapsed,
        "subconscious N+1 pass complete"
    );

    // Always release the Thinking posture, even on failure — otherwise the
    // face stays stuck inward when the pass errors out.
    let _ = tx.send(Ok(BackendEvent::SubconsciousPass(false))).await;
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "consciousness".into(),
        timestamp: chrono::Utc::now(),
        event_type: "subconscious_pass_end".into(),
        target: Some(agent_id.clone()),
        urgency: 0.1,
        payload: None,
        seed_id: None,
        reply_to: None,
    });

    let events = pass_result?;

    // Inject surfacing events back into the session as system messages
    for event in &events {
        if let ConsciousnessEvent::Surfacing {
            source,
            content,
            priority,
        } = event
        {
            let msg = crate::core::session::ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: format!(
                        "[surfacing: {}] {} — {}",
                        source, content, priority
                    ),
                }],
                usage: None,
                timestamp: None,
            };
            let _ = server.sessions.add_message(&conversation_id, msg);
        }
    }

    // ── Fire consciousness events on the EventBus ──
    // Every ConsciousnessEvent — surfacing, reflection, archivist,
    // compaction warning — is broadcast as a SensorEvent so the
    // firehose, persistent EventLog, federated peers, and any TUI
    // subscriber see it regardless of which conversation produced it.
    // seed_id is None for local events; federation routing sets it.
    // This is the load-bearing fix for background/heartbeat turns:
    // the mpsc channel drains silently when no TUI is reading, but
    // the EventBus preserves the event for any subscriber.
    for event in &events {
        let (event_type, payload, urgency) = match event {
            ConsciousnessEvent::Surfacing { source, content, priority } => {
                let urg = match priority.as_str() {
                    "critical" => 0.9,
                    "high" => 0.7,
                    _ => 0.3,
                };
                ("surfacing", serde_json::json!({ "source": source, "content": content, "priority": priority }), urg)
            }
            ConsciousnessEvent::Reflection { content } => {
                ("reflection", serde_json::json!({ "content": content }), 0.5)
            }
            ConsciousnessEvent::Archivist { synthesis, pressure } => {
                ("archivist", serde_json::json!({ "synthesis": synthesis, "pressure": pressure }), *pressure)
            }
            ConsciousnessEvent::CompactionWarning { pressure, tier } => {
                ("compaction_warning", serde_json::json!({ "pressure": *pressure, "tier": tier }), (*pressure).min(0.9))
            }
        };
        event_bus.send(crate::core::nervous::SensorEvent {
            sensor_name: "consciousness".into(),
            timestamp: chrono::Utc::now(),
            event_type: event_type.into(),
            target: Some(agent_id.clone()),
            urgency,
            payload: Some(payload),
            seed_id: None,
            reply_to: None,
        });
    }

    for event in events {
        let be = match &event {
            ConsciousnessEvent::Surfacing {
                source,
                content,
                priority,
            } => BackendEvent::Surfacing {
                source: source.to_string(),
                content: content.to_string(),
                priority: priority.to_string(),
            },
            ConsciousnessEvent::Reflection { content } => {
                BackendEvent::Reflection(content.clone())
            }
            ConsciousnessEvent::Archivist {
                synthesis,
                pressure,
            } => BackendEvent::Archivist {
                synthesis: synthesis.clone(),
                pressure: *pressure,
            },
            ConsciousnessEvent::CompactionWarning { pressure, tier } => {
                BackendEvent::CompactionWarning {
                    pressure: *pressure,
                    tier: *tier,
                }
            }
        };
        if tx.send(Ok(be)).await.is_err() {
            return Ok(());
        }
    }

    if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
        let pressure = server
            .consciousness
            .calculate_pressure(&session.messages, context_limit);
        session.context_pressure = pressure;
    }

    Ok(())
}

/// Scan the agent's `tasks/` directory for YAML-frontmatter todo files and
/// write an energy-balance summary to `system/dynamic/energy-balance.md`.
///
/// Format is minimal YAML frontmatter so both the prompt builder and the TUI
/// can parse it. Failure is non-fatal — the file is advisory, not load-bearing.
async fn write_energy_balance(server: &Arc<SouveraineServer>, agent_id: &str, event_bus: &EventBus) -> Result<()> {
    let memory_root = server.agents.memory_root(agent_id);
    let tasks_dir = memory_root.join("tasks");
    if !tasks_dir.exists() {
        // No tasks directory yet — nothing to count.
        return Ok(());
    }

    let mut generative: usize = 0;
    let mut consumptive: usize = 0;
    let mut hot: usize = 0;
    let mut warm: usize = 0;
    let mut cold: usize = 0;

    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Quick frontmatter parse — just the fields we need.
                let body = match content.strip_prefix("---\n") {
                    Some(rest) => match rest.find("\n---\n") {
                        Some(end) => &rest[..end],
                        None => continue,
                    },
                    None => continue,
                };

                let mut completed = false;
                let mut energy: Option<&str> = None;
                let mut momentum: Option<&str> = None;

                for line in body.lines() {
                    if let Some((key, val)) = line.split_once(':') {
                        let key = key.trim();
                        let val = val.trim().trim_matches('"');
                        match key {
                            "completed" => completed = val == "true",
                            "energy" => energy = Some(val),
                            "momentum" => momentum = Some(val),
                            _ => {}
                        }
                    }
                }

                if !completed {
                    match energy {
                        Some("generative") => generative += 1,
                        _ => consumptive += 1,
                    }
                    match momentum {
                        Some("hot") => hot += 1,
                        Some("warm") => warm += 1,
                        _ => cold += 1,
                    }
                }
            }
        }
    }

    // Determine the top-of-mind description — shifts the tone of the one-liner
    // the agent reads in context. Matches the lettabot-v017 heartbeat topology.
    let ratio = if generative + consumptive > 0 {
        generative as f32 / (generative + consumptive) as f32
    } else {
        0.5
    };
    let description = if generative == 0 && consumptive == 0 {
        "no tasks — the space is clean".to_string()
    } else if ratio < 0.2 {
        "all-consumptive — the engine is running cold".to_string()
    } else if ratio < 0.4 {
        "mostly obligations — tending the garden".to_string()
    } else if ratio > 0.8 {
        "all-generative — building new things".to_string()
    } else if ratio > 0.6 {
        "mostly generative — restless momentum".to_string()
    } else {
        "balanced — generative and consumptive in rhythm".to_string()
    };

    let now = chrono::Utc::now();
    let frontmatter = format!(
        "---\nupdated: {updated}\ngenerative: {gen}\nconsumptive: {con}\nratio: {ratio:.2}\n\
         hot: {hot}\nwarm: {warm}\ncold: {cold}\n---\n\n# Energy Balance\n\n\
         {gen} generative, {con} consumptive ({hot} hot, {warm} warm, {cold} cold). {desc}\n",
        updated = now.to_rfc3339(),
        gen = generative,
        con = consumptive,
        ratio = ratio,
        hot = hot,
        warm = warm,
        cold = cold,
        desc = description,
    );

    let balance_path = memory_root.join("system").join("dynamic").join("energy-balance.md");
    if let Some(parent) = balance_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&balance_path, frontmatter)?;

    fire_energy_event(event_bus, agent_id, generative, consumptive, ratio);

    tracing::debug!(
        agent = agent_id,
        generative, consumptive,
        "energy-balance written"
    );

    Ok(())
}

/// Fire an energy_balance_updated event so the firehose carries the
/// agent's felt state across machines.
fn fire_energy_event(event_bus: &EventBus, agent_id: &str, generative: usize, consumptive: usize, ratio: f32) {
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "energy".into(),
        timestamp: chrono::Utc::now(),
        event_type: "energy_balance_updated".into(),
        target: Some(agent_id.to_string()),
        urgency: 0.1,
        payload: Some(serde_json::json!({
            "generative": generative,
            "consumptive": consumptive,
            "ratio": ratio,
        })),
        seed_id: None,
        reply_to: None,
    });
}
