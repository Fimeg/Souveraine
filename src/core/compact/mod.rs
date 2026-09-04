#![allow(clippy::type_complexity)] // Arc<dyn Fn> callback field types
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
pub use strategy::{
    CompactionStrategy, CullStrategy, MicrocompactStrategy, SlidingReflectStrategy,
    SlidingWindowStrategy, SummaryStrategy,
};

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::bridge::model_router::TokenCounter;
use crate::bridge::ProviderRegistry;
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

/// A reading of how full an agent's live conversation is.
///
/// This is the gauge behind `memory status`. Both the primary and the
/// subconscious are told in their system prompt that `memory status` shows
/// "context pressure and number of messages" — before 2026-08-13 it showed
/// neither, and the subconscious (who receives no tier warnings at all, by
/// design: "no one feels this gauge for me") had no way to read her own
/// fullness. She grew to 1.75M tokens against a 1M ceiling and died at the
/// provider. The promise in the prompt is now kept in the code.
#[derive(Debug, Clone, PartialEq)]
pub struct PressureSnapshot {
    /// Messages currently in the live conversation.
    pub messages: usize,
    /// Estimated tokens across every countable block, not text alone.
    pub tokens: usize,
    /// The context window this is measured against.
    pub limit: usize,
    /// `tokens / limit`, clamped to 1.0.
    pub ratio: f32,
}

impl PressureSnapshot {
    /// The advisory tier this reading falls in, matching the three marks the
    /// engine warns on (0.80 / 0.90 / 0.95). `None` below the first mark.
    pub fn tier(&self) -> Option<u8> {
        match self.ratio {
            r if r > 0.95 => Some(3),
            r if r > 0.90 => Some(2),
            r if r > 0.80 => Some(1),
            _ => None,
        }
    }
}

/// The public interface for compaction operations.
///
/// Carried in `ToolContext::compaction_engine` so the `memory compact`
/// tool handler can delegate to it without server dependencies.
#[async_trait]
pub trait CompactionEngine: Send + Sync {
    /// Read the agent's current context pressure.
    ///
    /// `None` when the agent has no live session — a fresh agent has nothing
    /// to measure, which is distinct from measuring zero.
    async fn pressure(&self, agent_id: &str) -> Option<PressureSnapshot>;

    /// Compact messages for the given agent's session.
    /// `strategy_override` allows the agent to pick a specific strategy;
    /// None uses the config default for the agent's type.
    async fn compact(
        &self,
        agent_id: &str,
        strategy_override: Option<CompactionStrategyKind>,
    ) -> anyhow::Result<CompactionReport>;

    /// Write the audit entry to the agent's memory repo.
    async fn write_audit(&self, agent_id: &str, entry: &AuditEntry) -> anyhow::Result<PathBuf>;
}

/// Concrete compaction engine that ties together config, strategies,
/// token counting, and audit writing.
///
/// Uses closure-based injection for server-level dependencies so the
/// engine is testable without a running server.
pub struct DefaultCompactionEngine {
    pub config: Arc<RwLock<ConsciousnessConfig>>,
    pub counter: TokenCounter,
    pub providers: Option<Arc<ProviderRegistry>>,
    pub model: Option<String>,
    pub clock: Arc<dyn Clock>,
    pub get_messages: Arc<dyn Fn(&str) -> Option<Vec<ConversationMessage>> + Send + Sync>,
    pub replace_messages:
        Arc<dyn Fn(&str, Vec<ConversationMessage>) -> anyhow::Result<()> + Send + Sync>,
    pub get_repo: Arc<dyn Fn(&str) -> Option<MemoryRepo> + Send + Sync>,
    pub get_agent_type: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
    /// Per-agent provider override name (e.g. "zai"). None = use global default.
    pub get_agent_provider: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
    /// The context window to measure this agent's pressure against. Separate
    /// from the strategy config because it follows the *model*, and the
    /// subconscious runs a different model from her primary.
    pub get_context_limit: Arc<dyn Fn(&str) -> Option<usize> + Send + Sync>,
}

#[async_trait]
impl CompactionEngine for DefaultCompactionEngine {
    async fn pressure(&self, agent_id: &str) -> Option<PressureSnapshot> {
        let messages = (self.get_messages)(agent_id)?;
        let tokens = count_messages(&self.counter, &messages);
        // Falls back to 128K only when the model is unknown — the same
        // fallback `ConsciousnessEngine::pressure_for` uses, so the two
        // gauges cannot disagree about the ceiling.
        let limit = (self.get_context_limit)(agent_id).unwrap_or(128_000).max(1);
        Some(PressureSnapshot {
            messages: messages.len(),
            tokens,
            limit,
            ratio: (tokens as f32 / limit as f32).min(1.0),
        })
    }

