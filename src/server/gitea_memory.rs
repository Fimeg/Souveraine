#![allow(dead_code)] // WIP scaffolding not yet wired
//! Gitea-backed Memory
//!
//! Send-safe alternative to GitMemory using Gitea HTTP API.
//! All operations go through REST API calls instead of libgit2.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::config::ConsciousnessConfig;
use crate::server::gitea_client::GiteaClient;

#[derive(Clone)]
pub struct GiteaMemory {
    client: GiteaClient,
    config: Arc<RwLock<ConsciousnessConfig>>,
    /// Local cache of file contents (path -> content)
    cache: Arc<RwLock<HashMap<String, String>>>,
}

impl GiteaMemory {
    /// Create new GiteaMemory from config
    ///
    /// Expects config.gitea.url and config.gitea.token
    pub async fn new(config: Arc<RwLock<ConsciousnessConfig>>) -> Result<Self> {
        let cfg = config.read().await;

        // Get Gitea URL from config - fall back to env var or default
        let gitea_url = std::env::var("SOUVERAINE_GITEA_URL").unwrap_or_else(|_| {
            // Default to localhost Gitea
            "http://localhost:3000".to_string()
        });

        let gitea_token = std::env::var("SOUVERAINE_GITEA_TOKEN")
            .map_err(|_| anyhow!("SOUVERAINE_GITEA_TOKEN env var required for server mode"))?;

        drop(cfg);

        let client = GiteaClient::new(gitea_url, gitea_token);

        Ok(Self {
            client,
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Read file from Gitea
    pub async fn read(&self, agent_id: &str, path: &str) -> Result<Option<String>> {
        // Try cache first
        let cache_key = format!("{}/{}", agent_id, path);
        {
            let cache = self.cache.read().await;
            if let Some(content) = cache.get(&cache_key) {
                return Ok(Some(content.clone()));
            }
        }

        // Fetch from Gitea
        match self.client.get_file(agent_id, path).await? {
            Some(content) => {
                // Update cache
                let mut cache = self.cache.write().await;
                cache.insert(cache_key, content.clone());
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }

    /// Write file to Gitea
    pub async fn write(&self, agent_id: &str, path: &str, content: &str) -> Result<()> {
        let cache_key = format!("{}/{}", agent_id, path);

        // Ensure repo exists
        if !self.client.repo_exists(agent_id).await? {
            self.client.create_repo(agent_id).await?;
        }

        // Write to Gitea
        self.client
            .put_file(agent_id, path, content, &format!("Update {}", path))
            .await?;

        // Update cache
        let mut cache = self.cache.write().await;
        cache.insert(cache_key, content.to_string());

        Ok(())
    }

    /// Append to file in Gitea
    pub async fn append(&self, agent_id: &str, path: &str, content: &str) -> Result<()> {
        let existing = self.read(agent_id, path).await?;
        let new_content = match existing {
            Some(mut existing) => {
                if !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing + content
            }
            None => content.to_string(),
        };
        self.write(agent_id, path, &new_content).await
    }

    /// List files in agent's memory
    pub async fn list(&self, agent_id: &str, path: &str) -> Result<Vec<String>> {
        self.client.list_files(agent_id, path).await
    }

    /// Sync all memory from Gitea to local cache
    pub async fn sync_from_gitea(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let files = self.list(agent_id, "").await?;
        let mut synced = Vec::new();

        for file in files {
            if let Some(content) = self.read(agent_id, &file).await? {
                synced.push((file, content));
            }
        }

        Ok(synced)
    }
}

// The blanket `impl Memory for GiteaMemory` lived here. It will return when
// the `Memory` trait is reintroduced in `crates/memory` (Stage 1). For now
// GiteaMemory exposes its inherent agent-scoped methods directly.
