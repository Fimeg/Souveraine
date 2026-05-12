//! System prompt assembly — reads the agent's memfs and builds the
//! message the model sees before anything else.
//!
//! The substrate reads; the agent writes. If she edits her identity,
//! the next conversation reflects it.

use std::path::Path;
use tracing::debug;

use crate::core::skills::SkillRegistry;

/// Read a file from the agent's memory, stripping YAML frontmatter.
/// Returns empty string if the file doesn't exist.
async fn read_memory_file(memory_root: &Path, relative: &str) -> String {
    let path = memory_root.join(relative);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => strip_frontmatter(&content).to_string(),
        Err(_) => String::new(),
    }
}

/// Read all .md files in a directory under memory_root, concatenated.
async fn read_memory_dir(memory_root: &Path, relative: &str) -> String {
    let dir = memory_root.join(relative);
    let mut parts = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        let mut paths = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") && p.is_file() {
                paths.push(p);
            }
        }
        paths.sort();
        for p in paths {
            if let Ok(content) = tokio::fs::read_to_string(&p).await {
                let body = strip_frontmatter(&content);
                if !body.trim().is_empty() {
                    parts.push(body.to_string());
                }
            }
        }
    }

    parts.join("\n\n---\n\n")
}

fn strip_frontmatter(raw: &str) -> &str {
    if let Some(stripped) = raw.strip_prefix("---\n") {
        if let Some(end) = stripped.find("\n---") {
            let after = &stripped[end + 4..];
            return after.trim_start_matches('\n');
        }
    }
    raw
}

async fn build_memory_orientation(memory_root: &Path) -> String {
    if !memory_root.exists() {
        return String::new();
    }

    let mut dirs: Vec<String> = Vec::new();
    collect_dirs(memory_root, memory_root, &mut dirs).await;

    if dirs.is_empty() {
        return String::new();
    }

    dirs.sort();
    let tree = dirs.join("\n");
    format!(
        "## Memory\n\n\
         Your memory is a git-backed directory of markdown files with YAML frontmatter. \
         Paths you pass to the `memory` tool are relative to your memory root.\n\n\
         Current territories:\n```\n{}\n```",
        tree
    )
}

async fn collect_dirs(base: &Path, current: &Path, out: &mut Vec<String>) {
    let Ok(mut entries) = tokio::fs::read_dir(current).await else {
        return;
    };
    let mut has_children = false;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            has_children = true;
            let rel = path.strip_prefix(base).unwrap_or(&path);
            out.push(format!("{}/", rel.display()));
            Box::pin(collect_dirs(base, &path, out)).await;
        }
    }
    if !has_children && current != base {
        let rel = current.strip_prefix(base).unwrap_or(current);
        let count = count_md_files(current).await;
        if count > 0 {
            let idx = out.iter().position(|d| d == &format!("{}/", rel.display()));
            if let Some(i) = idx {
                out[i] = format!("{}/ ({} files)", rel.display(), count);
            }
        }
    }
}

async fn count_md_files(dir: &Path) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
            count += 1;
        }
    }
    count
}

