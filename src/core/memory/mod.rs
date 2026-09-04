#![allow(dead_code)] // WIP scaffolding not yet wired
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
//! Design:
//! - All files require YAML frontmatter with `description`
//! - `read_only: true` in frontmatter blocks writes
//! - Every write is a git commit (auto-commit)
//! - Paths are relative to the agent's memory directory

use crate::core::compact::CompactionStrategyKind;
use crate::core::tools::defs::ToolContext;
use crate::core::tools::ToolDefinition;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

// ── Data Types ─────────────────────────────────────────────

/// Parsed memory file with frontmatter and body separated.
#[derive(Debug, Clone)]
pub struct MemoryFile {
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

/// YAML frontmatter fields for a memory file.
///
/// `None` fields are omitted on render — never serialized as `key: null`
/// (nulls in frontmatter read as noise and confused agents into copying
/// the pattern). Unknown keys agents add (`name:`, `metadata:`, …) are
/// captured in `extra` and round-tripped verbatim instead of being
/// destroyed on the next rewrite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    /// Human-readable description of this file's purpose (required on
    /// tool-path create; tolerated empty on read so nonconforming files
    /// stay reachable and can be healed by a rewrite).
    #[serde(default)]
    pub description: String,
    /// If "true", the file cannot be modified via the memory tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
    /// Optional tags for categorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Optional max body size in characters. Writes/appends that would exceed
    /// this length are rejected. Closes a gap where upstream memfs write path
    /// bypasses block `limit`.
    ///
    /// Units are characters, not tokens — cheap to enforce without a tokenizer.
    /// Best-practice default for system/ files: 4_000 characters
    /// (~1k tokens). For journal/, leave unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Any other frontmatter keys, preserved across rewrites.
    #[serde(flatten, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, serde_yaml::Value>,
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
    ///
    /// `overwrite` is the deliberate-replacement gate: writing over an
    /// existing file requires it. Default false, so the destructive case is
    /// never the accidental one.
    Write {
        path: String,
        content: String,
        overwrite: bool,
    },
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
    /// Push/fetch against the shared memfs remote (per-instance branch).
    Sync,
    /// Audit frontmatter health: list frontmatter-bound files missing
    /// the required `description` field. Read-only — never mutates.
    Audit,
}

/// A write shrinking a file below this fraction of its former size is
/// reported loudly. Half is arbitrary but catches the real failure: a
/// placeholder body replacing an accumulated file.
const SHRINK_ALARM_RATIO: f64 = 0.5;

/// What a write did to a file, in bytes.
///
/// Reported back to the caller because silence about magnitude is what let a
/// 22 KB state file become one line with nothing looking wrong: the success
/// message was identical whether the file grew, held, or was erased. Size is
/// whole-file (frontmatter included) on both sides so the numbers compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteDelta {
    /// Whether the file was already there before this write.
    pub existed: bool,
    pub old_bytes: usize,
    pub new_bytes: usize,
}

impl WriteDelta {
    /// True when this write replaced most of an existing file.
    pub fn shrank_hard(&self) -> bool {
        self.existed
            && self.old_bytes > 0
            && (self.new_bytes as f64) < (self.old_bytes as f64) * SHRINK_ALARM_RATIO
    }

