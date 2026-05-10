//! In-session message compaction — conversation context management
//! for the consciousness engine.
//!
//! ## Architecture
//!
//! - **CompactionEngine** trait — the public interface, carried in ToolContext
//! - **DefaultCompactionEngine** — concrete implementation using strategies + counter
//! - **CompactionStrategy** trait — one per strategy type (Summary, KeyValue, Quote, Cull)
//! - **TokenCounter** — from bridge/model_router (tiktoken + chars/4 fallback)
//! - **AuditEntry** — git-backed audit trail in journal/compactions/
//!
//! ## Design Principles
//!
//! - Compaction is ALWAYS tool-call driven. The engine never forces it.
//!   Pressure warnings are advisory (nervous system, not governor).
//! - Messages are never truly deleted — originals remain in git history.
//! - Per-agent-type configuration (Primary, Subconscious, Subagent).
//! - The engine is stateless with respect to sessions — operates on message slices.

pub mod config;
pub mod plan;
pub mod strategy;

pub use config::{CompactionConfig, CompactionStrategyKind};
pub use plan::{AuditEntry, CompactionPlan, CompactionReport};
pub use strategy::{CompactionStrategy, CullStrategy, KeyValueStrategy, QuoteStrategy, SummaryStrategy};

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::bridge::bifrost::BifrostClient;
use crate::bridge::model_router::TokenCounter;
use crate::core::config::ConsciousnessConfig;
use crate::core::memory::MemoryRepo;
use crate::core::session::ConversationMessage;

use self::strategy::count_messages;

/// Abstract clock so the engine is testable without real time.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock using `Utc::now()`.
pub struct UtcClock;

impl Clock for UtcClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The public interface for compaction operations.
///
/// Carried in `ToolContext::compaction_engine` so the `memory compact`
/// tool handler can delegate to it without server dependencies.
#[async_trait]
pub trait CompactionEngine: Send + Sync {
    /// Compact messages for the given agent's session.
    /// `strategy_override` allows the agent to pick a specific strategy;
    /// None uses the config default for the agent's type.
    async fn compact(
        &self,
        agent_id: &str,
        strategy_override: Option<CompactionStrategyKind>,
    ) -> anyhow::Result<CompactionReport>;

    /// Write the audit entry to the agent's memory repo.
    async fn write_audit(
        &self,
        agent_id: &str,
        entry: &AuditEntry,
    ) -> anyhow::Result<PathBuf>;
}

