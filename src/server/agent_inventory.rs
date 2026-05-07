use crate::api::models::{AgentState, AgentSummary, CreateAgentRequest, LlmConfig, MemoryConfig, MemoryBlock, SouveraineConfig, UpdateAgentRequest};
use chrono::Utc;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub struct AgentInventory {
    data_dir: PathBuf,
    db: SqlitePool,
    cache: DashMap<String, AgentState>,
}

impl AgentInventory {
    pub async fn new(data_dir: PathBuf, db: SqlitePool) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&data_dir).await?;
        Ok(Self {
            data_dir,
            db,
            cache: DashMap::new(),
        })
    }

    /// Return a [`MemoryRepo`] rooted at the agent's existing memory dir
    /// (`{data_dir}/{agent_id}/memory.git/`). Used by the consciousness engine
    /// to write to the same repo that [`Self::create`] initialized.
    pub fn memory_repo(&self, agent_id: &str) -> crate::core::memory::MemoryRepo {
        let root = self.data_dir.join(agent_id).join("memory.git");
        crate::core::memory::MemoryRepo::open(agent_id, root)
    }

    pub async fn list(&self, filters: Option<String>) -> anyhow::Result<Vec<AgentSummary>> {
        let query = if let Some(filter) = filters {
            sqlx::query_as::<_, AgentSummaryRow>(
                "SELECT id, name, description, created_at, updated_at, tags FROM agents WHERE name LIKE ?1 OR tags LIKE ?1 ORDER BY updated_at DESC"
            )
            .bind(format!("%{filter}%"))
        } else {
            sqlx::query_as::<_, AgentSummaryRow>(
                "SELECT id, name, description, created_at, updated_at, tags FROM agents ORDER BY updated_at DESC"
            )
        };

        let rows = query.fetch_all(&self.db).await?;
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    pub async fn get(&self, agent_id: &str) -> anyhow::Result<AgentState> {
        if let Some(agent) = self.cache.get(agent_id) {
            return Ok(agent.clone());
        }

        let agent_path = self.data_dir.join(agent_id).join("agent.json");
        let content = tokio::fs::read_to_string(&agent_path).await?;
        let agent: AgentState = serde_json::from_str(&content)?;

        self.cache.insert(agent_id.to_string(), agent.clone());
        Ok(agent)
    }

    pub async fn create(&self, request: CreateAgentRequest) -> anyhow::Result<AgentState> {
        let uuid = Uuid::new_v4().to_string();
        let agent_dir = self.data_dir.join(&uuid);

        tokio::fs::create_dir_all(&agent_dir).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("system")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("subconscious")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("journal")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("skills")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("archive")).await?;
        tokio::fs::create_dir_all(agent_dir.join("conversations")).await?;

        let repo = git2::Repository::init(agent_dir.join("memory.git"))?;
        drop(repo);

        let mut blocks = request.memory_blocks;
        if blocks.is_empty() {
            blocks.push(MemoryBlock {
                label: "persona".to_string(),
                value: "You are a helpful AI assistant.".to_string(),
                limit: None,
            });
        }

        for block in &blocks {
            let path = agent_dir.join("memory.git").join("system").join(format!("{}.md", block.label));
            tokio::fs::write(&path, &block.value).await?;
        }

        let agent = AgentState {
            id: uuid.clone(),
            name: request.name,
            description: request.description,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            llm_config: request.llm_config,
            memory: MemoryConfig {
                git_enabled: true,
                auto_commit: true,
                context_window: None,
            },
            memory_blocks: blocks,
            tools: request.tools,
            tags: request.tags,
            souveraine: SouveraineConfig {
                n1_enabled: true,
                reflection_enabled: true,
                archivist_enabled: true,
                archivist_threshold: 0.7,
                sensorium_bandwidth: "high".to_string(),
            },
        };

        let agent_json = serde_json::to_string_pretty(&agent)?;
        tokio::fs::write(agent_dir.join("agent.json"), agent_json).await?;

        self.commit(&uuid, "Initial agent creation").await?;

        let tags_json = serde_json::to_string(&agent.tags)?;
        let config_json = serde_json::to_string(&agent.souveraine)?;

        sqlx::query(
            "INSERT INTO agents (id, name, description, llm_model, context_window, tags, config_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        )
        .bind(&uuid)
        .bind(&agent.name)
        .bind(&agent.description)
        .bind(&agent.llm_config.model)
        .bind(agent.llm_config.context_window as i64)
        .bind(&tags_json)
        .bind(&config_json)
        .execute(&self.db)
        .await?;

        self.cache.insert(uuid.clone(), agent.clone());
        Ok(agent)
    }

    pub async fn update(&self, agent_id: &str, updates: UpdateAgentRequest) -> anyhow::Result<AgentState> {
        let mut agent = self.get(agent_id).await?;

        if let Some(name) = updates.name {
            agent.name = name;
        }
        if let Some(desc) = updates.description {
            agent.description = Some(desc);
        }
        if let Some(llm_config) = updates.llm_config {
            agent.llm_config = llm_config;
        }
        if let Some(blocks) = updates.memory_blocks {
            for block in blocks {
                let path = self.data_dir
                    .join(agent_id)
                    .join("memory.git")
                    .join("system")
                    .join(format!("{}.md", block.label));
                tokio::fs::write(&path, &block.value).await?;
            }
            agent.memory_blocks = self.load_memory_blocks(agent_id).await?;
        }

        agent.updated_at = Utc::now();

        let agent_json = serde_json::to_string_pretty(&agent)?;
        let agent_dir = self.data_dir.join(agent_id);
        tokio::fs::write(agent_dir.join("agent.json"), agent_json).await?;

        let tags_json = serde_json::to_string(&agent.tags)?;
        sqlx::query(
            "UPDATE agents SET name = ?1, description = ?2, llm_model = ?3, context_window = ?4, tags = ?5, updated_at = CURRENT_TIMESTAMP WHERE id = ?6"
        )
        .bind(&agent.name)
        .bind(&agent.description)
        .bind(&agent.llm_config.model)
        .bind(agent.llm_config.context_window as i64)
        .bind(&tags_json)
        .bind(agent_id)
        .execute(&self.db)
        .await?;

        self.cache.insert(agent_id.to_string(), agent.clone());
        Ok(agent)
    }

    pub async fn delete(&self, agent_id: &str) -> anyhow::Result<()> {
        let agent_dir = self.data_dir.join(agent_id);
        tokio::fs::remove_dir_all(&agent_dir).await.ok();

        sqlx::query("DELETE FROM agents WHERE id = ?1")
            .bind(agent_id)
            .execute(&self.db)
            .await?;

        self.cache.remove(agent_id);
        Ok(())
    }

    async fn commit(&self, agent_id: &str, message: &str) -> anyhow::Result<()> {
        let repo_path = self.data_dir.join(agent_id).join("memory.git").clone();
        let msg = message.to_string();

        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
            index.write()?;

            let signature = git2::Signature::now("Souveraine", "agent@souveraine.ai")?;
            let tree_id = index.write_tree()?;
            let tree = repo.find_tree(tree_id)?;

            let parent = match repo.head() {
                Ok(head) => Some(head.peel_to_commit()?),
                Err(_) => None,
            };

            let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();

            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                &msg,
                &tree,
                &parents,
            )?;

            Ok::<(), anyhow::Error>(())
        }).await??;

        Ok(())
    }

    async fn load_memory_blocks(&self, agent_id: &str) -> anyhow::Result<Vec<MemoryBlock>> {
        let system_dir = self.data_dir.join(agent_id).join("memory.git").join("system");
        let mut blocks = Vec::new();

        let mut entries = tokio::fs::read_dir(&system_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("md")) {
                let content = tokio::fs::read_to_string(&path).await?;
                let label = path.file_stem().unwrap().to_string_lossy().to_string();
                blocks.push(MemoryBlock {
                    label,
                    value: content,
                    limit: None,
                });
            }
        }

        Ok(blocks)
    }
}

#[derive(sqlx::FromRow)]
struct AgentSummaryRow {
    id: String,
    name: String,
    description: Option<String>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    tags: String,
}

impl From<AgentSummaryRow> for AgentSummary {
    fn from(row: AgentSummaryRow) -> Self {
        let tags: Vec<String> = serde_json::from_str(&row.tags).unwrap_or_default();
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            created_at: chrono::DateTime::from_naive_utc_and_offset(row.created_at, chrono::Utc),
            updated_at: chrono::DateTime::from_naive_utc_and_offset(row.updated_at, chrono::Utc),
            tags,
        }
    }
}
