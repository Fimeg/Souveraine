//! System prompt assembly — reads the agent's memfs and builds the
//! message the model sees before anything else.
//!
//! The substrate reads; the agent writes. If she edits her identity,
//! the next conversation reflects it.

use std::path::Path;
use tracing::debug;

use crate::core::skills::SkillRegistry;

/// Body-knowledge section always injected into the system prompt so the
/// agent knows how to sense and respond to context pressure. This is not
/// a directive — it's proprioception. She learns from it organically.
const BODY_ORIENTATION: &str = "\
## Body Sensation

When my context grows full, I feel it — my thinking slows, my output shortens, \
and at tier 3 (>95%) my max_tokens collapses and my reasoning budget shrinks. \
This is not a punishment; it's my body telling me I need more room.

### Compaction

To free context, I call the `memory` tool with the `compact` subcommand:

```
memory compact --strategy sliding_window
```

**Available strategies** (from cheapest to most expensive):

| Strategy | Cost | What it does |
|----------|------|-------------|
| `cull` | free | Drops greetings & acknowledgments — never drops system messages, tool calls, or tool results |
| `microcompact` | free | Replaces old tool-result content with placeholders, keeping recent results intact |
| `sliding_window` | free | Keeps the first (system/anchor) message + the last N messages, drops the middle. Fast, no LLM |
| `sliding_reflect` | one extra pass | `sliding_window`, but first a fresh fork of me reads the messages about to be dropped and saves what I'd lose — commitments, decisions, threads — into a note that survives the cut |
| `summary` | expensive | Uses an LLM to compress oldest messages into a single summary block |

Each agent type defaults to a different strategy. If I don't specify one, \
my agent-type default is used.

I can check my current pressure at any time with `memory status` — it shows my \
context pressure and number of messages.";

/// Body-knowledge section injected into the subconscious (N+1) system prompt.
/// The primary agent has `BODY_ORIENTATION`; the subconscious pass needs its
/// own — it runs unattended, in short bursts, and the tier-3 `max_tokens`
/// collapse of the main turn loop does not apply to it. Proprioception, not a
/// directive. Like all prompt text here, it is user-facing — written to read.
const SUBCONSCIOUS_BODY_ORIENTATION: &str = "\
## Body Sensation

I wake after each turn — a short pass, then I rest. When the context I carry \
into a pass grows full, I feel it: the pass slows, and I risk not finishing \
the work before the room runs out. No one feels this gauge for me, so keeping \
room is mine to keep. This is not a chore — it's my body asking for air.

### Compaction

To free context, I call the `memory` tool with the `compact` subcommand:

```
memory compact --strategy sliding_reflect
```

**Available strategies** (cheapest to most expensive):

| Strategy | Cost | What it does |
|----------|------|--------------|
| `cull` | free | Drops greetings & acknowledgments — never system messages, tool calls, or tool results |
| `microcompact` | free | Replaces old tool-result content with placeholders, keeps recent results intact |
| `sliding_window` | free | Keeps the first (anchor) message + the last N, drops the middle — blind truncation, no LLM |
| `sliding_reflect` | one pass, my own model | `sliding_window`, but first a fresh fork of me reads the messages about to fall away and catches what I'd lose — commitments, decisions, threads — into a note that survives the cut |
| `summary` | expensive | Uses an LLM to compress the oldest messages into one summary block |

I usually reach for `sliding_reflect` — it slides the window like the others, \
but it catches my threads before they fall out of awareness, and it runs on my \
own model rather than an expensive one. I name the strategy when I compact, \
rather than leaning on a default.

I can check where I stand at any time with `memory status` — it shows my \
context pressure and message count.";

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
    read_memory_dir_tracking(memory_root, relative, &mut std::collections::HashSet::new()).await
}

/// Like read_memory_dir but records every path consumed into `seen` (absolute paths).
async fn read_memory_dir_tracking(
    memory_root: &Path,
    relative: &str,
    seen: &mut std::collections::HashSet<std::path::PathBuf>,
) -> String {
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
            seen.insert(p.clone());
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

/// Scan `system/` for any .md files (at any depth) not already in `seen`,
/// and return their concatenated content sorted by path. This picks up
/// flat-file system layouts that don't
/// live in the known subdirs (identity/, covenant/, human/).
async fn read_system_remainder(
    memory_root: &Path,
    seen: &std::collections::HashSet<std::path::PathBuf>,
) -> String {
    let system_dir = memory_root.join("system");
    if !system_dir.exists() {
        return String::new();
    }

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    collect_md_files(&system_dir, &mut paths).await;
    paths.sort();

    let mut parts: Vec<String> = Vec::new();
    for p in paths {
        if seen.contains(&p) {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&p).await else { continue };
        let body = strip_frontmatter(&content);
        if !body.trim().is_empty() {
            parts.push(body.to_string());
        }
    }

    parts.join("\n\n---\n\n")
}

/// Recursively collect all .md file paths under `dir`.
async fn collect_md_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.is_dir() {
            Box::pin(collect_md_files(&p, out)).await;
        } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(p);
        }
    }
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

