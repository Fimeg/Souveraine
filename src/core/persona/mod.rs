use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{info, warn, debug};

use crate::core::config::ConsciousnessConfig;
use crate::core::memory::GitMemory;

pub struct PersonaRouter {
    config: Arc<RwLock<ConsciousnessConfig>>,
    memory: Arc<GitMemory>,
    agents_base: PathBuf,
    active_persona: RwLock<String>,
    cache: RwLock<Vec<AgentConfig>>,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub config: AgentYamlConfig,
    pub persona_prompt: Option<String>,
    pub agent_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentYamlConfig {
    pub persona: PersonaDefinition,
    #[serde(default)]
    pub memory: AgentMemoryConfig,
    #[serde(default)]
    pub aster: AsterConfig,
    #[serde(default)]
    pub chains: ChainsConfig,
    pub matrix: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PersonaDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub triggers: Option<TriggersConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TriggersConfig {
    pub matrix: Option<MatrixTriggers>,
    pub project: Option<ProjectTriggers>,
    pub keywords: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixTriggers {
    pub rooms: Option<Vec<String>>,
    pub users: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectTriggers {
    pub paths: Option<Vec<String>>,
    #[serde(rename = "filePatterns")]
    pub file_patterns: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentMemoryConfig {
    #[serde(default)]
    pub git_remote: Option<String>,
    #[serde(default)]
    pub auto_sync: Option<bool>,
    #[serde(default)]
    pub blocks: Option<MemoryBlocksConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryBlocksConfig {
    #[serde(default)]
    pub system: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AsterConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub audit_interval: Option<u64>,
    #[serde(default)]
    pub reflection_interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChainsConfig {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub background: Option<Vec<String>>,
}

impl PersonaRouter {
    pub async fn new(
        config: Arc<RwLock<ConsciousnessConfig>>,
        memory: Arc<GitMemory>,
    ) -> Result<Self> {
        let base_path = config.read().await
            .memory.base_path
            .clone()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .expect("Home dir")
                    .join(".pi/unified")
            });

        let agents_base = base_path.join("agents");

        debug!("Persona Router initialized — scanning: {}", agents_base.display());

        Ok(Self {
            config,
            memory,
            agents_base,
            active_persona: RwLock::new("system".to_string()),
            cache: RwLock::new(Vec::new()),
        })
    }

    pub async fn list_personas(&self) -> Result<Vec<AgentConfig>> {
        let cached = self.cache.read().await;
        if !cached.is_empty() {
            return Ok(cached.clone());
        }
        drop(cached);

        let mut agents = Vec::new();

        if !self.agents_base.exists() {
            warn!("Agents directory not found: {}", self.agents_base.display());
            return Ok(agents);
        }

        let mut entries = tokio::fs::read_dir(&self.agents_base).await
            .with_context(|| format!("Reading agents directory: {}", self.agents_base.display()))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let config_path = path.join("config.yaml");
            if !config_path.exists() {
                continue;
            }

            match self.load_agent_from_dir(&path).await {
                Ok(Some(agent)) => agents.push(agent),
                Ok(None) => {}
                Err(e) => warn!("Failed to load agent from {}: {}", path.display(), e),
            }
        }

        agents.sort_by(|a, b| a.config.persona.name.cmp(&b.config.persona.name));

        let mut cache = self.cache.write().await;
        *cache = agents.clone();
        Ok(agents)
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let agents = self.list_personas().await?;
        Ok(agents.into_iter().find(|a| a.config.persona.name.eq_ignore_ascii_case(name)))
    }

    pub async fn detect_persona(&self, cwd: &Path) -> Result<String> {
        let agents = self.list_personas().await?;
        
        let cwd_str = cwd.to_string_lossy();
        for agent in &agents {
            if let Some(ref triggers) = agent.config.persona.triggers {
                if let Some(ref project) = triggers.project {
                    if let Some(ref paths) = project.paths {
                        for path in paths {
                            if cwd_str.contains(path) {
                                return Ok(agent.config.persona.name.clone());
                            }
                        }
                    }
                }
            }
        }
        
        Ok("Ani".to_string())
    }

    async fn load_agent_from_dir(&self, path: &Path) -> Result<Option<AgentConfig>> {
        let config_path = path.join("config.yaml");
        let config_content = tokio::fs::read_to_string(&config_path).await?;
        let config: AgentYamlConfig = serde_yaml::from_str(&config_content)?;

        let persona_path = path.join("memory").join("system").join("persona.md");
        let persona_prompt = if persona_path.exists() {
            Some(tokio::fs::read_to_string(&persona_path).await?)
        } else {
            None
        };

        Ok(Some(AgentConfig {
            config,
            persona_prompt,
            agent_dir: path.to_path_buf(),
        }))
    }
}