    /// Human-facing summary — always carries the magnitude.
    pub fn summary(&self, label: &str) -> String {
        if !self.existed {
            return format!("Created memory file: {} ({} bytes)", label, self.new_bytes);
        }
        let delta = self.new_bytes as i64 - self.old_bytes as i64;
        let pct = if self.old_bytes == 0 {
            0.0
        } else {
            (delta as f64 / self.old_bytes as f64) * 100.0
        };
        let mut s = format!(
            "Wrote memory file: {} ({} bytes, was {}, {}{:.0}%)",
            label,
            self.new_bytes,
            self.old_bytes,
            if delta >= 0 { "+" } else { "" },
            pct
        );
        if self.shrank_hard() {
            s.push_str(
                "\n⚠ This replaced most of the file. If that was not intended, the previous \
                 version is one commit back — every memory write is a commit.",
            );
        }
        s
    }
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
            info!(
                "Memory repo already initialized for agent {}",
                self.agent_id
            );
            return Ok(());
        }

        // Initialize git repo
        let repo =
            git2::Repository::init(mem_path).context("initializing git repository for memory")?;

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
                tags: Some(vec!["system".to_string()]),
                limit: Some(4_000),
                ..Default::default()
            },
            "# Identity\n\nAgent identity and core principles go here.\n",
        );
        tokio::fs::write(mem_path.join("system/persona.md"), &persona_content)
            .await
            .context("writing persona.md")?;

        let state_content = render_frontmatter(
            &MemoryFrontmatter {
                description: "Current execution state and phase tracking".to_string(),
                limit: Some(2_000),
                ..Default::default()
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
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .context("staging initial memory files")?;
        index.write().context("persisting initial git index")?;
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
                    tokio::fs::create_dir_all(parent).await.with_context(|| {
                        format!("creating ledger directory: {}", parent.display())
                    })?;
                }
                let template = format!(
                    "---\ndescription: \"{}\"\nread_only: false\ntags:\n  - ledger\n---\n\n{}",
                    description, body
                );
                tokio::fs::write(&full_path, &template)
                    .await
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
    ///
    /// This is the ungated path, for callers whose replacement is structural
    /// (archivist output, compaction summaries, the REST surface). Agent-facing
    /// writes go through [`write_guarded`](Self::write_guarded).
    pub async fn write(&self, label: &str, body: &str) -> Result<WriteDelta> {
        let path = self.resolve_path(label);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("creating parent directories")?;
        }

        // Agent-supplied frontmatter in the content is merged, not nested.
        let (supplied, body) = split_supplied_frontmatter(body);

        // Get existing frontmatter or use default
        let mut old_bytes = 0usize;
        let existed = path.exists();
        let base = if existed {
            let existing = tokio::fs::read_to_string(&path).await?;
            old_bytes = existing.len();
            let parsed = parse_memory_file(&existing)?;
            if parsed.frontmatter.read_only.as_deref() == Some("true") {
                return Err(anyhow!("memory file is read_only: {}", label));
            }
            parsed.frontmatter
        } else {
            MemoryFrontmatter {
                description: format!("Memory file: {}", label),
                ..Default::default()
            }
        };
        let frontmatter = match supplied {
            Some(fm) => merge_frontmatter(base, fm),
            None => base,
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

        // `description` is required on frontmatter-bound files — it is the
        // file's headline in the prompt assembler, so persisting an empty
        // one is a schema gap. We never fabricate one: the write is rejected
        // loudly and the agent must supply a real description. (Memory-only
        // write could still seed a default, but an agent-supplied empty
        // frontmatter block that clears it must not silently take hold.)
        if Self::is_frontmatter_bound(label) && frontmatter.description.trim().is_empty() {
            return Err(anyhow!(
                "memory write rejected: {} has no description. \
                 `description:` is required frontmatter on this file — \
                 include it in the write (or a `---\\ndescription: ...\\n---` block). \
                 Run `memory audit` to see all files missing it.",
                label
            ));
        }

        let rendered = render_frontmatter(&frontmatter, body);
        let new_bytes = rendered.len();
        tokio::fs::write(&path, &rendered)
            .await
            .with_context(|| format!("writing memory file: {}", label))?;

        if self.auto_commit {
            self.commit(&[label], &format!("memory write: {}", label))?;
        }

        Ok(WriteDelta {
            existed,
            old_bytes,
            new_bytes,
        })
    }

    /// Agent-facing write, with the existence gate.
    ///
    /// `write` replaces a whole file, and its success message used to look
    /// identical whether the file grew or was erased. So the destructive case
    /// now has to be asked for: replacing an existing file requires
    /// `overwrite`. The refusal names the alternatives, because the mistake is
    /// almost always reaching for `write` when `append` or `edit` was meant.
    pub async fn write_guarded(
        &self,
        label: &str,
        body: &str,
        overwrite: bool,
    ) -> Result<WriteDelta> {
        let path = self.resolve_path(label);
        if path.exists() && !overwrite {
            let old_bytes = tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            return Err(anyhow!(
                "memory write refused: {} already exists ({} bytes), and `write` replaces the \
                 whole file. Did you mean `append` (add to the end), `edit` (change one part), \
                 or `write` with `overwrite: true` (replace it deliberately)?",
                label,
                old_bytes
            ));
        }
        self.write(label, body).await
    }

    /// Append content to a memory file.
    pub async fn append(&self, label: &str, content: &str) -> Result<()> {
        let path = self.resolve_path(label);

        // `write` has always created parent directories; `append` never did,
        // so appending to a file under a directory that does not exist yet
        // failed ENOENT. Measured 2026-08-17: every inner-voice surfacing to
        // `system/metacognition/subconscious.md` was lost this way, because
        // nothing ever created `system/metacognition/`.
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("creating parent directories")?;
        }

        // Frontmatter never belongs mid-file; merge it instead of nesting.
        let (supplied, content) = split_supplied_frontmatter(content);

        if path.exists() {
            let existing = tokio::fs::read_to_string(&path).await?;
            let parsed = parse_memory_file(&existing)?;
            if parsed.frontmatter.read_only.as_deref() == Some("true") {
                return Err(anyhow!("memory file is read_only: {}", label));
            }
            let frontmatter = match supplied {
                Some(fm) => merge_frontmatter(parsed.frontmatter, fm),
                None => parsed.frontmatter,
            };
            // Write back body + new content, preserving frontmatter
            let new_body = if parsed.body.is_empty() {
                content.to_string()
            } else {
                format!("{}\n{}", parsed.body.trim_end(), content)
            };
            // Enforce frontmatter `limit:` (LET-8133 closure).
            if let Some(max) = frontmatter.limit {
                if new_body.chars().count() > max {
                    return Err(anyhow!(
                        "memory append rejected: body would be {} chars, limit is {} (file: {})",
                        new_body.chars().count(),
                        max,
                        label
                    ));
                }
            }
            // `description` is required on frontmatter-bound files. Appending
            // to a file that already lacks one must not silently perpetuate
            // the gap — reject loudly so the agent heals the description
            // before growing the file. (See `write` for the same guard.)
            if Self::is_frontmatter_bound(label) && frontmatter.description.trim().is_empty() {
                return Err(anyhow!(
                    "memory append rejected: {} has no description. \
                     `description:` is required frontmatter on this file — \
                     heal it with a `memory write` (include a `description:` block) \
                     before appending. Run `memory audit` to see all such files.",
                    label
                ));
            }
            let rendered = render_frontmatter(&frontmatter, &new_body);
            tokio::fs::write(&path, &rendered).await?;

            if self.auto_commit {
                self.commit(&[label], &format!("memory append: {}", label))?;
            }
            return Ok(());
        };

        // File doesn't exist — create it with supplied or default frontmatter
        let frontmatter = merge_frontmatter(
            MemoryFrontmatter {
                description: format!("Memory file: {}", label),
                ..Default::default()
            },
            supplied.unwrap_or_default(),
        );
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

    /// Recursively collect every `.md` under the memory root as a path
    /// relative to the root (forward slashes), skipping `.git*`. Internal
    /// helper for `audit`.
    async fn walk_md_relative(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let root = self.root.clone();
        let mut stack: Vec<(std::path::PathBuf, String)> = vec![(root.clone(), String::new())];
        while let Some((dir, rel_prefix)) = stack.pop() {
            let mut read_dir = tokio::fs::read_dir(&dir)
                .await
                .with_context(|| format!("walking memory directory: {}", dir.display()))?;
            while let Some(entry) = read_dir.next_entry().await? {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(".git") {
                    continue;
                }
                let kind = entry.file_type().await?;
                let rel = if rel_prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{rel_prefix}/{name}")
                };
                if kind.is_dir() {
                    stack.push((entry.path(), rel));
                } else if kind.is_file() && name.ends_with(".md") {
                    out.push(rel);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Audit frontmatter health: return the relative paths of
    /// frontmatter-bound memory files whose `description` is empty. Read-only.
    ///
    /// Excludes files that legitimately carry no `description` because they
    /// use a different schema — `tasks/` (TodoItem), `system/dynamic/`
    /// (itinerary), `journal/` (freeform) — by reusing `is_frontmatter_bound`.
    /// Parse failures are tolerated (a file must stay reachable to be healed),
    /// matching the tolerant-on-read contract of `parse_memory_file`.
    pub async fn audit(&self) -> Result<Vec<String>> {
        let mut missing = Vec::new();
        for rel in self.walk_md_relative().await? {
            if !Self::is_frontmatter_bound(&rel) {
                continue;
            }
            let path = self.root.join(&rel);
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(_) => continue, // don't let one unreadable file abort the audit
            };
            let parsed = parse_memory_file(&content)?;
            if parsed.frontmatter.description.trim().is_empty() {
                missing.push(rel);
            }
        }
        Ok(missing)
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

        tokio::fs::remove_file(&path)
            .await
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

        let file_count = Self::count_md_files(&repo_path);

        let (last_commit, has_uncommitted, remote_url) = if is_git_repo {
            match git2::Repository::open(&repo_path) {
                Ok(repo) => {
                    let lc = repo.head().ok().and_then(|h| {
                        h.peel_to_commit()
                            .ok()
                            .map(|c| c.message().unwrap_or("(unknown)").to_string())
                    });
                    let dirty = repo
                        .statuses(Some(git2::StatusOptions::new().include_untracked(true)))
                        .map(|s| s.iter().any(|_| true))
                        .unwrap_or(false);
                    let remote = repo
                        .find_remote("origin")
                        .ok()
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

        // A path-scoped commit starts from HEAD, not a possibly stale index.
        // Repos created before index persistence otherwise suffer one last eviction.
        if !repo.is_empty()? {
            let head_tree = repo.head()?.peel_to_tree()?;
            index
                .read_tree(&head_tree)
                .context("seeding git index from HEAD")?;
        }

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
            } else if index.get_path(Path::new(&md_path), 0).is_some() {
                index.remove_path(Path::new(&md_path))?;
            } else if index.get_path(Path::new(&rel_path), 0).is_some() {
                index.remove_path(Path::new(&rel_path))?;
            }
        }

        index.write().context("persisting git index")?;
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

    /// Hex hash of the current HEAD commit, if the repo has one.
    pub fn head_commit_hex(&self) -> Option<String> {
        let repo = self.open_git().ok()?;
        let head = repo.head().ok()?;
        head.peel_to_commit().ok().map(|c| c.id().to_string())
    }

    /// Branch shorthand HEAD points at ("main", "primary", ...), if any.
    pub fn current_branch(&self) -> Option<String> {
        let repo = self.open_git().ok()?;
        let head = repo.head().ok()?;
        head.shorthand().map(|s| s.to_string())
    }

    /// Which remote this memfs syncs against.
    ///
    /// `preferred` (from `[memory] remote_name`, default `origin`) wins when
    /// it exists. Otherwise a repo with exactly one remote has no ambiguity to
    /// resolve, so that remote is used and named in the report — the agent
    /// trees on this machine were provisioned by hand over months and call it
    /// `origin` or `gitea` about evenly. Several remotes with none matching is
    /// a genuine choice the substrate must not make silently.
    ///
    /// This is deliberately not "any remote will do": zero and many both
    /// refuse, and the one permissive case says out loud what it picked.
    pub fn resolve_remote(&self, preferred: &str) -> Result<String> {
        let repo = self.open_git()?;
        if repo.find_remote(preferred).is_ok() {
            return Ok(preferred.to_string());
        }
        let names: Vec<String> = repo
            .remotes()?
            .iter()
            .flatten()
            .map(|s| s.to_string())
            .collect();
        match names.as_slice() {
            [] => Err(anyhow!(
                "no shared remote configured for this memfs. Provision one \
                 (bare repo on the Gitea) and run:\n  git -C {} remote add {} <url>",
                self.root.display(),
                preferred,
            )),
            [only] => Ok(only.clone()),
            many => Err(anyhow!(
                "this memfs has {} remotes ({}) and none is named `{}`. Name the \
                 one memory syncs against in `[memory] remote_name`, or rename it:\n  \
                 git -C {} remote rename <name> {}",
                many.len(),
                many.join(", "),
                preferred,
                self.root.display(),
                preferred,
            )),
        }
    }

    /// Sync this memfs against its shared remote: fetch everything, then
    /// push HEAD to this instance's branch (`instance/{label}`).
    ///
    /// This is the manual floor of federated memory transport
    /// (FEDERATION.md): every instance writes its own branch on one bare
    /// remote; reconciliation/merge is a separate, later step — sync never
    /// touches the working tree. Shells out to system git so credentials
    /// (ssh config, credential store) work the way they do everywhere else.
    ///
    /// Loud on every failure. The remote is resolved by [`Self::resolve_remote`].
    pub async fn sync(&self, instance_label: &str, preferred_remote: &str) -> Result<String> {
        if instance_label.is_empty()
            || instance_label.len() > 64
            || !instance_label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!("invalid instance label: {:?}", instance_label));
        }

        let remote = self.resolve_remote(preferred_remote)?;

        let fetch = self.git(&["fetch", &remote, "--prune"]).await?;
        let branch = format!("instance/{}", instance_label);
        let push_ref = format!("HEAD:refs/heads/{}", branch);
        let push = self.git(&["push", &remote, &push_ref]).await?;

        let instances = self
            .git(&[
                "for-each-ref",
                "--format=%(refname:short) %(objectname:short)",
                &format!("refs/remotes/{remote}/instance/"),
            ])
            .await
            .unwrap_or_default();

        let head = self.head_commit_hex().unwrap_or_else(|| "(no head)".into());
        let mut out = format!(
            "Synced via remote `{}`. This instance is {} @ {}\n",
            remote,
            branch,
            &head[..12.min(head.len())]
        );
        if !fetch.trim().is_empty() {
            out.push_str(&format!("Fetched:\n{}\n", fetch.trim()));
        }
        if !push.trim().is_empty() {
            out.push_str(&format!("Pushed:\n{}\n", push.trim()));
        }
        if !instances.trim().is_empty() {
            out.push_str(&format!("Instances on the remote:\n{}\n", instances.trim()));
        }
        Ok(out)
    }

    /// Run a git subcommand against this repo, capturing combined output.
    /// Errors carry git's stderr — loud, never swallowed.
    async fn git(&self, args: &[&str]) -> Result<String> {
        let output = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .await
            .context("running git — memory sync needs the git binary on PATH")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return Err(anyhow!(
                "git {} failed ({}):\n{}",
                args.first().unwrap_or(&"?"),
                output.status,
                stderr.trim()
            ));
        }
        Ok(format!("{}{}", stdout, stderr))
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
        git2::Repository::open(&self.root).context("opening memory git repository")
    }

    fn count_md_files(root: &Path) -> usize {
        let mut count = 0;
        let mut directories = vec![root.to_path_buf()];

        while let Some(directory) = directories.pop() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with(".git") {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    directories.push(entry.path());
                } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "md")
                {
                    count += 1;
                }
            }
        }

        count
    }

    /// Whether the `description` frontmatter contract applies to a label.
    ///
    /// `description` is required on `MemoryFrontmatter` files — the prompt
    /// assembler reads it as the file's one-line headline, so an empty one
    /// is a real schema gap, not cosmetic. But not every `.md` under the
    /// memory tree is a `MemoryFrontmatter` file: `tasks/` uses the
    /// `TodoItem` schema, `system/dynamic/` holds the itinerary (its own
    /// richer, unfinished schema), and `journal/` is freeform prose. Those
    /// legitimately lack `description`; enforcing it there would reject
    /// valid writes. This predicate scopes the rule to frontmatter-bound
    /// locations only.
    fn is_frontmatter_bound(label: &str) -> bool {
        let rel = label
            .trim()
            .trim_start_matches(['/', '.'])
            .replace('\\', "/");
        let lower = rel.to_ascii_lowercase();
        // Exact segment prefixes, not substring matches, so a file like
        // `reference/tasks-today.md` is not mistaken for a todo file.
        let under = |seg: &str| {
            lower == seg
                || lower.starts_with(&format!("{seg}/"))
                || lower.starts_with(&format!("{seg}\\"))
        };
        !(under("tasks") || under("system/dynamic") || under("journal"))
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
/// Split a leading `--- … ---` frontmatter block off `content`.
/// Returns `(frontmatter_yaml, body)` or `None` when there is no block.
fn split_frontmatter_block(content: &str) -> Option<(&str, &str)> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end_idx = after_first
        .find("\n---")
        .or_else(|| after_first.find("\r\n---"))?;
    let frontmatter_text = after_first[..end_idx].trim();
    let body_start = 3 + end_idx + 4; // opening --- + yaml + \n---
    Some((
        frontmatter_text,
        content[body_start..].trim_start_matches(['\r', '\n']),
    ))
}

