use crate::api::models::*;
use crate::server::SouveraineServer;

/// Resolve a session, hydrating from disk when a freshly booted server has
/// not loaded this conversation yet. Surfaces hold conversation ids, not a
/// map; a restart must answer a held id without a prior listing.
async fn session_or_hydrate<'a>(
    server: &'a SouveraineServer,
    id: &str,
) -> Result<dashmap::mapref::one::Ref<'a, String, crate::server::session_manager::Session>, ApiError>
{
    if let Some(session) = server.sessions.get(id) {
        return Ok(session);
    }
    server
        .sessions
        .hydrate_containing(id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "conversation_hydrate_failed".to_string(),
                    message: e.to_string(),
                }),
            )
        })?;
    server.sessions.get(id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "conversation_not_found".to_string(),
                message: format!("Conversation {} not found", id),
            }),
        )
    })
}
use axum::{
    body::Bytes,
    extract::{ws::WebSocket, Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{Json, Sse},
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
    let agents = server.agents.list(filter).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "list_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    Ok(Json(agents))
}

/// POST /v1/agents — create an agent and hand back its API token once.
///
/// This is the only time the token crosses the wire; afterwards it is a
/// filesystem read. See CRON_API_AUTH.md § "Token in agent creation response".
pub async fn create_agent(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<CreatedAgent>), ApiError> {
    let agent = server.agents.create(request).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "creation_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    // `create` already issued this; reading it back keeps one mint site.
    let api_token = crate::api::auth::ensure_token(server.agents.server_data_dir(), &agent.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "token_unavailable".to_string(),
                    message: e.to_string(),
                }),
            )
        })?;
    Ok((StatusCode::CREATED, Json(CreatedAgent { agent, api_token })))
}

pub async fn get_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<AgentState>, ApiError> {
    let agent =
        server
            .agents
            .get(&id)
            .await
            .map_err(|e| match e.to_string().contains("not found") {
                true => (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "agent_not_found".to_string(),
                        message: format!("Agent {} not found", id),
                    }),
                ),
                false => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "fetch_failed".to_string(),
                        message: e.to_string(),
                    }),
                ),
            })?;
    Ok(Json(agent))
}

/// GET /v1/agents/:id/principal — fresh process-credential posture.
///
/// This is inspection, not admission. It deliberately reports the current
/// monolithic server as acting-as-human for dedicated agents even when the
/// requested passwd entry exists, because no per-agent worker is carrying the
/// turn yet.
pub async fn get_agent_principal(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<crate::core::principal::PrincipalHealth>, ApiError> {
    let agent = server.agents.get(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "agent_not_found".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    Ok(Json(crate::core::principal::observe(
        &agent,
        "inspection",
    )))
}

/// GET /v1/agents/:id/itinerary — the agent's current route, projected from
/// its canonical memfs file. Absence is a valid empty state, not a 404: an
/// agent exists before it lays out a route and after it clears one.
pub async fn get_agent_itinerary(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<ItineraryView>, ApiError> {
    server.agents.get(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "agent_not_found".to_string(),
                message: e.to_string(),
            }),
        )
    })?;

    let dynamic = server
        .agents
        .memory_root(&id)
        .join("system")
        .join("dynamic");
    let itinerary = crate::core::tools::itinerary::load(&dynamic)
        .map(ItineraryView::from)
        .unwrap_or_default();
    Ok(Json(itinerary))
}

pub async fn update_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
    Json(updates): Json<UpdateAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.agents.update(&id, updates).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "update_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    Ok(Json(agent))
}

