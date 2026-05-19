//! Server Conversation Handler
//!
//! Simplified conversation flow for server mode:
//! - Uses Bifrost directly (no git-backed components)
//! - Persists to SQLite
//! - Can be enhanced later with full tool-calling

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, ContentPart, ImageUrlSource, Message as BifrostMessage};
use crate::core::session::{ContentBlock, ConversationMessage, Session};

pub struct ServerConversation {
    pub session: Session,
    bifrost: BifrostClient,
    model: String,
}

pub struct ServerTurnResult {
    pub response_text: String,
}

impl ServerConversation {
    pub fn new(agent_name: &str, bifrost: BifrostClient, model: String) -> Self {
        Self {
            session: Session::new(agent_name),
            bifrost,
            model,
        }
    }

    /// Simple turn without full tool loop (for now)
    pub async fn turn(&mut self, user_input: &str) -> anyhow::Result<ServerTurnResult> {
        // Add user message
        self.session.add_message(ConversationMessage::user_text(user_input));

        // Build messages for Bifrost — flatten text blocks; ignore tool blocks
        // until the server tool loop lands.
        let messages: Vec<BifrostMessage> = self.session.messages.iter().map(|m| {
            let role = match m.role {
                crate::core::session::MessageRole::System => "system",
                crate::core::session::MessageRole::User => "user",
                crate::core::session::MessageRole::Assistant => "assistant",
                crate::core::session::MessageRole::Tool => "tool",
            };

            let has_images = m.blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. }));

            if has_images {
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

                BifrostMessage::multimodal_user(text_content, parts)
            } else {
                let content = m.blocks.iter().filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                }).collect::<Vec<_>>().join("\n");
                BifrostMessage::text(role, content)
            }
        }).collect();

        // Call Bifrost
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: Some(false),
            max_tokens: None,
            temperature: None,
            tools: None,
        };

        let response = self.bifrost.chat_completion(req).await?;
        let content = response.content.clone();

        // Store assistant response
        self.session.add_message(ConversationMessage::assistant_text(&content));

        Ok(ServerTurnResult {
            response_text: content,
        })
    }
}
