//! Conversation Events — streamed during turn execution
//!
//! These events map to SDK message types for OSS UI compatibility.

use tokio::sync::mpsc;

/// Events emitted during a conversation turn
#[derive(Clone, Debug)]
pub enum ConversationEvent {
    /// System prompt loaded from persona
    SystemPrompt { content: String },
    /// Assistant message content (streaming)
    AssistantMessage { content: String },
    /// Reasoning/thought from the model
    Reasoning { content: String },
    /// Tool call requested by assistant
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// Tool execution result
    ToolResult {
        tool_call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    /// N+1 subconscious surfacing
    Surfacing {
        source: &'static str,
        content: String,
        priority: &'static str,
    },
    /// N+25 reflection triggered
    Reflection { content: String },
    /// N+100 archivist synthesis
    Archivist { synthesis: String, pressure: f32 },
    /// Turn completed
    TurnComplete,
    /// Error during turn
    Error { message: String },
}

impl ConversationEvent {
    /// Get the event type name for SSE
    pub fn event_type(&self) -> &'static str {
        match self {
            ConversationEvent::SystemPrompt { .. } => "system_message",
            ConversationEvent::AssistantMessage { .. } => "assistant_message",
            ConversationEvent::Reasoning { .. } => "reasoning_message",
            ConversationEvent::ToolCall { .. } => "tool_call_message",
            ConversationEvent::ToolResult { .. } => "tool_return_message",
            ConversationEvent::Surfacing { .. } => "souveraine_surfacing",
            ConversationEvent::Reflection { .. } => "souveraine_reflection",
            ConversationEvent::Archivist { .. } => "souveraine_archivist",
            ConversationEvent::TurnComplete => "turn_complete",
            ConversationEvent::Error { .. } => "error",
        }
    }
}

/// Optional event sender handle
#[derive(Clone)]
pub struct EventSender {
    pub(crate) tx: mpsc::Sender<ConversationEvent>,
}

impl EventSender {
    pub fn new(tx: mpsc::Sender<ConversationEvent>) -> Self {
        Self { tx }
    }

    /// Emit an event if sender exists
    pub async fn emit(&self, event: ConversationEvent) {
        let _ = self.tx.send(event).await;
    }

    /// Emit immediately (non-async)
    pub fn try_emit(&self, event: ConversationEvent) {
        let tx = self.tx.clone();
        let _ = tokio::spawn(async move {
            let _ = tx.send(event).await;
        });
    }
}

impl From<mpsc::Sender<ConversationEvent>> for EventSender {
    fn from(tx: mpsc::Sender<ConversationEvent>) -> Self {
        Self::new(tx)
    }
}