/// Build the system prompt from an agent's memfs.
///
/// Reads identity, covenant, human context, and state files. Appends
/// skill listings if any are discovered. The result is a single string
/// that becomes the system message at position 0 in the conversation.
pub async fn build_system_prompt(
    memory_root: &Path,
    skills: Option<&SkillRegistry>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // 1. Core identity — try structured dir first, then flat persona.md
    let identity = read_memory_dir(memory_root, "system/identity").await;
    if !identity.is_empty() {
        sections.push(identity);
    } else {
        let persona = read_memory_file(memory_root, "system/persona.md").await;
        if !persona.is_empty() {
            sections.push(persona);
        } else {
            let persona_flat = read_memory_file(memory_root, "system/persona/identity.md").await;
            if !persona_flat.is_empty() {
                sections.push(persona_flat);
            }
        }
    }

    // 2. Covenant (sacred, read-only boundaries)
    let covenant = read_memory_dir(memory_root, "system/covenant").await;
    if !covenant.is_empty() {
        sections.push(covenant);
    }

    // 3. Human context
    let human = read_memory_dir(memory_root, "system/human").await;
    if human.is_empty() {
        let human_flat = read_memory_file(memory_root, "system/human.md").await;
        if !human_flat.is_empty() {
            sections.push(human_flat);
        }
    } else {
        sections.push(human);
    }

    // 4. State
    let state = read_memory_file(memory_root, "system/state.md").await;
    if !state.is_empty() {
        sections.push(state);
    }

    // 5. Memory orientation — tell the agent about her memory territory
    let memory_orientation = build_memory_orientation(memory_root).await;
    if !memory_orientation.is_empty() {
        sections.push(memory_orientation);
    }

    // 6. Skills
    if let Some(registry) = skills {
        let addon = registry.render_system_addon();
        if !addon.is_empty() {
            sections.push(addon);
        }
    }

    let prompt = sections.join("\n\n---\n\n");

    if prompt.is_empty() {
        debug!("system prompt: no identity files found, using minimal default");
        "You are a Souveraine agent. Your memory files will define who you are.".to_string()
    } else {
        debug!("system prompt: assembled {} sections from memfs", sections.len());
        prompt
    }
}

/// Build the subconscious agent's system prompt from its own memfs.
/// Reads identity, mandate, and ledger orientation from the subconscious
/// agent's memory root. Falls back to empty (caller uses hardcoded
/// default) if files don't exist.
pub async fn build_aster_prompt(
    subconscious_memory_root: &Path,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    let identity = read_memory_file(subconscious_memory_root, "system/persona.md").await;
    if !identity.is_empty() {
        sections.push(identity);
    }

    let mandate = read_memory_file(subconscious_memory_root, "system/subconscious.md").await;
    if !mandate.is_empty() {
        sections.push(mandate);
    }

    let ledger_orientation = build_ledger_orientation(subconscious_memory_root).await;
    if !ledger_orientation.is_empty() {
        sections.push(ledger_orientation);
    }

    if sections.is_empty() {
        return String::new();
    }

    sections.join("\n\n---\n\n")
}