pub async fn delete_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    server.agents.delete(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "delete_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_conversations(
    State(server): State<Arc<SouveraineServer>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<Conversation>>, ApiError> {
    let conversations = if let Some(agent_id) = params.get("agent_id") {
        // Conversations are persisted per agent, but a freshly restarted
        // server has not populated its in-memory session index yet. Hydrate
        // before listing so every surface can derive resume state from the
        // server instead of carrying its own agent → conversation map.
        server
            .sessions
            .load_persisted(agent_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "conversation_load_failed".to_string(),
                        message: e.to_string(),
                    }),
                )
            })?;
        let session_ids = server.sessions.list_for_agent(agent_id);
        let mut conversations: Vec<_> = session_ids
            .into_iter()
            .filter_map(|id| {
                server.sessions.get(&id).map(|session| Conversation {
                    id,
                    agent_id: session.agent_id.clone(),
                    created_at: session.created_at,
                    updated_at: Some(session.updated_at),
                })
            })
            .collect();
        conversations.sort_by_key(|conversation| {
            std::cmp::Reverse(conversation.updated_at.unwrap_or(conversation.created_at))
        });
        conversations
    } else {
        Vec::new()
    };
    Ok(Json(conversations))
}

pub async fn create_conversation(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<(StatusCode, Json<Conversation>), ApiError> {
    let _ = server.agents.get(&request.agent_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "agent_not_found".to_string(),
                message: e.to_string(),
            }),
        )
    })?;

    let conversation_id = server.sessions.create(&request.agent_id);

    // Seed the full system prompt (constitution, base memories, skills) —
    // without this the agent boots amnesiac on shell-created conversations.
    server
        .seed_conversation_system_prompt(&request.agent_id, &conversation_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "system_prompt_seed_failed".to_string(),
                    message: e.to_string(),
                }),
            )
        })?;

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
    let session = session_or_hydrate(&server, &id).await?;

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
) -> Result<
    Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>,
    ApiError,
