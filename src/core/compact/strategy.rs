use std::collections::HashMap;

use async_trait::async_trait;

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message};
use crate::bridge::model_router::TokenCounter;
use crate::core::session::ConversationMessage;

use super::config::{AgentCompactionConfig, CompactionStrategyKind};
use super::plan::CompactionPlan;

/// Token-count a slice of messages using the bridge's TokenCounter.
pub fn count_messages(counter: &TokenCounter, messages: &[ConversationMessage]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter_map(|b| match b {
            crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .map(|t| counter.count(t))
        .sum()
}

/// A single compaction strategy.
///
/// Each strategy is an async function from messages+config to a plan.
/// A strategy does NOT modify messages directly — it returns a plan
/// describing what to keep, what to replace, and what to drop.
#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    fn kind(&self) -> CompactionStrategyKind;

    /// Analyze messages and produce a compaction plan.
    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan>;
}

/// Helper: call a Bifrost model with system+user prompt, get text response.
async fn bifrost_complete(
    client: &BifrostClient,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: u32,
) -> anyhow::Result<String> {
    let request = ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: system.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: Some(0.3),
        max_tokens: Some(max_tokens),
        stream: None,
        tools: None,
    };
    let result = client.chat_completion(request).await?;
    Ok(result.content)
}

// ── Summary Strategy ─────────────────────────────────────────────────────────

/// LLM-based summarization. Replaces oldest messages with a single summary.
pub struct SummaryStrategy {
    pub client: BifrostClient,
    pub model: String,
}

#[async_trait]
impl CompactionStrategy for SummaryStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Summary
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);

        let to_summarize = &messages[1..cutoff];
        if to_summarize.is_empty() {
            return Ok(CompactionPlan::empty());
        }

        let conversation_text: String = to_summarize
            .iter()
            .flat_map(|m| &m.blocks)
            .filter_map(|b| match b {
                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let truncated: String = conversation_text
            .chars()
            .take(config.max_summary_length * 2)
            .collect();

        let summary = bifrost_complete(
            &self.client,
            &self.model,
            "You are a conversation summarizer. Be concise but preserve key decisions, \
             commitments, file paths, and unresolved questions.",
            &format!(
                "Summarize this conversation segment (max {} chars):\n\n{}",
                config.max_summary_length, truncated
            ),
            config.max_summary_length as u32,
        )
        .await?;

        let summary_trimmed: String = summary.chars().take(config.max_summary_length).collect();

        Ok(CompactionPlan {
            keep_indices: (cutoff..messages.len()).collect(),
            summary_text: Some(summary_trimmed),
            kv_pairs: HashMap::new(),
            quotes: Vec::new(),
            culled_count: 0,
            token_savings: cutoff * 100,
        })
    }
}

// ── Key-Value Strategy ───────────────────────────────────────────────────────

/// LLM-based extraction. Pulls key facts, decisions, and plans from old messages.
pub struct KeyValueStrategy {
    pub client: BifrostClient,
    pub model: String,
}

#[async_trait]
impl CompactionStrategy for KeyValueStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::KeyValue
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);

        let to_extract = &messages[1..cutoff];
        if to_extract.is_empty() {
            return Ok(CompactionPlan::empty());
        }

        let conversation_text: String = to_extract
            .iter()
            .flat_map(|m| &m.blocks)
            .filter_map(|b| match b {
                crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let truncated: String = conversation_text.chars().take(4000).collect();

        let response = bifrost_complete(
            &self.client,
            &self.model,
            "Extract key facts, decisions, preferences, and plans from this conversation. \
             Format each as '- key: value' on its own line. Max 12 pairs.",
            &format!("Extract up to {} key-value pairs:\n\n{}", config.kv_target, truncated),
            1024,
        )
        .await?;

        let mut kv_pairs = HashMap::new();
        for line in response.lines() {
            let line = line.trim();
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim_matches(|c: char| c == '-' || c == ' ' || c == '"').to_string();
                let value = v.trim().trim_matches('"').to_string();
                if !key.is_empty() && !value.is_empty() {
                    kv_pairs.insert(key, value);
                }
            }
        }

        Ok(CompactionPlan {
            keep_indices: (cutoff..messages.len()).collect(),
            summary_text: None,
            kv_pairs,
            quotes: Vec::new(),
            culled_count: 0,
            token_savings: cutoff * 120,
        })
    }
}

// ── Quote Strategy ───────────────────────────────────────────────────────────

/// Pattern-based quote preservation. No LLM dependency.
pub struct QuoteStrategy;

