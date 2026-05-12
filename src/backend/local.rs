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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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
        let mut messages = vec![BifrostMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];

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
                messages.push(BifrostMessage {
                    role: "system".to_string(),
                    content: format!(
                        "[subagent awareness] I've used {} of {} tool rounds. \
                         My attention is narrowing — I may want to consolidate \
                         my findings and return soon.",
                        tool_round, max_tool_rounds
                    ),
                });
            }

            // Warning 2: nearing the limit, this is the last stretch
            if !warned_2 && progress >= warning_2_threshold {
                warned_2 = true;
                messages.push(BifrostMessage {
                    role: "system".to_string(),
                    content: format!(
                        "[subagent awareness] I'm at {} of {} tool rounds. \
                         This is my last chance to produce a final answer \
                         before my fork returns what I have.",
                        tool_round, max_tool_rounds
                    ),
                });
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

            // Add assistant tool-call message
            let call_text = serde_json::json!({
                "tool_calls": response.tool_calls.iter().map(|tc| {
                    serde_json::json!({"id": tc.id, "name": tc.name, "arguments": tc.arguments})
                }).collect::<Vec<_>>()
            }).to_string();
            messages.push(BifrostMessage {
                role: "assistant".to_string(),
                content: call_text,
            });

            // Execute tools with context
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

                messages.push(BifrostMessage {
                    role: "tool".to_string(),
                    content: output,
                });
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
}

impl LocalBackend {
    pub async fn new(config: ConsciousnessConfig) -> Result<Self> {
        let server = SouveraineServer::new(config.clone())
            .await
            .context("LocalBackend: SouveraineServer init")?;
        let event_bus = EventBus::default();

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

        Ok(Self {
            server: Arc::new(server),
            event_bus,
            seed_id,
        })
    }

    pub fn from_server(server: Arc<SouveraineServer>) -> Self {
        let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
        let seed_id = Arc::new(
            SeedId::load_or_generate(&SeedId::default_dir(&base))
                .unwrap_or_else(|_| SeedId::generate()),
        );
        Self {
            event_bus: EventBus::default(),
            server,
            seed_id,
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

        let memory_root = self.server.agents.memory_root(agent_id);
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

        let system_prompt =
            crate::core::prompt::build_system_prompt(&memory_root, Some(&skills)).await;

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

        let system_prompt =
            crate::core::prompt::build_system_prompt(&memory_root, Some(&skills)).await;

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
        self.server.sessions.add_message(
            conversation_id,
            ConversationMessage::user_text(text),
        )?;

        let (tx, rx) = mpsc::channel::<Result<BackendEvent>>(64);
        let server = self.server.clone();
        let conv_id = conversation_id.to_string();
        let event_bus = self.event_bus.clone();

        tokio::spawn(async move {
            if let Err(e) = run_turn(server, conv_id, &tx, event_bus).await {
                let _ = tx.send(Err(e)).await;
            }
            let _ = tx.send(Ok(BackendEvent::Done)).await;
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

// ── Turn Loop ────────────────────────────────────────────────────

async fn run_turn(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    tx: &mpsc::Sender<Result<BackendEvent>>,
    event_bus: EventBus,
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
                BifrostMessage {
                    role: role.to_string(),
                    content,
                }
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

    // Resolve the model's configured output limit for pressure scaling
    let output_limit = {
        let cfg = server.app_config.read().await;
        cfg.models.get(&model).map(|m| m.output_limit as u32).unwrap_or(8192)
    };

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
        event_bus: Some(event_bus),
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
    let final_content: String;
    let counter = TokenCounter::new();

    loop {
        let pressure = bifrost_pressure(&counter, &messages, context_limit);
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

        let (response, strain) = server.bifrost.chat_completion_with_strain(req).await?;

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

        if response.tool_calls.is_empty() || tool_round >= max_rounds {
            // Text response (or hit max rounds) — this is the final output
            final_content = response.content.clone();

            // If we hit max rounds with pending tool calls, add a note
            if !response.tool_calls.is_empty() && tool_round >= max_rounds {
                let note =
                    "\n\n[Max tool rounds reached — continuing with text response]";
                let _ = tx.send(Ok(BackendEvent::Token(note.to_string()))).await;
            }

            // Stream the final content in chunks
            let chars: Vec<char> = final_content.chars().collect();
            for chunk in chars.chunks(10) {
                let s: String = chunk.iter().collect();
                if tx.send(Ok(BackendEvent::Token(s))).await.is_err() {
                    return Ok(());
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            }
            break;
        }

        tool_round += 1;

        // Tell the UI we're executing tools
        let tool_names: Vec<&str> =
            response.tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        let announce = format!(
            "\n🔧 Round {} — executing: {}\n",
            tool_round,
            tool_names.join(", ")
        );
        let _ = tx
            .send(Ok(BackendEvent::Token(announce)))
            .await;

        // Add the assistant's tool-call message to the Bifrost conversation
        let call_text = serde_json::json!({
            "tool_calls": response.tool_calls.iter().map(|tc| {
                serde_json::json!({"id": tc.id, "name": tc.name, "arguments": tc.arguments})
            }).collect::<Vec<_>>()
        }).to_string();
        messages.push(BifrostMessage {
            role: "assistant".to_string(),
            content: call_text,
        });

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

            let status = if result.is_error { "❌" } else { "✅" };
            let result_line = format!("{} **{}**: {} char(s)\n", status, tc.name, output.len());
            let _ = tx
                .send(Ok(BackendEvent::Token(result_line)))
                .await;

            // Add tool result to bifrost messages for next loop iteration
            messages.push(BifrostMessage {
                role: "tool".to_string(),
                content: output,
            });
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

        // Continue loop — model will see tool results and respond
    }

    // ── Post-turn processing ───────────────────────────────────
    server.sessions.add_message(
        &conversation_id,
        ConversationMessage::assistant_text(&final_content),
    )?;

    // Breather between Ani finishing and Aster firing — unconditional,
    // so the upstream always gets a gap before the N+1 pass starts.
    tokio::time::sleep(Duration::from_millis(2000)).await;

    let events = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        server
            .consciousness
            .on_response(&*session, &final_content)
            .await?
    };

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