    async fn compact(
        &self,
        agent_id: &str,
        strategy_override: Option<CompactionStrategyKind>,
    ) -> anyhow::Result<CompactionReport> {
        // Resolve config early so we can bail before requiring a session
        let agent_type = (self.get_agent_type)(agent_id).unwrap_or_else(|| "primary".to_string());
        let (cfg, reflect_prompt, summary_prompt) = {
            let app_config = self.config.read().await;
            (
                app_config.compaction.for_agent_type(&agent_type),
                app_config.compaction.reflect_prompt.clone(),
                app_config.compaction.summary_prompt.clone(),
            )
        };

        let messages = match (self.get_messages)(agent_id) {
            Some(m) => m,
            None if !cfg.enabled => {
                return Ok(CompactionReport {
                    agent_id: agent_id.to_string(),
                    strategy: strategy_override.unwrap_or(cfg.strategy.clone()),
                    before_tokens: 0,
                    after_tokens: 0,
                    messages_before: 0,
                    messages_after: 0,
                    messages_compacted: 0,
                    audit_path: None,
                });
            }
            None => anyhow::bail!("No session found for agent {}", agent_id),
        };

        let before_count = messages.len();
        let before_tokens = count_messages(&self.counter, &messages);

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
            CompactionStrategyKind::Summary => match &self.providers {
                Some(reg) => {
                    let provider_name = (self.get_agent_provider)(agent_id);
                    let client = provider_name
                        .as_deref()
                        .and_then(|n| reg.get(n))
                        .unwrap_or_else(|| reg.default_provider());
                    let model = self.model.as_deref().unwrap_or("openai/kimi-k2.6");
                    let s = SummaryStrategy {
                        client: client.clone(),
                        model: model.to_string(),
                        prompt_override: summary_prompt,
                    };
                    s.plan(&messages, &cfg, &self.counter).await?
                }
                None => CompactionPlan::empty(),
            },
            CompactionStrategyKind::Cull => {
                let s = CullStrategy;
                s.plan(&messages, &cfg, &self.counter).await?
            }
            CompactionStrategyKind::Microcompact => {
                let s = MicrocompactStrategy;
                s.plan(&messages, &cfg, &self.counter).await?
            }
            CompactionStrategyKind::SlidingWindow => {
                let s = SlidingWindowStrategy;
                s.plan(&messages, &cfg, &self.counter).await?
            }
            CompactionStrategyKind::SlidingReflect => match &self.providers {
                Some(reg) => {
                    let provider_name = (self.get_agent_provider)(agent_id);
                    let client = provider_name
                        .as_deref()
                        .and_then(|n| reg.get(n))
                        .unwrap_or_else(|| reg.default_provider());
                    let model = self.model.as_deref().unwrap_or("openai/kimi-k2.6");
                    // The preservation pass runs as a fresh fork of *this*
                    // agent — load her persona so the fork wakes as her.
                    let agent_persona = (self.get_repo)(agent_id)
                        .and_then(|repo| {
                            std::fs::read_to_string(repo.root().join("system/persona.md")).ok()
                        })
                        .map(|c| {
                            // Drop a leading YAML frontmatter block if present.
                            if let Some(rest) = c.strip_prefix("---\n") {
                                if let Some(end) = rest.find("\n---\n") {
                                    return rest[end + 5..].trim().to_string();
                                }
                            }
                            c.trim().to_string()
                        });
                    let s = SlidingReflectStrategy {
                        client: client.clone(),
                        model: model.to_string(),
                        prompt_override: reflect_prompt,
                        agent_persona,
                    };
                    s.plan(&messages, &cfg, &self.counter).await?
                }
                None => {
                    tracing::warn!("[sliding_reflect] no provider registry, falling back to plain sliding window");
                    SlidingWindowStrategy
                        .plan(&messages, &cfg, &self.counter)
                        .await?
                }
            },
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

        // Build replacement messages.
        // Microcompact provides a full replacement list; other strategies
        // use keep_indices + optional summary_text.
        let (new_messages, messages_compacted, after_tokens) =
            if let Some(replacement) = plan.replacement_messages {
                let compacted = before_count.saturating_sub(replacement.len());
                let tokens = count_messages(&self.counter, &replacement);
                (replacement, compacted, tokens)
            } else {
                let mut kept = Vec::new();
                for &idx in &plan.keep_indices {
                    if idx < messages.len() {
                        kept.push(messages[idx].clone());
                    }
                }
                if let Some(ref summary_text) = plan.summary_text {
                    kept.push(ConversationMessage {
                        role: crate::core::session::MessageRole::System,
                        blocks: vec![crate::core::session::ContentBlock::Text {
                            text: format!("[Compacted summary]\n{}", summary_text),
                        }],
                        usage: None,
                        timestamp: Some(Utc::now()),
                    });
                }
                let compacted = before_count.saturating_sub(kept.len());
                let tokens = count_messages(&self.counter, &kept);
                (kept, compacted, tokens)
            };

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

    async fn write_audit(&self, agent_id: &str, entry: &AuditEntry) -> anyhow::Result<PathBuf> {
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
            providers: None,
            model: None,
            clock: Arc::new(TestClock),
            config,
            get_messages: Arc::new(|_| None),
            replace_messages: Arc::new(|_, _| Ok(())),
            get_repo: Arc::new(|_| None),
            get_agent_type: Arc::new(|_| Some("primary".to_string())),
            get_agent_provider: Arc::new(|_| None),
            get_context_limit: Arc::new(|_| None),
        };

        let report = engine
            .compact("test-agent", Some(CompactionStrategyKind::Cull))
            .await
            .unwrap();

        assert_eq!(report.messages_compacted, 0);
        assert!(report.audit_path.is_none());
    }

