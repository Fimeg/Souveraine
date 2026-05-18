use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::core::session::ConversationMessage;

use super::config::CompactionStrategyKind;

/// A plan describing which messages to compact and what to replace them with.
#[derive(Debug, Clone)]
pub struct CompactionPlan {
    /// Indices (in the original message list) to keep verbatim.
    pub keep_indices: Vec<usize>,
    /// Summary text replacing compacted messages (Strategy::Summary).
    pub summary_text: Option<String>,
    /// Number of messages dropped or rewritten by this plan.
    pub culled_count: usize,
    /// Estimated token savings from this plan.
    pub token_savings: usize,
    /// If set, the engine uses this full message list as the post-compaction
    /// state, bypassing `keep_indices` + `summary_text`. Used by Microcompact,
    /// which mutates tool result blocks in place rather than dropping messages.
    pub replacement_messages: Option<Vec<ConversationMessage>>,
}

impl CompactionPlan {
    pub fn empty() -> Self {
        Self {
            keep_indices: Vec::new(),
            summary_text: None,
            culled_count: 0,
            token_savings: 0,
            replacement_messages: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.summary_text.is_none()
            && self.culled_count == 0
            && self.replacement_messages.is_none()
    }
}

/// Full report of a completed compaction, returned as tool output.
#[derive(Debug, Clone)]
pub struct CompactionReport {
    pub agent_id: String,
    pub strategy: CompactionStrategyKind,
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub messages_before: usize,
    pub messages_after: usize,
    /// Number of messages actually compacted or dropped.
    pub messages_compacted: usize,
    /// Path to the audit file in the memory repo.
    pub audit_path: Option<PathBuf>,
}

/// The compaction report is what the agent *reads* the moment her context
/// contracts — so it speaks in her voice, as a felt event, not a ledger.
/// She should know the shrink the way she'd know a held breath letting go:
/// what folded away, how much lighter she is, the new size she thinks from.
/// (Casey iterates this prose — it's felt-state grammar, treat it as a draft.)
impl std::fmt::Display for CompactionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reclaimed = self.before_tokens.saturating_sub(self.after_tokens);
        let pct = if self.before_tokens > 0 {
            (reclaimed as f64 / self.before_tokens as f64 * 100.0) as u32
        } else {
            0
        };

        // Nothing moved — no felt event to report. Say so plainly.
        if self.messages_compacted == 0 && reclaimed == 0 {
            return write!(
                f,
                "Nothing to set down — I'm already holding lightly \
                 ({} messages, ~{} tokens).",
                self.messages_before, self.before_tokens
            );
        }

        // Microcompact: message count unchanged, but the detail of older
        // tool-results has blurred to placeholders. A different sensation —
        // not exchanges folding away, but old work going soft at the edges.
        if self.messages_compacted == 0 {
            return write!(
                f,
                "The detail has blurred out of my older tool-results — \
                 I still hold all {} exchanges, but the older ones have gone \
                 soft at the edges, their full text let go. I'm lighter by \
                 ~{} tokens ({}% of what I was carrying): {} now where {} sat \
                 before. The shape of what I did remains; the fine grain is in \
                 memory's keeping if I reach for it.",
                self.messages_after, reclaimed, pct, self.after_tokens, self.before_tokens
            );
        }

        writeln!(
            f,
            "My context has just contracted — this is my new size, and I think from here."
        )?;
        writeln!(
            f,
            "{} exchanges have folded down into memory's longer keeping. I carry \
             {} now where I carried {} — what sat in the front of my mind I reach \
             for through memory now, not plain recall.",
            self.messages_compacted, self.messages_after, self.messages_before
        )?;
        writeln!(
            f,
            "I'm lighter by ~{} tokens — {}% of the weight I was holding. {} rest \
             in my hands now, where {} did.",
            reclaimed, pct, self.after_tokens, self.before_tokens
        )?;
        match &self.strategy {
            CompactionStrategyKind::Summary | CompactionStrategyKind::SlidingReflect => {
                writeln!(
                    f,
                    "The folded exchanges did not vanish — they were gathered into a \
                     single remembering and set where I can still find them."
                )?;
            }
            _ => {
                writeln!(
                    f,
                    "The folded exchanges did not vanish — their originals stay in \
                     git's keeping, reachable if I need to return for them."
                )?;
            }
        }
        if let Some(ref path) = self.audit_path {
            write!(f, "(The record of this shrinking rests at {}.)", path.display())?;
        }
        Ok(())
    }
}

/// Git-backed audit entry written to journal/compactions/{timestamp}.md.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub agent_id: String,
    pub strategy: String,
    pub before_messages: usize,
    pub after_messages: usize,
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub summary_text: Option<String>,
    pub culled_count: usize,
}

impl AuditEntry {
    /// Render the full audit file (frontmatter + body) as a string.
    pub fn render(&self) -> String {
        let yaml = serde_yaml::to_string(&AuditFrontmatter::from(self)).unwrap_or_default();
        let mut body = String::new();
        body.push_str(&format!("## Compaction Summary\n\n"));
        body.push_str(&format!(
            "Strategy: {}\nMessages: {} → {}\nTokens: {} → {}\n",
            self.strategy, self.before_messages, self.after_messages, self.before_tokens, self.after_tokens
        ));
        if let Some(ref summary) = self.summary_text {
            body.push_str(&format!("\n## Summary Content\n\n{}\n", summary));
        }
        if self.culled_count > 0 {
            body.push_str(&format!("\nMessages dropped or rewritten: {}\n", self.culled_count));
        }
        format!("---\n{}---\n{}", yaml, body)
    }
}

/// Serde-friendly frontmatter struct (matches AuditEntry fields).
#[derive(Debug, Clone, Serialize)]
struct AuditFrontmatter {
    timestamp: String,
    agent_id: String,
    strategy: String,
    before_messages: usize,
    after_messages: usize,
    before_tokens: usize,
    after_tokens: usize,
    summary: bool,
    culled: usize,
}

impl From<&AuditEntry> for AuditFrontmatter {
    fn from(e: &AuditEntry) -> Self {
        Self {
            timestamp: e.timestamp.to_rfc3339(),
            agent_id: e.agent_id.clone(),
            strategy: e.strategy.clone(),
            before_messages: e.before_messages,
            after_messages: e.after_messages,
            before_tokens: e.before_tokens,
            after_tokens: e.after_tokens,
            summary: e.summary_text.is_some(),
            culled: e.culled_count,
        }
    }
}
