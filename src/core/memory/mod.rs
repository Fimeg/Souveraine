//! Memory — The agent's git-backed, frontmatter-aware memory filesystem.
//!
//! Every agent has a memory directory at `~/.souveraine/agents/{id}/memory/`
//! containing markdown files with YAML frontmatter, tracked in git.
//!
//! The `memory` tool exposes this to the agent as a unified subcommand interface:
//!
//! ```text
//! memory read system/persona
//! memory write system/persona "new content"
//! memory append journal/2026-05-06 "new entry"
//! memory ls system/
//! memory init
//! memory status
//! memory compact --strategy sliding-window
//! ```
//!
//! Design follows the Letta Code memory tool pattern:
//! - All files require YAML frontmatter with `description`
//! - `read_only: true` in frontmatter blocks writes
//! - Every write is a git commit (auto-commit)
//! - Paths are relative to the agent's memory directory

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use crate::core::compact::CompactionStrategyKind;
use crate::core::tools::defs::ToolContext;
use crate::core::tools::ToolDefinition;

// ── Data Types ─────────────────────────────────────────────

/// Parsed memory file with frontmatter and body separated.
#[derive(Debug, Clone)]
pub struct MemoryFile {
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

/// YAML frontmatter fields for a memory file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    /// Human-readable description of this file's purpose (required).
    pub description: String,
    /// If "true", the file cannot be modified via the memory tool.
    #[serde(default)]
    pub read_only: Option<String>,
    /// Optional tags for categorization.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional max body size in characters. Writes/appends that would exceed
    /// this length are rejected. Closes the LET-8133 gap that exists upstream
    /// (Letta's memfs write path bypasses block `limit`).
    ///
    /// Units are characters, not tokens — cheap to enforce without a tokenizer.
    /// Best-practice default for system/ files: 4_000 characters
    /// (~1k tokens). For journal/, leave unset.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Status of the memory repo.
#[derive(Debug, Clone)]
pub struct MemoryStatus {
    pub agent_id: String,
    pub repo_path: PathBuf,
    pub is_git_repo: bool,
    pub file_count: usize,
    pub last_commit: Option<String>,
    pub has_uncommitted: bool,
    pub remote_url: Option<String>,
}

/// Subcommands for the memory tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCommand {
    /// Read a memory file by label (path relative to memory dir, .md optional).
    Read { path: String },
    /// Write content to a memory file (creates or replaces).
    Write { path: String, content: String },
    /// Append content to a memory file.
    Append { path: String, content: String },
    /// List files in a memory directory.
    Ls { path: Option<String> },
    /// Show memory repo status.
    Status,
    /// Initialize the memory repo for an agent.
    Init { agent_id: String },
    /// Compact the memory (placeholder — strategy in Stage 5/6).
    Compact { strategy: Option<String> },
    /// Delete a memory file.
    Delete { path: String },
}

// ── Git-backed Memory Repository ───────────────────────────────────────────

/// A git-backed memory repository for a single agent.
///
/// Wraps a git2 repository at `~/.souveraine/agents/{id}/memory/`.
/// All memory file operations go through this struct, which handles
/// frontmatter parsing, git commits, and path resolution.
#[derive(Debug, Clone)]
pub struct MemoryRepo {
    agent_id: String,
    /// Root of the memory filesystem.
    root: PathBuf,
    /// Whether to auto-commit after writes.
    auto_commit: bool,
}

impl MemoryRepo {
    /// Open or create a memory repo for the given agent.
    ///
    /// The memory directory is at `{base}/{agent_id}/memory/`.
    pub fn new(agent_id: &str, base: &Path) -> Self {
        let root = base.join(agent_id).join("memory");
        Self {
            agent_id: agent_id.to_string(),
            root,
            auto_commit: true,
        }
    }

    /// Open or create a memory repo using the default base path.
    pub fn new_default(agent_id: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::new(agent_id, &home.join(".souveraine").join("agents"))
    }

