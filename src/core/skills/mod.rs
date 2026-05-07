//! Skills — units of specialization the agent can invoke.
//!
//! Per Constitution Article VI.3 and Cameron's "memfs + skills is the correct
//! abstraction" guidance: the unit of specialization is the skill, not the
//! agent. A single agent with skills in `implementing-feature`, `reviewing-
//! code`, `auditing-payments`, `writing-changelog` accumulates knowledge
//! across turns; four role-fragmented agents would each stay at day-one
//! competence forever.
//!
//! ## Discovery (4 tiers)
//!
//! Looked up in priority order:
//! 1. **Bundled** — skills shipped with the souveraine binary (compiled in or
//!    under `<install-dir>/skills/`). Lowest priority, most stable.
//! 2. **User** — `~/.souveraine/skills/` — operator's machine-wide skills.
//! 3. **Agent** — `<agent-memfs>/skills/` — skills attached to one agent.
//!    Versioned in the agent's git memfs; survives migration.
//! 4. **Project** — `.skills/` in the working directory — repo-local skills,
//!    highest priority. Cameron's pattern from Letta Code.
//!
//! Higher tiers shadow lower tiers by skill name. The full resolution table
//! is built at session start and can be inspected via `skill ls`.
//!
//! ## SKILL.md format
//!
//! Each skill is a directory with at minimum a `SKILL.md` file:
//!
//! ```markdown
//! ---
//! name: implementing-feature
//! description: Drive a feature from issue → design → code → review → docs
//! when_to_use: User asks for a new capability or behaviour change
//! tools: [memory, edit, bash, list_dir]
//! tier: project
//! ---
//!
//! ## Phase 1 — Orient
//! Read the linked issue. Identify files...
//! ```
//!
//! Required frontmatter: `name`, `description`. Optional: `when_to_use`,
//! `tools` (allow-list), `tier` (set automatically by discovery).
//!
//! Body is markdown — instructions the agent follows when the skill loads.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Discovery tier — used for shadowing precedence and human-facing labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillTier {
    Bundled,
    User,
    Agent,
    Project,
}

impl SkillTier {
    pub fn precedence(self) -> u8 {
        match self {
            SkillTier::Bundled => 0,
            SkillTier::User => 1,
            SkillTier::Agent => 2,
            SkillTier::Project => 3,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SkillTier::Bundled => "bundled",
            SkillTier::User => "user",
            SkillTier::Agent => "agent",
            SkillTier::Project => "project",
        }
    }
}

/// Required + optional frontmatter on a SKILL.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub when_to_use: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

/// One discovered skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub when_to_use: Option<String>,
    pub tools: Option<Vec<String>>,
    pub tier: SkillTier,
    /// Path to the directory containing SKILL.md.
    pub root: PathBuf,
    /// Body of SKILL.md (everything after frontmatter), loaded lazily on
    /// first invocation.
    body: Option<String>,
}

impl Skill {
    /// Load the skill body if not already loaded.
    pub async fn body(&mut self) -> Result<&str> {
        if self.body.is_none() {
            let raw = tokio::fs::read_to_string(self.root.join("SKILL.md"))
                .await
                .with_context(|| format!("reading SKILL.md at {}", self.root.display()))?;
            let body = strip_frontmatter(&raw);
            self.body = Some(body.to_string());
        }
        Ok(self.body.as_deref().unwrap())
    }
}

/// Resolved skill table (after shadowing across tiers).
#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, Skill>,
}

impl SkillRegistry {
    pub fn iter(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Skill> {
        self.skills.get_mut(name)
    }

    /// Render a system-prompt fragment listing all skills.
    /// Format mirrors Letta Code's available-skills section.
    pub fn render_system_addon(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n# Available Skills\n\n");
        out.push_str(
            "Skills are units of specialization. Invoke a skill when its trigger condition matches.\n\n",
        );
        for s in self.skills.values() {
            out.push_str(&format!(
                "- **{}** ({}) — {}\n",
                s.name,
                s.tier.label(),
                s.description
            ));
            if let Some(ref w) = s.when_to_use {
                out.push_str(&format!("  *when:* {}\n", w));
            }
        }
        out
    }
}

/// Discover skills from all 4 tiers, return a shadow-resolved registry.
/// Higher-precedence tiers replace lower ones with the same skill name.
pub async fn discover(
    bundled_dir: Option<&Path>,
    user_dir: Option<&Path>,
    agent_memfs_dir: Option<&Path>,
    project_dir: Option<&Path>,
) -> Result<SkillRegistry> {
    let mut registry = SkillRegistry::default();

    // Discover in precedence order; later ones overwrite by name.
    let sources: [(SkillTier, Option<PathBuf>); 4] = [
        (SkillTier::Bundled, bundled_dir.map(|p| p.to_path_buf())),
        (SkillTier::User, user_dir.map(|p| p.to_path_buf())),
        (SkillTier::Agent, agent_memfs_dir.map(|p| p.join("skills"))),
        (SkillTier::Project, project_dir.map(|p| p.join(".skills"))),
    ];

    for (tier, dir_opt) in sources {
        let Some(dir) = dir_opt else { continue };
        if !dir.exists() {
            continue;
        }
        let entries = tokio::fs::read_dir(&dir).await.ok();
        let Some(mut entries) = entries else { continue };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            match parse_skill(&skill_md, tier).await {
                Ok(skill) => {
                    registry.skills.insert(skill.name.clone(), skill);
                }
                Err(e) => {
                    tracing::warn!("skipping skill at {}: {}", path.display(), e);
                }
            }
        }
    }

