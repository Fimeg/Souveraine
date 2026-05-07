use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::core::config::{ModelConfig, TaskType};

/// Context pressure — how full the context window is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPressure {
    /// Normal operation, plenty of room
    Normal,
    /// Approaching threshold — prepare for synthesis
    Elevated,
    /// At or above threshold — trigger N+100 synthesis NOW
    Critical,
}

/// Active token usage tracker
#[derive(Debug, Default)]
pub struct TokenUsage {
    /// Current estimated tokens in context
    pub tokens: usize,
    /// Estimated prompt tokens
    pub prompt_tokens: usize,
    /// Estimated completion tokens
    pub completion_tokens: usize,
}

/// Token counter using real tiktoken (cl100k_base)
///
/// Matches the OSSUI pattern: js-tiktoken cl100k_base
/// Falls back to chars/4 if tiktoken fails
pub struct TokenCounter {
    /// Whether tiktoken is available
    pub available: bool,
    /// Cached encoding (static lifetime from tiktoken's built-in BPE data)
    encoding: Option<&'static tiktoken::CoreBpe>,
}

impl TokenCounter {
    pub fn new() -> Self {
        let encoding = tiktoken::get_encoding("cl100k_base");

        if encoding.is_some() {
            info!("🔢 Token counter initialized with tiktoken (cl100k_base)");
        } else {
            info!("🔢 Token counter using chars/4 fallback (tiktoken unavailable)");
        }

        Self { encoding, available: encoding.is_some() }
    }

    /// Count tokens using tiktoken or chars/4 fallback
    /// Matches the OSSUI pattern exactly
    pub fn count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        if let Some(ref enc) = self.encoding {
            enc.encode_with_special_tokens(text).len()
        } else {
            // No tiktoken available: chars/4 fallback
            (text.len() as f32 / 4.0).ceil() as usize
        }
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Model Router — physics-aware model selection and context monitoring
pub struct ModelRouter {
    configs: HashMap<String, ModelConfig>,
    current_usage: Arc<RwLock<TokenUsage>>,
    token_counter: TokenCounter,
}

impl ModelRouter {
    pub fn new(configs: HashMap<String, ModelConfig>) -> Self {
        let count = configs.len();
        info!("🧭 ModelRouter initialized with {} model configurations", count);
        for (name, cfg) in &configs {
            debug!(
                "  {} via {} — {} ctx, {} out, threshold {}",
                name, cfg.provider, cfg.context_limit, cfg.output_limit, cfg.archivist_threshold
            );
        }
        Self {
            configs,
            current_usage: Arc::new(RwLock::new(TokenUsage::default())),
            token_counter: TokenCounter::new(),
        }
    }

    /// Count tokens in text using real tiktoken
    pub fn count_tokens(&self, text: &str) -> usize {
        self.token_counter.count(text)
    }

    /// Get a model config by name
    pub fn get_model(&self, name: &str) -> Option<&ModelConfig> {
        self.configs.get(name)
    }

    /// Find the best model for a given task type
    pub fn find_best_for_task(&self, task: TaskType) -> Option<&ModelConfig> {
        self.configs
            .values()
            .filter(|m| m.preferred_for.contains(&task))
            .max_by_key(|m| m.context_limit)
    }

    /// Check context pressure for a specific model
    pub async fn context_pressure(&self, model_name: &str, context_window_limit: Option<usize>) -> ContextPressure {
        let config = match self.configs.get(model_name) {
            Some(c) => c,
            None => return ContextPressure::Normal,
        };

        // Use agent-level context_window_limit if set, otherwise model default
        let limit = context_window_limit.unwrap_or(config.context_limit);
        if limit == 0 {
            return ContextPressure::Normal;
        }

        let usage = self.current_usage.read().await;
        let ratio = usage.tokens as f32 / limit as f32;

        if ratio >= config.archivist_threshold {
            ContextPressure::Critical
        } else if ratio >= config.archivist_threshold * 0.8 {
            ContextPressure::Elevated
        } else {
            ContextPressure::Normal
        }
    }

    /// Update current token usage
    pub async fn update_usage(&self, tokens: usize, prompt: usize, completion: usize) {
        let mut usage = self.current_usage.write().await;
        usage.tokens = tokens;
        usage.prompt_tokens = prompt;
        usage.completion_tokens = completion;
    }

    /// Get a reference to the token usage tracker
    pub fn usage_tracker(&self) -> Arc<RwLock<TokenUsage>> {
        self.current_usage.clone()
    }

    /// Get the context limit for a model with agent override
    pub fn context_limit_for(&self, model_name: &str, agent_override: Option<usize>) -> usize {
        agent_override
            .or_else(|| self.configs.get(model_name).map(|c| c.context_limit))
            .unwrap_or(128_000)
    }

    /// Get the archivist interval for a model (or default 100)
    pub fn archivist_interval_for(&self, model_name: &str) -> usize {
        self.configs
            .get(model_name)
            .map(|c| c.archivist_interval)
            .unwrap_or(100)
    }

    /// Get all configured model names
    pub fn model_names(&self) -> Vec<String> {
        self.configs.keys().cloned().collect()
    }

    /// Get all configured providers
    pub fn providers(&self) -> Vec<String> {
        let mut providers: Vec<String> = self
            .configs
            .values()
            .map(|c| c.provider.clone())
            .collect();
        providers.sort();
        providers.dedup();
        providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::TaskType;

    fn test_configs() -> HashMap<String, ModelConfig> {
        let mut map = HashMap::new();
        map.insert("kimi-k2-5".to_string(), ModelConfig {
            provider: "bifrost".to_string(),
            model: "kimi-k2.5".to_string(),
            context_limit: 128000,
            output_limit: 8192,
            archivist_threshold: 0.7,
            archivist_interval: 100,
            preferred_for: vec![TaskType::Synthesis],
        });
        map
    }

    #[tokio::test]
    async fn test_context_pressure_normal() {
        let router = ModelRouter::new(test_configs());
        assert_eq!(router.context_pressure("kimi-k2-5", None).await, ContextPressure::Normal);
    }

    #[tokio::test]
    async fn test_context_pressure_critical() {
        let router = ModelRouter::new(test_configs());
        router.update_usage(100_000, 90_000, 10_000).await;
        assert_eq!(router.context_pressure("kimi-k2-5", None).await, ContextPressure::Critical);
    }

    #[tokio::test]
    async fn test_context_pressure_with_agent_override() {
        let router = ModelRouter::new(test_configs());
        router.update_usage(90_000, 80_000, 10_000).await;
        // With 90k/128k ≈ 0.7 — critical at threshold 0.7
        assert_eq!(router.context_pressure("kimi-k2-5", None).await, ContextPressure::Critical);
        // With 90k/64k ≈ 1.4 — also critical
        assert_eq!(router.context_pressure("kimi-k2-5", Some(64000)).await, ContextPressure::Critical);
    }

    #[tokio::test]
    async fn test_token_counter() {
        let counter = TokenCounter::new();
        let count = counter.count("Hello, world!");
        assert!(count > 0);
    }
}