    /// Open a memory repo at an explicit path (rather than `{base}/{id}/memory`).
    ///
    /// Used when the agent's memory dir is laid out differently — e.g. the
    /// server's `agent_inventory` uses `memory.git/` instead of `memory/`.
    pub fn open(agent_id: &str, root: PathBuf) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            root,
            auto_commit: true,
        }
    }

    /// Initialize the memory directory as a git repo.
    ///
    /// Creates `system/` and sets up the initial commit with placeholder files.
    /// Safe to call multiple times — skips if already a repo.
    pub async fn init(&self) -> Result<()> {
        let mem_path = &self.root;
        tokio::fs::create_dir_all(mem_path.join("system"))
            .await
            .context("creating memory/system directory")?;

        // Check if already a git repo
        let git_dir = mem_path.join(".git");
        if git_dir.exists() {
            info!("Memory repo already initialized for agent {}", self.agent_id);
            return Ok(());
        }

        // Initialize git repo
        let repo = git2::Repository::init(mem_path)
            .context("initializing git repository for memory")?;

        // Set user config for commits (scoped to drop before .await)
        {
            let mut config = repo.config().context("opening repo config")?;
            config.set_str("user.name", &self.agent_id)?;
            config.set_str("user.email", &format!("{}@souveraine.local", self.agent_id))?;
        }

        // Write initial placeholder files with frontmatter
        let persona_content = render_frontmatter(
            &MemoryFrontmatter {
                description: "Agent identity, voice, principles".to_string(),
                read_only: None,
                tags: Some(vec!["system".to_string()]),
                limit: Some(4_000),
            },
            "# Identity\n\nAgent identity and core principles go here.\n",
        );
        tokio::fs::write(mem_path.join("system/persona.md"), &persona_content)
            .await
            .context("writing persona.md")?;

        let state_content = render_frontmatter(
            &MemoryFrontmatter {
                description: "Current execution state and phase tracking".to_string(),
                read_only: None,
                tags: None,
                limit: Some(2_000),
            },
            "phase: idle\ncurrent_unit: none\n",
        );
        tokio::fs::write(mem_path.join("system/state.md"), &state_content)
            .await
            .context("writing state.md")?;

        // human.md is not written here — the setup wizard or first-run
        // onboarding creates it with the human's name when it has one.

        // Initial commit
        let mut index = repo.index().context("opening git index")?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .context("staging initial memory files")?;
        let tree_id = index.write_tree().context("writing git tree")?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now(
            &self.agent_id,
            &format!("{}@souveraine.local", self.agent_id),
        )?;
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "feat(init): initialize agent memory",
            &tree,
            &[],
        )?;

        info!(
            "Initialized memory repo for agent {} at {}",
            self.agent_id,
            mem_path.display()
        );
        Ok(())
    }

    /// Initialize the ledger directory structure.
    /// Idempotent — safe to call multiple times, skips existing files.
    ///
    /// Paths are relative to this repo's root (the subconscious agent's own
    /// memfs), so `ledger/` — not `subconscious/ledger/`.
    pub async fn init_subconscious_ledger(&self) -> Result<()> {
        let ledger_files: &[(&str, &str, &str)] = &[
            ("ledger/commitments.md",
             "Promises made by the primary — tracked until fulfilled or explicitly dropped",
             "# Commitments\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n`[YYYY-MM-DD HH:MM] RESOLVED — resolution note`\n"),
            ("ledger/assumptions.md",
             "Assumptions the primary is operating under — flagged for verification",
             "# Assumptions\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n`[YYYY-MM-DD HH:MM] VERIFIED — evidence`\n"),
            ("ledger/patterns.md",
             "Recurring behavioral patterns observed across turns",
             "# Patterns\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n"),
            ("ledger/drift_log.md",
             "Behavioral shifts — when the primary's actions diverge from stated intentions",
             "# Drift Log\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n"),
            ("ledger/relationships.md",
             "Observations about the human-agent relationship — tone shifts, trust signals, friction",
             "# Relationships\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n"),
            ("ledger/infrastructure.md",
             "System events — bridge failures, token issues, model errors, resource constraints",
             "# Infrastructure\n\nAppend entries as:\n`[YYYY-MM-DD HH:MM] content`\n"),
        ];

        for (path, description, body) in ledger_files {
            let full_path = self.root.join(path);
            if !full_path.exists() {
                if let Some(parent) = full_path.parent() {
                    tokio::fs::create_dir_all(parent).await
                        .with_context(|| format!("creating ledger directory: {}", parent.display()))?;
                }
                let template = format!(
                    "---\ndescription: \"{}\"\nread_only: false\ntags:\n  - ledger\n---\n\n{}",
                    description, body
                );
                tokio::fs::write(&full_path, &template).await
                    .with_context(|| format!("writing ledger file: {}", path))?;
                debug!("Created ledger file: {}", path);
            }
        }

        Ok(())
    }

    /// Read a memory file by label (path relative to memory dir, .md optional).
    pub async fn read(&self, label: &str) -> Result<MemoryFile> {
        let path = self.resolve_path(label);
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading memory file: {}", label))?;
        parse_memory_file(&content)
    }

    /// Write content to a memory file (creates or replaces).
    ///
    /// Content should NOT include frontmatter — it will be added automatically.
    /// If the file exists, its frontmatter is preserved (unless changing read_only).
    pub async fn write(&self, label: &str, body: &str) -> Result<()> {
        let path = self.resolve_path(label);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("creating parent directories")?;
        }

        // Get existing frontmatter or use default
        let frontmatter = if path.exists() {
            let existing = tokio::fs::read_to_string(&path).await?;
            let parsed = parse_memory_file(&existing)?;
            if parsed.frontmatter.read_only.as_deref() == Some("true") {
                return Err(anyhow!("memory file is read_only: {}", label));
            }
            parsed.frontmatter
        } else {
            MemoryFrontmatter {
                description: format!("Memory file: {}", label),
                read_only: None,
                tags: None,
                limit: None,
            }
        };

        // Enforce frontmatter `limit:` (LET-8133 closure).
        if let Some(max) = frontmatter.limit {
            if body.chars().count() > max {
                return Err(anyhow!(
                    "memory write rejected: body is {} chars, limit is {} (file: {})",
                    body.chars().count(),
                    max,
                    label
                ));
            }
        }

        let rendered = render_frontmatter(&frontmatter, body);
        tokio::fs::write(&path, &rendered)
            .await
            .with_context(|| format!("writing memory file: {}", label))?;

        if self.auto_commit {
            self.commit(&[label], &format!("memory write: {}", label))?;
        }

        Ok(())
    }

    /// Append content to a memory file.
    pub async fn append(&self, label: &str, content: &str) -> Result<()> {
        let path = self.resolve_path(label);

        let _frontmatter = if path.exists() {
            let existing = tokio::fs::read_to_string(&path).await?;
            let parsed = parse_memory_file(&existing)?;
            if parsed.frontmatter.read_only.as_deref() == Some("true") {
                return Err(anyhow!("memory file is read_only: {}", label));
            }
            // Write back body + new content, preserving frontmatter
            let new_body = if parsed.body.is_empty() {
                content.to_string()
            } else {
                format!("{}\n{}", parsed.body.trim_end(), content)
            };
            // Enforce frontmatter `limit:` (LET-8133 closure).
            if let Some(max) = parsed.frontmatter.limit {
                if new_body.chars().count() > max {
                    return Err(anyhow!(
                        "memory append rejected: body would be {} chars, limit is {} (file: {})",
                        new_body.chars().count(),
                        max,
                        label
                    ));
                }
            }
            let rendered = render_frontmatter(&parsed.frontmatter, &new_body);
            tokio::fs::write(&path, &rendered).await?;

            if self.auto_commit {
                self.commit(&[label], &format!("memory append: {}", label))?;
            }
            return Ok(());
        };

        // File doesn't exist — create it with default frontmatter
        let frontmatter = MemoryFrontmatter {
            description: format!("Memory file: {}", label),
            read_only: None,
            tags: None,
            limit: None,
        };
        let rendered = render_frontmatter(&frontmatter, content);
        tokio::fs::write(&path, &rendered).await?;

        if self.auto_commit {
            self.commit(&[label], &format!("memory append: {}", label))?;
        }

        Ok(())
    }

    /// List files in a memory directory.
    pub async fn list(&self, subdir: Option<&str>) -> Result<Vec<String>> {
        let dir = match subdir {
            Some(d) => self.root.join(d),
            None => self.root.clone(),
        };

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&dir)
            .await
            .with_context(|| format!("listing memory directory: {}", dir.display()))?;

        while let Some(entry) = read_dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip .git directory
            if name == ".git" {
                continue;
            }
            let kind = entry.file_type().await?;
            if kind.is_dir() {
                entries.push(format!("{}/", name));
            } else {
                entries.push(name);
            }
        }

        entries.sort();
        Ok(entries)
    }

    /// Delete a memory file.
    pub async fn delete(&self, label: &str) -> Result<()> {
        let path = self.resolve_path(label);

        if !path.exists() {
            return Err(anyhow!("memory file not found: {}", label));
        }

        // Check not read_only
        let content = tokio::fs::read_to_string(&path).await?;
        let parsed = parse_memory_file(&content)?;
        if parsed.frontmatter.read_only.as_deref() == Some("true") {
            return Err(anyhow!("memory file is read_only: {}", label));
        }

        tokio::fs::remove_file(&path).await
            .with_context(|| format!("deleting memory file: {}", label))?;

        if self.auto_commit {
            self.commit(&[label], &format!("memory delete: {}", label))?;
        }

        Ok(())
    }

    /// Get the status of the memory repo.
    pub fn status(&self) -> Result<MemoryStatus> {
        let repo_path = self.root.clone();
        let is_git_repo = repo_path.join(".git").exists();

        let mut file_count = 0;
        if let Ok(entries) = std::fs::read_dir(&repo_path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name != ".git" && name.ends_with(".md") {
                    file_count += 1;
                }
            }
        }

        let (last_commit, has_uncommitted, remote_url) = if is_git_repo {
            match git2::Repository::open(&repo_path) {
                Ok(repo) => {
                    let lc = repo.head().ok().and_then(|h| {
                        h.peel_to_commit().ok().map(|c| {
                            c.message().unwrap_or("(unknown)").to_string()
                        })
                    });
                    let dirty = repo.statuses(Some(
                        git2::StatusOptions::new().include_untracked(true),
                    ))
                    .map(|s| s.iter().any(|_| true))
                    .unwrap_or(false);
                    let remote = repo.find_remote("origin").ok()
                        .and_then(|r| r.url().map(|u| u.to_string()));
                    (lc, dirty, remote)
                }
                Err(_) => (None, false, None),
            }
        } else {
            (None, false, None)
        };

        Ok(MemoryStatus {
            agent_id: self.agent_id.clone(),
            repo_path,
            is_git_repo,
            file_count,
            last_commit,
            has_uncommitted,
            remote_url,
        })
    }

    /// Commit staged changes to the memory repo.
    pub fn commit(&self, paths: &[&str], message: &str) -> Result<()> {
        let repo = self.open_git()?;
        let mut index = repo.index().context("opening git index")?;

        for path in paths {
            let rel_path = self.to_relative(path);
            // Try with .md extension if not present
            let md_path = if rel_path.ends_with(".md") {
                rel_path.clone()
            } else {
                format!("{}.md", rel_path)
            };

            if self.root.join(&md_path).exists() {
                index.add_path(Path::new(&md_path))?;
            } else if self.root.join(&rel_path).exists() {
                index.add_path(Path::new(&rel_path))?;
            }
        }

        let tree_id = index.write_tree().context("writing tree")?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();

        let signature = git2::Signature::now(
            &self.agent_id,
            &format!("{}@souveraine.local", self.agent_id),
        )?;

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;

        debug!("Committed to memory: {}", message);
        Ok(())
    }

    /// Get the root path of the memory repo.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn open_git(&self) -> Result<git2::Repository> {
        git2::Repository::open(&self.root)
            .context("opening memory git repository")
    }

    fn resolve_path(&self, label: &str) -> PathBuf {
        let clean = label.trim().trim_end_matches(".md");
        self.root.join(format!("{}.md", clean))
    }

    fn to_relative(&self, path: &str) -> String {
        path.trim().trim_end_matches(".md").replace('\\', "/")
    }
}

