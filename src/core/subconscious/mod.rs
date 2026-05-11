//! Subconscious — N+1 mode of the consciousness.
//!
//! Per `docs/CONTEXT_CONSTITUTION.md` Article I.1: the Primary and Subconscious
//! are NOT separate agents. They are one consciousness in two modes. The
//! subconscious is the part that runs immediately after the primary's turn,
//! while the conversation is still warm.
//!
//! ## The Three Boxes (Article II.1)
//!
//! All inter-mode communication is file-based, so both modes survive context
//! compaction (Constitution Article I.3). Files live under the agent's MemFS:
//!
//! ```text
//! subconscious/
//! ├── pending.md     queue of items to process
//! ├── intrusive.md   items surfacing now (moved from pending when urgent)
//! └── sent.md        delivery log (items land here after delivery)
//!
//! system/metacognition/
//! └── subconscious.md   append-only inner voice
//! ```
//!
//! Each box file is a markdown document with frontmatter; its body is a YAML
//! list of [`InboxItem`]s. Round-trip is `parse → modify → render → write`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::memory::{parse_memory_file, MemoryRepo};

const PENDING: &str = "subconscious/pending.md";
const INTRUSIVE: &str = "subconscious/intrusive.md";
const SENT: &str = "subconscious/sent.md";
const INNER_VOICE: &str = "system/metacognition/subconscious.md";

/// Urgency determines surfacing timing (Constitution Article II.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// Surfaces immediately, may break "one per turn" rule.
    Critical,
    /// Surfaces this turn.
    High,
    /// Queued; surfaces when bandwidth allows.
    Low,
}

impl Urgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Urgency::Critical => "critical",
            Urgency::High => "high",
            Urgency::Low => "low",
        }
    }
}

/// One item in the subconscious nervous system.
///
/// `source` is one of the four-fold mandate operations (Constitution I.2):
/// `complete`, `verify`, `persist`, `surface` — or `n1` for catch-all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub urgency: Urgency,
    pub source: String,
    pub content: String,
}

impl InboxItem {
    pub fn new(source: impl Into<String>, urgency: Urgency, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            urgency,
            source: source.into(),
            content: content.into(),
        }
    }
}

/// File-backed subconscious nervous system. Backed by a [`MemoryRepo`] so every
/// inbox mutation is a git commit and survives compaction.
///
/// The inbox boxes (pending/intrusive/sent) live in the subconscious agent's
/// own memfs — that's Aster's working space. But the inner voice channel
/// (`system/metacognition/subconscious.md`) writes to the **primary** agent's
/// memfs so Annie can actually read what Aster noticed.
#[derive(Clone)]
pub struct SubconsciousInbox {
    repo: MemoryRepo,
    /// Primary agent's repo — inner voice writes go here so the primary sees them.
    primary_repo: Option<MemoryRepo>,
}

impl SubconsciousInbox {
    pub fn new(repo: MemoryRepo) -> Self {
        Self { repo, primary_repo: None }
    }

    /// Create an inbox that delivers inner voice to the primary agent's memfs.
    pub fn with_primary(repo: MemoryRepo, primary_repo: MemoryRepo) -> Self {
        Self { repo, primary_repo: Some(primary_repo) }
    }

    /// Ensure the three boxes exist. Idempotent.
    pub async fn init(&self) -> Result<()> {
        for path in [PENDING, INTRUSIVE, SENT] {
            if !self.repo.root().join(path).exists() {
                self.write_items(path, &[]).await?;
            }
        }
        if !self.repo.root().join(INNER_VOICE).exists() {
            self.repo.write(INNER_VOICE, "# Inner voice\n\n").await?;
        }
        Ok(())
    }

