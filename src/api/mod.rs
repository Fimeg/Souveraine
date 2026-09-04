use crate::server::SouveraineServer;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

pub mod auth;
pub mod handlers;
pub mod models;

pub fn create_routes(state: Arc<SouveraineServer>) -> Router {
    // Public routes — no auth required (agent listing/creation, conversation listing/creation, health, firehose).
    let public_routes = Router::new()
        .route(
            "/v1/agents",
            get(handlers::list_agents).post(handlers::create_agent),
        )
        .route(
            "/v1/conversations",
            get(handlers::list_conversations).post(handlers::create_conversation),
        )
        .route("/v1/firehose", get(handlers::firehose))
        .route("/v1/federation/events", get(handlers::federation_events))
        .route(
            "/v1/config",
            get(handlers::get_config).post(handlers::update_config),
        )
        .route("/v1/compaction-logs", get(handlers::get_compaction_logs))
        .route(
            "/v1/conversations/:id/tokens",
            get(handlers::get_conversation_tokens),
        )
        .route("/health", get(health_check))
        .route("/v1/server/status", get(handlers::server_status))
        .route("/v1/server/restart", post(handlers::restart_server));

    // Protected agent routes — require per-agent bearer token.
    let agent_routes = Router::new()
        .route(
            "/v1/agents/:id",
            get(handlers::get_agent)
                .patch(handlers::update_agent)
                .delete(handlers::delete_agent),
        )
        .route(
            "/v1/agents/:id/itinerary",
            get(handlers::get_agent_itinerary),
        )
        .route(
            "/v1/agents/:id/principal",
            get(handlers::get_agent_principal),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_agent_token,
        ));

    // Protected conversation routes — require bearer token for the conversation's agent.
    let conversation_routes = Router::new()
        .route("/v1/conversations/:id", get(handlers::get_conversation))
        .route(
            "/v1/conversations/:id/messages",
            get(handlers::get_conversation_messages).post(handlers::stream_messages),
        )
        .route(
            "/v1/conversations/:id/events",
            get(handlers::stream_active_turn),
        )
        .route(
            "/v1/conversations/:id/fork",
            post(handlers::fork_conversation),
        )
        .route("/v1/conversations/:id/cancel", post(handlers::cancel_turn))
        .route("/v1/conversations/:id/interject", post(handlers::interject))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_conversation_token,
        ));

    // Memory routes — require per-agent bearer token.
    //
    // (memfs HTTP write path for cron-into-memfs and external integration.
    // See SouveraineOS/docs/substrate/MEMORY_BLOCKS_DECISION.md.)
    let memory_routes = Router::new()
        .route("/v1/agents/:id/memory", get(handlers::list_memory))
        .route(
            "/v1/agents/:id/memory/*path",
            get(handlers::read_memory)
                .put(handlers::write_memory)
                .patch(handlers::append_memory)
                .delete(handlers::delete_memory),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    // Web UI — served from web/dist/ directory
    // Falls back to index.html for SPA routing (React/Vue/etc)
    let serve_dir = ServeDir::new("web/dist").fallback(ServeFile::new("web/dist/index.html"));
    let web_routes = Router::new().route_service("/", serve_dir);

    Router::new()
        .merge(public_routes)
        .merge(agent_routes)
        .merge(conversation_routes)
        .merge(memory_routes)
        .merge(web_routes)
        .with_state(state)
}

async fn health_check() -> &'static str {
    "ok"
}