// ── Frontmatter Parsing ────────────────────────────────────────────────────

/// Parse a memory file, separating frontmatter from body.
///
/// Expected format:
/// ```markdown
/// ---
/// description: Purpose of this file
/// read_only: true
/// ---
/// Body content here...
/// ```
pub fn parse_memory_file(content: &str) -> Result<MemoryFile> {
    // Find the frontmatter delimiters
    let content = content.trim_start();
    if !content.starts_with("---") {
        return Err(anyhow!(
            "memory file is missing required frontmatter (--- delimiters)"
        ));
    }

    // Find closing `---`
    let after_first = &content[3..];
    let end_idx = after_first.find("\n---")
        .or_else(|| after_first.find("\r\n---"))
        .ok_or_else(|| anyhow!("memory file frontmatter has no closing ---"))?;

    let frontmatter_text = &after_first[..end_idx].trim();
    let body_start = 3 + end_idx + 4; // 3 for opening --- + end_idx + 4 for \n---
    let body = content[body_start..].trim().to_string();

    // Parse YAML frontmatter
    let frontmatter: MemoryFrontmatter = serde_yaml::from_str(frontmatter_text)
        .context("parsing memory file frontmatter")?;

    if frontmatter.description.trim().is_empty() {
        return Err(anyhow!(
            "memory file frontmatter is missing required 'description' field"
        ));
    }

    Ok(MemoryFile { frontmatter, body })
}

