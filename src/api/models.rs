use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub llm_config: LlmConfig,
    pub memory: MemoryConfig,
    pub memory_blocks: Vec<MemoryBlock>,
    pub tools: Vec<String>,
    pub tags: Vec<String>,
    #[serde(rename = "_souveraine")]
    pub souveraine: SouveraineConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub model: String,
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum tool-calling rounds before forcing a text response.
    /// Configurable per-agent; 0 disables tools entirely.
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    /// Milliseconds to wait between tool rounds to avoid rate-limit cascades.
    /// The follow-up LLM call after a tool executes can trigger rate limits
    /// if it arrives too quickly. Default 500ms.
    #[serde(default = "default_inter_round_delay")]
    pub inter_round_delay_ms: u64,
}

fn default_context_window() -> u32 {
    128000
}

fn default_max_tool_rounds() -> u32 {
    10
}

fn default_inter_round_delay() -> u64 {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub git_enabled: bool,
    pub auto_commit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SouveraineConfig {
    pub n1_enabled: bool,
    pub reflection_enabled: bool,
    pub archivist_enabled: bool,
    pub archivist_threshold: f32,
    pub sensorium_bandwidth: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub llm_config: LlmConfig,
    #[serde(default)]
    pub memory_blocks: Vec<MemoryBlock>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub llm_config: Option<LlmConfig>,
    #[serde(default)]
    pub memory_blocks: Option<Vec<MemoryBlock>>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AgentFilters {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub agent_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Convert API Message to internal ConversationMessage
    pub fn to_conversation_message(&self) -> crate::core::session::ConversationMessage {
        use crate::core::session::{ConversationMessage, MessageRole, ContentBlock};

        let role = match self.role.as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            _ => MessageRole::User,
        };

        ConversationMessage {
            role,
            blocks: vec![ContentBlock::Text { text: self.content.clone() }],
            usage: None,
            timestamp: Some(chrono::Utc::now()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "message_type")]
pub enum StreamEvent {
    #[serde(rename = "assistant_message")]
    AssistantMessage { content: String },
    #[serde(rename = "reasoning_message")]
    ReasoningMessage { content: String },
    #[serde(rename = "tool_call_message")]
    ToolCallMessage { tool_call: ToolCall },
    #[serde(rename = "tool_return_message")]
    ToolReturnMessage { tool_return: ToolReturn },
    #[serde(rename = "souveraine_surfacing")]
    Surfacing { source: String, content: String, priority: String },
    #[serde(rename = "souveraine_reflection")]
    Reflection { content: String },
    #[serde(rename = "souveraine_archivist")]
    Archivist { synthesis: String, pressure: f32 },
    #[serde(rename = "ping")]
    Ping,
}

impl StreamEvent {
    pub fn message_type(&self) -> &'static str {
        match self {
            StreamEvent::AssistantMessage { .. } => "message",
            StreamEvent::ReasoningMessage { .. } => "reasoning",
            StreamEvent::ToolCallMessage { .. } => "tool_call",
            StreamEvent::ToolReturnMessage { .. } => "tool_return",
            StreamEvent::Surfacing { .. } => "souveraine_surfacing",
            StreamEvent::Reflection { .. } => "souveraine_reflection",
            StreamEvent::Archivist { .. } => "souveraine_archivist",
            StreamEvent::Ping => "ping",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolReturn {
    pub status: String,
    pub output: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}
