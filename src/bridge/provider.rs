//! The `LlmProvider` trait — the engine↔LLM seam.
//!
//! This is the inference-side sibling of the harness↔engine [`Backend`] trait
//! (`crate::backend`). Where `Backend` decides *where the engine runs* (in
//! process vs. a remote `souveraine server`), `LlmProvider` decides *who
//! answers the model call*. Everything the engine speaks is the existing
//! OpenAI-chat currency (`ChatCompletionRequest` / `CompletionResult` /
//! `InferenceStrain`); an implementation that talks a different wire format
//! (e.g. the ChatGPT Responses API) adapts internally and hands back that same
//! currency, so the primary loop, subconscious, archivist, reflection, and
//! compaction paths never learn which provider they are talking to.

use anyhow::Result;
use async_trait::async_trait;

use super::bifrost::{ChatCompletionRequest, CompletionResult, InferenceStrain};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable provider id for logs / routing, e.g. `"bifrost"` or `"openai-oauth"`.
    fn id(&self) -> &str;

    /// The model used when a request does not pin one of its own.
    fn default_model(&self) -> &str;

    /// Models this provider can drive — feeds the `/model` picker and
    /// `ModelRouter`'s merged catalog.
    async fn list_models(&self) -> Result<Vec<String>>;

    /// The one inference verb. Returns the completion plus any strain the body
    /// felt (retries, rate limits) so the organism can register provider health
    /// on the event bus / TUI.
    async fn chat_completion_with_strain(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<(CompletionResult, Vec<InferenceStrain>)>;

    /// Convenience wrapper that drops the strain channel.
    async fn chat_completion(&self, request: ChatCompletionRequest) -> Result<CompletionResult> {
        Ok(self.chat_completion_with_strain(request).await?.0)
    }
}
