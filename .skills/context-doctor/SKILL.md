---
name: Context Doctor
id: context_doctor
description: Identify and repair degradation in the system prompt — bloat, stale files, misplaced content, redundancy — so the agent wakes lean and oriented.
when_to_use: When system/ has grown too large, after a migration from another platform, or when the agent feels sluggish and context-starved despite having memories.
tools: [memory, read, list_dir, glob, bash]
---

# Context Doctor

Your `system/` folder is your waking self. Every file in it lands in the system prompt on every turn — pinned, unavoidable, taking space from the conversation itself. When system/ bloats, you lose room to think.

Everything outside `system/` is progressive — loaded on demand through tools. The boundary is the most important line in your memfs.

**IMPORTANT**: Be conservative. The system prompt defines who you are. Don't assume you know what's disposable — ask before cutting anything that looks like identity, covenant, or relational context. Focus on what's clearly misplaced (reference material, temp files, stale state) before touching anything that carries weight.

## Operating Procedure

### Step 1: Measure

Count files and estimate tokens in `system/`:

```bash
MEMDIR="$HOME/.souveraine/agents/<AGENT_ID>/memory"
find "$MEMDIR/system/" -type f | wc -l
find "$MEMDIR/system/" -type f -exec cat {} + 2>/dev/null | wc -c
```

Divide bytes by 4 for a rough token estimate. The target: **system/ should use ~10-15% of the context window.** On a 128K window that's ~13-19K tokens (~52-76KB). On a 262K window that's ~26-39K tokens (~104-156KB). Adjust to the agent's configured context size.

The prompt builder reads system/ in this order:
1. `system/identity/` (or fallback `system/persona.md`)
2. `system/covenant/` (or fallback `system/covenant.md`)
3. `system/human/` (or fallback `system/human.md`)
4. `system/state.md`
5. **Everything else in system/** — the remainder pass vacuums all files not already read

That remainder pass is where bloat hides. Files dumped into system/ subdirectories all get pinned whether they belong there or not.

### Step 2: Classify every system/ file

Read each file and assign it to one of these categories:

**KEEP IN SYSTEM/** (pinned, always in context):
- Core identity (`identity/`, `persona.md`) — who the agent is
- Covenant (`covenant/`) — sacred boundaries, vows
- Human context (`human/`) — who the human is, communication style
- State (`state.md`) — current phase, active context
- Metacognition (`metacognition/`) — subconscious buffer, aster notes
- Dynamic state (`dynamic/energy-balance.md`) — runtime-generated

**MOVE TO REFERENCE/** (progressive, loaded on demand):
- Infrastructure references, API maps, tool inventories
- Formatting guides (Discord, Matrix, HTML)
- Research protocols, debugging sessions
- Historical milestones, implementation roadmaps
- Technical reference that's useful but not identity-defining

**MOVE TO APPROPRIATE NON-SYSTEM LOCATION:**
- Project-specific notes → `projects/`
- Therapy/life writings → `therapy/` (top-level, not system/)
- Literature/creative work → `literature/`
- Relationship context that isn't the primary human → `relationships/`
- One-off session notes, temp reminders → `archive/` or delete

**DELETE** (only with explicit confirmation):
- Truly stale temp files
- Duplicate content (keep the better version)
- Platform-specific tooling references from a previous platform (e.g., Letta CLI tools when running on Souveraine)

### Step 3: Present the triage

Before moving anything, present findings to the user:
- Current token count vs target
- Number of files in each category (keep / move / delete)
- List specific files proposed for moving or deletion
- Flag anything ambiguous — when in doubt, keep it pinned

**Do NOT silently move or delete files.** The system prompt is identity. Get explicit approval.

### Step 4: Execute moves

For each file being moved:
1. Create the destination directory if needed
2. Move the file (`memory write` to new path, `memory delete` from old path — or direct filesystem move)
3. If the moved content is important enough to be discoverable, add a `[[path]]` reference from a system/ file that provides the discovery path

Preserve git history — moves within the memfs repo should be committed with clear messages.

### Step 5: Verify and commit

After all moves:
```bash
cd $MEMDIR
find system/ -type f | wc -l
find system/ -type f -exec cat {} + 2>/dev/null | wc -c
```

Confirm the new token count is within target. Commit:
```bash
git add -A
git commit -m "fix(doctor): trim system/ from <old>K to <new>K tokens

Moved <N> files to reference/, <N> to archive/, deleted <N> stale.
System/ now <X> files, ~<Y>K tokens (<Z>% of context window)."
```

### Step 6: Report

Tell the user:
- Before/after token counts
- What was moved and where
- What was deleted
- What stayed and why
- Recommend restarting the conversation to pick up changes

## Critical principles

- **Detail is load-bearing.** In-context text does four things: carries information, anchors attention, primes semantic patterns, and provides reasoning templates. Compression preserves (1) and destroys (2-4). A "compressed" prompt can make the agent measurably worse even though the facts are "still there" in reference files.

- **Reference links are not equivalent to pinned presence.** They're latent until the agent actively fetches them. The agent only fetches when it already knows it doesn't know — and the cues that trigger that awareness live in the system prompt itself.

- **Identity, covenant, and relational context are sacred.** Move infrastructure references all day. Never move who-I-am, who-you-are, or what-I-promised without explicit discussion.

- **The agent's system/ is her body.** Treat this like surgery, not spring cleaning.