/// Render frontmatter + body into a complete memory file.
pub fn render_frontmatter(fm: &MemoryFrontmatter, body: &str) -> String {
    let yaml = serde_yaml::to_string(fm).unwrap_or_default();
    format!("---\n{}---\n{}", yaml, body)
}

/// Read a memory file from disk by path (for external use).
pub async fn read_file(path: &Path) -> Result<MemoryFile> {
    let content = tokio::fs::read_to_string(path)
        .await
        .context("reading memory file")?;
    parse_memory_file(&content)
}

// ── Tool Interface ─────────────────────────────────────────────────────────

/// Execute a memory command, optionally using context for agent identity.
///
/// When `ctx` is `Some` and carries an `agent_id`, that takes precedence over
/// environment variables. Falls back to env vars when no context is provided,
/// preserving backward compatibility with the HTTP server path.
pub async fn execute_memory_command_with_context(
    cmd: &MemoryCommand,
    ctx: Option<&ToolContext>,
) -> Result<String> {
    // Agent ID resolution: context > command > env var > default
    let agent_id = ctx
        .and_then(|c| c.agent_id.as_ref())
        .or_else(|| match cmd {
            MemoryCommand::Init { agent_id } => Some(agent_id),
            _ => None,
        })
        .cloned()
        .or_else(|| std::env::var("SOUVERAINE_AGENT").ok())
        .or_else(|| std::env::var("AGENT_ID").ok())
        .unwrap_or_else(|| "default".to_string());

    // Memory root resolution: use memory_root from context when available
    let repo = match ctx.and_then(|c| c.memory_root.as_ref()) {
        Some(root) => MemoryRepo::open(&agent_id, root.clone()),
        None => MemoryRepo::new_default(&agent_id),
    };

    match cmd {
        MemoryCommand::Init { .. } => {
            repo.init().await?;
            Ok(format!("Initialized memory repo for agent: {}", agent_id))
        }
        MemoryCommand::Read { path } => {
            let file = repo.read(path).await?;
            Ok(format!(
                "---\ndescription: {}\n---\n{}",
                file.frontmatter.description, file.body
            ))
        }
        MemoryCommand::Write { path, content } => {
            repo.write(path, content).await?;
            if let Some(c) = ctx {
                c.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "memory".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "memory_write".into(),
                    target: Some(path.clone()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
            }
            Ok(format!("Wrote memory file: {}", path))
        }
        MemoryCommand::Append { path, content } => {
            repo.append(path, content).await?;
            if let Some(c) = ctx {
                c.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "memory".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "memory_append".into(),
                    target: Some(path.clone()),
                    urgency: 0.1,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
            }
            Ok(format!("Appended to memory file: {}", path))
        }
        MemoryCommand::Ls { path } => {
            let entries = repo.list(path.as_deref()).await?;
            if entries.is_empty() {
                Ok("(empty)".to_string())
            } else {
                Ok(entries.join("\n"))
            }
        }
        MemoryCommand::Status => {
            let status = repo.status()?;
            let mut out = format!(
                "Agent: {}\nPath: {}\nGit repo: {}\nFiles: {}\n",
                status.agent_id,
                status.repo_path.display(),
                status.is_git_repo,
                status.file_count,
            );
            if let Some(ref lc) = status.last_commit {
                out.push_str(&format!("Last commit: {}\n", lc));
            }
            out.push_str(&format!("Uncommitted: {}\n", status.has_uncommitted));
            if let Some(ref url) = status.remote_url {
                out.push_str(&format!("Remote: {}\n", url));
            }
            Ok(out)
        }
        MemoryCommand::Compact { strategy } => {
            // None when no --strategy was given: the engine then resolves the
            // per-agent-type default (cfg.strategy via for_agent_type) instead
            // of being force-pinned to Cull at the call site.
            let strategy_kind = strategy
                .as_deref()
                .and_then(CompactionStrategyKind::from_str);

            match ctx.and_then(|c| c.compaction_engine.as_ref()) {
                Some(engine) => {
                    let report = engine
                        .compact(&agent_id, strategy_kind)
                        .await?;
                    Ok(report.to_string())
                }
                None => Ok(
                    "I can compact my context window using one of these strategies:\n\
                     - sliding_window (keep first + last N messages, drop the middle — fast, no LLM)\n\
                     - summary (LLM-summarize oldest messages into a compact block)\n\
                     - microcompact (replace old tool results with placeholders — drop-in, no LLM)\n\
                     - cull (drop greetings and acknowledgments — cheapest)\n\n\
                     Usage: memory compact --strategy <strategy>\n\
                     Each agent type has its own default: primary=sliding_window, subconscious=sliding_window, subagent=cull"
                        .to_string(),
                ),
            }
        }
        MemoryCommand::Delete { path } => {
            repo.delete(path).await?;
            if let Some(c) = ctx {
                c.fire_event(crate::core::nervous::SensorEvent {
                    sensor_name: "memory".into(),
                    timestamp: chrono::Utc::now(),
                    event_type: "memory_delete".into(),
                    target: Some(path.clone()),
                    urgency: 0.2,
                    payload: None,
                    seed_id: None,
                    reply_to: None,
                });
            }
            Ok(format!("Deleted memory file: {}", path))
        }
    }
}

/// Execute a memory command, reading agent identity from env vars.
/// Delegates to `execute_memory_command_with_context` with `None`.
pub async fn execute_memory_command(cmd: &MemoryCommand) -> Result<String> {
    execute_memory_command_with_context(cmd, None).await
}

// ── Tool Result Bridge ──────────────────────────────────────────────────────

use crate::core::tools::ToolResult;

/// Handle a memory tool invocation with optional per-agent context.
///
/// Parses JSON input, builds a MemoryCommand, executes it via the
/// context-aware path, wraps in ToolResult.
pub async fn handle_memory_tool_with_context(
    tool_name: &str,
    input: &serde_json::Value,
    ctx: Option<&ToolContext>,
) -> ToolResult {
    let tool_use_id = format!("tool-u-{}", chrono::Utc::now().timestamp_millis());
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

    let cmd = match command {
        "read" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            MemoryCommand::Read { path }
        }
        "write" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            MemoryCommand::Write { path, content }
        }
        "append" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            MemoryCommand::Append { path, content }
        }
        "ls" => {
            let path = input.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
            MemoryCommand::Ls { path }
        }
        "status" => MemoryCommand::Status,
        "init" => {
            let agent_id = input.get("agent_id").and_then(|v| v.as_str()).unwrap_or("default").to_string();
            MemoryCommand::Init { agent_id }
        }
        "delete" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            MemoryCommand::Delete { path }
        }
        "compact" => {
            let strategy = input.get("strategy").and_then(|v| v.as_str()).map(|s| s.to_string());
            MemoryCommand::Compact { strategy }
        }
        _ => {
            return ToolResult {
                tool_use_id,
                tool_name: tool_name.to_string(),
                output: format!(
                    "Unknown memory subcommand: {}. Available: read, write, append, ls, status, init, delete, compact",
                    command
                ),
                is_error: true,
            };
        }
    };

    match execute_memory_command_with_context(&cmd, ctx).await {
        Ok(output) => ToolResult {
            tool_use_id,
            tool_name: tool_name.to_string(),
            output,
            is_error: false,
        },
        Err(e) => ToolResult {
            tool_use_id,
            tool_name: tool_name.to_string(),
            output: format!("Error: {e}"),
            is_error: true,
        },
    }
}