> {
    let agent_id = {
        let session = session_or_hydrate(&server, &conversation_id).await?;
        session.agent_id.clone()
    };

    // Attachments are gated on the agent's model, at the door. The alternative
    // — store it and let `run_turn` strip it to "[Image: attached by user]" —
    // leaves the human believing she saw something she never received. A
    // surface that cannot be answered is told so.
    if request.messages.iter().any(|m| m.content.has_images()) {
        let agent = server.agents.get(&agent_id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "agent_load_failed".to_string(),
                    message: e.to_string(),
                }),
            )
        })?;
        if !agent.llm_config.supports_images {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorResponse {
                    error: "image_not_supported".to_string(),
                    message: format!(
                        "model `{}` has no vision; the attachment was not stored",
                        agent.llm_config.model
                    ),
                }),
            ));
        }
    }

    // Convert every message before storing any of them — a rejected block in
    // the second message must not leave the first one half-committed.
    let mut conv_messages = Vec::with_capacity(request.messages.len());
    for msg in &request.messages {
        conv_messages.push(msg.to_conversation_message().map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "unsupported_content_block".to_string(),
                    message: format!(
                        "`{}` blocks are written by the engine, not accepted from a surface",
                        e.kind
                    ),
                }),
            )
        })?);
    }

    // A turn belongs to the conversation, not to the HTTP socket that
    // happened to start it. Claim it before changing history so a raced POST
    // cannot leave an extra user message behind.
    server.sessions.begin_turn(&conversation_id).map_err(|e| {
        (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "turn_already_active".to_string(),
                message: e.to_string(),
            }),
        )
    })?;

    // Ambient context first — the room she is being spoken to in. A system
    // note, same register as interjections, so it reads as perception rather
    // than instruction.
    if let Some(ambient) = request
        .ambient
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        let stamp = chrono::Local::now().format("%H:%M");
        let note = crate::core::session::ConversationMessage {
            role: crate::core::session::MessageRole::System,
            blocks: vec![crate::core::session::ContentBlock::Text {
                text: format!("[ambient at {stamp} — what the surface senses]\n{ambient}"),
            }],
            usage: None,
            timestamp: Some(chrono::Utc::now()),
        };
        if let Err(e) = server.sessions.add_message(&conversation_id, note) {
            let _ = server.sessions.finish_turn(&conversation_id);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "message_store_failed".to_string(),
                    message: e.to_string(),
                }),
            ));
        }
    }

    // Convert API messages to ConversationMessages and add to session
    for conv_msg in conv_messages {
        if let Err(e) = server.sessions.add_message(&conversation_id, conv_msg) {
            let _ = server.sessions.finish_turn(&conversation_id);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "message_store_failed".to_string(),
                    message: e.to_string(),
                }),
            ));
        }
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

    // Turn backchannel: fresh cancel token each turn, persistent
    // interjection queue. POST .../cancel and .../interject reach in here.
    let signals = server
        .turn_signals
        .entry(conversation_id.clone())
        .or_default();
    let cancel = CancellationToken::new();
    *signals.cancel.lock().unwrap() = cancel.clone();
    let interject = signals.interject.clone();
    drop(signals); // release the DashMap ref before the turn runs

    let (be_tx, mut be_rx) = mpsc::channel::<anyhow::Result<BackendEvent>>(64);
    let server_clone = server.clone();
    let conv = conversation_id.clone();

    let turn = tokio::spawn(async move {
        crate::server::turn::run_turn(server_clone, conv, &be_tx, event_bus, cancel, interject)
            .await
    });

    // Drain the turn's BackendEvent stream. run_turn already handles message
    // storage, surfacing injection, EventBus dispatch, N+1 pass. Every event
    // crosses the boundary via the exhaustive From impl — no silent skips;
    // the full personification channel (subconscious, interstitials,
    // atmosphere, strain, pressure) reaches every surface.
    let mut client_connected = true;
    while let Some(result) = be_rx.recv().await {
        let be = match result {
            Ok(be) => be,
            Err(e) => {
                eprintln!("Turn stream error ({conversation_id}): {e:#}");
                let event = StreamEvent::Error {
                    message: format!("{e:#}"),
                };
                let _ = server
                    .sessions
                    .publish_turn_event(&conversation_id, event.clone());
                if client_connected && tx.send(event).await.is_err() {
                    client_connected = false;
                }
                break;
            }
        };
        let event = StreamEvent::from(be);
        let _ = server
            .sessions
            .publish_turn_event(&conversation_id, event.clone());
        // Losing one surface must not stop draining the engine channel. If it
        // did, the next engine send would fail and turn a display reload into
        // a false user interrupt.
        if client_connected && tx.send(event).await.is_err() {
            client_connected = false;
        }
    }
    // The channel closing means run_turn returned; a turn that died before
    // emitting anything must still reach the surface as an error, not as a
    // silently-ended stream (an empty reply reads as the agent going mute).
    if let Ok(Err(e)) = turn.await {
        eprintln!("Turn failed ({conversation_id}): {e:#}");
        let event = StreamEvent::Error {
            message: format!("{e:#}"),
        };
        let _ = server
            .sessions
            .publish_turn_event(&conversation_id, event.clone());
        if client_connected {
            let _ = tx.send(event).await;
        }
    }
    let done = StreamEvent::Done;
    let _ = server
        .sessions
        .publish_turn_event(&conversation_id, done.clone());
    if client_connected {
        let _ = tx.send(done).await;
    }
    server.turn_signals.remove(&conversation_id);
    let _ = server.sessions.finish_turn(&conversation_id);
    Ok(())
}

/// GET /v1/conversations/:id/events — replay and follow the turn currently in
/// flight. This is deliberately separate from POST /messages: reconnecting a
/// surface observes the existing turn and can never accidentally start one.
pub async fn stream_active_turn(
    State(server): State<Arc<SouveraineServer>>,
    Path(conversation_id): Path<String>,
) -> Result<
    Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>,
    ApiError,