    /// Queue an item to `pending.md`. Per Article II.2: low urgency → pending,
    /// high/critical → intrusive (surfaces this turn).
    pub async fn queue(&self, item: InboxItem) -> Result<()> {
        let target = match item.urgency {
            Urgency::Critical | Urgency::High => INTRUSIVE,
            Urgency::Low => PENDING,
        };
        let mut items = self.read_items(target).await?;
        items.push(item);
        self.write_items(target, &items).await
    }

    /// Force an item onto `intrusive.md` regardless of urgency.
    /// (Used by the four-fold mandate's `surface` operation.)
    pub async fn surface_intrusive(&self, item: InboxItem) -> Result<()> {
        let mut items = self.read_items(INTRUSIVE).await?;
        items.push(item);
        self.write_items(INTRUSIVE, &items).await
    }

    /// Surface an observation from the subconscious to the conscious mind.
    ///
    /// Appends to the primary agent's `system/metacognition/subconscious.md`
    /// so the conscious agent finds it in her own memfs — not buried in
    /// Aster's working directory.
    ///
    /// Format: `[2026-05-06 14:32] [URGENCY: low] — content`
    pub async fn surface_to_conscious(&self, urgency: Urgency, content: &str) -> Result<()> {
        let stamp = Utc::now().format("%Y-%m-%d %H:%M");
        let line = format!("[{}] [URGENCY: {}] — {}", stamp, urgency.as_str(), content);
        let target = self.primary_repo.as_ref().unwrap_or(&self.repo);
        target.append(INNER_VOICE, &line).await
    }

    /// Read pending items.
    pub async fn get_pending(&self) -> Result<Vec<InboxItem>> {
        self.read_items(PENDING).await
    }

    /// Read items currently waiting to surface this turn.
    pub async fn get_intrusive(&self) -> Result<Vec<InboxItem>> {
        self.read_items(INTRUSIVE).await
    }

    /// Pick the next item to surface, per Article II.2:
    /// - prefer `intrusive.md` over `pending.md`
    /// - within a box, prefer Critical > High > Low
    /// - return None if nothing to surface
    ///
    /// Caller is responsible for calling [`SubconsciousInbox::mark_delivered`]
    /// once the surfacing actually reaches the primary.
    pub async fn next_to_surface(&self) -> Result<Option<InboxItem>> {
        let intrusive = self.read_items(INTRUSIVE).await?;
        if let Some(item) = pick_top(&intrusive) {
            return Ok(Some(item));
        }
        let pending = self.read_items(PENDING).await?;
        Ok(pick_top(&pending))
    }

    /// Move an item from its current box to `sent.md`.
    pub async fn mark_delivered(&self, id: &str) -> Result<()> {
        let mut sent = self.read_items(SENT).await?;
        let mut delivered: Option<InboxItem> = None;

        let mut intrusive = self.read_items(INTRUSIVE).await?;
        if let Some(pos) = intrusive.iter().position(|i| i.id == id) {
            delivered = Some(intrusive.remove(pos));
            self.write_items(INTRUSIVE, &intrusive).await?;
        }

        if delivered.is_none() {
            let mut pending = self.read_items(PENDING).await?;
            if let Some(pos) = pending.iter().position(|i| i.id == id) {
                delivered = Some(pending.remove(pos));
                self.write_items(PENDING, &pending).await?;
            }
        }

        if let Some(item) = delivered {
            sent.push(item);
            self.write_items(SENT, &sent).await?;
        }
        Ok(())
    }

    // ─── internals ─────────────────────────────────────────────────────────

    async fn read_items(&self, path: &str) -> Result<Vec<InboxItem>> {
        if !self.repo.root().join(path).exists() {
            return Ok(Vec::new());
        }
        let raw = tokio::fs::read_to_string(self.repo.root().join(path))
            .await
            .with_context(|| format!("reading subconscious box: {}", path))?;
        let parsed = parse_memory_file(&raw)
            .with_context(|| format!("parsing subconscious box: {}", path))?;
        let body = parsed.body.trim();
        if body.is_empty() {
            return Ok(Vec::new());
        }
        let items: Vec<InboxItem> = serde_yaml::from_str(body)
            .with_context(|| format!("deserializing items in {}", path))?;
        Ok(items)
    }