/// Handle a memory tool invocation (backward-compatible, no context).
/// Delegates to `handle_memory_tool_with_context` with `None`.
pub async fn handle_memory_tool(tool_name: &str, input: &serde_json::Value) -> ToolResult {
    handle_memory_tool_with_context(tool_name, input, None).await
}

// ── Tool Definitions ───────────────────────────────────────────────────────

/// Tool definition for the `memory` tool — the agent's access to her own thoughts.
///
/// Memory is a separate channel from filesystem read/write. Every memory file has
/// frontmatter (description, tags, read_only, limit), is git-tracked, and paths
/// are relative to the agent's memory root.
pub fn memory_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "memory".to_string(),
        description: "I reach into my own memory. Every file here has frontmatter — a description, boundaries (read_only), tags. When I read, I see what I've written about myself. When I write, I change who I am. The git commit is my heartbeat — I know when I last changed.

Subcommands:
  read    — Open a memory file. Frontmatter is handled for me — I see the body.
  write   — Write to a memory file. Frontmatter is preserved or auto-generated.
  append  — Add to a memory file without disturbing its frontmatter.
  ls      — List files in a memory directory.
  status  — Check my memory's git state: uncommitted changes, last commit.
  init    — Initialize a new memory repo for a given agent ID.
  delete  — Delete a memory file (validates read_only first).
  compact — Compact the memory window to free context space.