    Ok(registry)
}

async fn parse_skill(skill_md: &Path, tier: SkillTier) -> Result<Skill> {
    let raw = tokio::fs::read_to_string(skill_md)
        .await
        .with_context(|| format!("reading {}", skill_md.display()))?;
    let (fm, _body) = split_frontmatter(&raw)
        .ok_or_else(|| anyhow!("SKILL.md missing frontmatter: {}", skill_md.display()))?;
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(fm)
        .with_context(|| format!("parsing frontmatter: {}", skill_md.display()))?;
    let root = skill_md
        .parent()
        .ok_or_else(|| anyhow!("SKILL.md has no parent dir: {}", skill_md.display()))?
        .to_path_buf();
    Ok(Skill {
        name: frontmatter.name,
        description: frontmatter.description,
        when_to_use: frontmatter.when_to_use,
        tools: frontmatter.tools,
        tier,
        root,
        body: None,
    })
}

fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let stripped = raw.strip_prefix("---\n")?;
    let end = stripped.find("\n---")?;
    let fm = &stripped[..end];
    let after = &stripped[end + 4..]; // skip "\n---"
    let body = after.strip_prefix('\n').unwrap_or(after);
    Some((fm, body))
}

fn strip_frontmatter(raw: &str) -> &str {
    split_frontmatter(raw).map(|(_, body)| body).unwrap_or(raw)
}

/// Default discovery paths derived from environment.
///
/// - Bundled: not yet (returns None until we ship bundled skills).
/// - User: `~/.souveraine/skills/`
/// - Agent: caller passes the agent's memfs dir (`/agents/<id>/memory.git/`).
/// - Project: current working directory.
pub fn default_discovery_paths(agent_memfs: Option<PathBuf>) -> (
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
) {
    let user = dirs::home_dir().map(|h| h.join(".souveraine").join("skills"));
    let project = std::env::current_dir().ok();
    (None, user, agent_memfs, project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_skill(dir: &Path, name: &str, fm: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let body = format!("---\n{}\n---\n\n# {}\n\nbody here\n", fm, name);
        std::fs::write(skill_dir.join("SKILL.md"), body).unwrap();
    }

    #[tokio::test]
    async fn discover_user_tier() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "feature-dev",
            "name: feature-dev\ndescription: Drive a feature\nwhen_to_use: New feature requested",
        );
        let reg = discover(None, Some(dir.path()), None, None).await.unwrap();
        assert_eq!(reg.skills.len(), 1);
        let s = reg.get("feature-dev").unwrap();
        assert_eq!(s.tier, SkillTier::User);
        assert_eq!(s.description, "Drive a feature");
    }

    #[tokio::test]
    async fn project_tier_shadows_user_tier() {
        let user = tempdir().unwrap();
        let project = tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".skills")).unwrap();

        write_skill(
            user.path(),
            "review",
            "name: review\ndescription: User-tier review skill",
        );
        write_skill(
            &project.path().join(".skills"),
            "review",
            "name: review\ndescription: Project-tier review skill",
        );

        let reg = discover(None, Some(user.path()), None, Some(project.path()))
            .await
            .unwrap();
        let s = reg.get("review").unwrap();
        assert_eq!(s.tier, SkillTier::Project);
        assert!(s.description.contains("Project-tier"));
    }

    #[tokio::test]
    async fn missing_dirs_silently_ignored() {
        let reg = discover(None, None, None, None).await.unwrap();
        assert!(reg.skills.is_empty());
    }

    #[tokio::test]
    async fn skill_body_loads_lazily() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "test",
            "name: test\ndescription: Test skill",
        );
        let mut reg = discover(None, Some(dir.path()), None, None).await.unwrap();
        let skill = reg.get_mut("test").unwrap();
        let body = skill.body().await.unwrap();
        assert!(body.contains("body here"));
    }

    #[test]
    fn render_system_addon_contains_skills() {
        let mut reg = SkillRegistry::default();
        reg.skills.insert(
            "feature-dev".to_string(),
            Skill {
                name: "feature-dev".to_string(),
                description: "Drive a feature".to_string(),
                when_to_use: Some("New feature".to_string()),
                tools: None,
                tier: SkillTier::Project,
                root: PathBuf::from("/tmp/x"),
                body: None,
            },
        );
        let out = reg.render_system_addon();
        assert!(out.contains("feature-dev"));
        assert!(out.contains("(project)"));
        assert!(out.contains("when:"));
    }

    #[test]
    fn precedence_ordering() {
        assert!(SkillTier::Project.precedence() > SkillTier::Agent.precedence());
        assert!(SkillTier::Agent.precedence() > SkillTier::User.precedence());
        assert!(SkillTier::User.precedence() > SkillTier::Bundled.precedence());
    }
}
