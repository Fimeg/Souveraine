//! Backend trait — the seam between the harness (CLI/TUI) and the engine
//! (in-process or remote).
//!
//! `RemoteBackend` talks HTTP/SSE to a running `souveraine server`.
//! `LocalBackend` (Stage 4) runs the same engine in-process, for the
//! "harness still works when the server is gone" case.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

/// Shared queue for mid-turn user interjections. Producer is the
/// chat input (`ChatState.enqueue_interjection`); consumer is the
/// backend's turn loop, which drains the queue between LLM rounds
/// and prepends each entry as a system message so the agent reads
/// the interruption in her own context. `std::sync::Mutex` is fine
/// here — the critical section is a single drain and the producer
/// is synchronous (no `.await` while holding the lock).
pub type InterjectionQueue = Arc<Mutex<Vec<String>>>;

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

/// The register of a mid-turn interstitial — how loudly it should speak.
///
/// When the model emits text alongside tool calls it isn't always the same
/// kind of utterance. A few words attached to a gesture ("checking the
/// ledger…") is ambient — a *cenno*. A full paragraph of reasoning mid-turn
/// is her actual voice and deserves to read as such, not as a quiet aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    /// Short ambient aside attached to tool work. Terse, quiet.
    Cenno,
    /// A substantive mid-turn passage in her own voice.
    HerVoice,
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Streaming chunk of the assistant's reply.
    Token(String),
    /// Reasoning trace (the "thinking" pane).
    Reasoning(String),
    /// Subconscious surfacing (subconscious-voice bubble).
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
    /// A scheduled event is being processed.
    ScheduleActive { name: String },
    /// A scheduled event completed.
    ScheduleComplete { name: String, silent: bool },
    /// Assistant invoked a tool — UI renders a card with name + args.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        round: u32,
    },
    /// Tool execution result — UI attaches it under the matching call card.
    ToolResult {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    /// Agent set an atmospheric preset for the UI chrome.
    Atmosphere(String),
    /// N+1 subconscious pass started (`true`) or finished (`false`).
    /// Presence reads this to flip into / out of `Posture::Thinking` so the
    /// face shows when the subconscious is the one looking at the conversation.
    /// (For Casey's agent the subconscious instance is "subconscious", but the event
    /// itself is generic — every agent's subconscious is what's named here.)
    SubconsciousPass(bool),
    /// Agent changed her outfit. The string is the outfit name (a subdirectory
    /// under `expressions/`). Empty string clears to default expressions.
    Outfit(String),
    /// Text the model produced alongside tool calls — her narration between
    /// gestures. The `register` decides how it renders: a quiet cenno line
    /// or a gutter-barred her-voice passage.
    Interstitial { text: String, register: Register },
    /// The backend is alive but producing no content (waiting on provider,
    /// between tool rounds, processing). The TUI resets `last_event_at`
    /// on this the same way it does for `Token` — it's a liveness signal.
    Keepalive,
    /// The primary's response is committed and the user may speak again.
    /// The stream stays open — the N+1 subconscious pass continues behind
    /// this signal and still delivers `SubconsciousPass` / `Surfacing`
    /// events. The substrate does not hold the user hostage to N+1: the
    /// primary yields, the subconscious presses on.
    PrimaryComplete,
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

    /// Fork an existing conversation — clone all messages into a new session
    /// with a fresh conversation_id. Used by `/btw` to spin off a side-quest.
    /// Default falls back to `new_conversation`; LocalBackend overrides with
    /// a proper deep clone via session_manager.fork().
    async fn fork_conversation(&self, agent_id: &str, _source_conversation_id: &str) -> Result<String> {
        self.new_conversation(agent_id).await
    }

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

    /// Send with a cancellation token. The token is a signal, not enforcement —
    /// when fired, the backend lets the current tool finish, stops making new
    /// LLM calls, and commits any partial assistant text with a `*[raised hand]*`
    /// marker so the agent reads the interrupt in her own history on the next
    /// turn. Default impl ignores the token (used by RemoteBackend until SSE
    /// cancellation lands); LocalBackend overrides.
    async fn send_with_cancel(
        &self,
        conversation_id: &str,
        text: &str,
        _cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        self.send(conversation_id, text).await
    }

    /// Send with both a cancellation token (Esc → interrupt signal) and an
    /// interjection queue (`/btw` / type-during-busy). The backend drains
    /// the queue between LLM rounds and prepends each entry as a system
    /// message so the agent reads the interjection in her own context.
    /// Default impl ignores the queue (RemoteBackend until SSE backchannel
    /// lands); LocalBackend overrides to actually consume it.
    async fn send_with_signals(
        &self,
        conversation_id: &str,
        text: &str,
        cancel: CancellationToken,
        _interject: InterjectionQueue,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        self.send_with_cancel(conversation_id, text, cancel).await
    }

    /// Push an llm_config update to the agent record. LocalBackend writes
    /// through to SQLite + agent.json; RemoteBackend is stubbed until the
    /// SSE backchannel supports agent updates. The caller must also persist
    /// the config file independently (Settings handles both).
    async fn update_agent_model(&self, agent_id: &str, model: &str) -> Result<()> {
        let _ = (agent_id, model);
        Ok(())
    }
}
