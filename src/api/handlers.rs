use crate::api::models::*;
use crate::server::SouveraineServer;
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
    use crate::backend::BackendEvent;
    use tokio_util::sync::CancellationToken;

    let event_bus = server.event_bus.clone();
    let empty_interject: crate::backend::InterjectionQueue =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let cancel = CancellationToken::new();

    let (be_tx, mut be_rx) = mpsc::channel::<anyhow::Result<BackendEvent>>(64);
    let server_clone = server.clone();
    let conv = conversation_id.clone();

    tokio::spawn(async move {
        let _ = crate::server::turn::run_turn(
            server_clone, conv, &be_tx, event_bus,
            cancel, empty_interject,
        ).await;
    });

    // Drain the turn's BackendEvent stream and map to SSE events.
    // run_turn already handles: message storage, surficing injection,
    // EventBus dispatch, N+1 pass — none of that is needed here.
    while let Some(result) = be_rx.recv().await {
        let be = match result {
            Ok(be) => be,
            Err(_) => break,
        };
        let sse_event = match be {
            BackendEvent::Token(content) => StreamEvent::AssistantMessage { content },
            BackendEvent::Reasoning(content) => StreamEvent::ReasoningMessage { content },
            BackendEvent::Surfacing { source, content, priority } => {
                StreamEvent::Surfacing { source, content, priority }
            }
            BackendEvent::Reflection(content) => StreamEvent::Reflection { content },
            BackendEvent::Archivist { synthesis, pressure } => {
                StreamEvent::Archivist { synthesis, pressure }
            }
            BackendEvent::CompactionWarning { pressure, tier } => {
                StreamEvent::Archivist {
                    synthesis: format!("compaction warning tier {} at {:.0}%", tier, pressure * 100.0),
                    pressure,
                }
            }
            BackendEvent::ToolCall { id, name, arguments, .. } => {
                StreamEvent::ToolCallMessage {
                    tool_call: ToolCall {
                        id,
                        function: ToolFunction { name, arguments },
                    },
                }
            }
            BackendEvent::ToolResult { output, is_error, .. } => {
                StreamEvent::ToolReturnMessage {
                    tool_return: ToolReturn {
                        status: if is_error { "error".into() } else { "success".into() },
                        output,
                    },
                }
            }
            // Silently skip events that have no SSE counterpart
            _ => continue,
        };
        if tx.send(sse_event).await.is_err() {
            break;
        }
    }

    let _ = tx.send(StreamEvent::Ping).await;
    Ok(())
}


// ─── Memory (memfs HTTP write path) ───────────────────────────────────────
//
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

// ─── Phase 3 REST Handlers ────────────────────────────────────────────────

pub async fn get_config(
    State(server): State<Arc<SouveraineServer>>,
) -> Result<Json<crate::core::config::ConsciousnessConfig>, ApiError> {
    let config = server.app_config.read().await.clone();
    Ok(Json(config))
}

pub async fn update_config(
    State(server): State<Arc<SouveraineServer>>,
    Json(new_config): Json<crate::core::config::ConsciousnessConfig>,
) -> Result<Json<crate::core::config::ConsciousnessConfig>, ApiError> {
    // 1. Update active configuration in memory
    {
        let mut config = server.app_config.write().await;
        *config = new_config.clone();
    }

    // 2. Persist updated TOML configuration file to disk
    let config_path = crate::core::config::ConsciousnessConfig::discover_path()
        .unwrap_or_else(|| std::path::PathBuf::from("souveraine.toml"));

    if let Err(e) = new_config.save(&config_path) {
        tracing::warn!("Failed to persist config to disk at {:?}: {}", config_path, e);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "config_save_failed".to_string(),
            message: e.to_string(),
        })));
    } else {
        tracing::info!("Saved live settings to {:?}", config_path);
    }

    Ok(Json(new_config))
}

pub async fn get_compaction_logs(
    State(server): State<Arc<SouveraineServer>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
    let events_dir = base.join("events");

    let mut logs = Vec::new();

    // Scan date partitioned logs for the past 7 days
    let today = chrono::Utc::now().date_naive();
    for i in 0..7 {
        let date = today - chrono::Duration::days(i);
        let path = events_dir.join(format!("events-{}.jsonl", date.format("%Y-%m-%d")));
        if !path.exists() {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }

                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    let sensor = val.get("sensor_name").and_then(|s| s.as_str()).unwrap_or("");
                    let ev_type = val.get("event_type").and_then(|t| t.as_str()).unwrap_or("");
                    let content = val.get("payload").and_then(|p| p.get("content").and_then(|c| c.as_str())).unwrap_or("");

                    if sensor == "archivist" || 
                       ev_type.contains("compaction") || 
                       ev_type.contains("archive") ||
                       content.contains("compaction") {
                        logs.push(val);
                    }
                }
            }
        }
    }

    // Return newest events first
    logs.reverse();

    Ok(Json(serde_json::json!({ "logs": logs })))
}

pub async fn get_conversation_tokens(
    State(server): State<Arc<SouveraineServer>>,
    Path(conversation_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = server.sessions.get(&conversation_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "conversation_not_found".to_string(),
            message: format!("Conversation {} not found", conversation_id),
        })))?;

    let counter = crate::bridge::model_router::TokenCounter::new();

    let mut system_tokens = 0;
    let mut user_tokens = 0;
    let mut assistant_tokens = 0;

    for msg in &session.messages {
        let mut msg_tokens = 0;
        for block in &msg.blocks {
            let block_text = match block {
                crate::core::session::ContentBlock::Text { text } => text.clone(),
                crate::core::session::ContentBlock::ToolUse { id, name, input } => format!("{id} {name} {input}"),
                crate::core::session::ContentBlock::ToolResult { tool_use_id, tool_name, output, .. } => {
                    format!("{tool_use_id} {tool_name} {output}")
                }
                crate::core::session::ContentBlock::Reasoning { reasoning } => reasoning.clone(),
                crate::core::session::ContentBlock::Image { media_type, data } => format!("{media_type} {data}"),
            };
            msg_tokens += counter.count(&block_text);
        }

        match msg.role {
            crate::core::session::MessageRole::System => system_tokens += msg_tokens,
            crate::core::session::MessageRole::User => user_tokens += msg_tokens,
            crate::core::session::MessageRole::Assistant => assistant_tokens += msg_tokens,
            crate::core::session::MessageRole::Tool => user_tokens += msg_tokens, // Tool inputs/outputs consume context space
        }
    }

    let total_tokens = system_tokens + user_tokens + assistant_tokens;

    // Load agent to query their configured model's context limit
    let limit = if let Ok(agent) = server.agents.get(&session.agent_id).await {
        agent.llm_config.context_window as usize
    } else {
        128000
    };

    Ok(Json(serde_json::json!({
        "system_tokens": system_tokens,
        "user_tokens": user_tokens,
        "assistant_tokens": assistant_tokens,
        "total_tokens": total_tokens,
        "context_limit": limit,
        "percentage": if limit > 0 { (total_tokens as f32 / limit as f32).min(1.0) } else { 0.0 }
    })))
}