/// Concrete compaction engine that ties together config, strategies,
/// token counting, and audit writing.
///
/// Uses closure-based injection for server-level dependencies so the
/// engine is testable without a running server.
pub struct DefaultCompactionEngine {
    pub config: Arc<RwLock<ConsciousnessConfig>>,
    pub counter: TokenCounter,
    pub bifrost: Option<BifrostClient>,
    pub model: Option<String>,
    pub clock: Arc<dyn Clock>,
    pub get_messages: Arc<dyn Fn(&str) -> Option<Vec<ConversationMessage>> + Send + Sync>,
    pub replace_messages:
        Arc<dyn Fn(&str, Vec<ConversationMessage>) -> anyhow::Result<()> + Send + Sync>,
    pub get_repo: Arc<dyn Fn(&str) -> Option<MemoryRepo> + Send + Sync>,
    pub get_agent_type: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

#[async_trait]
impl CompactionEngine for DefaultCompactionEngine {
    async fn compact(
        &self,
        agent_id: &str,
        strategy_override: Option<CompactionStrategyKind>,
    ) -> anyhow::Result<CompactionReport> {
        let messages = (self.get_messages)(agent_id)
            .ok_or_else(|| anyhow::anyhow!("No session found for agent {}", agent_id))?;

        let before_count = messages.len();
        let before_tokens = count_messages(&self.counter, &messages);

        // Resolve config
        let agent_type = (self.get_agent_type)(agent_id)
            .unwrap_or_else(|| "primary".to_string());
        let cfg = {
            let app_config = self.config.read().await;
            app_config.compaction.for_agent_type(&agent_type)
        };

        if !cfg.enabled {
            return Ok(CompactionReport {
                agent_id: agent_id.to_string(),
                strategy: strategy_override.unwrap_or(cfg.strategy.clone()),
                before_tokens,
                after_tokens: before_tokens,
                messages_before: before_count,
                messages_after: before_count,
                messages_compacted: 0,
                audit_path: None,
            });
        }

        let strategy_kind = strategy_override.unwrap_or(cfg.strategy.clone());

        // Build strategy and get plan
        let plan = match strategy_kind {
            CompactionStrategyKind::Summary => match &self.bifrost {
                Some(client) => {
                    let model = self
                        .model
                        .as_deref()
                        .unwrap_or("openai/kimi-k2.6");
                    let s = SummaryStrategy {
                        client: client.clone(),
                        model: model.to_string(),
                    };
                    s.plan(&messages, &cfg, &self.counter).await?
                }
                None => CompactionPlan::empty(),
            },
            CompactionStrategyKind::KeyValue => match &self.bifrost {
                Some(client) => {
                    let model = self
                        .model
                        .as_deref()
                        .unwrap_or("openai/kimi-k2.6");
                    let s = KeyValueStrategy {
                        client: client.clone(),
                        model: model.to_string(),
                    };
                    s.plan(&messages, &cfg, &self.counter).await?
                }
                None => CompactionPlan::empty(),
            },
            CompactionStrategyKind::Quote => {
                let s = QuoteStrategy;
                s.plan(&messages, &cfg, &self.counter).await?
            }
            CompactionStrategyKind::Cull => {
                let s = CullStrategy;
                s.plan(&messages, &cfg, &self.counter).await?
            }
        };

        if plan.is_empty() {
            return Ok(CompactionReport {
                agent_id: agent_id.to_string(),
                strategy: strategy_kind,
                before_tokens,
                after_tokens: before_tokens,
                messages_before: before_count,
                messages_after: before_count,
                messages_compacted: 0,
                audit_path: None,
            });
        }

        // Build replacement messages
        let mut new_messages: Vec<ConversationMessage> = Vec::new();

        for &idx in &plan.keep_indices {
            if idx < messages.len() {
                new_messages.push(messages[idx].clone());
            }
        }

        if let Some(ref summary_text) = plan.summary_text {
            new_messages.push(ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: format!("[Compacted summary]\n{}", summary_text),
                }],
                usage: None,
                timestamp: Some(Utc::now()),
            });
        }

        let messages_compacted = before_count.saturating_sub(new_messages.len());
        let after_tokens = count_messages(&self.counter, &new_messages);

        if let Err(e) = (self.replace_messages)(agent_id, new_messages) {
            tracing::warn!("[compact] Failed to replace session messages: {}", e);
        }

        // Write audit trail
        let audit_path = {
            let strategy_name = strategy_kind.as_str().to_string();
            let entry = AuditEntry {
                timestamp: self.clock.now(),
                agent_id: agent_id.to_string(),
                strategy: strategy_name,
                before_messages: before_count,
                after_messages: before_count.saturating_sub(messages_compacted),
                before_tokens,
                after_tokens,
                summary_text: plan.summary_text.clone(),
                kv_count: plan.kv_pairs.len(),
                quote_count: plan.quotes.len(),
                culled_count: plan.culled_count,
            };
            self.write_audit(agent_id, &entry).await.ok()
        };

        Ok(CompactionReport {
            agent_id: agent_id.to_string(),
            strategy: strategy_kind,
            before_tokens,
            after_tokens,
            messages_before: before_count,
            messages_after: before_count.saturating_sub(messages_compacted),
            messages_compacted,
            audit_path,
        })
    }

    async fn write_audit(
        &self,
        agent_id: &str,
        entry: &AuditEntry,
    ) -> anyhow::Result<PathBuf> {
        let repo = (self.get_repo)(agent_id)
            .ok_or_else(|| anyhow::anyhow!("No memory repo for agent {}", agent_id))?;

        let timestamp = entry.timestamp.format("compactions/%Y-%m-%dT%H-%M-%SZ");
        let label = timestamp.to_string();

        let content = entry.render();
        repo.write(&label, &content).await?;

        let full_path = repo.root().join(format!("{}.md", label));
        Ok(full_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestClock;
    impl Clock for TestClock {
        fn now(&self) -> DateTime<Utc> {
            DateTime::parse_from_rfc3339("2026-05-10T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        }
    }

    #[tokio::test]
    async fn test_compaction_disabled_returns_passthrough() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig {
            compaction: CompactionConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        }));

        let engine = DefaultCompactionEngine {
            counter: TokenCounter::new(),
            bifrost: None,
            model: None,
            clock: Arc::new(TestClock),
            config,
            get_messages: Arc::new(|_| None),
            replace_messages: Arc::new(|_, _| Ok(())),
            get_repo: Arc::new(|_| None),
            get_agent_type: Arc::new(|_| Some("primary".to_string())),
        };

        let report = engine
            .compact("test-agent", Some(CompactionStrategyKind::Cull))
            .await
            .unwrap();

        assert_eq!(report.messages_compacted, 0);
        assert!(report.audit_path.is_none());
    }
}
