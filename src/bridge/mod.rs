/// Bridge module — LLM inference client
///
/// Connects Souveraine to inference providers (Bifrost, Ollama, etc.)
/// Abstraction over HTTP providers with streaming support.
pub mod bifrost;
pub mod model_router;

pub use bifrost::BifrostClient;
pub use model_router::ModelRouter;
