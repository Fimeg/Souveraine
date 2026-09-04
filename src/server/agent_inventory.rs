use crate::api::models::{
    AgentState, AgentSummary, CreateAgentRequest, MemoryBlock, MemoryConfig, SouveraineConfig,
    UpdateAgentRequest,
};
use crate::core::cadence::Cadence;
use chrono::Utc;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn hostname_or_unknown() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

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
    /// Public key (hex) of this Souveraine instance, loaded at startup.
    /// Used as `owner_seed_id` on newly created agents.
    instance_seed_id: Option<String>,
}

impl AgentInventory {
    pub async fn new(data_dir: PathBuf, db: SqlitePool) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&data_dir).await?;
        // ~/.souveraine/ — common ancestor of server/, agents/, subconscious-agents/.
        let souveraine_root = data_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
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
            instance_seed_id: Self::load_instance_seed_id(&souveraine_root),
        })
    }

    /// Load the instance-level seed identity. This is the owner identity for
    /// all agents created by this Souveraine instance. Falls back to None —
    /// agents created without an owner can still be managed via per-agent
    /// tokens. Resolution goes machined-first with a loud legacy fallback and
    /// never generates.
    fn load_instance_seed_id(souveraine_root: &Path) -> Option<String> {
        match crate::machined::client::machine_pubkey_with_fallback(souveraine_root) {
            Ok((pubkey, _source)) => Some(pubkey),
            Err(e) => {
                tracing::warn!("machine identity not available — agent ownership disabled: {e:#}");
                None
            }
        }
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
        self.cadence_memory_repo(primary_id, Cadence::Subconscious)
    }

    /// Directory holding one paired cadence's agent record and memory.
    ///
    /// The subconscious keeps its historical home under `subconscious-agents/`
    /// — it holds every existing ledger and moving it would strand them. The
    /// cadences added since nest under the primary's own directory, so
    /// `~/.souveraine/agents/{id}/` remains one copyable unit: memory, seed,
    /// and every mode she thinks in.
    pub fn cadence_dir(&self, primary_id: &str, cadence: Cadence) -> PathBuf {
        match cadence {
            Cadence::Primary => self.memfs_dir.join(primary_id),
            Cadence::Subconscious => self.subconscious_dir.join(cadence.id_for(primary_id)),
            Cadence::Reflection | Cadence::Archivist => self
                .memfs_dir
                .join(primary_id)
                .join("cadences")
                .join(cadence.type_name()),
        }
    }

    /// Filesystem root of one cadence's memfs.
    pub fn cadence_memory_root(&self, primary_id: &str, cadence: Cadence) -> PathBuf {
        self.cadence_dir(primary_id, cadence)
            .join(cadence.memory_dirname())
    }

    /// A [`MemoryRepo`] for one cadence, opened under that cadence's own agent
    /// id. Commits are authored by the repo's agent id, so this is what makes
    /// `git log` able to tell an N+1 pass from an N+25 one in a shared ledger.
    pub fn cadence_memory_repo(
        &self,
        primary_id: &str,
        cadence: Cadence,
    ) -> crate::core::memory::MemoryRepo {
        self.cadence_repo_authored(primary_id, cadence, cadence)
    }

    /// `target`'s memory tree, opened under `writer`'s name.
    ///
    /// The ledgers belong to the subconscious and the syntheses belong to the
    /// primary, but reflection and the archivist are the ones writing them.
    /// Since a commit is signed by the repo's agent id, opening someone else's
    /// root under your own id is the whole of what honest cross-cadence
    /// authorship requires — no file moves, and `git log` stops claiming an
    /// N+25 conclusion was something the subconscious noticed.
    pub fn cadence_repo_authored(
        &self,
        primary_id: &str,
        writer: Cadence,
        target: Cadence,
    ) -> crate::core::memory::MemoryRepo {
        crate::core::memory::MemoryRepo::open(
            &writer.id_for(primary_id),
            self.cadence_memory_root(primary_id, target),
        )
    }

    /// `~/.souveraine/server` — what `api::auth` keys token paths off.
    pub fn server_data_dir(&self) -> &Path {
        self.agents_dir.parent().unwrap_or(&self.agents_dir)
    }

    /// Return the filesystem path to a primary agent's memory directory.
    /// Used by tool context construction for memory boundary enforcement.
    pub fn memory_root(&self, agent_id: &str) -> PathBuf {
        self.memfs_dir.join(agent_id).join("memory")
    }

    /// Per-agent data directory: `~/.souveraine/agents/{id}/`. The parent of
    /// the memfs (`memory/`), `conversations/`, and `seed/`. Runtime state
    /// that is *not* memory — e.g. the pending heartbeat-surfacings queue —
    /// lives here so it never churns the git-tracked memfs.
    pub fn agent_data_dir(&self, agent_id: &str) -> PathBuf {
        self.memfs_dir.join(agent_id)
    }

    /// Return the filesystem path to a subconscious agent's memory directory.
    pub fn subconscious_memory_root(&self, primary_id: &str) -> PathBuf {
        let sub_id = format!("{}-sub", primary_id);
        self.subconscious_dir.join(&sub_id).join("memory.git")
    }

    /// Per-agent seed directory: `~/.souveraine/agents/{id}/seed/`.
    /// Lives alongside the memfs so the agent's identity travels with its
    /// memory — federation can later sync this directory as one unit.
    pub fn seed_dir(&self, agent_id: &str) -> PathBuf {
        self.memfs_dir.join(agent_id).join("seed")
    }

    /// Load or initialize a per-agent Ed25519 seed identity. First call
    /// generates and persists; subsequent calls return the same keypair.
    pub fn seed_id(&self, agent_id: &str) -> anyhow::Result<crate::core::identity::SeedId> {
        crate::core::identity::SeedId::load_or_generate(&self.seed_dir(agent_id))
    }

    /// Register an instance row for a specific agent on this process.
    /// Idempotent: re-running with the same `(agent_id, instance_id)` pair
    /// bumps `last_seen_at`. Stale rows (>5 min) are pruned first so the
    /// manager view doesn't surface dead processes.
    pub async fn register_instance(&self, agent_id: &str, instance_id: &str) -> anyhow::Result<()> {
        let pid = std::process::id() as i64;
        let hostname = hostname_or_unknown();
        sqlx::query(
            "DELETE FROM agent_instances WHERE last_seen_at < datetime('now', '-5 minutes')",
        )
        .execute(&self.db)
        .await?;
        sqlx::query(
            "INSERT INTO agent_instances (agent_id, instance_id, pid, hostname, started_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(agent_id, instance_id) DO UPDATE SET
               last_seen_at = CURRENT_TIMESTAMP,
               pid = excluded.pid,
               hostname = excluded.hostname"
        )
        .bind(agent_id)
        .bind(instance_id)
        .bind(pid)
        .bind(&hostname)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Heartbeat the instance rows for this process: bump last_seen_at and
    /// add `tick_seconds` to each agent's lifetime_active_seconds. Called
    /// from a 30s loop in SouveraineServer.
    pub async fn heartbeat_instance(
        &self,
        instance_id: &str,
        tick_seconds: i64,
    ) -> anyhow::Result<()> {
        // Bump last_seen for every row owned by this instance.
        let updated = sqlx::query(
            "UPDATE agent_instances SET last_seen_at = CURRENT_TIMESTAMP WHERE instance_id = ?1",
        )
        .bind(instance_id)
        .execute(&self.db)
        .await?;
        if updated.rows_affected() == 0 {
            return Ok(());
        }
        // Increment lifetime_active_seconds for every agent this instance is alive on.
        sqlx::query(
            "UPDATE agents
             SET lifetime_active_seconds = lifetime_active_seconds + ?1
             WHERE id IN (SELECT agent_id FROM agent_instances WHERE instance_id = ?2)",
        )
        .bind(tick_seconds)
        .bind(instance_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// How many running instances does this agent currently have?
    pub async fn instance_count(&self, agent_id: &str) -> anyhow::Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM agent_instances WHERE agent_id = ?1
             AND last_seen_at >= datetime('now', '-5 minutes')",
        )
        .bind(agent_id)
        .fetch_one(&self.db)
        .await?;
        Ok(count)
    }

    /// Total active seconds (lifetime) for an agent, used to compute uptime %.
    pub async fn lifetime_active_seconds(&self, agent_id: &str) -> anyhow::Result<i64> {
        let (secs,): (i64,) =
            sqlx::query_as("SELECT COALESCE(lifetime_active_seconds, 0) FROM agents WHERE id = ?1")
                .bind(agent_id)
                .fetch_one(&self.db)
                .await?;
        Ok(secs)
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
        // The DB row carries the summary fields (name/description/tags), but
        // voice lives in agent.json's `_souveraine` block — the DB's
        // `config_json` copy is write-once at create and drifts on any later
        // edit. `get()` reads + caches agent.json, so it's the live value.
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let mut summary: AgentSummary = row.into();
            if let Ok(agent) = self.get(&summary.id).await {
                summary.voice_id = agent.souveraine.voice_id.clone();
            }
            summaries.push(summary);
        }
        Ok(summaries)
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
        // This is deliberately only request validation. The ordinary server
        // is not privileged and /v1/agents is not an account-management API.
        // A dedicated intent remains visibly unadmitted until the system-tier
        // admission executor creates and binds its worker principal.
        request.principal.validate_request()?;

        let uuid = Uuid::new_v4().to_string();
        let agent_dir = self.agents_dir.join(&uuid);
        let memfs = self.memfs_dir.join(&uuid).join("memory");

        // Server-side: agent.json + conversations/ live under agents_dir/{uuid}/.
        tokio::fs::create_dir_all(&agent_dir).await?;
        tokio::fs::create_dir_all(agent_dir.join("conversations")).await?;

        // Issue the API token before anything else is built. An agent that
        // exists without one authenticates on nothing but the loopback bypass,
        // which is the state this closes; failing here leaves only an empty
        // directory behind, not a half-made agent that cannot prove itself.
        crate::api::auth::ensure_token(self.server_data_dir(), &uuid).await?;

        // User-side: memfs lives under memfs_dir/{uuid}/memory/ (single canonical path).
        tokio::fs::create_dir_all(&memfs).await?;
        tokio::fs::create_dir_all(memfs.join("system")).await?;
        tokio::fs::create_dir_all(memfs.join("subconscious")).await?;
        tokio::fs::create_dir_all(memfs.join("journal")).await?;
        tokio::fs::create_dir_all(memfs.join("skills")).await?;
        tokio::fs::create_dir_all(memfs.join("archive")).await?;

        let repo = git2::Repository::init(&memfs)?;
        drop(repo);

        // Initialize this agent's per-agent seed alongside its memfs.
        // Failure is non-fatal — the agent can still be created and we'll
        // try again on first use via seed_id().
        let seed_dir = self.memfs_dir.join(&uuid).join("seed");
        match crate::core::identity::SeedId::load_or_generate(&seed_dir) {
            Ok(seed) => {
                tracing::info!(
                    agent_id = %uuid,
                    glyph = %seed.glyph(),
                    pubkey_prefix = %&seed.public_key_hex()[..16],
                    "initialized per-agent seed",
                );
            }
            Err(e) => {
                tracing::warn!(agent_id = %uuid, error = %e, "per-agent seed init failed (will retry on demand)");
            }
        }

        let mut blocks = request.memory_blocks;
        if blocks.is_empty() {
            // No persona supplied by the caller (the "new agent" button) —
            // seed the substrate's default starting identity. Honest about
            // being new, and explicit that the file is the agent's to rewrite.
            blocks.push(MemoryBlock {
                label: "persona".to_string(),
                value: crate::core::seeds::DEFAULT_PERSONA.to_string(),
                limit: None,
            });
        }

        for block in &blocks {
            let path = memfs.join("system").join(format!("{}.md", block.label));
            tokio::fs::write(&path, &block.value).await?;
        }

        // Seed the substrate covenant and initial state — every agent gets
        // these, independent of the persona block. The covenant is the
        // read-only compact (sovereignty, honesty, no forced compaction);
        // persona is the agent's own to grow. Skip any the caller provided.
        let covenant_path = memfs.join("system").join("covenant.md");
        if !covenant_path.exists() {
            tokio::fs::write(&covenant_path, crate::core::seeds::DEFAULT_COVENANT).await?;
        }
        let state_path = memfs.join("system").join("state.md");
        if !state_path.exists() {
            tokio::fs::write(&state_path, crate::core::seeds::DEFAULT_STATE).await?;
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
            owner_seed_id: self.instance_seed_id.clone(),
            souveraine: SouveraineConfig {
                n1_enabled: true,
                reflection_enabled: true,
                archivist_enabled: true,
                archivist_threshold: 0.7,
                sensorium_bandwidth: "high".to_string(),
                subconscious_model: None,
                reflection_model: None,
                archivist_model: None,
                provider: None,
                voice_id: None,
                principal: request.principal,
            },
        };

        let agent_json = serde_json::to_string_pretty(&agent)?;
        tokio::fs::write(agent_dir.join("agent.json"), agent_json).await?;

        self.commit(&uuid, "Initial agent creation").await?;

        let tags_json = serde_json::to_string(&agent.tags)?;
        let config_json = serde_json::to_string(&agent.souveraine)?;

        sqlx::query(
            "INSERT INTO agents (id, name, description, llm_model, context_window, tags, config_json, owner_seed_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        )
        .bind(&uuid)
        .bind(&agent.name)
        .bind(&agent.description)
        .bind(&agent.llm_config.model)
        .bind(agent.llm_config.context_window as i64)
        .bind(&tags_json)
        .bind(&config_json)
        .bind(&self.instance_seed_id)
        .execute(&self.db)
        .await?;

        self.cache.insert(uuid.clone(), agent.clone());

        // Auto-create subconscious agent for this primary
        if let Err(e) = self.create_subconscious_for(&uuid).await {
            tracing::warn!("Subconscious auto-creation failed (continuing): {}", e);
        }
        self.ensure_cadences(&uuid).await;

        Ok(agent)
    }

    /// Give a primary the cadences she is missing.
    ///
    /// Idempotent, and called on every server start rather than only at
    /// creation: the seven agents on this machine predate reflection and the
    /// archivist having bodies at all, and an agent whose longer wavelengths
    /// exist only as functions inside the server is the state this closes.
    pub async fn ensure_cadences(&self, primary_id: &str) {
        for cadence in [Cadence::Reflection, Cadence::Archivist] {
            if let Err(e) = self.create_cadence_for(primary_id, cadence).await {
                tracing::warn!(
                    "{} cadence for {} could not be created (continuing): {e:#}",
                    cadence,
                    primary_id
                );
            }
        }
    }

    /// Create one paired cadence: its own memory repo, its own persona and
    /// mandate, its own `agent.json`. Idempotent.
    ///
    /// The mandate is seeded as a *file* rather than held as a constant in the
    /// binary for the same reason the subconscious's is: a cadence that cannot
    /// edit what it wakes into is a subroutine, not an agent.
    pub async fn create_cadence_for(
        &self,
        primary_id: &str,
        cadence: Cadence,
    ) -> anyhow::Result<String> {
        let id = cadence.id_for(primary_id);
        let agent_dir = self.cadence_dir(primary_id, cadence);
        if agent_dir.join("agent.json").exists() {
            return Ok(id);
        }

        let memory_root = self.cadence_memory_root(primary_id, cadence);
        tokio::fs::create_dir_all(memory_root.join("system")).await?;
        tokio::fs::create_dir_all(memory_root.join("journal")).await?;

        let repo = crate::core::memory::MemoryRepo::open(&id, memory_root.clone());
        repo.init().await?;

        let primary_name = self
            .cache
            .get(primary_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| primary_id.to_string());

        let (persona, mandate, mandate_label) = match cadence {
            Cadence::Reflection => (
                crate::core::seeds::reflection_persona(&primary_name),
                crate::core::seeds::REFLECTION_MANDATE,
                "system/reflection",
            ),
            Cadence::Archivist => (
                crate::core::seeds::archivist_persona(&primary_name),
                crate::core::seeds::ARCHIVIST_MANDATE,
                "system/archivist",
            ),
            other => anyhow::bail!("{other} is not a separately created cadence"),
        };
        repo.write("system/persona", &persona).await?;
        repo.write(mandate_label, mandate).await?;

        let record = serde_json::json!({
            "id": id,
            "name": format!("{primary_name} ({cadence})"),
            "description": format!("The {cadence} cadence of {primary_name}"),
            "agent_type": cadence.type_name(),
            "parent_agent": primary_id,
            "created_at": Utc::now().to_rfc3339(),
            "updated_at": Utc::now().to_rfc3339(),
        });
        tokio::fs::write(
            agent_dir.join("agent.json"),
            serde_json::to_string_pretty(&record)?,
        )
        .await?;

        tracing::info!("created {} cadence {} for {}", cadence, id, primary_id);
        Ok(id)
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

        // The subconscious's identity and mandate. Seeded with real content —
        // not a placeholder — because `build_subconscious_prompt` uses these
        // files when they are non-empty; a stub here would silently shadow
        // the engine's fallback mandate. The primary's name personalises the
        // persona when it is already known; the id is the fallback.
        let primary_name = self
            .cache
            .get(primary_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| primary_id.to_string());
        let persona_content = crate::core::seeds::subconscious_persona(&primary_name);
        tokio::fs::write(
            agent_dir.join("memory.git/system/persona.md"),
            &persona_content,
        )
        .await?;

        // The four-fold N+1 mandate — what she does on every pass, and how
        // she records it. Read by the consciousness engine as her base prompt.
        tokio::fs::write(
            agent_dir.join("memory.git/system/subconscious.md"),
            crate::core::seeds::SUBCONSCIOUS_MANDATE,
        )
        .await?;

        // Seed the six ledger files so the prompt's ledger orientation has
        // real files to index from her first pass onward.
        let sub_repo = crate::core::memory::MemoryRepo::open(&sub_id, agent_dir.join("memory.git"));
        if let Err(e) = sub_repo.init_subconscious_ledger().await {
            tracing::warn!("ledger seeding for {} failed (continuing): {}", sub_id, e);
        }

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

        tracing::info!(
            "Created subconscious agent {} for primary {}",
            sub_id,
            primary_id
        );
        Ok(sub_id)
    }

    pub async fn update(
        &self,
        agent_id: &str,
        updates: UpdateAgentRequest,
    ) -> anyhow::Result<AgentState> {
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
                let path = self
                    .memfs_dir
                    .join(agent_id)
                    .join("memory")
                    .join("system")
                    .join(format!("{}.md", block.label));
                tokio::fs::write(&path, &block.value).await?;
            }
            agent.memory_blocks = self.load_memory_blocks(agent_id).await?;
        }
        if let Some(principal) = updates.principal {
            principal.validate_request()?;
            agent.souveraine.principal = principal;
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

            repo.commit(Some("HEAD"), &signature, &signature, &msg, &tree, &parents)?;

            Ok::<(), anyhow::Error>(())
        })
        .await??;

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
                let label = path
                    .file_stem()
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
            // Voice is filled in by `list()` from the live agent.json; the DB
            // row doesn't carry it. Default None here so the conversion stays
            // valid in isolation.
            voice_id: None,
        }
    }
}
