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
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::bridge::bifrost::{ChatCompletionRequest, Message as BifrostMessage};
use crate::core::config::ConsciousnessConfig;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::server::{ConsciousnessEvent, SouveraineServer};

use super::{AgentInfo, Backend, BackendEvent};

#[derive(Clone)]
pub struct LocalBackend {
    server: Arc<SouveraineServer>,
}

impl LocalBackend {
    pub async fn new(config: ConsciousnessConfig) -> Result<Self> {
        let server = SouveraineServer::new(config)
            .await
            .context("LocalBackend: SouveraineServer init")?;
        Ok(Self { server: Arc::new(server) })
    }

    pub fn from_server(server: Arc<SouveraineServer>) -> Self {
        Self { server }
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

    async fn ensure_conversation(&self, agent_id: &str) -> Result<String> {
        // Validate the agent exists; matches RemoteBackend's contract.
        let _ = self.server.agents.get(agent_id).await?;
        Ok(self.server.sessions.create(agent_id))
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

        tokio::spawn(async move {
            if let Err(e) = run_turn(server, conv_id, &tx).await {
                let _ = tx.send(Err(e)).await;
            }
            let _ = tx.send(Ok(BackendEvent::Done)).await;
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

async fn run_turn(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    tx: &mpsc::Sender<Result<BackendEvent>>,
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

    // Build bifrost-format tool definitions from the core tool set
    let core_tools = crate::core::tools::tool_definitions();
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

    loop {
        let req = ChatCompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            stream: Some(false),
            max_tokens: None,
            temperature,
            tools: if max_rounds > 0 {
                Some(bifrost_tools.clone())
            } else {
                None
            },
        };

        let response = server.bifrost.chat_completion(req).await?;

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

        // Execute each tool and stream results back
        for tc in &response.tool_calls {
            let input_str = tc.arguments.to_string();
            let result = crate::core::tools::execute_tool(&tc.name, &input_str).await;

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

        // Continue loop — model will see tool results and respond
    }

    // ── Post-turn processing (unchanged) ───────────────────────
    server.sessions.add_message(
        &conversation_id,
        ConversationMessage::assistant_text(&final_content),
    )?;

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
        };
        if tx.send(Ok(be)).await.is_err() {
            return Ok(());
        }
    }

    if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
        let pressure = server
            .consciousness
            .calculate_pressure(&session.messages);
        session.context_pressure = pressure;
    }

    Ok(())
}
