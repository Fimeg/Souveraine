use crate::core::session::ConversationMessage;
use crate::api::models::StreamEvent;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast::{self, Sender};
use uuid::Uuid;

pub struct SessionManager {
    sessions: DashMap<String, Session>,
    agent_conversations: DashMap<String, Vec<String>>,
}

pub struct Session {
    pub conversation_id: String,
    pub agent_id: String,
    pub messages: Vec<ConversationMessage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub turn_count: u32,
    pub last_n25: DateTime<Utc>,
    pub context_pressure: f32,
    pub event_sender: Sender<StreamEvent>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            agent_conversations: DashMap::new(),
        }
    }

    pub fn create(&self, agent_id: &str) -> String {
        let conversation_id = Uuid::new_v4().to_string();
        let (sender, _receiver) = broadcast::channel(100);

        let session = Session {
            conversation_id: conversation_id.clone(),
            agent_id: agent_id.to_string(),
            messages: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            last_n25: Utc::now(),
            context_pressure: 0.0,
            event_sender: sender,
        };

        self.sessions.insert(conversation_id.clone(), session);
        self.agent_conversations
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new)
            .push(conversation_id.clone());

        conversation_id
    }

    pub fn get(&self, conversation_id: &str) -> Option<dashmap::mapref::one::Ref<String, Session>> {
        self.sessions.get(conversation_id)
    }

    pub fn get_mut(&self, conversation_id: &str) -> Option<dashmap::mapref::one::RefMut<String, Session>> {
        self.sessions.get_mut(conversation_id)
    }

    pub fn add_message(&self, conversation_id: &str, message: ConversationMessage) -> anyhow::Result<()> {
        let mut session = self.sessions
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;

        let is_assistant = message.role == crate::core::session::MessageRole::Assistant;
        session.messages.push(message);
        session.updated_at = Utc::now();

        if is_assistant {
            session.turn_count += 1;
        }

        Ok(())
    }

    pub fn subscribe(&self, conversation_id: &str) -> anyhow::Result<broadcast::Receiver<StreamEvent>> {
        let session = self.sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        Ok(session.event_sender.subscribe())
    }

    pub fn broadcast(&self, conversation_id: &str, event: StreamEvent) -> anyhow::Result<()> {
        let session = self.sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        let _ = session.event_sender.send(event);
        Ok(())
    }

    pub fn update_pressure(&self, conversation_id: &str, pressure: f32) -> anyhow::Result<()> {
        let mut session = self.sessions
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        session.context_pressure = pressure;
        Ok(())
    }

    pub fn list_for_agent(&self, agent_id: &str) -> Vec<String> {
        self.agent_conversations
            .get(agent_id)
            .map(|v| v.clone())
            .unwrap_or_default()
    }
}
