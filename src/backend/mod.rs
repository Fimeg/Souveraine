//! Backend trait — the seam between the harness (CLI/TUI) and the engine
//! (in-process or remote).
//!
//! `RemoteBackend` talks HTTP/SSE to a running `souveraine server`.
//! `LocalBackend` (Stage 4) runs the same engine in-process, for the
//! "harness still works when the server is gone" case.

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;

pub mod local;
pub mod remote;

pub use local::LocalBackend;
pub use remote::RemoteBackend;

#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationInfo {
    pub id: String,
    pub agent_id: String,
    pub summary: Option<String>,
    pub message_count: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Streaming chunk of the assistant's reply.
    Token(String),
    /// Reasoning trace (the "thinking" pane).
    Reasoning(String),
    /// Subconscious surfacing (Aster-voice bubble).
    Surfacing {
        source: String,
        content: String,
        priority: String,
    },
    /// Reflection (N+25 witness).
    Reflection(String),
    /// Archivist event (N+100 synthesis).
    Archivist { synthesis: String, pressure: f32 },
    /// Compaction pressure warning (advisory only).
    CompactionWarning { pressure: f32, tier: u8 },
    /// Continuous context pressure update (sub-threshold).
    /// Fires every round so the TUI ctx counter reflects live state
    /// rather than only updating when a warning crosses a threshold.
    ContextPressure(f32),
    /// Inference strain — the voice is hoarse, providers are slow.
    /// Correlates to health over time.
    InferenceStrain {
        attempt: u32,
        status: u16,
        model: String,
    },
    /// Stream ended cleanly.
    Done,
}

#[async_trait]
pub trait Backend: Send + Sync {
    /// Cheap reachability probe. Used for auto-fallback between Remote/Local.
    async fn health(&self) -> bool;

    async fn list_agents(&self) -> Result<Vec<AgentInfo>>;

    /// Create or reuse a conversation for this agent. Returns a conversation ID.
    async fn ensure_conversation(&self, agent_id: &str) -> Result<String>;

    /// Create a new conversation for this agent. Always creates fresh.
    async fn new_conversation(&self, agent_id: &str) -> Result<String>;

    /// List persisted conversations for an agent (excludes archived).
    async fn list_conversations(&self, agent_id: &str) -> Result<Vec<ConversationInfo>>;

    /// Switch to an existing conversation, returning its messages for backfill.
    async fn load_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::core::session::ConversationMessage>>;

    /// Send a user message; receive a stream of incremental events.
    async fn send(
        &self,
        conversation_id: &str,
        text: &str,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>>;
}