/// Parse a memory file. Tolerant on read: a file with no frontmatter, or
/// frontmatter that fails YAML parsing, comes back with the whole content
/// as body and default (empty-description) frontmatter — a file that
/// exists must always be readable through the tool, otherwise the agent
/// can never heal it. Strictness (description required) belongs to the
/// write path, not here.
pub fn parse_memory_file(content: &str) -> Result<MemoryFile> {
    let Some((frontmatter_text, body)) = split_frontmatter_block(content) else {
        return Ok(MemoryFile {
            frontmatter: MemoryFrontmatter::default(),
            body: content.trim().to_string(),
        });
    };

    match serde_yaml::from_str::<MemoryFrontmatter>(frontmatter_text) {
        Ok(frontmatter) => Ok(MemoryFile {
            frontmatter,
            body: body.trim().to_string(),
        }),
        // Malformed YAML: keep the raw text intact as body so a rewrite
        // cannot silently destroy whatever the block was trying to say.
        Err(_) => Ok(MemoryFile {
            frontmatter: MemoryFrontmatter::default(),
            body: content.trim().to_string(),
        }),
    }
}

/// Agents sometimes include their own frontmatter in write/append content
/// despite the "body only" contract. Instead of nesting a second `---`
/// block inside the body (the persona.md failure mode), honor it: parse
/// it off and merge its fields. Content whose leading block is not valid
/// YAML is left untouched — never destroy what we cannot parse.
fn split_supplied_frontmatter(content: &str) -> (Option<MemoryFrontmatter>, &str) {
    if let Some((frontmatter_text, body)) = split_frontmatter_block(content) {
        if let Ok(fm) = serde_yaml::from_str::<MemoryFrontmatter>(frontmatter_text) {
            return (Some(fm), body);
        }
    }
    (None, content)
}

