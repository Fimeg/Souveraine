use crate::api::models::*;
use crate::server::SouveraineServer;
use crate::core::session::{ContentBlock, ConversationMessage};
use axum::{
    extract::{Path, Query, State, WebSocketUpgrade, ws::WebSocket},
    response::{Json, Sse},
    http::StatusCode,
    body::Bytes,
};
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub type ApiError = (StatusCode, Json<ErrorResponse>);

pub async fn list_agents(
    State(server): State<Arc<SouveraineServer>>,
    Query(filters): Query<AgentFilters>,
) -> Result<Json<Vec<AgentSummary>>, ApiError> {
    let filter = filters.name.or(filters.tags);
    let agents = server.agents.list(filter).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "list_failed".to_string(),
            message: e.to_string(),
        })))?;
    Ok(Json(agents))
}

pub async fn create_agent(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentState>), ApiError> {
    let agent = server.agents.create(request).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "creation_failed".to_string(),
            message: e.to_string(),
        })))?;
    Ok((StatusCode::CREATED, Json(agent)))
}

pub async fn get_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.agents.get(&id).await
        .map_err(|e| match e.to_string().contains("not found") {
            true => (StatusCode::NOT_FOUND, Json(ErrorResponse {
                error: "agent_not_found".to_string(),
                message: format!("Agent {} not found", id),
            })),
            false => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: "fetch_failed".to_string(),
                message: e.to_string(),
            })),
        })?;
    Ok(Json(agent))
}

pub async fn update_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
    Json(updates): Json<UpdateAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.agents.update(&id, updates).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "update_failed".to_string(),
            message: e.to_string(),
        })))?;
    Ok(Json(agent))
}

pub async fn delete_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    server.agents.delete(&id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "delete_failed".to_string(),
            message: e.to_string(),
        })))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_conversations(
    State(server): State<Arc<SouveraineServer>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<Conversation>>, ApiError> {
    let conversations = if let Some(agent_id) = params.get("agent_id") {
        let session_ids = server.sessions.list_for_agent(agent_id);
        session_ids.into_iter()
            .map(|id| Conversation {
                id,
                agent_id: agent_id.clone(),
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(conversations))
}

pub async fn create_conversation(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<(StatusCode, Json<Conversation>), ApiError> {
    let _ = server.agents.get(&request.agent_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "agent_not_found".to_string(),
            message: e.to_string(),
        })))?;

    let conversation_id = server.sessions.create(&request.agent_id);

    let conversation = Conversation {
        id: conversation_id,
        agent_id: request.agent_id,
        created_at: chrono::Utc::now(),
        updated_at: None,
    };

    Ok((StatusCode::CREATED, Json(conversation)))
}

pub async fn get_conversation(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<Conversation>, ApiError> {
    let session = server.sessions.get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "conversation_not_found".to_string(),
            message: format!("Conversation {} not found", id),
        })))?;

    let conversation = Conversation {
        id: session.conversation_id.clone(),
        agent_id: session.agent_id.clone(),
        created_at: session.created_at,
        updated_at: Some(session.updated_at),
    };

    Ok(Json(conversation))
}

pub async fn stream_messages(
    State(server): State<Arc<SouveraineServer>>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>, ApiError> {
    let _ = server.sessions.get(&conversation_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "conversation_not_found".to_string(),
            message: format!("Conversation {} not found", conversation_id),
        })))?;

    // Convert API messages to ConversationMessages and add to session
    for msg in &request.messages {
        let conv_msg = msg.to_conversation_message();
        server.sessions.add_message(&conversation_id, conv_msg)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: "message_store_failed".to_string(),
                message: e.to_string(),
            })))?;
    }

    let (tx, rx) = mpsc::channel(100);
    let server_clone = server.clone();
    let conv_id = conversation_id.clone();

    tokio::spawn(async move {
        if let Err(e) = handle_conversation_stream(server_clone, conv_id, tx).await {
            eprintln!("Stream error: {}", e);
        }
    });

    let stream = ReceiverStream::new(rx);
    let sse_stream = stream.map(|event: StreamEvent| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok(axum::response::sse::Event::default()
            .event(event.message_type())
            .data(json))
    });

    Ok(Sse::new(sse_stream))
}

