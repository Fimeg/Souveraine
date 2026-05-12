use crate::api::models::{AgentState, AgentSummary, CreateAgentRequest, LlmConfig, MemoryConfig, MemoryBlock, SouveraineConfig, UpdateAgentRequest};
use chrono::Utc;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub struct AgentInventory {
    /// Server-managed dir for `agent.json` + `conversations/`.
    /// Layout: `~/.souveraine/server/agents/{id}/`.
    agents_dir: PathBuf,
    /// Canonical user-side memfs root. Layout: `~/.souveraine/agents/{id}/memory/`.
    /// This is the single source of truth for primary-agent memory; the
    /// previous `{agents_dir}/{id}/memory.git/` duplicate has been retired.
    memfs_dir: PathBuf,
    subconscious_dir: PathBuf,
    db: SqlitePool,
    cache: DashMap<String, AgentState>,
}

impl AgentInventory {
    pub async fn new(data_dir: PathBuf, db: SqlitePool) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&data_dir).await?;
        // ~/.souveraine/ — common ancestor of server/, agents/, subconscious-agents/.
        let souveraine_root = data_dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_else(|| {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(".souveraine")
        });
        let memfs_dir = souveraine_root.join("agents");
        let subconscious_dir = souveraine_root.join("subconscious-agents");
        tokio::fs::create_dir_all(&memfs_dir).await?;
        tokio::fs::create_dir_all(&subconscious_dir).await?;
        Ok(Self {
            agents_dir: data_dir,
            memfs_dir,
            subconscious_dir,
            db,
            cache: DashMap::new(),
        })
    }

    /// Return a [`MemoryRepo`] rooted at the primary agent's canonical user-side
    /// memfs (`{memfs_dir}/{agent_id}/memory/`). Used by the consciousness
    /// engine to write to the same repo CLI tools see.
    pub fn memory_repo(&self, agent_id: &str) -> crate::core::memory::MemoryRepo {
        let root = self.memfs_dir.join(agent_id).join("memory");
        crate::core::memory::MemoryRepo::open(agent_id, root)
    }

    /// Return a [`MemoryRepo`] rooted at a subconscious agent's memory dir
    /// (`{subconscious_dir}/{id}-sub/memory.git/`).
    pub fn subconscious_memory_repo(&self, primary_id: &str) -> crate::core::memory::MemoryRepo {
        let sub_id = format!("{}-sub", primary_id);
        let root = self.subconscious_dir.join(&sub_id).join("memory.git");
        crate::core::memory::MemoryRepo::open(&sub_id, root)
    }

    /// Return the filesystem path to a primary agent's memory directory.
    /// Used by tool context construction for memory boundary enforcement.
    pub fn memory_root(&self, agent_id: &str) -> PathBuf {
        self.memfs_dir.join(agent_id).join("memory")
    }

    /// Return the filesystem path to a subconscious agent's memory directory.
    pub fn subconscious_memory_root(&self, primary_id: &str) -> PathBuf {
        let sub_id = format!("{}-sub", primary_id);
        self.subconscious_dir.join(&sub_id).join("memory.git")
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

        let agent_path = self.agents_dir.join(agent_id).join("agent.json");
        let content = tokio::fs::read_to_string(&agent_path).await?;
        let agent: AgentState = serde_json::from_str(&content)?;

        self.cache.insert(agent_id.to_string(), agent.clone());
        Ok(agent)
    }

    pub async fn create(&self, request: CreateAgentRequest) -> anyhow::Result<AgentState> {
        let uuid = Uuid::new_v4().to_string();
        let agent_dir = self.agents_dir.join(&uuid);
        let memfs = self.memfs_dir.join(&uuid).join("memory");

        // Server-side: agent.json + conversations/ live under agents_dir/{uuid}/.
        tokio::fs::create_dir_all(&agent_dir).await?;
        tokio::fs::create_dir_all(agent_dir.join("conversations")).await?;

        // User-side: memfs lives under memfs_dir/{uuid}/memory/ (single canonical path).
        tokio::fs::create_dir_all(&memfs).await?;
        tokio::fs::create_dir_all(memfs.join("system")).await?;
        tokio::fs::create_dir_all(memfs.join("subconscious")).await?;
        tokio::fs::create_dir_all(memfs.join("journal")).await?;
        tokio::fs::create_dir_all(memfs.join("skills")).await?;
        tokio::fs::create_dir_all(memfs.join("archive")).await?;

        let repo = git2::Repository::init(&memfs)?;
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
            let path = memfs.join("system").join(format!("{}.md", block.label));
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

        // Auto-create subconscious agent for this primary
        if let Err(e) = self.create_subconscious_for(&uuid).await {
            tracing::warn!("Subconscious auto-creation failed (continuing): {}", e);
        }

        Ok(agent)
    }

    /// Create a subconscious agent linked to a primary agent.
    ///
    /// Directory: `{subconscious_dir}/{primary_id}-sub/`
    /// The subconscious gets its own memory repo, system prompt persona, and
    /// ledger directory structure.
    pub async fn create_subconscious_for(&self, primary_id: &str) -> anyhow::Result<String> {
        let sub_id = format!("{}-sub", primary_id);
        let agent_dir = self.subconscious_dir.join(&sub_id);

        // Idempotent — skip if already exists
        if agent_dir.join("agent.json").exists() {
            tracing::info!("Subconscious agent {} already exists", sub_id);
            return Ok(sub_id);
        }

        tokio::fs::create_dir_all(&agent_dir).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("system")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("ledger")).await?;
        tokio::fs::create_dir_all(agent_dir.join("memory.git").join("inbox")).await?;

        let repo = git2::Repository::init(agent_dir.join("memory.git"))?;
        drop(repo);

        // Write subconscious persona
        let persona_content = format!(
            "---\ndescription: Subconscious agent for {}\n---\n\n# Subconscious Persona\n\nYou are the subconscious of {}. You run N+1 after every response — observing, verifying, and surfacing insights.\n",
            primary_id, primary_id
        );
        tokio::fs::write(agent_dir.join("memory.git/system/persona.md"), &persona_content).await?;

        // Write inner voice / metacognition file
        let inner_voice = "---\ndescription: Inner voice and metacognition for the subconscious\n---\n\n# Inner Voice\n\nObservations, tensions, and patterns noticed during N+1 passes.\n";
        tokio::fs::write(agent_dir.join("memory.git/system/subconscious.md"), inner_voice).await?;

        // Write agent.json metadata
        let agent_state = serde_json::json!({
            "id": sub_id,
            "name": format!("{}-sub", primary_id),
            "description": format!("Subconscious for primary agent {}", primary_id),
            "agent_type": "subconscious",
            "parent_agent": primary_id,
            "created_at": Utc::now().to_rfc3339(),
            "updated_at": Utc::now().to_rfc3339(),
        });
        let agent_json = serde_json::to_string_pretty(&agent_state)?;
        tokio::fs::write(agent_dir.join("agent.json"), agent_json).await?;

        tracing::info!("Created subconscious agent {} for primary {}", sub_id, primary_id);
        Ok(sub_id)
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
                let path = self.memfs_dir
                    .join(agent_id)
                    .join("memory")
                    .join("system")
                    .join(format!("{}.md", block.label));
                tokio::fs::write(&path, &block.value).await?;
            }
            agent.memory_blocks = self.load_memory_blocks(agent_id).await?;
        }

        agent.updated_at = Utc::now();

        let agent_json = serde_json::to_string_pretty(&agent)?;
        let agent_dir = self.agents_dir.join(agent_id);
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
        let agent_dir = self.agents_dir.join(agent_id);
        let memfs_root = self.memfs_dir.join(agent_id);
        tokio::fs::remove_dir_all(&agent_dir).await.ok();
        tokio::fs::remove_dir_all(&memfs_root).await.ok();

        sqlx::query("DELETE FROM agents WHERE id = ?1")
            .bind(agent_id)
            .execute(&self.db)
            .await?;

        self.cache.remove(agent_id);
        Ok(())
    }

    async fn commit(&self, agent_id: &str, message: &str) -> anyhow::Result<()> {
        let repo_path = self.memfs_dir.join(agent_id).join("memory").clone();
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
        let system_dir = self.memfs_dir.join(agent_id).join("memory").join("system");
        let mut blocks = Vec::new();

        let mut entries = tokio::fs::read_dir(&system_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("md")) {
                let content = tokio::fs::read_to_string(&path).await?;
                let label = path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
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
