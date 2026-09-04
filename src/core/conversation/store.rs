#![allow(dead_code)] // WIP scaffolding not yet wired
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::core::session::ConversationMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub archived: bool,
}

impl ConversationRecord {
    pub fn new(id: String, agent_id: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            agent_id,
            summary: None,
            created_at: now,
            updated_at: now,
            last_message_at: None,
            message_count: 0,
            archived: false,
        }
    }
}

/// File-based conversation persistence.
///
/// Layout under the agent's data directory:
/// ```text
/// conversations/
///   {conv-id}/
///     conversation.json   # ConversationRecord metadata
///     messages.jsonl       # append-only message log
/// ```
///
/// JSON (not TOML) for metadata because ConversationMessage already
/// derives serde JSON and consistency with messages.jsonl matters
/// more than config-file aesthetics here.
pub struct ConversationStore {
    base_dir: PathBuf,
}

impl ConversationStore {
    pub fn new(agent_data_dir: &Path) -> Self {
        Self {
            base_dir: agent_data_dir.join("conversations"),
        }
    }

    fn conv_dir(&self, conversation_id: &str) -> PathBuf {
        self.base_dir.join(conversation_id)
    }

    fn metadata_path(&self, conversation_id: &str) -> PathBuf {
        self.conv_dir(conversation_id).join("conversation.json")
    }

    fn messages_path(&self, conversation_id: &str) -> PathBuf {
        self.conv_dir(conversation_id).join("messages.jsonl")
    }

    pub async fn save_metadata(&self, record: &ConversationRecord) -> Result<()> {
        let dir = self.conv_dir(&record.id);
        tokio::fs::create_dir_all(&dir).await?;
        let json = serde_json::to_string_pretty(record)?;
        tokio::fs::write(self.metadata_path(&record.id), json).await?;
        debug!("Conversation metadata saved: {}", record.id);
        Ok(())
    }

    pub async fn load_metadata(&self, conversation_id: &str) -> Result<Option<ConversationRecord>> {
        let path = self.metadata_path(conversation_id);
        if !path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&path).await?;
        let record: ConversationRecord =
            serde_json::from_str(&json).context("parsing conversation metadata")?;
        Ok(Some(record))
    }

    pub async fn save_messages(
        &self,
        conversation_id: &str,
        messages: &[ConversationMessage],
    ) -> Result<()> {
        let dir = self.conv_dir(conversation_id);
        tokio::fs::create_dir_all(&dir).await?;
        let mut lines = String::new();
        for msg in messages {
            let line = serde_json::to_string(msg)?;
            lines.push_str(&line);
            lines.push('\n');
        }
        tokio::fs::write(self.messages_path(conversation_id), lines).await?;
        debug!(
            "Conversation messages saved: {} ({} messages)",
            conversation_id,
            messages.len()
        );
        Ok(())
    }

    pub async fn load_messages(&self, conversation_id: &str) -> Result<Vec<ConversationMessage>> {
        let path = self.messages_path(conversation_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let mut messages = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let msg: ConversationMessage = serde_json::from_str(line)
                .with_context(|| format!("parsing message line {}", i + 1))?;
            messages.push(msg);
        }
        Ok(messages)
    }

    pub async fn list(&self) -> Result<Vec<ConversationRecord>> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.base_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let conv_id = entry.file_name().to_string_lossy().to_string();
            // One unreadable metadata file must not cost the agent every other
            // conversation. Two corrupt records (a stray trailing brace, May
            // 2026) aborted this whole listing, so hydration returned nothing
            // and the subconscious opened a fresh thread on every restart.
            // Skip the damaged record, keep the rest, and say so loudly.
            match self.load_metadata(&conv_id).await {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        "skipping conversation {} — unreadable metadata: {:#}",
                        conv_id,
                        e
                    );
                }
            }
        }
        records.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(records)
    }

    pub async fn list_active(&self) -> Result<Vec<ConversationRecord>> {
        let all = self.list().await?;
        Ok(all.into_iter().filter(|r| !r.archived).collect())
    }

    pub async fn delete(&self, conversation_id: &str) -> Result<()> {
        let dir = self.conv_dir(conversation_id);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_roundtrip_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path());
        let record = ConversationRecord::new("conv-1".into(), "agent-1".into());
        store.save_metadata(&record).await.unwrap();
        let loaded = store.load_metadata("conv-1").await.unwrap().unwrap();
        assert_eq!(loaded.id, "conv-1");
        assert_eq!(loaded.agent_id, "agent-1");
        assert!(loaded.summary.is_none());
    }

    #[tokio::test]
    async fn test_roundtrip_messages() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path());
        let messages = vec![
            ConversationMessage::user_text("hello"),
            ConversationMessage::assistant_text("hi there"),
        ];
        store.save_messages("conv-1", &messages).await.unwrap();
        let loaded = store.load_messages("conv-1").await.unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[tokio::test]
    async fn test_list_conversations() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path());
        let r1 = ConversationRecord::new("conv-1".into(), "agent-1".into());
        let r2 = ConversationRecord::new("conv-2".into(), "agent-1".into());
        store.save_metadata(&r1).await.unwrap();
        store.save_metadata(&r2).await.unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    /// A single damaged metadata file used to abort the whole listing, so
    /// `load_persisted` returned an error, the caller saw "no prior thread",
    /// and the subconscious opened a fresh conversation on every restart.
    /// The healthy records must survive their corrupt neighbour.
    #[tokio::test]
    async fn one_corrupt_metadata_file_does_not_hide_every_other_conversation() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path());
        let good = ConversationRecord::new("conv-good".into(), "agent-1".into());
        store.save_metadata(&good).await.unwrap();

        // Exactly the damage found on disk: a valid record with one stray
        // trailing brace, which serde rejects as trailing characters.
        // `ConversationStore::new` roots itself at <agent_data_dir>/conversations,
        // so the damaged record has to be planted inside that subdirectory.
        let bad_dir = tmp.path().join("conversations").join("conv-bad");
        tokio::fs::create_dir_all(&bad_dir).await.unwrap();
        let bad = ConversationRecord::new("conv-bad".into(), "agent-1".into());
        let mut json = serde_json::to_string_pretty(&bad).unwrap();
        json.push('}');
        tokio::fs::write(bad_dir.join("conversation.json"), json)
            .await
            .unwrap();

        assert!(store.load_metadata("conv-bad").await.is_err());

        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1, "the healthy record must still be listed");
        assert_eq!(list[0].id, "conv-good");
        assert_eq!(store.list_active().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_list_excludes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path());
        let r1 = ConversationRecord::new("conv-1".into(), "agent-1".into());
        let mut r2 = ConversationRecord::new("conv-2".into(), "agent-1".into());
        r2.archived = true;
        store.save_metadata(&r1).await.unwrap();
        store.save_metadata(&r2).await.unwrap();
        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "conv-1");
    }
}
