#![allow(dead_code)] // WIP scaffolding not yet wired
//! The model catalog for the OAuth provider.
//!
//! The ChatGPT codex backend exposes no usable `/v1/models` listing, so — like
//! Letta — we ship a curated list of models a ChatGPT Plus/Pro/Team
//! subscription can drive through `backend-api/codex`. The `/models` *poll*
//! lives on the OpenAI-compatible (gateway) provider; this is its OAuth-side
//! counterpart.

/// `(model id, context window tokens)`.
pub const MODELS: &[(&str, usize)] = &[
    ("gpt-5.5", 272_000),
    ("gpt-5.5-codex", 272_000),
    ("gpt-5.4", 272_000),
    ("gpt-5.4-codex", 272_000),
    ("gpt-5.1", 272_000),
    ("gpt-5.1-codex", 272_000),
    ("gpt-4o", 128_000),
    ("o3", 200_000),
    ("o4-mini", 200_000),
];

/// Used when neither the requested model nor the configured fallback names a
/// model this backend serves.
pub const DEFAULT_MODEL: &str = "gpt-5.5";

pub fn model_ids() -> Vec<String> {
    MODELS.iter().map(|(name, _)| (*name).to_string()).collect()
}

pub fn context_window(model: &str) -> usize {
    MODELS
        .iter()
        .find(|(name, _)| *name == model)
        .map(|(_, ctx)| *ctx)
        .unwrap_or(128_000)
}

pub fn is_known(model: &str) -> bool {
    MODELS.iter().any(|(name, _)| *name == model)
}

/// Strip a gateway routing prefix (`"openai/gpt-5.5"` → `"gpt-5.5"`) and
/// Bifrost's `-precision` fallback suffix, which the ChatGPT backend has no
/// concept of. A trailing `-fast` is preserved so the payload builder can map
/// it to a priority service tier.
fn strip_routing(requested: &str) -> &str {
    let bare = requested.rsplit('/').next().unwrap_or(requested);
    bare.strip_suffix("-precision").unwrap_or(bare)
}

/// Resolve an engine-supplied model id onto one the ChatGPT codex backend
/// actually serves.
///
/// The engine is provider-agnostic and threads Bifrost-namespaced ids
/// (`openai/kimi-k2.6`, hardcoded subsystem fallbacks like `openai/glm-5.1`,
/// etc.) through every call. Those are meaningless to the codex backend, so the
/// provider translates at its boundary: strip the routing prefix/suffix, and if
/// the result still isn't in the catalog, fall back to `fallback` (the
/// provider's own default), then to [`DEFAULT_MODEL`]. This keeps the OAuth
/// provider runnable no matter what vocabulary the engine speaks.
pub fn resolve(requested: &str, fallback: &str) -> String {
    let bare = strip_routing(requested);
    let base = bare.strip_suffix("-fast").unwrap_or(bare);
    if is_known(base) {
        return bare.to_string();
    }
    let fb = strip_routing(fallback);
    let fb_base = fb.strip_suffix("-fast").unwrap_or(fb);
    if is_known(fb_base) {
        fb.to_string()
    } else {
        DEFAULT_MODEL.to_string()
    }
}