async fn handle_conversation_stream(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    tx: mpsc::Sender<StreamEvent>,
) -> anyhow::Result<()> {
    let session = server.sessions.get(&conversation_id)
        .ok_or_else(|| anyhow::anyhow!("Session disappeared"))?;

    let agent_id = session.agent_id.clone();
    let messages: Vec<_> = session.messages.iter().map(|m| {
        let role = match m.role {
            crate::core::session::MessageRole::System => "system",
            crate::core::session::MessageRole::User => "user",
            crate::core::session::MessageRole::Assistant => "assistant",
            crate::core::session::MessageRole::Tool => "tool",
        };

        let has_images = m.blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. }));

        if has_images {
            use crate::bridge::bifrost::{ContentPart, ImageUrlSource};
            let parts: Vec<ContentPart> = m.blocks.iter().filter_map(|b| match b {
                ContentBlock::Text { text } => Some(ContentPart::Text { text: text.clone() }),
                ContentBlock::Image { media_type, data } => {
                    let url = format!("data:{media_type};base64,{data}");
                    Some(ContentPart::ImageUrl { image_url: ImageUrlSource { url } })
                }
                _ => None,
            }).collect();
            let text_content = m.blocks.iter().filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }).collect::<Vec<_>>().join("\n");
            crate::bridge::bifrost::Message::multimodal_user(text_content, parts)
        } else {
            let content = m.blocks.first().map(|b| match b {
                ContentBlock::Text { text } => text.clone(),
                _ => String::new(),
            }).unwrap_or_default();
            crate::bridge::bifrost::Message::text(role, content)
        }
    }).collect();
    drop(session);

    // Get agent config
    let agent = server.agents.get(&agent_id).await?;
    let model = agent.llm_config.model.clone();

    // Simple Bifrost call (no tool loop for now - that requires git components)
    let req = crate::bridge::bifrost::ChatCompletionRequest {
        model: model.clone(),
        messages,
        stream: Some(false),
        max_tokens: None,
        temperature: agent.llm_config.temperature,
        tools: None,
    };

    match server.bifrost.chat_completion(req).await {
        Ok(response) => {
            let content = response.content.clone();
            
            // Stream the response in chunks
            for chunk in content.chars().collect::<Vec<_>>().chunks(10) {
                let chunk_str: String = chunk.iter().collect();
                let _ = tx.send(StreamEvent::AssistantMessage { content: chunk_str }).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            }
            
            // Store assistant response in session
            let _ = server.sessions.add_message(
                &conversation_id,
                ConversationMessage::assistant_text(&content)
            );
            
            // Run consciousness events. Snapshot the session first — holding
            // a live ref across the N+1 pass would block concurrent writes.
            let (n1_agent, n1_turn_count, n1_messages) = {
                let session = server.sessions.get(&conversation_id)
                    .ok_or_else(|| anyhow::anyhow!("Session disappeared"))?;
                (session.agent_id.clone(), session.turn_count, session.messages.clone())
            };
            let events = server.consciousness
                .on_response(&n1_agent, n1_turn_count, &n1_messages, &content)
                .await?;
            
            // Inject surfacing events back into the session as system messages
            // so the agent sees them in its context window on the next turn.
            for event in &events {
                if let crate::server::ConsciousnessEvent::Surfacing { source, content, priority } = event {
                    let msg = crate::core::session::ConversationMessage {
                        role: crate::core::session::MessageRole::System,
                        blocks: vec![crate::core::session::ContentBlock::Text {
                            text: format!("[surfacing: {}] {} — {}", source, content, priority),
                        }],
                        usage: None,
                        timestamp: None,
                    };
                    let _ = server.sessions.add_message(&conversation_id, msg);
                }
            }

            for event in events {
                let stream_event = match &event {
                    crate::server::ConsciousnessEvent::Surfacing { source, content, priority } => {
                        StreamEvent::Surfacing { source: source.clone(), content: content.clone(), priority: priority.clone() }
                    }
                    crate::server::ConsciousnessEvent::Reflection { content } => {
                        StreamEvent::Reflection { content: content.clone() }
                    }
                    crate::server::ConsciousnessEvent::Archivist { synthesis, pressure } => {
                        StreamEvent::Archivist { synthesis: synthesis.clone(), pressure: *pressure }
                    }
                    crate::server::ConsciousnessEvent::CompactionWarning { pressure, tier } => {
                        StreamEvent::Archivist {
                            synthesis: format!("compaction warning tier {} at {:.0}%", tier, pressure * 100.0),
                            pressure: *pressure,
                        }
                    }
                };
                let _ = tx.send(stream_event).await;
            }
            
            // Update pressure
            let mut session = server.sessions.get_mut(&conversation_id)
                .ok_or_else(|| anyhow::anyhow!("Session disappeared"))?;
            let pressure = server.consciousness.pressure_for_session(&session).await;
            session.context_pressure = pressure;
            drop(session);
        }
        Err(e) => {
            eprintln!("Bifrost error: {}", e);
            let _ = tx.send(StreamEvent::AssistantMessage { 
                content: format!("Error: {}", e) 
            }).await;
        }
    }

    // Send ping at end
    let _ = tx.send(StreamEvent::Ping).await;

    Ok(())
}