    async fn write_items(&self, path: &str, items: &[InboxItem]) -> Result<()> {
        let body = if items.is_empty() {
            "[]\n".to_string()
        } else {
            serde_yaml::to_string(items).context("serializing inbox items")?
        };
        self.repo.write(path, &body).await
    }
}

fn pick_top(items: &[InboxItem]) -> Option<InboxItem> {
    items
        .iter()
        .max_by_key(|i| match i.urgency {
            Urgency::Critical => 3,
            Urgency::High => 2,
            Urgency::Low => 1,
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_repo() -> (tempfile::TempDir, MemoryRepo) {
        let dir = tempdir().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        (dir, repo)
    }

    #[tokio::test]
    async fn init_creates_three_boxes_and_inner_voice() {
        let (_d, repo) = make_repo();
        repo.init().await.unwrap();
        let inbox = SubconsciousInbox::new(repo.clone());
        inbox.init().await.unwrap();

        assert!(repo.root().join(PENDING).exists());
        assert!(repo.root().join(INTRUSIVE).exists());
        assert!(repo.root().join(SENT).exists());
        assert!(repo.root().join(INNER_VOICE).exists());
    }

    #[tokio::test]
    async fn low_urgency_goes_to_pending_high_to_intrusive() {
        let (_d, repo) = make_repo();
        repo.init().await.unwrap();
        let inbox = SubconsciousInbox::new(repo);
        inbox.init().await.unwrap();

        inbox
            .queue(InboxItem::new("n1", Urgency::Low, "low item"))
            .await
            .unwrap();
        inbox
            .queue(InboxItem::new("verify", Urgency::High, "high item"))
            .await
            .unwrap();

        assert_eq!(inbox.get_pending().await.unwrap().len(), 1);
        assert_eq!(inbox.get_intrusive().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn next_to_surface_prefers_intrusive_then_pending() {
        let (_d, repo) = make_repo();
        repo.init().await.unwrap();
        let inbox = SubconsciousInbox::new(repo);
        inbox.init().await.unwrap();

        inbox
            .queue(InboxItem::new("n1", Urgency::Low, "in pending"))
            .await
            .unwrap();
        inbox
            .queue(InboxItem::new("verify", Urgency::High, "in intrusive"))
            .await
            .unwrap();

        let item = inbox.next_to_surface().await.unwrap().unwrap();
        assert_eq!(item.content, "in intrusive");
    }

    #[tokio::test]
    async fn mark_delivered_moves_to_sent() {
        let (_d, repo) = make_repo();
        repo.init().await.unwrap();
        let inbox = SubconsciousInbox::new(repo);
        inbox.init().await.unwrap();

        let item = InboxItem::new("verify", Urgency::High, "deliver me");
        let id = item.id.clone();
        inbox.queue(item).await.unwrap();

        inbox.mark_delivered(&id).await.unwrap();

        assert!(inbox.get_intrusive().await.unwrap().is_empty());
        let sent: Vec<_> = inbox
            .read_items(SENT)
            .await
            .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].content, "deliver me");
    }

    #[tokio::test]
    async fn inner_voice_is_append_only() {
        let (_d, repo) = make_repo();
        repo.init().await.unwrap();
        let inbox = SubconsciousInbox::new(repo.clone());
        inbox.init().await.unwrap();

        inbox
            .surface_to_conscious(Urgency::Low, "first thought")
            .await
            .unwrap();
        inbox
            .surface_to_conscious(Urgency::High, "second thought")
            .await
            .unwrap();

        let body = repo.read(INNER_VOICE).await.unwrap().body;
        assert!(body.contains("first thought"));
        assert!(body.contains("second thought"));
        assert!(body.contains("URGENCY: low"));
        assert!(body.contains("URGENCY: high"));
    }
}