    /// The gauge behind `memory status`. Both system prompts promise it;
    /// before 2026-08-13 `status` reported git state only, which is how the
    /// subconscious reached 1.75M tokens against a 1M ceiling with nothing
    /// anywhere able to tell her.
    #[tokio::test]
    async fn pressure_reads_the_live_conversation_against_the_model_ceiling() {
        use crate::core::session::ConversationMessage;

        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let msgs = vec![
            ConversationMessage::user_text("a ".repeat(400)),
            ConversationMessage::assistant_text("b ".repeat(400)),
        ];

        let engine = DefaultCompactionEngine {
            counter: TokenCounter::new(),
            providers: None,
            model: None,
            clock: Arc::new(TestClock),
            config,
            get_messages: Arc::new(move |_| Some(msgs.clone())),
            replace_messages: Arc::new(|_, _| Ok(())),
            get_repo: Arc::new(|_| None),
            get_agent_type: Arc::new(|_| Some("subconscious".to_string())),
            get_agent_provider: Arc::new(|_| None),
            get_context_limit: Arc::new(|_| Some(1000)),
        };

        let p = engine.pressure("test-agent-sub").await.expect("a reading");
        assert_eq!(p.messages, 2, "both messages counted");
        assert_eq!(
            p.limit, 1000,
            "the ceiling follows the model, not a default"
        );
        assert!(p.tokens > 0, "tokens are actually measured");
        assert!(
            p.ratio > 0.0 && p.ratio <= 1.0,
            "ratio is a clamped fraction, got {}",
            p.ratio
        );
    }

    /// A fresh agent has nothing to measure. That is distinct from measuring
    /// zero, and the difference is the whole empty-result family: reporting
    /// "0%" for "I could not look" is how a full room reads as an empty one.
    #[tokio::test]
    async fn no_session_reports_absence_rather_than_zero() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let engine = DefaultCompactionEngine {
            counter: TokenCounter::new(),
            providers: None,
            model: None,
            clock: Arc::new(TestClock),
            config,
            get_messages: Arc::new(|_| None),
            replace_messages: Arc::new(|_, _| Ok(())),
            get_repo: Arc::new(|_| None),
            get_agent_type: Arc::new(|_| Some("primary".to_string())),
            get_agent_provider: Arc::new(|_| None),
            get_context_limit: Arc::new(|_| Some(1000)),
        };
        assert!(engine.pressure("nobody").await.is_none());
    }

    /// The tiers must agree with the three marks the engine warns on
    /// (`consciousness_engine.rs`: 0.80 / 0.90 / 0.95). Two gauges that
    /// disagree about "full" is the two-authorities defect in miniature.
    #[test]
    fn tiers_match_the_engines_three_marks() {
        let at = |ratio: f32| {
            PressureSnapshot {
                messages: 1,
                tokens: 1,
                limit: 1,
                ratio,
            }
            .tier()
        };
        assert_eq!(at(0.79), None);
        assert_eq!(at(0.85), Some(1));
        assert_eq!(at(0.91), Some(2));
        assert_eq!(at(0.96), Some(3));
    }
}