// ─── Memory (memfs HTTP write path) ───────────────────────────────────────
//
// Replaces Letta's PATCH /v1/blocks/{id} for the cron-into-memfs pattern.
// Routes:
//   GET    /v1/agents/:id/memory                — list (?prefix=subdir)
//   GET    /v1/agents/:id/memory/*path          — read file
//   PUT    /v1/agents/:id/memory/*path          — write file (full replace)
//   PATCH  /v1/agents/:id/memory/*path          — append to file
//   DELETE /v1/agents/:id/memory/*path          — delete file

fn memory_err(status: StatusCode, kind: &str, e: impl ToString) -> ApiError {
    (status, Json(ErrorResponse {
        error: kind.to_string(),
        message: e.to_string(),
    }))
}

#[derive(serde::Deserialize)]
pub struct ListMemoryQuery {
    pub prefix: Option<String>,
}

pub async fn list_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path(agent_id): Path<String>,
    Query(q): Query<ListMemoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _ = server.agents.get(&agent_id).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    let entries = repo.list(q.prefix.as_deref()).await
        .map_err(|e| memory_err(StatusCode::INTERNAL_SERVER_ERROR, "list_failed", e))?;
    Ok(Json(serde_json::json!({ "entries": entries })))
}

pub async fn read_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _ = server.agents.get(&agent_id).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    let file = repo.read(&path).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "memory_not_found", e))?;
    Ok(Json(serde_json::json!({
        "path": path,
        "frontmatter": {
            "description": file.frontmatter.description,
            "read_only": file.frontmatter.read_only,
            "tags": file.frontmatter.tags,
        },
        "body": file.body,
    })))
}

pub async fn write_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let _ = server.agents.get(&agent_id).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let body_str = std::str::from_utf8(&body)
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "invalid_utf8", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.write(&path, body_str).await
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "write_failed", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn append_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let _ = server.agents.get(&agent_id).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let body_str = std::str::from_utf8(&body)
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "invalid_utf8", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.append(&path, body_str).await
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "append_failed", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let _ = server.agents.get(&agent_id).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.delete(&path).await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "delete_failed", e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// WebSocket firehose — streams every SensorEvent from the nervous
/// system as JSON. A second machine subscribes here and sees the
/// agent's energy, schedules, tool calls, posture changes in real time.
pub async fn firehose(
    State(server): State<Arc<SouveraineServer>>,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| firehose_stream(server, socket))
}

async fn firehose_stream(
    server: Arc<SouveraineServer>,
    mut socket: WebSocket,
) {
    use axum::extract::ws::Message;

    let mut rx = server.event_bus.subscribe();
    tracing::info!("firehose client connected");

    loop {
        match rx.recv().await {
            Ok(event) => {
                let json = match serde_json::to_string(&event) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(skipped = n, "firehose client lagged");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }

    tracing::info!("firehose client disconnected");
}

/// Inbound federation endpoint. Remote bridges connect here as WebSocket
/// clients and push Ed25519-signed SensorEvents. Each event's signature is
/// verified against its claimed signer pubkey; valid events are stamped
/// peer-originated and injected into the local EventBus. Invalid or
/// unparseable payloads are dropped. Authentication *is* the signature —
/// there is no bearer token on this route.
pub async fn federation_events(
    State(server): State<Arc<SouveraineServer>>,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| federation_events_stream(server, socket))
}

async fn federation_events_stream(server: Arc<SouveraineServer>, mut socket: WebSocket) {
    use axum::extract::ws::Message;

    tracing::info!("federation: peer bridge connected to inbound endpoint");

    while let Some(msg) = socket.recv().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        let signed: crate::server::federation::SignedEvent = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(error = %e, "federation: unparseable inbound payload — dropped");
                continue;
            }
        };
        match signed.verify() {
            Some(event) => {
                tracing::debug!(sensor = %event.sensor_name, "federation: inbound event verified");
                server.event_bus.send(event);
            }
            None => {
                tracing::warn!(
                    signer = %signed.signer_pubkey_hex,
                    "federation: signature verification failed — event dropped"
                );
            }
        }
    }

    tracing::info!("federation: peer bridge disconnected from inbound endpoint");
}