#[async_trait]
impl CompactionStrategy for QuoteStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Quote
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);
        let to_scan = &messages[1..cutoff];

        let mut quotes: Vec<String> = Vec::new();

        for msg in to_scan {
            for block in &msg.blocks {
                if let crate::core::session::ContentBlock::Text { text } = block {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("> ") {
                            quotes.push(trimmed.trim_start_matches("> ").to_string());
                        } else if trimmed.starts_with("**")
                            && (trimmed.to_lowercase().contains("key:")
                                || trimmed.to_lowercase().contains("decision:")
                                || trimmed.to_lowercase().contains("commitment:")
                                || trimmed.to_lowercase().contains("remember:"))
                        {
                            quotes.push(trimmed.to_string());
                        } else if trimmed.starts_with("- **")
                            && (trimmed.contains("decision") || trimmed.contains("commitment"))
                        {
                            quotes.push(trimmed.to_string());
                        } else if trimmed.starts_with("=>") || trimmed.starts_with("->") {
                            quotes.push(trimmed.to_string());
                        }
                    }
                }
            }
        }

        quotes.truncate(config.max_summary_length.max(32));

        Ok(CompactionPlan {
            keep_indices: (cutoff..messages.len()).collect(),
            summary_text: None,
            kv_pairs: HashMap::new(),
            quotes,
            culled_count: 0,
            token_savings: cutoff * 80,
        })
    }
}

// ── Cull Strategy ────────────────────────────────────────────────────────────

/// Drop trivial messages. No LLM dependency.
pub struct CullStrategy;

fn is_trivial(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 4 {
        return true;
    }
    let lower = trimmed.to_lowercase();
    matches!(
        lower.as_str(),
        "ok" | "okay"
            | "thanks"
            | "ty"
            | "got it"
            | "sure"
            | "yes"
            | "no"
            | "thx"
            | "k"
            | "👍"
            | "🙏"
            | "done"
            | "yep"
            | "nope"
            | "right"
            | "cool"
            | "great"
            | "will do"
            | "on it"
    )
}

#[async_trait]
impl CompactionStrategy for CullStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Cull
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);

        let mut keep_indices: Vec<usize> = vec![0];
        let mut culled_count = 0;

        for i in cutoff..messages.len() {
            keep_indices.push(i);
        }

        for i in 1..cutoff {
            let is_cullable = messages[i].blocks.iter().any(|b| match b {
                crate::core::session::ContentBlock::Text { text } => is_trivial(text),
                _ => false,
            });
            if is_cullable {
                culled_count += 1;
            } else {
                keep_indices.push(i);
            }
        }

        keep_indices.sort();
        keep_indices.dedup();

        Ok(CompactionPlan {
            keep_indices,
            summary_text: None,
            kv_pairs: HashMap::new(),
            quotes: Vec::new(),
            culled_count,
            token_savings: culled_count * 60,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};

    fn text_msg(role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            role,
            blocks: vec![ContentBlock::Text { text: text.to_string() }],
            usage: None,
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn test_cull_drops_trivial() {
        let messages = vec![
            text_msg(MessageRole::System, "System prompt"),
            text_msg(MessageRole::User, "ok"),
            text_msg(MessageRole::User, "What's the plan for today?"),
            text_msg(MessageRole::Assistant, "Sure, let me check."),
            text_msg(MessageRole::User, "thanks"),
            text_msg(MessageRole::Assistant, "Here's what I found."),
        ];

        let config = AgentCompactionConfig {
            min_messages: 2,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();

        assert!(plan.culled_count > 0, "should cull some messages");
        assert!(plan.is_empty(), "cull should not produce content");
    }

    #[tokio::test]
    async fn test_cull_preserves_substance() {
        let messages = vec![
            text_msg(MessageRole::System, "System prompt"),
            text_msg(MessageRole::User, "This is an important question about the architecture."),
            text_msg(MessageRole::Assistant, "Let me explain the design decisions."),
        ];

        let config = AgentCompactionConfig {
            min_messages: 1,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();

        assert_eq!(plan.culled_count, 0, "should not cull substantive messages");
    }

    #[tokio::test]
    async fn test_quote_detects_markers() {
        let messages = vec![
            text_msg(MessageRole::System, "System prompt"),
            text_msg(MessageRole::User, "Some regular text"),
            text_msg(
                MessageRole::Assistant,
                "Here's my analysis:\n> Key decision: use TOML for configs\n> Remember: always verify before commit",
            ),
        ];

        let config = AgentCompactionConfig {
            min_messages: 1,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = QuoteStrategy.plan(&messages, &config, &counter).await.unwrap();

        assert!(!plan.quotes.is_empty(), "should detect blockquote markers");
    }

    #[tokio::test]
    async fn test_empty_messages_return_empty_plan() {
        let config = AgentCompactionConfig::default();
        let counter = TokenCounter::new();

        let e1 = CullStrategy.plan(&[], &config, &counter).await.unwrap();
        assert!(e1.is_empty());

        let e2 = QuoteStrategy.plan(&[], &config, &counter).await.unwrap();
        assert!(e2.is_empty());
    }
}
