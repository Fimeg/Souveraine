#![allow(dead_code)] // WIP scaffolding not yet wired
//! Local cache for the model catalog fetched from the provider.
//!
//! Stored at `~/.souveraine/models.json`. Avoids a network round-trip on every
//! settings open — the picker shows cached results immediately, and a manual
//! refresh (`[r]`) updates the cache.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Maximum age before a cache is considered stale. Currently 7 days — models
/// don't churn that fast, and a stale cache is still better than an empty list.
const FRESHNESS: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCache {
    /// ISO 8601 timestamp of when the fetch succeeded.
    pub fetched_at: String,
    /// Provider id that produced this list ("bifrost", "openai-oauth").
    pub provider: String,
    /// Full model id list, e.g. `["openai/gpt-4", "anthropic/claude-3-opus", ...]`.
    pub models: Vec<String>,
}

impl ModelCache {
    /// Canonical path: `~/.souveraine/models.json`.
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".souveraine")
            .join("models.json")
    }

    /// Load the cache from disk. Returns `None` if the file is missing,
    /// unparseable, or belongs to a different provider.
    pub fn load(expected_provider: &str) -> Option<Self> {
        let path = Self::path();
        let data = std::fs::read_to_string(&path).ok()?;
        let cache: Self = serde_json::from_str(&data).ok()?;
        if cache.provider != expected_provider {
            return None;
        }
        Some(cache)
    }

    /// Write the cache to disk. Creates the parent directory if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Returns `true` if the cache was fetched within the last 7 days.
    pub fn is_fresh(&self) -> bool {
        let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&self.fetched_at) else {
            return false;
        };
        let age = chrono::Utc::now().signed_duration_since(parsed);
        age.to_std().map(|d| d < FRESHNESS).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cache = ModelCache {
            fetched_at: chrono::Utc::now().to_rfc3339(),
            provider: "bifrost".to_string(),
            models: vec!["openai/gpt-4".into(), "anthropic/claude-3".into()],
        };
        let json = serde_json::to_string(&cache).unwrap();
        let loaded: ModelCache = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.models.len(), 2);
        assert_eq!(loaded.provider, "bifrost");
        assert!(loaded.is_fresh());
    }

    #[test]
    fn stale_cache() {
        let cache = ModelCache {
            fetched_at: "2020-01-01T00:00:00Z".to_string(),
            provider: "bifrost".to_string(),
            models: vec![],
        };
        assert!(!cache.is_fresh());
    }
}