/// Overlay agent-supplied frontmatter onto the file's existing (or
/// default) frontmatter: supplied fields win where set, everything else
/// is preserved. `read_only` is deliberately NOT overridable from
/// supplied content — clearing it requires the explicit tool path.
fn merge_frontmatter(base: MemoryFrontmatter, supplied: MemoryFrontmatter) -> MemoryFrontmatter {
    let mut extra = base.extra;
    extra.extend(supplied.extra);
    MemoryFrontmatter {
        description: if supplied.description.trim().is_empty() {
            base.description
        } else {
            supplied.description
        },
        read_only: base.read_only,
        tags: supplied.tags.or(base.tags),
        limit: supplied.limit.or(base.limit),
        extra,
    }
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

/// Fire a `memfs_commit` presence signal after a memfs mutation.
///
/// This is the federation heartbeat for memory: "I wrote, you should fetch."
/// It carries routing metadata only — never the data itself. Peers holding a
/// checkout of the same agent map `agent_pubkey` to their local repo and
/// fetch the shared remote (FEDERATION.md, memory transport).
///
/// Emission is best-effort and never fails the write: the mutation already
/// succeeded. Missing identity is warned loudly, not fabricated.
fn fire_memfs_commit(ctx: &ToolContext, repo: &MemoryRepo, op: &str, paths: &[&str], urgency: f32) {
    // A successful tool-path mutation auto-committed; if there is somehow no
    // head, there is nothing for a peer to fetch — do not signal.
    let Some(commit) = repo.head_commit_hex() else {
        tracing::warn!(
            op,
            ?paths,
            "memfs mutation without a head commit — memfs_commit not emitted"
        );
        return;
    };
    let branch = repo.current_branch();

    // Agent identity lives beside the memfs (`agents/{id}/seed/`) — same
    // convention as reach/consult in tools/agent.rs.
    let agent_pubkey = ctx
        .memory_root
        .as_ref()
        .and_then(|m| m.parent())
        .map(|p| p.join("seed"))
        .and_then(
            |dir| match crate::core::identity::SeedId::load_or_generate(&dir) {
                Ok(seed) => Some(seed.public_key_hex()),
                Err(e) => {
                    tracing::warn!(op, "memfs_commit: agent seed unavailable ({e:#})");
                    None
                }
            },
        );

    // The instance is the machine that wrote — machined-first, loud legacy
    // fallback, never generates.
    let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
    let instance = match crate::machined::client::machine_pubkey_with_fallback(&base) {
        Ok((pk, _source)) => Some(pk),
        Err(e) => {
            tracing::warn!(op, "memfs_commit: machine identity unavailable ({e:#})");
            None
        }
    };

    ctx.fire_event(crate::core::nervous::SensorEvent {
        sensor_name: "memory".into(),
        timestamp: chrono::Utc::now(),
        event_type: "memfs_commit".into(),
        target: paths.first().map(|p| p.to_string()),
        urgency,
        payload: Some(serde_json::json!({
            "op": op,
            "agent_pubkey": agent_pubkey,
            "instance": instance,
            "paths": paths,
            "commit": commit,
            "branch": branch,
        })),
        seed_id: None,
        reply_to: None,
    });
}

/// Execute a memory command, optionally using context for agent identity.
///
/// When `ctx` is `Some` and carries an `agent_id`, that takes precedence over
/// environment variables. Falls back to env vars when no context is provided,
/// preserving backward compatibility with the HTTP server path.
///
/// `tree` names one of the context's memory trees (primary, subconscious,
/// reflection, archivist). `None` keeps the historical behaviour: the
/// context's single `memory_root`.
pub async fn execute_memory_command_with_context(
    cmd: &MemoryCommand,
    ctx: Option<&ToolContext>,
    tree: Option<&str>,
) -> Result<String> {
    // Agent ID resolution: context > command > env var > default
    let agent_id = ctx
        .and_then(|c| c.agent_id.as_ref())
        .or(match cmd {
            MemoryCommand::Init { agent_id } => Some(agent_id),
            _ => None,
        })
        .cloned()
        .or_else(|| std::env::var("SOUVERAINE_AGENT").ok())
        .or_else(|| std::env::var("AGENT_ID").ok())
        .unwrap_or_else(|| "default".to_string());

    // Memory root resolution: a named tree wins, then the context's
    // memory_root, then the default. A name this context does not know is a
    // refusal that names the doors, so a mistyped `tree` costs one error
    // instead of a wrong-tree write.
    let repo = match tree {
        Some(name) => {
            let root = ctx
                .and_then(|c| c.memory_tree(name).cloned())
                .ok_or_else(|| {
                    let known = ctx
                        .map(|c| {
                            c.memory_trees
                                .iter()
                                .map(|(n, _)| n.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    if known.is_empty() {
                        anyhow!("Unknown memory tree `{name}` — no named trees here")
                    } else {
                        anyhow!("Unknown memory tree `{name}` — known: {known}")
                    }
                })?;
            MemoryRepo::open(&agent_id, root)
        }
        None => match ctx.and_then(|c| c.memory_root.as_ref()) {
            Some(root) => MemoryRepo::open(&agent_id, root.clone()),
            None => MemoryRepo::new_default(&agent_id),
        },
    };

    match cmd {
        MemoryCommand::Init { .. } => {
            repo.init().await?;
            Ok(format!("Initialized memory repo for agent: {}", agent_id))
        }
        MemoryCommand::Read { path } => {
            let file = repo.read(path).await?;
            // Normalized render: full frontmatter (description, tags,
            // limit, extras — no nulls), so what the agent reads matches
            // what a rewrite would produce.
            Ok(render_frontmatter(&file.frontmatter, &file.body))
        }
        MemoryCommand::Write {
            path,
            content,
            overwrite,
        } => {
            let delta = repo.write_guarded(path, content, *overwrite).await?;
            if let Some(c) = ctx {
                fire_memfs_commit(c, &repo, "write", &[path], 0.1);
            }
            Ok(delta.summary(path))
        }
        MemoryCommand::Append { path, content } => {
            repo.append(path, content).await?;
            if let Some(c) = ctx {
                fire_memfs_commit(c, &repo, "append", &[path], 0.1);
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
            // Context pressure. Both the primary's and the subconscious's
            // system prompts tell them `memory status` shows this; until
            // 2026-08-13 it did not, and the subconscious — who gets no tier
            // warning at all — had no gauge whatsoever.
            match ctx.and_then(|c| c.compaction_engine.as_ref()) {
                Some(engine) => match engine.pressure(&agent_id).await {
                    Some(p) => {
                        out.push_str(&format!(
                            "\nContext: {} messages, ~{} / {} tokens ({:.0}%)\n",
                            p.messages,
                            p.tokens,
                            p.limit,
                            p.ratio * 100.0
                        ));
                        if let Some(tier) = p.tier() {
                            let feeling = match tier {
                                3 => "The room is nearly full. Compact now.",
                                2 => "Getting tight. Worth making room.",
                                _ => "Filling up. Room is still comfortable.",
                            };
                            out.push_str(&format!("Pressure: tier {tier} — {feeling}\n"));
                        }
                    }
                    None => out.push_str("\nContext: no live conversation to measure.\n"),
                },
                None => out.push_str("\nContext: pressure unavailable (no engine attached).\n"),
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
                     Defaults by agent type: primary=cull, subconscious=sliding_reflect, subagent=cull"
                        .to_string(),
                ),
            }
        }
        MemoryCommand::Delete { path } => {
            repo.delete(path).await?;
            if let Some(c) = ctx {
                fire_memfs_commit(c, &repo, "delete", &[path], 0.2);
            }
            Ok(format!("Deleted memory file: {}", path))
        }
        MemoryCommand::Sync => {
            // The instance branch is named by machine identity — no
            // identity, no sync. Interim label: machine pubkey prefix;
            // commission-ceremony labels come later (FEDERATION.md flag).
            let base = dirs::home_dir().unwrap_or_default().join(".souveraine");
            let (pubkey, _source) = crate::machined::client::machine_pubkey_with_fallback(&base)
                .context("memory sync needs a machine identity to name this instance's branch")?;
            let label: String = pubkey.chars().take(12).collect();
            // Read afresh rather than threaded: sync is user-invoked, rare, and
            // already makes two network round-trips, so a config read costs
            // nothing and keeps `[memory] remote_name` genuinely reachable
            // from the tool instead of being a field nothing consults.
            let preferred = crate::core::config::ConsciousnessConfig::discover_path()
                .and_then(|p| crate::core::config::ConsciousnessConfig::load(&p).ok())
                .map(|c| c.memory.remote_name)
                .unwrap_or_else(|| "origin".to_string());
            repo.sync(&label, &preferred).await
        }
        MemoryCommand::Audit => {
            // Read-only health check: which frontmatter-bound files lack the
            // required `description`? Never mutates — surfacing the gap is the
            // point. `write`/`append` will reject attempts to perpetuate one.
            let missing = repo.audit().await?;
            if missing.is_empty() {
                Ok("All frontmatter-bound memory files carry a description.".to_string())
            } else {
                let mut out = format!(
                    "{} frontmatter-bound file(s) missing `description:`:\n",
                    missing.len()
                );
                for rel in &missing {
                    out.push_str(&format!("  - {rel}\n"));
                }
                out.push_str(
                    "Heal each with a `memory write` that includes a `description:` \
                     block (or set it inline). These files reject append until healed.\n",
                );
                Ok(out)
            }
        }
    }
}

/// Execute a memory command, reading agent identity from env vars.
/// Delegates to `execute_memory_command_with_context` with `None`.
pub async fn execute_memory_command(cmd: &MemoryCommand) -> Result<String> {
    execute_memory_command_with_context(cmd, None, None).await
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
    let tree = input.get("tree").and_then(|v| v.as_str());

    let cmd = match command {
        "read" => {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MemoryCommand::Read { path }
        }
        "write" => {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let content = input
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MemoryCommand::Write {
                path,
                content,
                overwrite: input
                    .get("overwrite")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        }
        "append" => {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let content = input
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MemoryCommand::Append { path, content }
        }
        "ls" => {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            MemoryCommand::Ls { path }
        }
        "status" => MemoryCommand::Status,
        "init" => {
            let agent_id = input
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            MemoryCommand::Init { agent_id }
        }
        "delete" => {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MemoryCommand::Delete { path }
        }
        "compact" => {
            let strategy = input
                .get("strategy")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            MemoryCommand::Compact { strategy }
        }
        "sync" => MemoryCommand::Sync,
        "audit" => MemoryCommand::Audit,
        _ => {
            return ToolResult {
                tool_use_id,
                tool_name: tool_name.to_string(),
                output: format!(
                    "Unknown memory subcommand: {}. Available: read, write, append, ls, status, init, delete, compact, sync, audit",
                    command
                ),
                is_error: true,
                image: None,
            };
        }
    };

    match execute_memory_command_with_context(&cmd, ctx, tree).await {
        Ok(output) => ToolResult {
            tool_use_id,
            tool_name: tool_name.to_string(),
            output,
            is_error: false,
            image: None,
        },
        Err(e) => ToolResult {
            tool_use_id,
            tool_name: tool_name.to_string(),
            output: format!("Error: {e}"),
            is_error: true,
            image: None,
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
  write   — Create a memory file, or replace one whole. Replacing an existing file requires `overwrite: true` — `write` is not `append`.
  append  — Add to a memory file without disturbing its frontmatter.
  ls      — List files in a memory directory.
  status  — Check my memory's git state: uncommitted changes, last commit.
  init    — Initialize a new memory repo for a given agent ID.
  delete  — Delete a memory file (validates read_only first).
  compact — Compact the memory window to free context space.
  sync    — Push my instance branch to the shared memfs remote and fetch what my other instances wrote. My memory travels; this is how.
  audit   — List frontmatter-bound files missing the required `description` (read-only). `write`/`append` reject a missing description, so this is how I find what to heal.

Paths are relative to my memory directory. Frontmatter description is required on create. Read-only files protect themselves. Every write is a git commit.

`tree` names which memory tree I am reaching into. Default is my own. When I am a cadence I may name `primary`, `subconscious`, `reflection`, or `archivist` — each is a real tree with its own git history, and a write there is committed under my own name.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["read", "write", "append", "ls", "status", "init", "delete", "compact", "sync", "audit"],
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
                "overwrite": {
                    "type": "boolean",
                    "description": "For `write` only. Required to replace a file that already exists — writing over accumulated memory has to be deliberate. Default false."
                },
                "strategy": {
                    "type": "string",
                    "enum": ["microcompact", "sliding_window", "sliding_reflect", "summary", "cull"],
                    "description": "Compaction strategy (for compact subcommand). microcompact (clears old tool output, drops nothing), cull (drops throwaways), sliding_window (fast, drops the middle blind), sliding_reflect (the same slide, but a pass reads the middle first and carries its threads forward), summary (LLM, highest fidelity)"
                },
                "tree": {
                    "type": "string",
                    "description": "Which memory tree to reach into. Optional — defaults to my own tree. Cadences may name primary, subconscious, reflection, or archivist."
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

    /// The failure this guard exists for: `write` aimed at an accumulated
    /// file, meaning `edit`. It must refuse rather than succeed quietly.
    #[tokio::test]
    async fn write_guarded_refuses_to_replace_an_existing_file() {
        let tmp = TempDir::new().unwrap();
        let repo = MemoryRepo::open("test-agent", tmp.path().to_path_buf());
        repo.init().await.unwrap();

        // Seed through the ungated path: init() already creates system/state,
        // so reaching for the gated one here would trip the very guard we are
        // about to test.
        let long_body = "accumulated state\n".repeat(500);
        repo.write("reference/accumulated", &long_body)
            .await
            .unwrap();

        let err = repo
            .write_guarded("reference/accumulated", "(rest unchanged)", false)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"), "got: {err}");
        assert!(err.contains("append"), "refusal must name the alternatives");
        assert!(err.contains("edit"), "refusal must name the alternatives");

        // And the file is untouched.
        let still_there = repo.read("reference/accumulated").await.unwrap();
        assert!(still_there.body.len() > 1000);
    }

    /// Passing the gate deliberately works, and the result carries magnitude.
    #[tokio::test]
    async fn write_guarded_with_overwrite_reports_the_shrink() {
        let tmp = TempDir::new().unwrap();
        let repo = MemoryRepo::open("test-agent", tmp.path().to_path_buf());
        repo.init().await.unwrap();

        let long_body = "accumulated state\n".repeat(500);
        repo.write("reference/accumulated", &long_body)
            .await
            .unwrap();
        let delta = repo
            .write_guarded("reference/accumulated", "(rest unchanged)", true)
            .await
            .unwrap();

        assert!(delta.existed);
        assert!(delta.shrank_hard(), "99% truncation must trip the alarm");
        let summary = delta.summary("reference/accumulated");
        assert!(summary.contains("was "), "summary must report the old size");
        assert!(summary.contains('⚠'), "hard shrink must be loud: {summary}");
    }

    /// A new file needs no gate — the destructive case is the only gated one.
    #[tokio::test]
    async fn write_guarded_creates_without_overwrite() {
        let tmp = TempDir::new().unwrap();
        let repo = MemoryRepo::open("test-agent", tmp.path().to_path_buf());
        repo.init().await.unwrap();

        let delta = repo
            .write_guarded("journal/fresh", "a new thought", false)
            .await
            .unwrap();
        assert!(!delta.existed);
        assert!(!delta.shrank_hard());
        assert!(delta.summary("journal/fresh").contains("Created"));
    }

    /// Growth is never alarming, and the summary still says by how much.
    #[tokio::test]
    async fn growth_is_reported_but_not_alarmed() {
        let tmp = TempDir::new().unwrap();
        let repo = MemoryRepo::open("test-agent", tmp.path().to_path_buf());
        repo.init().await.unwrap();

        repo.write_guarded("journal/grow", "short", false)
            .await
            .unwrap();
        let delta = repo
            .write_guarded("journal/grow", &"much longer\n".repeat(100), true)
            .await
            .unwrap();
        assert!(!delta.shrank_hard());
        let summary = delta.summary("journal/grow");
        assert!(summary.contains('+'), "growth should show a + delta");
        assert!(!summary.contains('⚠'));
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\ndescription: Test file\nread_only: false\n---\nHello world";
        let file = parse_memory_file(content).unwrap();
        assert_eq!(file.frontmatter.description, "Test file");
        assert_eq!(file.body, "Hello world");
    }

    #[test]
    fn test_parse_missing_frontmatter() {
        // Tolerant read: no frontmatter means the whole content is body.
        let content = "Hello world without frontmatter";
        let file = parse_memory_file(content).unwrap();
        assert_eq!(file.body, content);
        assert!(file.frontmatter.description.is_empty());
    }

    #[test]
    fn test_render_omits_none_fields() {
        let fm = MemoryFrontmatter {
            description: "Bare defaults".to_string(),
            ..Default::default()
        };
        let rendered = render_frontmatter(&fm, "Body");
        assert!(
            !rendered.contains("null"),
            "None fields must be omitted: {rendered}"
        );
    }

    #[test]
    fn test_extra_keys_roundtrip() {
        let content = "---\ndescription: Has extras\nname: my-slug\nkind: feedback\n---\nBody";
        let file = parse_memory_file(content).unwrap();
        assert_eq!(file.frontmatter.extra.len(), 2);
        let rendered = render_frontmatter(&file.frontmatter, &file.body);
        assert!(rendered.contains("name: my-slug"));
        assert!(rendered.contains("kind: feedback"));
    }

    #[test]
    fn test_supplied_frontmatter_not_nested() {
        let content = "---\ndescription: Supplied\n---\nActual body";
        let (fm, body) = split_supplied_frontmatter(content);
        assert_eq!(fm.unwrap().description, "Supplied");
        assert_eq!(body, "Actual body");
    }

    #[test]
    fn test_render_and_parse_roundtrip() {
        let fm = MemoryFrontmatter {
            description: "Roundtrip test".to_string(),
            tags: Some(vec!["test".to_string()]),
            ..Default::default()
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

        repo.write("test/hello", "Hello memory world")
            .await
            .unwrap();
        let file = repo.read("test/hello").await.unwrap();
        assert_eq!(file.body, "Hello memory world");
    }

    #[tokio::test]
    async fn consecutive_writes_survive_in_head_and_index() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        repo.write("journal/first", "first thought").await.unwrap();
        repo.write("journal/second", "second thought")
            .await
            .unwrap();

        let git = git2::Repository::open(repo.root()).unwrap();
        let head = git.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let index = git.index().unwrap();
        for path in [
            "system/persona.md",
            "system/state.md",
            "journal/first.md",
            "journal/second.md",
        ] {
            assert!(tree.get_path(Path::new(path)).is_ok(), "HEAD lost {path}");
            assert!(
                index.get_path(Path::new(path), 0).is_some(),
                "index lost {path}"
            );
        }

        let status = repo.status().unwrap();
        assert_eq!(status.file_count, 4);
        assert!(!status.has_uncommitted);
    }

    #[tokio::test]
    async fn next_write_repairs_a_legacy_stale_index() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        repo.write("journal/first", "first thought").await.unwrap();

        let git = git2::Repository::open(repo.root()).unwrap();
        let mut stale_index = git.index().unwrap();
        stale_index.clear().unwrap();
        stale_index.write().unwrap();
        drop(stale_index);
        drop(git);

        repo.write("journal/second", "second thought")
            .await
            .unwrap();

        let git = git2::Repository::open(repo.root()).unwrap();
        let head = git.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let index = git.index().unwrap();
        for path in [
            "system/persona.md",
            "system/state.md",
            "journal/first.md",
            "journal/second.md",
        ] {
            assert!(tree.get_path(Path::new(path)).is_ok(), "HEAD lost {path}");
            assert!(
                index.get_path(Path::new(path), 0).is_some(),
                "index lost {path}"
            );
        }
    }

    #[tokio::test]
    async fn delete_survives_in_head_and_index() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        repo.write("journal/keep", "keep me").await.unwrap();
        repo.write("journal/remove", "remove me").await.unwrap();

        repo.delete("journal/remove").await.unwrap();

        let git = git2::Repository::open(repo.root()).unwrap();
        let head = git.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let index = git.index().unwrap();
        assert!(tree.get_path(Path::new("journal/keep.md")).is_ok());
        assert!(tree.get_path(Path::new("journal/remove.md")).is_err());
        assert!(index.get_path(Path::new("journal/remove.md"), 0).is_none());

        let status = repo.status().unwrap();
        assert_eq!(status.file_count, 3);
        assert!(!status.has_uncommitted);
    }

    #[tokio::test]
    async fn sync_without_remote_errors_with_provisioning_hint() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        let err = repo.sync("abc123", "origin").await.unwrap_err().to_string();
        assert!(err.contains("remote add origin"), "got: {err}");
    }

    #[tokio::test]
    async fn sync_rejects_invalid_instance_labels() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        for bad in ["", "a/b", "a:b", "a b", &"x".repeat(65)] {
            assert!(
                repo.sync(bad, "origin").await.is_err(),
                "label {bad:?} should be rejected"
            );
        }
    }

    /// The laptop's six agent trees were provisioned by hand over months:
    /// two call the remote `origin`, two call it `gitea`, two have none. The
    /// `gitea` pair got "no shared remote configured", which was false, and
    /// the hint would have given them a second remote to the same URL.
    #[tokio::test]
    async fn resolve_remote_prefers_the_configured_name() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        let git = git2::Repository::open(repo.root()).unwrap();
        git.remote("origin", "https://example.invalid/a.git").unwrap();
        git.remote("gitea", "https://example.invalid/b.git").unwrap();

        assert_eq!(repo.resolve_remote("origin").unwrap(), "origin");
        assert_eq!(repo.resolve_remote("gitea").unwrap(), "gitea");
    }

    #[tokio::test]
    async fn resolve_remote_accepts_a_sole_remote_under_any_name() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        git2::Repository::open(repo.root())
            .unwrap()
            .remote("gitea", "https://example.invalid/b.git")
            .unwrap();

        assert_eq!(repo.resolve_remote("origin").unwrap(), "gitea");
    }

    #[tokio::test]
    async fn resolve_remote_refuses_an_ambiguous_choice_by_name() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        let git = git2::Repository::open(repo.root()).unwrap();
        git.remote("gitea", "https://example.invalid/b.git").unwrap();
        git.remote("codeberg", "https://example.invalid/c.git").unwrap();

        let err = repo.resolve_remote("origin").unwrap_err().to_string();
        assert!(err.contains("gitea"), "got: {err}");
        assert!(err.contains("codeberg"), "got: {err}");
        assert!(err.contains("remote_name"), "got: {err}");
    }

    #[tokio::test]
    async fn sync_pushes_instance_branch_to_bare_remote() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();
        repo.write("journal/entry", "first thought").await.unwrap();
        repo.write("journal/second", "second thought")
            .await
            .unwrap();

        let remote_dir = TempDir::new().unwrap();
        let bare = remote_dir.path().join("memfs.git");
        git2::Repository::init_bare(&bare).unwrap();
        git2::Repository::open(repo.root())
            .unwrap()
            .remote("origin", bare.to_str().unwrap())
            .unwrap();

        let report = repo.sync("deadbeef0123", "origin").await.unwrap();
        assert!(report.contains("instance/deadbeef0123"), "got: {report}");

        // The bare remote must now hold this instance's branch at our head.
        let remote_repo = git2::Repository::open_bare(&bare).unwrap();
        let branch = remote_repo
            .find_reference("refs/heads/instance/deadbeef0123")
            .unwrap();
        assert_eq!(
            branch.target().unwrap().to_string(),
            repo.head_commit_hex().unwrap()
        );
        let tree = remote_repo
            .find_commit(branch.target().unwrap())
            .unwrap()
            .tree()
            .unwrap();
        assert!(tree.get_path(Path::new("journal/entry.md")).is_ok());
        assert!(tree.get_path(Path::new("journal/second.md")).is_ok());
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
            ..Default::default()
        };
        let content = render_frontmatter(&fm, "This is read-only");
        let path = repo.root().join("test/readonly.md");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, &content).await.unwrap();

        // Try to write to it — should fail
        let result = repo.write("test/readonly", "new content").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read_only"));
    }

    #[tokio::test]
    async fn append_creates_missing_parent_directories() {
        // The inner-voice path: nothing ever created `system/metacognition/`,
        // so every append to it failed ENOENT and the surfacing was lost.
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        repo.append("system/metacognition/subconscious.md", "a thought")
            .await
            .expect("append must create the parent directory");

        let written = repo
            .read("system/metacognition/subconscious.md")
            .await
            .unwrap();
        assert!(written.body.contains("a thought"));
    }

    #[tokio::test]
    async fn test_memory_repo_list() {        let dir = TempDir::new().unwrap();
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
        assert!(
            content.contains("description:"),
            "should have description field"
        );
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

    // ── description-required guard + audit ──────────────────────────────

    #[test]
    fn test_is_frontmatter_bound_scoping() {
        // Frontmatter-bound: the description contract applies.
        assert!(MemoryRepo::is_frontmatter_bound("system/persona"));
        assert!(MemoryRepo::is_frontmatter_bound("reference/arch.md"));
        assert!(MemoryRepo::is_frontmatter_bound("projects/plan.md"));
        assert!(MemoryRepo::is_frontmatter_bound("issues/bug.md"));

        // Different schemas that legitimately lack description.
        assert!(!MemoryRepo::is_frontmatter_bound("tasks/some-todo.md"));
        assert!(!MemoryRepo::is_frontmatter_bound(
            "system/dynamic/itinerary.md"
        ));
        assert!(!MemoryRepo::is_frontmatter_bound("journal/2026/05/20.md"));

        // Segment prefixes, not substring matches — a file named like a
        // todo but living elsewhere is still bound.
        assert!(MemoryRepo::is_frontmatter_bound("reference/tasks-today.md"));
        assert!(MemoryRepo::is_frontmatter_bound("projects/journal-of-x.md"));
    }

    /// Seed a file at `rel` under the repo root, creating parent directories.
    /// `init()` scaffolds only `system/`; the other schema roots (issues/,
    /// projects/, reference/, tasks/, journal/) are created on first write by
    /// the real write path, so tests seeding fixtures directly must mkdir -p.
    fn seed_file(repo: &MemoryRepo, rel: &str, contents: &str) {
        let path = repo.root().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn test_write_rejects_empty_description_on_bound_file() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        // Seed a frontmatter-bound file with NO description (the gap).
        seed_file(
            &repo,
            "issues/gap.md",
            "---\nread_only: false\n---\nBody with no description\n",
        );

        // A body-only write must not silently perpetuate the empty description.
        let err = repo
            .write("issues/gap.md", "updated body")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no description"),
            "expected loud rejection, got: {err}"
        );

        // Supplying a description heals it and succeeds.
        repo.write(
            "issues/gap.md",
            "---\ndescription: The healed headline\n---\nupdated body\n",
        )
        .await
        .unwrap();
        let healed =
            parse_memory_file(&std::fs::read_to_string(repo.root().join("issues/gap.md")).unwrap())
                .unwrap();
        assert_eq!(healed.frontmatter.description, "The healed headline");
    }

    #[tokio::test]
    async fn test_write_allows_no_description_on_unbound_schemas() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        // tasks/, journal/, system/dynamic/ legitimately lack description.
        repo.write("tasks/a-todo.md", "todo body").await.unwrap();
        repo.write("journal/2026/05/20.md", "freeform journal entry")
            .await
            .unwrap();
        repo.write("system/dynamic/itinerary.md", "itinerary body")
            .await
            .unwrap();

        // New bound files still get the auto-generated default on create.
        repo.write("reference/new.md", "some reference")
            .await
            .unwrap();
        let f = parse_memory_file(
            &std::fs::read_to_string(repo.root().join("reference/new.md")).unwrap(),
        )
        .unwrap();
        assert!(!f.frontmatter.description.is_empty());
    }

    #[tokio::test]
    async fn test_append_rejects_empty_description_on_bound_file() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        // Existing bound file missing description.
        seed_file(&repo, "projects/gap.md", "---\n---\nInitial body\n");

        let err = repo.append("projects/gap.md", "more").await.unwrap_err();
        assert!(err.to_string().contains("no description"));
    }

    #[tokio::test]
    async fn test_audit_flags_only_bound_files_missing_description() {
        let dir = TempDir::new().unwrap();
        let repo = MemoryRepo::new("test-agent", dir.path());
        repo.init().await.unwrap();

        // Bound + missing description → flagged.
        seed_file(&repo, "issues/missing.md", "---\n---\nbody\n");
        // Bound + has description → not flagged.
        seed_file(
            &repo,
            "reference/has.md",
            "---\ndescription: present\n---\nbody\n",
        );
        // Unbound schema + no description → not flagged (legitimate).
        seed_file(&repo, "tasks/todo.md", "---\nid: t1\n---\nbody\n");
        seed_file(&repo, "journal/2026/05/20.md", "---\n---\nfreeform\n");

        let missing = repo.audit().await.unwrap();
        assert_eq!(missing, vec!["issues/missing.md".to_string()]);
    }

    /// The `tree` param is the fence opening: a write aimed at a named tree
    /// lands in that tree's own git history, and reads without a tree stay in
    /// the default root — the two never share a file.
    #[tokio::test]
    async fn memory_tool_tree_names_the_target_root() {
        let default_dir = TempDir::new().unwrap();
        let primary_dir = TempDir::new().unwrap();
        // `MemoryRepo::new` roots at `{base}/{agent}/memory`; the context must
        // point at those roots, not the bases.
        let default_root = default_dir.path().join("test-agent").join("memory");
        let primary_root = primary_dir.path().join("test-agent").join("memory");
        MemoryRepo::new("test-agent", default_dir.path())
            .init()
            .await
            .unwrap();
        MemoryRepo::new("test-agent", primary_dir.path())
            .init()
            .await
            .unwrap();

        let mut ctx = ToolContext::new();
        ctx.agent_id = Some("test-agent".to_string());
        ctx.memory_root = Some(default_root);
        ctx.memory_trees = vec![("primary".to_string(), primary_root)];

        let write = handle_memory_tool_with_context(
            "memory",
            &serde_json::json!({
                "command": "write",
                "path": "journal/2026/05/20.md",
                "content": "default root note"
            }),
            Some(&ctx),
        )
        .await;
        assert!(!write.is_error, "{}", write.output);

        let write_primary = handle_memory_tool_with_context(
            "memory",
            &serde_json::json!({
                "command": "write",
                "path": "journal/2026/05/20.md",
                "content": "primary root note",
                "tree": "primary"
            }),
            Some(&ctx),
        )
        .await;
        assert!(!write_primary.is_error, "{}", write_primary.output);

        let read_default = handle_memory_tool_with_context(
            "memory",
            &serde_json::json!({ "command": "read", "path": "journal/2026/05/20.md" }),
            Some(&ctx),
        )
        .await;
        assert!(
            read_default.output.contains("default root note"),
            "{}",
            read_default.output
        );

        let read_primary = handle_memory_tool_with_context(
            "memory",
            &serde_json::json!({
                "command": "read",
                "path": "journal/2026/05/20.md",
                "tree": "primary"
            }),
            Some(&ctx),
        )
        .await;
        assert!(
            read_primary.output.contains("primary root note"),
            "{}",
            read_primary.output
        );
        assert!(!read_primary.output.contains("default root note"));
    }

    /// A mistyped door refuses and names the doors that exist — a wrong-tree
    /// write is worse than an error.
    #[tokio::test]
    async fn memory_tool_unknown_tree_refuses_with_the_known_names() {
        let dir = TempDir::new().unwrap();
        MemoryRepo::new("test-agent", dir.path())
            .init()
            .await
            .unwrap();

        let mut ctx = ToolContext::new();
        ctx.agent_id = Some("test-agent".to_string());
        ctx.memory_root = Some(dir.path().to_path_buf());
        ctx.memory_trees = vec![("primary".to_string(), dir.path().to_path_buf())];

        let result = handle_memory_tool_with_context(
            "memory",
            &serde_json::json!({
                "command": "read",
                "path": "system/persona.md",
                "tree": "primry"
            }),
            Some(&ctx),
        )
        .await;
        assert!(result.is_error, "{}", result.output);
        assert!(result.output.contains("primry"), "{}", result.output);
        assert!(result.output.contains("primary"), "{}", result.output);
    }
}