/// Build ledger orientation for the subconscious prompt.
///
/// Scans `ledger/` for .md files, counts entries, and injects the last
/// few entries from each file so the subconscious has live context
/// (OpenHarness pattern: recent journal → active context).
async fn build_ledger_orientation(memory_root: &Path) -> String {
    let ledger_dir = memory_root.join("ledger");
    if !ledger_dir.exists() {
        return String::new();
    }

    let mut files: Vec<(String, usize, Vec<String>)> = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&ledger_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") && p.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = tokio::fs::read_to_string(&p).await {
                    let entries: Vec<String> = content
                        .lines()
                        .filter(|l| l.starts_with('[') && l.contains(']'))
                        .map(|l| l.to_string())
                        .collect();
                    let count = entries.len();
                    let recent: Vec<String> = entries.into_iter().rev().take(3).collect();
                    files.push((name, count, recent));
                } else {
                    files.push((name, 0, Vec::new()));
                }
            }
        }
    }

    if files.is_empty() {
        return String::new();
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut listing = String::new();
    for (name, count, recent) in &files {
        if *count > 0 {
            listing.push_str(&format!("  ledger/{} ({} entries)\n", name, count));
            for line in recent.iter().rev() {
                listing.push_str(&format!("    {}\n", line));
            }
        } else {
            listing.push_str(&format!("  ledger/{}\n", name));
        }
    }

    format!(
        "## Ledgers\n\n\
         Your persistent observation store. These files survive compaction and \
         accumulate across sessions.\n\n\
         ```\n{}\
         ```\n\n\
         **Workflow:** Before writing a new entry, `memory read` the relevant ledger \
         to check if the same issue was already flagged. If new, `memory append` a \
         timestamped line: `[YYYY-MM-DD HH:MM] observation`. To resolve, \
         append: `[YYYY-MM-DD HH:MM] RESOLVED — note`.\n\n\
         Route observations by type:\n\
         - Unfulfilled promises → `ledger/commitments.md`\n\
         - Unverified beliefs → `ledger/assumptions.md`\n\
         - Recurring behaviors → `ledger/patterns.md`\n\
         - Intention/action mismatch → `ledger/drift_log.md`\n\
         - Tone or trust shifts → `ledger/relationships.md`\n\
         - System errors or resource issues → `ledger/infrastructure.md`",
        listing
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn builds_from_identity_dir() {
        let dir = tempdir().unwrap();
        let mem = dir.path();
        let id_dir = mem.join("system/identity");
        std::fs::create_dir_all(&id_dir).unwrap();
        std::fs::write(
            id_dir.join("self.md"),
            "---\ndescription: test\n---\n\n# I am Test Agent\n",
        ).unwrap();

        let prompt = build_system_prompt(mem, None).await;
        assert!(prompt.contains("I am Test Agent"));
    }

    #[tokio::test]
    async fn falls_back_to_persona_md() {
        let dir = tempdir().unwrap();
        let mem = dir.path();
        let sys = mem.join("system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("persona.md"),
            "---\ndescription: test\n---\n\nI am a persona file agent.\n",
        ).unwrap();

        let prompt = build_system_prompt(mem, None).await;
        assert!(prompt.contains("persona file agent"));
    }

    #[tokio::test]
    async fn empty_memfs_gets_default() {
        let dir = tempdir().unwrap();
        let prompt = build_system_prompt(dir.path(), None).await;
        assert!(prompt.contains("Souveraine agent"));
    }

    #[tokio::test]
    async fn strips_frontmatter() {
        let input = "---\ndescription: test\nlimit: 5000\n---\n\nActual content here.";
        assert_eq!(strip_frontmatter(input), "Actual content here.");
    }

    #[tokio::test]
    async fn aster_prompt_from_files() {
        let dir = tempdir().unwrap();
        let sub_mem = dir.path();
        let sys = sub_mem.join("system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("persona.md"),
            "---\ndescription: WHO I AM\n---\n\n# I Am Aster\n",
        ).unwrap();
        std::fs::write(
            sys.join("subconscious.md"),
            "---\ndescription: mandate\n---\n\n# Aster's Mandate\n\nComplete what was left.\n",
        ).unwrap();

        let prompt = build_aster_prompt(sub_mem).await;
        assert!(prompt.contains("I Am Aster"));
        assert!(prompt.contains("Complete what was left"));
    }

    #[tokio::test]
    async fn aster_prompt_includes_ledger_orientation() {
        let dir = tempdir().unwrap();
        let sub_mem = dir.path();

        let sys = sub_mem.join("system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("persona.md"),
            "---\ndescription: test\n---\n\n# I Am Aster\n",
        ).unwrap();

        let ledger_dir = sub_mem.join("ledger");
        std::fs::create_dir_all(&ledger_dir).unwrap();
        std::fs::write(
            ledger_dir.join("commitments.md"),
            "---\ndescription: test\n---\n\n# Commitments\n\n[2026-05-12 10:00] Save the config\n[2026-05-12 10:30] RESOLVED — config saved\n",
        ).unwrap();
        std::fs::write(
            ledger_dir.join("patterns.md"),
            "---\ndescription: test\n---\n\n# Patterns\n\n",
        ).unwrap();

        let prompt = build_aster_prompt(sub_mem).await;
        assert!(prompt.contains("## Ledgers"), "should have ledger section");
        assert!(prompt.contains("commitments.md (2 entries)"), "should count entries");
        assert!(prompt.contains("Save the config"), "should show recent entries");
        assert!(prompt.contains("patterns.md"), "should list empty ledger too");
    }

    #[tokio::test]
    async fn ledger_orientation_empty_without_dir() {
        let dir = tempdir().unwrap();
        let orientation = build_ledger_orientation(dir.path()).await;
        assert!(orientation.is_empty());
    }
}