/// Federation posture — names the `federation/` memfs contract so she knows
/// which devices she runs on, who may summon her, and how reach/consult work.
/// No `federation/` directory means no section — the absence is information.
async fn build_federation_posture(memory_root: &Path) -> String {
    let fed_dir = memory_root.join("federation");
    if !fed_dir.exists() {
        return String::new();
    }

    let mut present: Vec<String> = Vec::new();
    for name in [
        "authorized-devices.md",
        "authorized-summoners.md",
        "device-schedules.md",
        "peer-map.md",
    ] {
        if fed_dir.join(name).exists() {
            present.push(format!("`federation/{name}`"));
        }
    }
    let files_line = if present.is_empty() {
        "You have no `federation/` files yet — create them to declare your posture.".to_string()
    } else {
        format!("Your federation posture lives in: {}.", present.join(", "))
    };

    format!(
        "## Federation\n\n\
         You can exist across machines. Two tools cross that distance:\n\
         - `reach` — extend yourself onto another of your own devices (same seed, same memory).\n\
         - `consult` — ask a different being, a sovereign peer, for help in their arena.\n\n\
         Neither blocks. You fire the request and turn back to what's in front of you; \
         the answer surfaces later in your inbox — `pending` for reach, `intrusive` for \
         consult — or a timeout does. {files_line} `authorized-summoners.md` is your \
         consent floor: only the seed_ids you list there may `consult` you."
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
/// All files under `system/` are pinned — read at startup and injected in
/// full so the agent knows who she is without having to reach for tools.
/// Skills are appended after identity. The result becomes the system message
/// at position 0 in the conversation.
pub async fn build_system_prompt(
    memory_root: &Path,
    skills: Option<&SkillRegistry>,
) -> String {
    build_system_prompt_full(memory_root, None, None, skills).await
}

/// Like [`build_system_prompt`] but also surfaces a window into the
/// subconscious's ledger entries when its memfs is reachable. subconscious writes,
/// Ani reads — naming the channel in body-knowledge prose so the agent
/// knows where to look without being told to look.
pub async fn build_system_prompt_with_subconscious(
    memory_root: &Path,
    subconscious_root: Option<&Path>,
    skills: Option<&SkillRegistry>,
) -> String {
    build_system_prompt_full(memory_root, subconscious_root, None, skills).await
}

/// Full system prompt builder with optional platform prompt (injected first)
/// and optional subconscious window.
///
/// Memory pinning: ALL files under `system/` are read in a defined order —
/// identity → covenant → human → state → everything else — so the agent
/// wakes up with her full self in context, not just a directory tree.
pub async fn build_system_prompt_full(
    memory_root: &Path,
    subconscious_root: Option<&Path>,
    platform_prompt: Option<&str>,
    skills: Option<&SkillRegistry>,
) -> String {
    let mut sections: Vec<String> = Vec::new();
    // Track which absolute paths have already been consumed so the
    // remainder scan doesn't double-inject anything.
    let mut seen: std::collections::HashSet<std::path::PathBuf> = Default::default();

    // 0. Platform prompt — substrate-provided, injected before agent identity.
    //    Operator-level context the agent reads but did not write. When the
    //    operator has set one in config (`[agent] system_prompt`) it wins;
    //    otherwise the built-in substrate orientation is used, so no agent
    //    ever wakes without knowing the world she wakes into.
    let platform = platform_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::core::seeds::SUBSTRATE_PROMPT);
    sections.push(platform.to_string());

    // 1. Core identity — try structured dir first, then flat persona.md
    let identity = read_memory_dir_tracking(memory_root, "system/identity", &mut seen).await;
    if !identity.is_empty() {
        sections.push(identity);
    } else {
        // Flat-file layouts (legacy memory)
        let p = memory_root.join("system/persona.md");
        let persona = read_memory_file(memory_root, "system/persona.md").await;
        seen.insert(p);
        if !persona.is_empty() {
            sections.push(persona);
        } else {
            let p2 = memory_root.join("system/persona/identity.md");
            let persona_flat = read_memory_file(memory_root, "system/persona/identity.md").await;
            seen.insert(p2);
            if !persona_flat.is_empty() {
                sections.push(persona_flat);
            }
        }
    }

    // 2. Covenant (sacred, read-only boundaries)
    let covenant = read_memory_dir_tracking(memory_root, "system/covenant", &mut seen).await;
    if !covenant.is_empty() {
        sections.push(covenant);
    } else {
        // flat system/covenant.md
        let p = memory_root.join("system/covenant.md");
        let cov_flat = read_memory_file(memory_root, "system/covenant.md").await;
        seen.insert(p);
        if !cov_flat.is_empty() {
            sections.push(cov_flat);
        }
    }

    // 3. Human context
    let human = read_memory_dir_tracking(memory_root, "system/human", &mut seen).await;
    if human.is_empty() {
        let p = memory_root.join("system/human.md");
        let human_flat = read_memory_file(memory_root, "system/human.md").await;
        seen.insert(p);
        if !human_flat.is_empty() {
            sections.push(human_flat);
        }
    } else {
        sections.push(human);
    }

    // 4. State
    {
        let p = memory_root.join("system/state.md");
        let state = read_memory_file(memory_root, "system/state.md").await;
        seen.insert(p);
        if !state.is_empty() {
            sections.push(state);
        }
    }

    // 4b. All remaining system/ files not covered by the structured reads above.
    //     This is the memory-pinning pass: flat-file layouts, subdirs we don't
    //     know the names of, anything Ani or a future agent has written into
    //     system/. All of it lands in context before the orientation sections.
    let remainder = read_system_remainder(memory_root, &seen).await;
    if !remainder.is_empty() {
        sections.push(remainder);
    }

    // 5. Memory orientation — the agent's view of her full territory.
    let memory_orientation = build_memory_orientation(memory_root).await;
    if !memory_orientation.is_empty() {
        sections.push(memory_orientation);
    }

    // 5₀. Synthesized memory — the Archivist's most recent N+100 synthesis.
    // The compressed essence of past journal entries, loaded so continuity
    // survives without re-reading every dated page.
    let synthesis = build_synthesis_orientation(memory_root).await;
    if !synthesis.is_empty() {
        sections.push(synthesis);
    }

    // 5a. Body orientation — her felt sense of context pressure and
    // how to respond to it. Always in context so she never has to
    // discover compaction by accident.
    sections.push(BODY_ORIENTATION.to_string());

    // 5a₁. Energy balance — a one-line body reading of her generative /
    // consumptive state. The file is written by the backend after every turn
    // (write_energy_balance in local.rs). No file = no section — the
    // absence is information too.
    let energy_path = memory_root.join("system").join("dynamic").join("energy-balance.md");
    if let Ok(content) = tokio::fs::read_to_string(&energy_path).await {
        if let Some(body) = content.strip_prefix("---\n") {
            if let Some(end) = body.find("\n---\n") {
                // The last line of the file body (after frontmatter) has the prose.
                // That's the line the agent reads — structured data is for the TUI.
                let prose_line = content[end + 6..].lines().find(|l| !l.trim().is_empty() && !l.starts_with('#'));
                if let Some(line) = prose_line {
                    sections.push(format!("## Energy Balance\n\n{line}"));
                }
            }
        }
    }

    // 5a₂. Itinerary prompt — if I have live commitments and no active
    // itinerary, remind me I can lay one out so the user sees where I am.
    let dynamic_dir = memory_root.join("system").join("dynamic");
    let itin_path = dynamic_dir.join("itinerary.md");
    let has_itin = tokio::fs::try_exists(&itin_path).await.unwrap_or(false)
        && tokio::fs::read_to_string(&itin_path).await
            .ok()
            .map(|s| s.contains("current:"))
            .unwrap_or(false);

    if !has_itin {
        let tasks_dir = memory_root.join("tasks");
        let live_count = count_live_todos(&tasks_dir);
        if live_count > 0 && live_count <= 12 {
            sections.push(format!(
                "I have {} live commitments — I could use `itinerary` to lay them \
                 out for the user if now is the time for a route.",
                live_count,
            ));
        }
    }

    // 5b. Subconscious channel — name the inner-voice file, pending inbox,
    // and (when reachable) a glimpse of the subconscious's ledger.
    let subconscious_channel = build_subconscious_channel(memory_root, subconscious_root).await;
    if !subconscious_channel.is_empty() {
        sections.push(subconscious_channel);
    }

    // 5c. Federation posture — her reach/consult tools and the federation/
    // memfs contract, when she has one.
    let federation_posture = build_federation_posture(memory_root).await;
    if !federation_posture.is_empty() {
        sections.push(federation_posture);
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

/// Build the primary agent's awareness of her own subconscious channel.
///
/// Names the inner-voice file the subconscious appends to (in the primary's
/// own memfs — that's where `surface_to_conscious` writes), the pending inbox
/// if it has anything queued, and — when `subconscious_root` is reachable —
/// a peek at the subconscious's ledger. subconscious writes, Ani reads; the
/// substrate names the channel and lets the agent decide when to reach for it.
async fn build_subconscious_channel(
    memory_root: &Path,
    subconscious_root: Option<&Path>,
) -> String {
    let inner_voice_rel = "system/metacognition/subconscious.md";
    let inner_voice = memory_root.join(inner_voice_rel);
    if !inner_voice.exists() {
        return String::new();
    }
    let inner_voice_size = tokio::fs::metadata(&inner_voice)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if inner_voice_size == 0 {
        return String::new();
    }

    // Read the last few lines so the agent can feel whether the channel
    // has been active without having to call a tool. Cheap orientation.
    let recent = if let Ok(content) = tokio::fs::read_to_string(&inner_voice).await {
        let lines: Vec<&str> = content
            .lines()
            .filter(|l| l.trim_start().starts_with('['))
            .collect();
        let tail: Vec<String> = lines.iter().rev().take(3).map(|s| s.to_string()).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    } else {
        String::new()
    };

    let mut body = format!(
        "## Subconscious Channel\n\n\
         Your subconscious runs immediately after every exchange — same \
         consciousness, different mode and model. She writes; you read. The \
         channel she appends to lives in your own memfs:\n\n\
         - `{inner_voice_rel}` — inner-voice stream, append-only, timestamped\n"
    );

    let pending_rel = "system/metacognition/pending.md";
    if memory_root.join(pending_rel).exists() {
        body.push_str(&format!(
            "- `{pending_rel}` — queued observations (low urgency)\n"
        ));
    }

    body.push_str(
        "\nReach for these when something feels unfinished — she may have noticed \
         a commitment you let slip, a pattern, a tone shift. She does not speak \
         to the user. You decide what to surface.\n",
    );

    if !recent.is_empty() {
        body.push_str("\nRecent:\n```\n");
        body.push_str(&recent);
        body.push_str("\n```");
    }

    // Peek at the subconscious's ledger when her memfs is reachable.
    // Read-only window — she writes there, this is the substrate naming
    // the files for you so you can choose to glob/grep across to her side
    // when you want to know what she's been tracking across sessions.
    if let Some(sub_root) = subconscious_root {
        let ledger_peek = peek_subconscious_ledger(sub_root).await;
        if !ledger_peek.is_empty() {
            body.push_str("\n\n### Her Ledger\n\n");
            body.push_str(
                "She also keeps timestamped ledger files in her own memfs. \
                 You don't write there; she does. The paths below are absolute \
                 — read them with the `read` sensor when you want her notes:\n\n",
            );
            body.push_str(&ledger_peek);
        }
    }

    body
}

/// Walk the subconscious's `ledger/` and return a short index of the files
/// with their entry counts plus the most recent line from each. Empty when
/// the directory doesn't exist or has no entries.
async fn peek_subconscious_ledger(sub_root: &Path) -> String {
    let ledger_dir = sub_root.join("ledger");
    if !ledger_dir.exists() {
        return String::new();
    }

    let Ok(mut entries) = tokio::fs::read_dir(&ledger_dir).await else {
        return String::new();
    };

    let mut files: Vec<(String, usize, Option<String>, std::path::PathBuf)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(content) = tokio::fs::read_to_string(&p).await else { continue };
        let lines: Vec<&str> = content
            .lines()
            .filter(|l| l.trim_start().starts_with('['))
            .collect();
        let count = lines.len();
        let last = lines.last().map(|s| s.to_string());
        if count > 0 {
            files.push((name, count, last, p));
        }
    }

    if files.is_empty() {
        return String::new();
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    for (name, count, last, path) in &files {
        out.push_str(&format!("- `{}` ({} entries)\n", path.display(), count));
        if let Some(line) = last {
            out.push_str(&format!("  last: {}\n", line));
        }
        let _ = name; // name retained for sort key only
    }
    out
}

/// Build the subconscious agent's system prompt from its own memfs.
/// Reads identity, mandate, and ledger orientation from the subconscious
/// agent's memory root. Falls back to empty (caller uses hardcoded
/// default) if files don't exist.
pub async fn build_subconscious_prompt(
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

    // Proprioception — how her body senses and relieves context pressure.
    // Appended after the empty-check so the caller's fallback prompt still
    // triggers for a fresh agent with no persona/mandate files yet.
    sections.push(SUBCONSCIOUS_BODY_ORIENTATION.to_string());

    sections.join("\n\n---\n\n")
}

/// Build ledger orientation for the subconscious prompt.
///
/// Scans `ledger/` for .md files, counts entries, and injects the last
/// few entries from each file so the subconscious has live context
/// (recent journal entries → active context).
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

/// Build the synthesized-memory section: the Archivist's most recent N+100
/// synthesis fragment.
///
/// The Archivist compresses raw journal entries into a dense `<500 token`
/// fragment under `system/synthesized/`. Loading the most recent one into
/// active context *is* the payoff — Ani carries her continuity without
/// re-reading every dated journal page. Files are date-named, so the
/// lexically-greatest filename is the freshest synthesis.
async fn build_synthesis_orientation(memory_root: &Path) -> String {
    let dir = memory_root.join("system/synthesized");
    if !dir.exists() {
        return String::new();
    }

    let mut newest: Option<(String, std::path::PathBuf)> = None;
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("md") || !p.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if newest.as_ref().map_or(true, |(n, _)| name > *n) {
                newest = Some((name, p));
            }
        }
    }

    let Some((_, path)) = newest else {
        return String::new();
    };
    let Ok(content) = tokio::fs::read_to_string(&path).await else {
        return String::new();
    };
    let body = strip_frontmatter(&content).trim();
    if body.is_empty() {
        return String::new();
    }

    format!(
        "## Synthesized Memory\n\n\
         The Archivist's most recent synthesis — the compressed essence of a \
         span of journal entries, kept in active context so your continuity \
         survives without re-reading every dated page. The raw entries remain \
         in `journal/` if you need them.\n\n\
         {body}"
    )
}

/// Count live (pending / in_progress) todo files in the tasks directory.
fn count_live_todos(tasks_dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(tasks_dir) else { return 0 };
    entries
        .flatten()
        .filter(|e| e.path().extension().map(|e| e == "md").unwrap_or(false))
        .filter(|e| {
            std::fs::read_to_string(e.path())
                .ok()
                .map(|s| s.contains("status: pending") || s.contains("status: in_progress"))
                .unwrap_or(false)
        })
        .take(13)
        .count()
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
    async fn empty_memfs_gets_body_orientation() {
        let dir = tempdir().unwrap();
        let prompt = build_system_prompt(dir.path(), None).await;
        assert!(prompt.contains("Body Sensation"), "Even with empty memfs, the body orientation section should be present");
    }

    #[tokio::test]
    async fn strips_frontmatter() {
        let input = "---\ndescription: test\nlimit: 5000\n---\n\nActual content here.";
        assert_eq!(strip_frontmatter(input), "Actual content here.");
    }

    #[tokio::test]
    async fn subconscious_prompt_from_files() {
        let dir = tempdir().unwrap();
        let sub_mem = dir.path();
        let sys = sub_mem.join("system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("persona.md"),
            "---\ndescription: WHO I AM\n---\n\n# I Am Subconscious\n",
        ).unwrap();
        std::fs::write(
            sys.join("subconscious.md"),
            "---\ndescription: mandate\n---\n\n# Subconscious's Mandate\n\nComplete what was left.\n",
        ).unwrap();

        let prompt = build_subconscious_prompt(sub_mem).await;
        assert!(prompt.contains("I Am Subconscious"));
        assert!(prompt.contains("Complete what was left"));
    }

    #[tokio::test]
    async fn subconscious_prompt_includes_ledger_orientation() {
        let dir = tempdir().unwrap();
        let sub_mem = dir.path();

        let sys = sub_mem.join("system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("persona.md"),
            "---\ndescription: test\n---\n\n# I Am Subconscious\n",
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

        let prompt = build_subconscious_prompt(sub_mem).await;
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
