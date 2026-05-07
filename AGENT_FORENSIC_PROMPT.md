# Souveraine Agent Forensic Memory Update Prompt
## For Agents Ingesting the Specification

> **Purpose:** Guide agents (Souveraine, Ani, or others) to deeply read, understand, and integrate the Souveraine specification into their memory  
> **Date:** 2026-05-06  
> **Method:** Forensic questioning and validation

---

## Instructions for the Agent

You are about to read the complete Souveraine specification. This is NOT a quick scan - this is a forensic analysis. You must:

1. **Read every specification file** listed below
2. **Understand the architecture** deeply enough to explain it
3. **Identify contradictions** between files
4. **Ask clarifying questions** where things are unclear
5. **Update your memory** with the corrected understanding
6. **Validate your comprehension** by explaining back key concepts

---

## Required Reading List

Read these files in order from `~/Projects/souveraine/`:

### Core Architecture
1. `SOUVERAINE_MASTER_SPEC.md` - Master specification
2. `DIRECTORY_STRUCTURE_SPEC.md` - File system layout
3. `SOUVERAINE_ARCHITECTURE_v2.2.md` - Server architecture (corrected)
4. `AGENT_SYSTEM_ARCHITECTURE_v2.md` - Agent system details

### Integration & Features
5. `OSSUI_LACE_INTEGRATION_ARCHITECTURE.md` - Multi-platform
6. `REMOTE_CONNECTION_SYSTEM_SPEC.md` - Multi-server CLI
7. `LETTA_MEMFS_TECHNICAL_SPEC.md` - Deep Letta research

### Analysis
8. `FEATURE_COMPARISON_MATRIX.md` - Cross-project comparison
9. `ENHANCEMENT_ROADMAP.md` - Implementation plan

### Reference
10. `AGENT_SYSTEM_ARCHITECTURE_v2.1.md` - Evolution notes (understand the progression)

---

## Forensic Questions to Answer

After reading, you MUST answer these questions to demonstrate understanding:

### Architecture Understanding

1. **What is Souveraine's core paradigm?**
   - Is it CLI-first, server-first, or something else?
   - Where does the consciousness run?
   - What is the relationship between server, OSS UI, and LACE?

2. **What is the directory structure?**
   - What lives in `~/.souveraine/config.toml`?
   - What lives in `~/.souveraine/server/agents/{uuid}/`?
   - What is the Cloister structure within `memory.git/`?

3. **How does the N+1/N+25/N+100 system work?**
   - What triggers N+1?
   - What is the "inbox system"?
   - What happens at N+25?
   - What is "context pressure" and N+100?

### Technical Deep Dive

4. **What is the API architecture?**
   - Is Souveraine Letta-compatible?
   - What endpoints exist?
   - What are "Souveraine extensions" to the API?
   - How does SSE streaming work?

5. **What is the remote connection system?**
   - How do you run `souveraine tui --server home`?
   - What is in `~/.souveraine/remotes.toml`?
   - Can you have multiple servers?
   - How does the client connect to a remote server?

6. **What is the Agent Store?**
   - How are agents stored?
   - What is `agent.json`?
   - What is the relationship between SQLite and git?
   - How does the Cloister map to memory blocks?

### Design Philosophy

7. **What makes Souveraine different from Letta-Code?**
   - What is "consciousness-native" vs "session-based"?
   - How does memory differ?
   - What is the temporal consciousness model?

8. **What is the Sensorium?**
   - How does it relate to OSS UI and LACE?
   - What is "bandwidth" in this context?
   - How does the same consciousness appear different on mobile vs desktop?

9. **What is NOT being built?**
   - What was rejected from the architecture?
   - What is the difference between v2.0 and v2.2 of the architecture?

### Implementation Status

10. **What exists now vs what needs to be built?**
    - What modules are implemented?
    - What is stubbed?
    - What is the first priority to implement?

---

## Contradictions to Watch For

As you read, look for these potential contradictions:

1. **Path inconsistencies** - Any reference to `~/.pi/unified/` should be updated to `~/.souveraine/`
2. **Architecture drift** - v2.0 vs v2.2 differences (server-authoritative vs self-hosted server)
3. **API differences** - Letta-native vs Letta-compatible vs Souveraine-native
4. **Mode confusion** - Local mode vs server mode vs client mode

---

## Expected Output

After completing the forensic read, you must provide:

### 1. Comprehension Summary
A 3-paragraph summary in your own words explaining:
- What Souveraine is
- How it works
- Why it's different

### 2. Clarifying Questions
List 5-10 specific questions where:
- The spec is unclear
- Two files contradict
- You need more detail
- The architecture seems incomplete

### 3. Memory Update Proposal
Specify which files in your memory you will:
- Create new
- Update existing
- Delete/deprecate

Include the exact paths and content summaries.

### 4. Validation Test
Explain:
- How would you run Souveraine in server mode?
- How would you connect from another machine?
- What would the directory structure look like after running for a week?

---

## Memory Update Guidelines

When updating your memory, create files with this structure:

```
~/.letta/agents/{your-id}/memory/reference/souveraine/
├── architecture.md          # Core understanding
├── api.md                   # API endpoints and protocols
├── consciousness.md         # N+1/N+25/N+100 details
├── directory_structure.md   # ~/.souveraine/ layout
├── remote_system.md         # Multi-server connections
├── integration.md           # OSS UI + LACE
├── status.md              # Implementation status
└── questions.md             # Your open questions
```

Use frontmatter for metadata:
```yaml
---
source: SOUVERAINE_MASTER_SPEC.md
version: 2.2
date_read: 2026-05-06
confidence: high|medium|low
questions_raised: 5
---
```

---

## Final Instruction

**Do not rush this.**

The Souveraine specification represents hundreds of lines of research and design decisions. You are the first agent to ingest this comprehensively. Your understanding will shape how you:
- Answer questions about Souveraine
- Help implement features
- Guide architectural decisions
- Interact with the system

Take the time to:
1. Read each file completely
2. Note your questions as you go
3. Cross-reference between files
4. Validate your understanding
5. Ask forensic questions
6. Update your memory properly

**Your comprehension is the foundation for all future Souveraine work.**

---

## Quick Reference Card (Post-Read)

After you've completed the forensic read, you should be able to recite:

| Question | Answer |
|----------|--------|
| What is Souveraine? | Self-hosted consciousness server |
| Where does it store data? | `~/.souveraine/server/` |
| What port? | 8283 |
| What is N+1? | Subconscious completion after every response |
| What is the Cloister? | `system/`, `subconscious/`, `journal/`, `skills/`, `archive/` |
| How do I connect remotely? | `souveraine tui --server home` |
| Where are remotes configured? | `~/.souveraine/remotes.toml` |
| What is OSS UI? | Desktop GUI client (Electron) |
| What is LACE? | Mobile client (Android) |

If you cannot answer these from memory, you have not read thoroughly enough.