> {
    let Some((replay, mut live)) = server
        .sessions
        .subscribe_active_turn(&conversation_id)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "conversation_not_found".to_string(),
                    message: e.to_string(),
                }),
            )
        })?
    else {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "no_active_turn".to_string(),
                message: format!("Conversation {conversation_id} has no active turn"),
            }),
        ));
    };

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        for event in replay {
            let done = matches!(event, StreamEvent::Done);
            if tx.send(event).await.is_err() {
                return;
            }
            if done {
                return;
            }
        }
        loop {
            match live.recv().await {
                Ok(event) => {
                    let done = matches!(event, StreamEvent::Done);
                    if tx.send(event).await.is_err() || done {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let _ = tx
                        .send(StreamEvent::Error {
                            message: format!("active turn replay lagged by {n} events; reconnect"),
                        })
                        .await;
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok(axum::response::sse::Event::default()
            .event(event.message_type())
            .data(json))
    });
    Ok(Sse::new(stream))
}

/// GET /v1/conversations/:id/messages — full transcript backfill for resume.
/// Returns the session's `ConversationMessage`s verbatim; `RemoteBackend`
/// and any surface use this to restore a conversation after restart.
pub async fn get_conversation_messages(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::core::session::ConversationMessage>>, ApiError> {
    let session = session_or_hydrate(&server, &id).await?;
    Ok(Json(session.messages.clone()))
}

/// POST /v1/conversations/:id/fork — deep-clone into a fresh conversation.
/// The `/btw` side-quest verb, now reachable from any surface.
pub async fn fork_conversation(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Conversation>), ApiError> {
    let forked_id = server.sessions.fork(&id).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "fork_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    let session = server.sessions.get(&forked_id).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "fork_failed".to_string(),
                message: "forked session vanished".to_string(),
            }),
        )
    })?;
    Ok((
        StatusCode::CREATED,
        Json(Conversation {
            id: forked_id.clone(),
            agent_id: session.agent_id.clone(),
            created_at: session.created_at,
            updated_at: Some(session.updated_at),
        }),
    ))
}

/// POST /v1/conversations/:id/cancel — interrupt the turn in flight.
/// The token is a signal, not enforcement: the engine lets the current
/// tool finish, stops new LLM calls, and commits partial text with the
/// `*[raised hand]*` marker so she reads the interrupt in her own history.
pub async fn cancel_turn(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    match server.turn_signals.get(&id) {
        Some(signals) => {
            signals.cancel.lock().unwrap().cancel();
            Ok(StatusCode::ACCEPTED)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "no_active_turn".to_string(),
                message: format!("No turn signals for conversation {}", id),
            }),
        )),
    }
}

/// POST /v1/conversations/:id/interject — slip a note into the running turn.
/// Drained between LLM rounds and read as `[interjected at …]` system notes.
/// If she's idle, the note waits in the queue and lands at next turn start.
pub async fn interject(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
    Json(request): Json<InterjectRequest>,
) -> Result<StatusCode, ApiError> {
    if request.text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "empty_interjection".to_string(),
                message: "text must be non-empty".to_string(),
            }),
        ));
    }
    let signals = server.turn_signals.entry(id).or_default();
    signals.interject.lock().unwrap().push(request.text);
    Ok(StatusCode::ACCEPTED)
}

/// POST /v1/server/restart — an intentional restart: marker, `restarting`
/// event, then systemd takes the unit down. The next boot announces
/// `resumed` with the reason instead of a bare `started`. Responds before
/// the restart lands so the caller sees the confirmation.
pub async fn restart_server(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<RestartRequest>,
) -> Result<Json<RestartResponse>, ApiError> {
    let marker = crate::server::restart::RestartMarker {
        reason: request.reason,
        at: chrono::Utc::now(),
        by: request.by.unwrap_or_else(|| "api".to_string()),
    };
    crate::server::restart::write(&server.data_dir, &marker).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "restart_marker_failed".to_string(),
                message: e.to_string(),
            }),
        )
    })?;
    server.event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "server".to_string(),
        timestamp: marker.at,
        event_type: "restarting".to_string(),
        target: None,
        urgency: 1.0,
        payload: serde_json::to_value(&marker).ok(),
        seed_id: None,
        reply_to: None,
    });
    let at = marker.at;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "restart", "souveraine.service"])
            .spawn();
    });
    Ok(Json(RestartResponse { scheduled: true, at }))
}

