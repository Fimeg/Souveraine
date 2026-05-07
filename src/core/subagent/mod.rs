use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;

use crate::core::config::ConsciousnessConfig;

/// Subagent Pool — fork/spawn system. Stub: lifecycle TBD in Stage 5.
pub struct SubagentPool {
    #[allow(dead_code)]
    config: Arc<RwLock<ConsciousnessConfig>>,
}

impl SubagentPool {
    pub async fn new(config: Arc<RwLock<ConsciousnessConfig>>) -> Result<Self> {
        Ok(Self { config })
    }
}
