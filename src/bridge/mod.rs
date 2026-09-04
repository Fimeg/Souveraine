#![allow(dead_code)] // WIP scaffolding not yet wired
/// Bridge module — LLM inference providers.
///
/// Connects Souveraine to inference providers behind the [`LlmProvider`] trait.
/// `OpenAiCompatibleClient` speaks to any OpenAI-compatible gateway; the OAuth-riding
/// ChatGPT provider lives in [`providers`]. [`build_provider`] selects the
/// active one from config; [`ProviderRegistry`] resolves a provider per agent.
pub mod openai_compatible;
pub mod claude_subscription;
pub mod model_router;
pub mod oauth;
pub mod provider;
pub mod providers;

pub use openai_compatible::OpenAiCompatibleClient;
pub use provider::LlmProvider;

use std::collections::HashMap;
use std::sync::Arc;

use crate::api::models::AgentState;
use crate::core::config::{ConsciousnessConfig, ProviderConfig};

/// Build the active (global default) LLM provider from config.
///
/// Convenience wrapper that builds the full registry and returns its default.
/// For inference, prefer [`build_registry`] + [`ProviderRegistry::for_agent`].
pub fn build_provider(config: &ConsciousnessConfig) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let reg = build_registry(config)?;
    Ok(reg.default_provider())
}

/// Build a provider from a [`ProviderConfig`] entry, dispatching on `provider_type`.
pub fn build_provider_from_config(
    name: &str,
    cfg: &ProviderConfig,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    match cfg.provider_type.as_str() {
        "openai-oauth" => {
            let provider = providers::openai_oauth::OpenAiOAuthProvider::from_codex_login(
                cfg.primary_model.clone(),
                cfg.timeout_secs,
            )?;
            Ok(Arc::new(provider))
        }
        "claude-subscription" => {
            let provider = claude_subscription::ClaudeSubscriptionProvider::new(
                name,
                &cfg.base_url,
                &cfg.primary_model,
                cfg.timeout_secs,
                cfg.credential_file.as_deref(),
                cfg.credential_files.as_deref(),
                cfg.cc_version.as_deref(),
                cfg.account_uuid.as_deref(),
                cfg.device_id.as_deref(),
                cfg.extra_metadata.clone(),
            )?;
            Ok(Arc::new(provider))
        }
        _ => {
            // "openai-compatible" and any unrecognized type → OpenAI-compatible gateway.
            let client = OpenAiCompatibleClient::new(
                name,
                &cfg.base_url,
                &cfg.api_key,
                &cfg.virtual_key,
                &cfg.primary_model,
                cfg.timeout_secs,
            )?;
            Ok(Arc::new(client))
        }
    }
}

/// Build the per-agent provider registry from the unified `[providers]` map.
///
/// Iterates every entry in `config.providers`, builds a provider for each, and
/// resolves the default from `[inference] provider`. Returns an error if no
/// providers are configured.
pub fn build_registry(config: &ConsciousnessConfig) -> anyhow::Result<ProviderRegistry> {
    let mut map = HashMap::new();

    for (name, pcfg) in &config.providers {
        let provider = build_provider_from_config(name, pcfg)?;
        map.insert(name.clone(), provider);
    }

    if map.is_empty() {
        anyhow::bail!("no providers configured — add at least one [providers.<name>] section");
    }

    let default_name = config.inference.provider.clone();
    let default = map
        .get(&default_name)
        .cloned()
        .unwrap_or_else(|| map.values().next().unwrap().clone());

    let model_providers = config
        .models
        .iter()
        .map(|(model, mcfg)| (model.clone(), mcfg.provider.clone()))
        .collect();

    Ok(ProviderRegistry {
        map,
        default_name,
        default,
        model_providers,
    })
}

/// Maps a provider name to a live [`LlmProvider`], resolving per agent.
#[derive(Clone)]
pub struct ProviderRegistry {
    map: HashMap<String, Arc<dyn LlmProvider>>,
    default_name: String,
    default: Arc<dyn LlmProvider>,
    /// `[models.<name>] provider` — which provider serves a given model.
    /// Needed wherever a model is chosen independently of the agent that owns
    /// the turn, e.g. the subconscious running somewhere the primary does not.
    model_providers: HashMap<String, String>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.map.keys().collect::<Vec<_>>())
            .field("default", &self.default_name)
            .finish()
    }
}

impl ProviderRegistry {
    /// Resolve the provider for an agent: its `_souveraine.provider` override,
    /// else the global default. An unknown/missing name falls back to the
    /// default rather than failing.
    pub fn for_agent(&self, agent: &AgentState) -> Arc<dyn LlmProvider> {
        match agent.souveraine.provider.as_deref() {
            Some(name) if name != self.default_name => self
                .map
                .get(name)
                .cloned()
                .unwrap_or_else(|| self.default.clone()),
            _ => self.default.clone(),
        }
    }

    /// All registered provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.map.keys().map(|s| s.as_str()).collect()
    }

    /// Look up a provider by name. Returns `None` for unknown names.
    pub fn get(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.map.get(name).cloned()
    }

    /// Resolve the provider that serves a model, via `[models.<name>] provider`.
    ///
    /// `None` when the model has no config entry or names a provider that
    /// isn't registered — the caller decides the fallback, because "no entry"
    /// and "wrong entry" both mean *don't guess a wire for this model*.
    pub fn for_model(&self, model: &str) -> Option<Arc<dyn LlmProvider>> {
        let name = self.model_providers.get(model)?;
        self.map.get(name).cloned()
    }

    /// The global default provider (for contexts with no agent).
    pub fn default_provider(&self) -> Arc<dyn LlmProvider> {
        self.default.clone()
    }

    /// Name of the global default provider.
    pub fn default_name(&self) -> &str {
        &self.default_name
    }
}