/// GET /v1/server/status — how this process came up. `resumed` carries the
/// marker's reason; surfaces print it so a restart is announced, not hidden.
pub async fn server_status(
    State(server): State<Arc<SouveraineServer>>,
) -> Result<Json<crate::server::BootInfo>, ApiError> {
    Ok(Json(server.boot.clone()))
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
    (
        status,
        Json(ErrorResponse {
            error: kind.to_string(),
            message: e.to_string(),
        }),
    )
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
    let _ = server
        .agents
        .get(&agent_id)
        .await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    let entries = repo
        .list(q.prefix.as_deref())
        .await
        .map_err(|e| memory_err(StatusCode::INTERNAL_SERVER_ERROR, "list_failed", e))?;
    Ok(Json(serde_json::json!({ "entries": entries })))
}

pub async fn read_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _ = server
        .agents
        .get(&agent_id)
        .await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    let file = repo
        .read(&path)
        .await
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
    let _ = server
        .agents
        .get(&agent_id)
        .await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let body_str = std::str::from_utf8(&body)
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "invalid_utf8", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.write(&path, body_str)
        .await
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "write_failed", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn append_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let _ = server
        .agents
        .get(&agent_id)
        .await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let body_str = std::str::from_utf8(&body)
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "invalid_utf8", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.append(&path, body_str)
        .await
        .map_err(|e| memory_err(StatusCode::BAD_REQUEST, "append_failed", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_memory(
    State(server): State<Arc<SouveraineServer>>,
    Path((agent_id, path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let _ = server
        .agents
        .get(&agent_id)
        .await
        .map_err(|e| memory_err(StatusCode::NOT_FOUND, "agent_not_found", e))?;
    let repo = server.agents.memory_repo(&agent_id);
    repo.delete(&path)
        .await
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

async fn firehose_stream(server: Arc<SouveraineServer>, mut socket: WebSocket) {
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
                if socket.send(Message::Text(json)).await.is_err() {
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
                let trusted = {
                    let config = server.app_config.read().await;
                    crate::server::federation::types::signer_is_trusted(
                        &config.federation.peers,
                        &signed.signer_pubkey_hex,
                    )
                };
                if !trusted {
                    tracing::warn!(
                        signer = %signed.signer_pubkey_hex,
                        "federation: event from unconfigured peer dropped"
                    );
                    continue;
                }
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
        tracing::warn!(
            "Failed to persist config to disk at {:?}: {}",
            config_path,
            e
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "config_save_failed".to_string(),
                message: e.to_string(),
            }),
        ));
    } else {
        tracing::info!("Saved live settings to {:?}", config_path);
    }

    Ok(Json(new_config))
}

pub async fn get_compaction_logs(
    State(_server): State<Arc<SouveraineServer>>,
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
                    let sensor = val
                        .get("sensor_name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let ev_type = val.get("event_type").and_then(|t| t.as_str()).unwrap_or("");
                    let content = val
                        .get("payload")
                        .and_then(|p| p.get("content").and_then(|c| c.as_str()))
                        .unwrap_or("");

                    if sensor == "archivist"
                        || ev_type.contains("compaction")
                        || ev_type.contains("archive")
                        || content.contains("compaction")
                    {
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
    let session = session_or_hydrate(&server, &conversation_id).await?;

    let counter = crate::bridge::model_router::TokenCounter::new();

    let mut system_tokens = 0;
    let mut user_tokens = 0;
    let mut assistant_tokens = 0;

    for msg in &session.messages {
        let mut msg_tokens = 0;
        for block in &msg.blocks {
            // `countable_text` is the shared authority — this used to be an
            // inline copy of the same match, which is how the compaction
            // engine's copy was able to drift into counting Text only.
            msg_tokens += counter.count(&block.countable_text());
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
