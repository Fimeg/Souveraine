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
    let (agent_id, messages) = {
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

    let req = ChatCompletionRequest {
        model: agent.llm_config.model.clone(),
        messages,
        stream: Some(false),
        max_tokens: None,
        temperature: agent.llm_config.temperature,
        tools: None,
    };

    let response = server.bifrost.chat_completion(req).await?;
    let content = response.content.clone();

    // Mirror the server's chunked streaming so the CLI/TUI sees progressive
    // tokens (the underlying call is non-streaming today; replace once Bifrost
    // SSE lands).
    let chars: Vec<char> = content.chars().collect();
    for chunk in chars.chunks(10) {
        let s: String = chunk.iter().collect();
        if tx.send(Ok(BackendEvent::Token(s))).await.is_err() {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    server.sessions.add_message(
        &conversation_id,
        ConversationMessage::assistant_text(&content),
    )?;

    let events = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        server.consciousness.on_response(&*session, &content).await?
    };

    for event in events {
        let be = match event {
            ConsciousnessEvent::Surfacing { source, content, priority } => BackendEvent::Surfacing {
                source: source.to_string(),
                content,
                priority: priority.to_string(),
            },
            ConsciousnessEvent::Reflection { content } => BackendEvent::Reflection(content),
            ConsciousnessEvent::Archivist { synthesis, pressure } => BackendEvent::Archivist {
                synthesis,
                pressure,
            },
        };
        if tx.send(Ok(be)).await.is_err() {
            return Ok(());
        }
    }

    if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
        let pressure = server.consciousness.calculate_pressure(&session.messages);
        session.context_pressure = pressure;
    }

    Ok(())
}