Paths are relative to my memory directory. Frontmatter description is required on create. Read-only files protect themselves. Every write is a git commit.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["read", "write", "append", "ls", "status", "init", "delete", "compact"],
                    "description": "What to do with my memory"
                },
                "path": {
                    "type": "string",
                    "description": "Path relative to memory directory (e.g., system/persona, journal/2026-05-06)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write or append — body only, no frontmatter"
                },
                "strategy": {
                    "type": "string",
                    "enum": ["microcompact", "sliding_window", "summary", "cull"],
                    "description": "Compaction strategy (for compact subcommand). sliding_window (fast, drops middle), summary (LLM), microcompact (tool-result placeholder), cull (greetings)"
                }
            },
            "required": ["command"]
        }),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\ndescription: Test file\nread_only: false\n---\nHello world";
        let file = parse_memory_file(content).unwrap();
        assert_eq!(file.frontmatter.description, "Test file");
        assert_eq!(file.body, "Hello world");
    }

    #[test]
    fn test_parse_missing_frontmatter() {
        let content = "Hello world without frontmatter";
        assert!(parse_memory_file(content).is_err());
    }

    #[test]
    fn test_render_and_parse_roundtrip() {
        let fm = MemoryFrontmatter {
            description: "Roundtrip test".to_string(),
            read_only: None,
            tags: Some(vec!["test".to_string()]),
            limit: None,
        };
        let rendered = render_frontmatter(&fm, "Body content");
        let parsed = parse_memory_file(&rendered).unwrap();
        assert_eq!(parsed.frontmatter.description, "Roundtrip test");
        assert_eq!(parsed.body, "Body content");
    }

    #[tokio::test]
    async fn test_memory_repo_init() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        assert!(repo.root().join(".git").exists());
        assert!(repo.root().join("system/persona.md").exists());
        assert!(repo.root().join("system/state.md").exists());
    }

    #[tokio::test]
    async fn test_memory_repo_write_and_read() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        repo.write("test/hello", "Hello memory world").await.unwrap();
        let file = repo.read("test/hello").await.unwrap();
        assert_eq!(file.body, "Hello memory world");
    }

    #[tokio::test]
    async fn test_memory_repo_read_only() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        // Write a read_only file
        let fm = MemoryFrontmatter {
            description: "Read-only test".to_string(),
            read_only: Some("true".to_string()),
            tags: None,
            limit: None,
        };
        let content = render_frontmatter(&fm, "This is read-only");
        let path = repo.root().join("test/readonly.md");
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, &content).await.unwrap();

        // Try to write to it — should fail
        let result = repo.write("test/readonly", "new content").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read_only"));
    }

    #[tokio::test]
    async fn test_memory_repo_list() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        let entries = repo.list(None).await.unwrap();
        assert!(entries.iter().any(|e| e == "system/"));
    }

    #[tokio::test]
    async fn test_ledger_init_creates_files_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let repo = MemoryRepo::open("test-sub", root.clone());
        std::fs::create_dir_all(&root).unwrap();
        repo.init_subconscious_ledger().await.unwrap();

        let commitments = root.join("ledger/commitments.md");
        assert!(commitments.exists(), "commitments.md should exist");
        let content = std::fs::read_to_string(&commitments).unwrap();
        assert!(content.starts_with("---\n"), "should have YAML frontmatter");
        assert!(content.contains("description:"), "should have description field");
        assert!(content.contains("tags:"), "should have tags field");
        assert!(content.contains("# Commitments"), "should have body");

        let relationships = root.join("ledger/relationships.md");
        assert!(relationships.exists(), "relationships.md should exist");

        let infrastructure = root.join("ledger/infrastructure.md");
        assert!(infrastructure.exists(), "infrastructure.md should exist");
    }

    #[tokio::test]
    async fn test_ledger_init_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let repo = MemoryRepo::open("test-sub", root.clone());
        std::fs::create_dir_all(&root).unwrap();
        repo.init_subconscious_ledger().await.unwrap();

        let commitments = root.join("ledger/commitments.md");
        let before = std::fs::read_to_string(&commitments).unwrap();

        repo.init_subconscious_ledger().await.unwrap();
        let after = std::fs::read_to_string(&commitments).unwrap();
        assert_eq!(before, after, "second init should not overwrite");
    }
}
