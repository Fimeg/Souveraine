//! Session — conversation message types and persistence
//!
//! Defines the message model shared across all Souveraine interfaces:
//! TUI, CLI, Web API, and subagent forks.
//!
//! Uses serde for serialization (unlike claw-code's custom JSON).
//! Persists sessions as JSON files in the agent's memory directory.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;

/// Role of a message participant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A single content block within a message
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: String },
    ToolResult { tool_use_id: String, tool_name: String, output: String, is_error: bool },
    Reasoning { reasoning: String },
}

/// Token usage metadata
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// A single message in a conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

impl ConversationMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn assistant_with_usage(blocks: Vec<ContentBlock>, usage: Option<TokenUsage>) -> Self {
        Self {
            role: MessageRole::Assistant,
            blocks,
            usage,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, tool_name: impl Into<String>, output: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: MessageRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                output: output.into(),
                is_error,
            }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }
}

/// A complete conversation session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub messages: Vec<ConversationMessage>,
    pub agent_name: String,
    pub conversation_id: String,
}

impl Session {
    pub fn new(agent_name: &str) -> Self {
        let conversation_id = uuid::Uuid::new_v4().to_string();
        Self {
            version: 1,
            messages: Vec::new(),
            agent_name: agent_name.to_string(),
            conversation_id,
        }
    }

    pub fn with_id(agent_name: &str, conversation_id: &str) -> Self {
        Self {
            version: 1,
            messages: Vec::new(),
            agent_name: agent_name.to_string(),
            conversation_id: conversation_id.to_string(),
        }
    }

    pub fn add_message(&mut self, message: ConversationMessage) {
        self.messages.push(message);
    }

    /// Serialize to pretty JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Estimate token count (chars/4 heuristic, use for pre-tiktoken estimation)
    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(|m| {
            m.blocks.iter().map(|b| match b {
                ContentBlock::Text { text } => text.len() / 4 + 1,
                ContentBlock::ToolUse { name, input, .. } => (name.len() + input.len()) / 4 + 1,
                ContentBlock::ToolResult { tool_name, output, .. } => (tool_name.len() + output.len()) / 4 + 1,
                ContentBlock::Reasoning { reasoning } => reasoning.len() / 4 + 1,
            }).sum::<usize>()
        }).sum()
    }

    /// Save session to a directory
    pub async fn save_to_dir(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join(format!("{}.json", self.conversation_id));
        let json = self.to_json()?;
        tokio::fs::write(&path, json).await?;
        debug!("Session saved: {}", path.display());
        Ok(())
    }

    /// Load session from a directory by conversation ID
    pub async fn load_from_dir(dir: &std::path::Path, conversation_id: &str) -> anyhow::Result<Option<Self>> {
        let path = dir.join(format!("{conversation_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&path).await?;
        let session = Self::from_json(&json)?;
        Ok(Some(session))
    }

    /// Convert to Bifrost API message format (list of {role, content} maps)
    pub fn to_bifrost_messages(&self) -> Vec<crate::bridge::bifrost::Message> {
        let mut messages = Vec::new();
        for msg in &self.messages {
            for block in &msg.blocks {
                match block {
                    ContentBlock::Text { text } => {
                        messages.push(crate::bridge::bifrost::Message {
                            role: match msg.role {
                                MessageRole::System => "system".to_string(),
                                MessageRole::User => "user".to_string(),
                                MessageRole::Assistant => "assistant".to_string(),
                                MessageRole::Tool => "tool".to_string(),
                            },
                            content: text.clone(),
                        });
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        // Tool calls are encoded as assistant messages with tool content
                        messages.push(crate::bridge::bifrost::Message {
                            role: "assistant".to_string(),
                            content: format!("Tool use: {name}({input})"),
                        });
                    }
                    ContentBlock::ToolResult { tool_name, output, is_error, .. } => {
                        messages.push(crate::bridge::bifrost::Message {
                            role: "tool".to_string(),
                            content: if *is_error {
                                format!("Error ({tool_name}): {output}")
                            } else {
                                format!("Result ({tool_name}): {output}")
                            },
                        });
                    }
                    ContentBlock::Reasoning { reasoning } => {
                        messages.push(crate::bridge::bifrost::Message {
                            role: "assistant".to_string(),
                            content: format!("[Reasoning]: {reasoning}"),
                        });
                    }
                }
            }
        }
        messages
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new("system")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_create_and_serialize() {
        let mut session = Session::new("ani");
        session.add_message(ConversationMessage::user_text("hello"));
        session.add_message(ConversationMessage::assistant_text("hi there"));
        let json = session.to_json().unwrap();
        let restored: Session = Session::from_json(&json).unwrap();
        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.agent_name, "ani");
    }

    #[test]
    fn test_bifrost_conversion() {
        let mut session = Session::new("ani");
        session.add_message(ConversationMessage::user_text("hello"));
        let msgs = session.to_bifrost_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
    }
}
