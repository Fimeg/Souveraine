use crate::core::conversation::{ConversationRecord, ConversationStore};
use crate::core::session::ConversationMessage;
use crate::api::models::StreamEvent;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast::{self, Sender};
use uuid::Uuid;

pub struct SessionManager {
    sessions: DashMap<String, Session>,
    agent_conversations: DashMap<String, Vec<String>>,
    store: Option<Arc<ConversationStoreHandle>>,
}

struct ConversationStoreHandle {
    agents_dir: PathBuf,
}

impl ConversationStoreHandle {
    fn store_for(&self, agent_id: &str) -> ConversationStore {
        ConversationStore::new(&self.agents_dir.join(agent_id))
    }
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
            store: None,
        }
    }

    pub fn with_persistence(agents_dir: PathBuf) -> Self {
        Self {
            sessions: DashMap::new(),
            agent_conversations: DashMap::new(),
            store: Some(Arc::new(ConversationStoreHandle {
                agents_dir,
            })),
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

        if let Some(handle) = &self.store {
            let record = ConversationRecord::new(
                conversation_id.clone(),
                agent_id.to_string(),
            );
            let store = handle.store_for(agent_id);
            tokio::spawn(async move {
                if let Err(e) = store.save_metadata(&record).await {
                    tracing::warn!("Failed to persist conversation metadata: {}", e);
                }
            });
        }

        conversation_id
    }

    /// Create a conversation and load existing messages from an in-memory
    /// session that was previously active. Used for restoring from disk.
    pub fn create_with_messages(
        &self,
        agent_id: &str,
        conversation_id: String,
        messages: Vec<ConversationMessage>,
    ) -> String {
        let (sender, _receiver) = broadcast::channel(100);
        let turn_count = messages
            .iter()
            .filter(|m| m.role == crate::core::session::MessageRole::Assistant)
            .count() as u32;

        let session = Session {
            conversation_id: conversation_id.clone(),
            agent_id: agent_id.to_string(),
            messages,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count,
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

    pub fn get(&self, conversation_id: &str) -> Option<dashmap::mapref::one::Ref<'_, String, Session>> {
        self.sessions.get(conversation_id)
    }

    pub fn get_mut(&self, conversation_id: &str) -> Option<dashmap::mapref::one::RefMut<'_, String, Session>> {
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

        if let Some(handle) = &self.store {
            let agent_id = session.agent_id.clone();
            let conv_id = conversation_id.to_string();
            let messages = session.messages.clone();
            let msg_count = messages.len() as u32;
            let store = handle.store_for(&agent_id);
            tokio::spawn(async move {
                if let Err(e) = store.save_messages(&conv_id, &messages).await {
                    tracing::warn!("Failed to persist messages: {}", e);
                }
                if let Ok(Some(mut record)) = store.load_metadata(&conv_id).await {
                    record.message_count = msg_count;
                    record.updated_at = Utc::now();
                    record.last_message_at = Some(Utc::now());
                    let _ = store.save_metadata(&record).await;
                }
            });
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

    /// Load persisted conversations for an agent from disk into the
    /// session manager. Call once at startup per agent.
    pub async fn load_persisted(&self, agent_id: &str) -> anyhow::Result<Vec<ConversationRecord>> {
        let handle = match &self.store {
            Some(h) => h,
            None => return Ok(Vec::new()),
        };
        let store = handle.store_for(agent_id);
        let records = store.list_active().await?;

        for record in &records {
            if self.sessions.contains_key(&record.id) {
                continue;
            }
            let messages = store.load_messages(&record.id).await.unwrap_or_default();
            self.create_with_messages(agent_id, record.id.clone(), messages);
        }

        Ok(records)
    }

    pub fn conversation_store_for(&self, agent_id: &str) -> Option<ConversationStore> {
        self.store.as_ref().map(|h| h.store_for(agent_id))
    }

    /// Fork an existing conversation — clone all messages into a new session
    /// with a fresh conversation_id. Returns the new conversation_id.
    /// Used by `/btw` to spin off a side-quest conversation in parallel.
    pub fn fork(&self, conversation_id: &str) -> anyhow::Result<String> {
        let source = self
            .sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;

        let agent_id = source.agent_id.clone();
        let messages = source.messages.clone();
        drop(source); // release the DashMap ref

        let forked_id = Uuid::new_v4().to_string();
        let (sender, _receiver) = broadcast::channel(100);

        let session = Session {
            conversation_id: forked_id.clone(),
            agent_id: agent_id.clone(),
            messages,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            last_n25: Utc::now(),
            context_pressure: 0.0,
            event_sender: sender,
        };

        self.sessions.insert(forked_id.clone(), session);
        self.agent_conversations
            .entry(agent_id.clone())
            .or_insert_with(Vec::new)
            .push(forked_id.clone());

        if let Some(handle) = &self.store {
            let record = ConversationRecord::new(forked_id.clone(), agent_id.clone());
            let store = handle.store_for(&agent_id);
            tokio::spawn(async move {
                if let Err(e) = store.save_metadata(&record).await {
                    tracing::warn!("Failed to persist forked conversation metadata: {}", e);
                }
            });
        }

        Ok(forked_id)
    }
}
