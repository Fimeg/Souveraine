#![allow(dead_code)] // WIP scaffolding not yet wired
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::config::ConsciousnessConfig;

/// Chain Orchestrator — Talking vs Thinking. Stub: implementation deferred.
pub struct ChainOrchestrator {
    #[allow(dead_code)]
    config: Arc<RwLock<ConsciousnessConfig>>,
    #[allow(dead_code)]
    talking_enabled: bool,
    #[allow(dead_code)]
    thinking_enabled: bool,
}

impl ChainOrchestrator {
    pub async fn new(
        config: Arc<RwLock<ConsciousnessConfig>>,
        talking_enabled: bool,
        thinking_enabled: bool,
    ) -> Result<Self> {
        Ok(Self {
            config,
            talking_enabled,
            thinking_enabled,
        })
    }
}
