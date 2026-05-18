//! Pending heartbeat surfacings — the stash between an autonomous cycle and
//! the next time a human opens a conversation.
//!
//! When the CronSensor fires a background turn, the subconscious runs her
//! N+1 pass and may surface an observation, a reflection, or an archivist
//! synthesis. Those events are emitted onto a stream that
//! `inject_background_turn` drains silently — no UI is listening. They also
//! broadcast on the EventBus, but a TUI that wasn't running never saw them.
//!
//! This module is the bridge: the background drain appends what surfaced to a
//! JSONL file *beside* the agent's memfs — not *inside* it. A pickup queue is
//! transient runtime state, not memory; writing it into the git-tracked memfs
//! would churn her history with ephemeral files. The next TUI/CLI session
//! reads the file, shows her what happened while she was away, and clears it.
//! Read-once: no cursor, no dedup bookkeeping.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One thing the subconscious surfaced during an autonomous cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSurfacing {
    /// `"surfacing"`, `"reflection"`, or `"archivist"`.
    pub kind: String,
    /// Surfacing source (`complete`/`verify`/`persist`/`surface`); empty for
    /// reflections and archivist syntheses.
    #[serde(default)]
    pub source: String,
    /// The observation / reflection / synthesis text.
    pub content: String,
    /// Surfacing priority (`low`/`high`/`critical`); empty for non-surfacings.
    #[serde(default)]
    pub priority: String,
    /// When the autonomous cycle produced it.
    pub at: chrono::DateTime<chrono::Utc>,
}

/// JSONL file path: `<agent_data_dir>/pending-surfacings.jsonl`.
fn path(agent_data_dir: &Path) -> PathBuf {
    agent_data_dir.join("pending-surfacings.jsonl")
}

/// Append surfacings produced by a background turn. One JSON object per line:
/// append-friendly across multiple heartbeats between sessions, and a single
/// corrupt line can't poison the rest.
pub async fn append(agent_data_dir: &Path, items: &[PendingSurfacing]) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    use tokio::io::AsyncWriteExt;

    let p = path(agent_data_dir);
    if let Some(parent) = p.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .await?;

    let mut buf = String::new();
    for item in items {
        buf.push_str(&serde_json::to_string(item)?);
        buf.push('\n');
    }
    file.write_all(buf.as_bytes()).await?;
    Ok(())
}

/// Read every surfacing stashed since the last pickup, then delete the file.
///
/// Corrupt lines are skipped, not fatal — a half-written line from a crash
/// mid-append loses that one entry, never the rest. Returns empty when no
/// autonomous cycle ran (the common case).
pub async fn take(agent_data_dir: &Path) -> Vec<PendingSurfacing> {
    let p = path(agent_data_dir);
    let raw = match tokio::fs::read_to_string(&p).await {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let items: Vec<PendingSurfacing> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    // Read-once: clear the queue so the next session starts fresh.
    let _ = tokio::fs::remove_file(&p).await;
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: &str, content: &str) -> PendingSurfacing {
        PendingSurfacing {
            kind: kind.to_string(),
            source: String::new(),
            content: content.to_string(),
            priority: String::new(),
            at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn append_then_take_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &[sample("surfacing", "first")])
            .await
            .unwrap();
        let items = take(dir.path()).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "first");
    }

    #[tokio::test]
    async fn take_clears_the_file() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &[sample("reflection", "x")])
            .await
            .unwrap();
        assert_eq!(take(dir.path()).await.len(), 1);
        // Second take finds nothing — the queue was drained.
        assert!(take(dir.path()).await.is_empty());
    }

    #[tokio::test]
    async fn multiple_appends_accumulate() {
        let dir = tempfile::tempdir().unwrap();
        // Two heartbeats fire between sessions — each appends.
        append(dir.path(), &[sample("surfacing", "a")])
            .await
            .unwrap();
        append(dir.path(), &[sample("archivist", "b")])
            .await
            .unwrap();
        let items = take(dir.path()).await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].content, "a");
        assert_eq!(items[1].content, "b");
    }

    #[tokio::test]
    async fn corrupt_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &[sample("surfacing", "good")])
            .await
            .unwrap();
        // Simulate a half-written line from a crash mid-append.
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path(dir.path()))
            .await
            .unwrap();
        f.write_all(b"{not valid json\n").await.unwrap();

        let items = take(dir.path()).await;
        assert_eq!(items.len(), 1, "the valid entry survives a corrupt line");
        assert_eq!(items[0].content, "good");
    }

    #[tokio::test]
    async fn take_on_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take(dir.path()).await.is_empty());
    }

    #[tokio::test]
    async fn append_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &[]).await.unwrap();
        assert!(!path(dir.path()).exists());
    }
}
