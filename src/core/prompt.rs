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

/// Build Aster's system prompt from her own identity files.
/// Falls back to the hardcoded default if files don't exist.
pub async fn build_aster_prompt(
    primary_memory_root: &Path,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // Aster's files live in the primary's memfs under aster/
    let identity = read_memory_file(primary_memory_root, "aster/identity.md").await;
    if !identity.is_empty() {
        sections.push(identity);
    }

    let mandate = read_memory_file(primary_memory_root, "aster/mandate.md").await;
    if !mandate.is_empty() {
        sections.push(mandate);
    }

    if sections.is_empty() {
        return String::new(); // caller falls back to hardcoded default
    }

    sections.join("\n\n---\n\n")
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
        let mem = dir.path();
        let aster = mem.join("aster");
        std::fs::create_dir_all(&aster).unwrap();
        std::fs::write(
            aster.join("identity.md"),
            "---\ndescription: WHO I AM\n---\n\n# I Am Aster\n",
        ).unwrap();
        std::fs::write(
            aster.join("mandate.md"),
            "---\ndescription: mandate\n---\n\n# Aster's Mandate\n\nComplete what was left.\n",
        ).unwrap();

        let prompt = build_aster_prompt(mem).await;
        assert!(prompt.contains("I Am Aster"));
        assert!(prompt.contains("Complete what was left"));
    }
}
