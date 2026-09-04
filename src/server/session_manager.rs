#![allow(dead_code)] // WIP scaffolding not yet wired
use crate::api::models::StreamEvent;
use crate::core::conversation::{ConversationRecord, ConversationStore};
use crate::core::session::ConversationMessage;
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
    /// Events belonging to the turn currently in flight. Unlike the broadcast
    /// channel, this survives a surface disconnect so a reattached panel can
    /// replay the turn from its beginning before following the live tail.
    pub active_turn_events: Vec<StreamEvent>,
    pub turn_active: bool,
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
            store: Some(Arc::new(ConversationStoreHandle { agents_dir })),
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
            active_turn_events: Vec::new(),
            turn_active: false,
        };

        self.sessions.insert(conversation_id.clone(), session);
        self.agent_conversations
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new)
            .push(conversation_id.clone());

        if let Some(handle) = &self.store {
            let record = ConversationRecord::new(conversation_id.clone(), agent_id.to_string());
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
        self.create_with_messages_and_timestamps(
            agent_id,
            conversation_id,
            messages,
            Utc::now(),
            Utc::now(),
        )
    }

    /// Like `create_with_messages` but preserves the persisted timestamps so
    /// the sort order in `list_conversations` reflects actual last activity,
    /// not the load time.
    pub fn create_with_messages_and_timestamps(
        &self,
        agent_id: &str,
        conversation_id: String,
        messages: Vec<ConversationMessage>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
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
            created_at,
            updated_at,
            turn_count,
            last_n25: Utc::now(),
            context_pressure: 0.0,
            event_sender: sender,
            active_turn_events: Vec::new(),
            turn_active: false,
        };

        self.sessions.insert(conversation_id.clone(), session);
        self.agent_conversations
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new)
            .push(conversation_id.clone());

        conversation_id
    }

    pub fn get(
        &self,
        conversation_id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, Session>> {
        self.sessions.get(conversation_id)
    }

    pub fn get_mut(
        &self,
        conversation_id: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, Session>> {
        self.sessions.get_mut(conversation_id)
    }

    pub fn add_message(
        &self,
        conversation_id: &str,
        message: ConversationMessage,
    ) -> anyhow::Result<()> {
        let mut session = self
            .sessions
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

    pub fn subscribe(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<broadcast::Receiver<StreamEvent>> {
        let session = self
            .sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        Ok(session.event_sender.subscribe())
    }

    pub fn broadcast(&self, conversation_id: &str, event: StreamEvent) -> anyhow::Result<()> {
        let session = self
            .sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        let _ = session.event_sender.send(event);
        Ok(())
    }

    /// Claim the conversation for one turn and reset its replay journal.
    pub fn begin_turn(&self, conversation_id: &str) -> anyhow::Result<()> {
        let mut session = self
            .sessions
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        if session.turn_active {
            anyhow::bail!("Conversation already has an active turn");
        }
        session.active_turn_events.clear();
        session.turn_active = true;
        Ok(())
    }

    /// Record an event for replay, then deliver it to every live follower.
    /// Consecutive token frames are coalesced in the journal only; subscribers
    /// still receive the original cadence in real time.
    pub fn publish_turn_event(
        &self,
        conversation_id: &str,
        event: StreamEvent,
    ) -> anyhow::Result<()> {
        let mut session = self
            .sessions
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        if session.turn_active {
            match (session.active_turn_events.last_mut(), &event) {
                (
                    Some(StreamEvent::AssistantMessage { content: prior }),
                    StreamEvent::AssistantMessage { content },
                ) => prior.push_str(content),
                (
                    Some(StreamEvent::SubconsciousToken { content: prior }),
                    StreamEvent::SubconsciousToken { content },
                ) => prior.push_str(content),
                _ => session.active_turn_events.push(event.clone()),
            }
        }
        let _ = session.event_sender.send(event);
        Ok(())
    }

    /// Atomically take a replay snapshot and subscribe to everything that
    /// follows it. Holding the session shard across both operations prevents
    /// the usual snapshot/subscribe race.
    pub fn subscribe_active_turn(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Option<(Vec<StreamEvent>, broadcast::Receiver<StreamEvent>)>> {
        let session = self
            .sessions
            .get(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        if !session.turn_active {
            return Ok(None);
        }
        let replay = session.active_turn_events.clone();
        let receiver = session.event_sender.subscribe();
        Ok(Some((replay, receiver)))
    }

    pub fn finish_turn(&self, conversation_id: &str) -> anyhow::Result<()> {
        let mut session = self
            .sessions
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        session.turn_active = false;
        session.active_turn_events.clear();
        Ok(())
    }

    pub fn update_pressure(&self, conversation_id: &str, pressure: f32) -> anyhow::Result<()> {
        let mut session = self
            .sessions
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

    /// The agent's most recently active conversation, by `updated_at`.
    ///
    /// `list_for_agent` returns *registration* order, which is not activity
    /// order. Hydration at startup pushes persisted conversations sorted
    /// newest-first, so `.last()` on that vec is the agent's *oldest*
    /// conversation. Anything that means "the conversation this agent is in
    /// right now" must ask here instead.
    pub fn latest_for_agent(&self, agent_id: &str) -> Option<String> {
        self.list_for_agent(agent_id)
            .into_iter()
            .filter_map(|id| {
                let updated = self.sessions.get(&id)?.updated_at;
                Some((updated, id))
            })
            .max_by_key(|(updated, _)| *updated)
            .map(|(_, id)| id)
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
            self.create_with_messages_and_timestamps(
                agent_id,
                record.id.clone(),
                messages,
                record.created_at,
                record.updated_at,
            );
        }

        Ok(records)
    }

    /// Resolve a conversation id the index does not know yet by hydrating
    /// the agent whose persisted conversations contain it. A restarted
    /// server must answer a held conversation id without the client having
    /// to list first. Returns true when the session is live afterwards.
    pub async fn hydrate_containing(&self, conversation_id: &str) -> anyhow::Result<bool> {
        if self.sessions.contains_key(conversation_id) {
            return Ok(true);
        }
        let Some(handle) = &self.store else {
            return Ok(false);
        };
        let mut entries = tokio::fs::read_dir(&handle.agents_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let agent_id = entry.file_name().to_string_lossy().into_owned();
            let conv_dir = handle
                .agents_dir
                .join(&agent_id)
                .join("conversations")
                .join(conversation_id);
            if tokio::fs::try_exists(&conv_dir).await.unwrap_or(false) {
                self.load_persisted(&agent_id).await?;
                return Ok(self.sessions.contains_key(conversation_id));
            }
        }
        Ok(false)
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
            active_turn_events: Vec::new(),
            turn_active: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn active_turn_can_be_replayed_and_followed_after_disconnect() {
        let sessions = SessionManager::new();
        let conversation_id = sessions.create("agent-replay");

        assert!(sessions
            .subscribe_active_turn(&conversation_id)
            .unwrap()
            .is_none());
        sessions.begin_turn(&conversation_id).unwrap();
        assert!(sessions.begin_turn(&conversation_id).is_err());

        sessions
            .publish_turn_event(
                &conversation_id,
                StreamEvent::AssistantMessage {
                    content: "still ".into(),
                },
            )
            .unwrap();
        sessions
            .publish_turn_event(
                &conversation_id,
                StreamEvent::AssistantMessage {
                    content: "working".into(),
                },
            )
            .unwrap();

        let (replay, mut live) = sessions
            .subscribe_active_turn(&conversation_id)
            .unwrap()
            .unwrap();
        assert!(matches!(
            replay.as_slice(),
            [StreamEvent::AssistantMessage { content }] if content == "still working"
        ));

        sessions
            .publish_turn_event(
                &conversation_id,
                StreamEvent::ToolCallMessage {
                    tool_call: crate::api::models::ToolCall {
                        id: "call-1".into(),
                        function: crate::api::models::ToolFunction {
                            name: "read".into(),
                            arguments: "{}".into(),
                        },
                    },
                    round: 1,
                },
            )
            .unwrap();
        assert!(matches!(
            live.recv().await.unwrap(),
            StreamEvent::ToolCallMessage { round: 1, .. }
        ));

        sessions.finish_turn(&conversation_id).unwrap();
        assert!(sessions
            .subscribe_active_turn(&conversation_id)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn list_hydration_restores_persisted_agent_conversations() {
        let temp = tempfile::tempdir().unwrap();
        let agent_id = "agent-phone";
        let conversation_id = "conversation-resume";
        let store = ConversationStore::new(&temp.path().join(agent_id));
        let record = ConversationRecord::new(conversation_id.into(), agent_id.into());
        store.save_metadata(&record).await.unwrap();
        store
            .save_messages(
                conversation_id,
                &[ConversationMessage::user_text("resume me")],
            )
            .await
            .unwrap();

        let sessions = SessionManager::with_persistence(temp.path().to_path_buf());
        let records = sessions.load_persisted(agent_id).await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(sessions.list_for_agent(agent_id), vec![conversation_id]);
        let session = sessions.get(conversation_id).unwrap();
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].role,
            crate::core::session::MessageRole::User
        );
        assert_eq!(
            session.messages[0].blocks,
            vec![crate::core::session::ContentBlock::Text {
                text: "resume me".to_string(),
            }]
        );
    }

    /// Registration order is not activity order. Hydration pushes persisted
    /// conversations newest-first, so the *last* registered is the oldest one.
    /// Compaction asking for "the current conversation" must not get that.
    #[tokio::test]
    async fn latest_for_agent_is_activity_order_not_registration_order() {
        let agent_id = "agent-drift";
        let sessions = SessionManager::new();

        // Registered in the order hydration would produce: newest first.
        let newest = Utc::now();
        let oldest = newest - chrono::Duration::days(90);
        sessions.create_with_messages_and_timestamps(
            agent_id,
            "conversation-live".into(),
            vec![ConversationMessage::user_text("this is the live thread")],
            newest,
            newest,
        );
        sessions.create_with_messages_and_timestamps(
            agent_id,
            "conversation-ancient".into(),
            vec![ConversationMessage::user_text(
                "a four-message stub from May",
            )],
            oldest,
            oldest,
        );

        // The naive read — and the bug it caused.
        assert_eq!(
            sessions.list_for_agent(agent_id).last().unwrap(),
            "conversation-ancient"
        );

        assert_eq!(
            sessions.latest_for_agent(agent_id).unwrap(),
            "conversation-live"
        );
    }
}
