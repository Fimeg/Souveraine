/// Bridge module — LLM inference providers.
///
/// Connects Souveraine to inference providers behind the [`LlmProvider`] trait.
/// `BifrostClient` speaks to any OpenAI-compatible gateway; the OAuth-riding
/// ChatGPT provider lives in [`providers`]. [`build_provider`] selects the
/// active one from config.
pub mod bifrost;
pub mod model_router;
pub mod oauth;
pub mod provider;
pub mod providers;

pub use bifrost::BifrostClient;
pub use provider::LlmProvider;

use std::sync::Arc;

use crate::core::config::ConsciousnessConfig;

/// Build the active LLM provider from config.
///
/// `[bifrost] provider` selects the implementation:
/// - `"bifrost"` (default) — any OpenAI-compatible gateway via `BifrostClient`.
/// - `"openai-oauth"` — ride the Codex CLI's ChatGPT login and drive
///   `backend-api/codex/responses`.
pub fn build_provider(config: &ConsciousnessConfig) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let bf = &config.bifrost;
    match bf.provider.as_str() {
        "openai-oauth" | "openai-codex" => {
            let provider = providers::openai_oauth::OpenAiOAuthProvider::from_codex_login(
                bf.primary_model.clone(),
                bf.timeout_secs,
            )?;
            Ok(Arc::new(provider))
        }
        _ => {
            // Mirror the precision fallback the server used to build inline.
            let mut fallbacks = Vec::new();
            if !bf.primary_model.ends_with("-precision") {
                fallbacks.push(format!("{}-precision", bf.primary_model));
            }
            let client = BifrostClient::new(
                &bf.base_url,
                &bf.api_key,
                &bf.virtual_key,
                &bf.primary_model,
                bf.timeout_secs,
            )
            .with_fallbacks(fallbacks);
            Ok(Arc::new(client))
        }
    }
}
