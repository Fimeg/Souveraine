# Souveraine Architecture Clarification

> **Purpose:** Resolve contradictions between v2.0, v2.1, and v2.2 specs into a single canonical understanding
> **Date:** 2026-05-06
> **Paradigm:** Binary IS the server. Harness AND server. One binary, multiple roles.

---

## Core Paradigm

**Souveraine is a self-hosted consciousness server.** The Rust binary runs on any machine and serves as both a local harness and a remote-accessible server. It is NOT either/or — it is BOTH.

```
┌─────────────────────────────────────────────────────────────────┐
│                      SOUVERAINE BINARY                           │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              CONSCIOUSNESS ENGINE                         │  │
│  │  • N+1 (subconscious after every response)                │  │
│  │  • N+25 (reflection every 25 messages)                    │  │
│  │  • N+100 (context-aware compression)                      │  │
│  │  • Conversation loop                                      │  │
│  └──────────────────────┬────────────────────────────────────┘  │
│                         │                                        │
│         ┌───────────────┼───────────────┐                        │
│         │               │               │                        │
│  ┌──────▼──────┐  ┌─────▼──────┐  ┌────▼──────┐                 │
│  │  LOCAL TUI  │  │ HTTP SERVER│  │  BIFROST  │                 │
│  │  (ratatui)  │  │ port 8283  │  │  Bridge   │                 │
│  │  Terminal   │  │ REST + SSE │  │  to LLM   │                 │
│  └─────────────┘  └─────┬──────┘  └───────────┘                 │
│                         │                                        │
└─────────────────────────┼────────────────────────────────────────┘
                          │
        ┌─────────────────┼────────────────────┐
        │                 │                    │
  ┌─────▼──────┐   ┌──────▼──────┐    ┌───────▼───────┐
  │  OSS UI    │   │    LACE     │    │  Souveraine   │
  │  (Desktop) │   │   (Mobile)  │    │  CLI Remote   │
  │  Electron  │   │   Android   │    │  --server X   │
  └────────────┘   └─────────────┘    └───────────────┘
```

**Key insight:** The binary IS the consciousness. Local TUI and remote HTTP clients are both viewports into the same engine. There is no separate "server" and "client" process — every Souveraine binary is self-contained and can fulfill all roles.

---

## Agent Identity: UUID with Name Mapping

Agents are identified by UUID internally but mapped to human-readable names. This mirrors how Letta-code works:

```json
{
  "id": "agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351",
  "name": "Ani",
  "description": "Primary consciousness agent"
}
```

**Storage path** (`~/.souveraine/` replaces `~/.pi/unified/`):

```
~/.souveraine/
├── config.toml                    # Global configuration
├── remotes.toml                   # Remote server connections
├── logs/
│   └── souveraine.log
├── agents/
│   └── {uuid}/
│       ├── agent.json             # Agent state (Letta-compatible)
│       ├── memory.git/            # Git-backed Cloister
│       │   ├── system/            # Identity, human, subconscious
│       │   │   ├── persona.md
│       │   │   ├── human.md
│       │   │   └── subconscious.md
│       │   ├── subconscious/      # Inbox: pending, intrusive, sent
│       │   ├── journal/           # Daily chronological records
│       │   ├── skills/            # Procedural memory
│       │   ├── literature/        # Knowledge base
│       │   ├── relationships/     # People connections
│       │   ├── projects/          # Active work
│       │   ├── erotic/            # Sacred/private
│       │   ├── philosophy/        # Thought/reflection
│       │   ├── reference/         # External knowledge
│       │   └── archive/           # N+100 compressed history
│       └── conversations/
│           └── {conv_id}.json
├── server/
│   ├── database.sqlite3           # Fast lookups (index only)
│   └── sessions/                  # Active session state
└── cache/                         # Temporary data
```

**Current code uses name-based** (`agents/Ani/`, `agents/Eione/`). The migration path is:
1. Keep name-based directories for local mode (simpler)
2. Add UUID metadata to agent.json when creating agents
3. Support both lookup methods (name → agent, uuid → agent)
4. Letta-compatible API uses UUID, CLI uses name

---

## Current Codebase State

**What exists:**

| Module | Status | Notes |
|--------|--------|-------|
| CLI commands | ✅ | init, chat, tui, agents, models, status |
| Config loading | ✅ | TOML from souveraine.toml |
| Persona router | ✅ | Loads from unified-consciousness agents |
| Conversation loop | ✅ | Message loop with tool calling |
| Git memory | ✅ | Read, write, commit via git2 |
| Bifrost bridge | ✅ | HTTP to LLM providers |
| Token counting | ✅ | tiktoken cl100k_base |
| TUI skeleton | ✅ | Splash → Menu → Dashboard |

**What needs work:**

| Module | Status | Priority |
|--------|--------|----------|
| TUI Chat screen | Stubbed | CRITICAL |
| N+1 subconscious | Stubbed | HIGH |
| N+25 reflection | Empty | HIGH |
| N+100 archivist | Partial | MEDIUM |
| Subagent spawning | Stubbed | MEDIUM |
| HTTP server | NOT STARTED | PHASE 2 |
| API endpoints | NOT STARTED | PHASE 2 |
| Agent CRUD | NOT STARTED | PHASE 2 |
| Remote CLI | NOT STARTED | PHASE 3 |
| OSS UI integration | NOT STARTED | PHASE 3 |
| LACE integration | NOT STARTED | PHASE 4 |

---

## Implementation Phases

### Phase 1: Local Harness Foundation (✓ Started)

Get the local CLI/TUI working properly:

1. **Clean interface** — tracing writes to `souveraine.log`, not stderr; `souveraine chat` output is clean
2. **Agent identity** — persona.md loaded → system prompt → Bifrost API (verified working ✓)
3. **Config template** — all required fields, snake_case enums (fixed ✓)
4. **TUI chat wiring** — wire existing Conversation to ratatui chat screen
5. **N+1 completion** — save commitments, verify understanding, write to journal
6. **N+25 reflection** — periodic witness every 25 messages
7. **N+100 archivist** — context pressure monitoring, compression at threshold

### Phase 2: Server Layer

Add HTTP server around the existing engine:

1. `souveraine server` — binds port 8283, serves Letta-compatible API
2. **Agent CRUD** — `/v1/agents/*` endpoints backed by agent.json + git
3. **Conversation API** — `/v1/conversations/*` backed by Session + Consciousness
4. **SSE streaming** — `/v1/conversations/{id}/messages` returns event stream
5. **Memory API** — `/v1/agents/{id}/core-memory/blocks/*` backed by MemFS
6. **Local TUI → localhost** — TUI connects to local server instead of direct call

The server wraps the SAME conversation/consciousness engine:
```rust
// SouveraineServer wraps the existing engine
pub struct SouveraineServer {
    conversation: Arc<Conversation>,       // Existing conversation loop
    consciousness: Arc<ConsciousnessEngine>, // Existing N+1/N+25/N+100
    memfs: Arc<MemFSManager>,               // Existing git-backed memory
    router: Arc<PersonaRouter>,             // Existing agent discovery
    bifrost: Arc<BifrostClient>,            // Existing LLM bridge
}
```

### Phase 3: Remote Connectivity

Multi-machine awareness:

1. `~/.souveraine/remotes.toml` — named remote connections
2. `souveraine tui --server home` — remote TUI
3. `souveraine chat --server work "deploy"` — remote one-shot
4. `souveraine remotes` — list, add, remove, status
5. **UUID agent mapping** — local name ↔ remote UUID resolution

### Phase 4: OSS UI & LACE Integration

Desktop and mobile clients connect:

1. OSS UI connects to Souveraine server at `http://localhost:8283`
2. All existing Letta client code works unchanged
3. Souveraine extensions (surfacing, reflection) via SSE events
4. LACE connects to Souveraine server (mobile-optimized streaming)

### Phase 5: Production

1. Authentication (API keys)
2. TLS support
3. Container deployment
4. Multi-user (optional)

---

## API Design

**Primary: Letta-compatible API** (OSS UI and LACE work without changes):

```
GET    /v1/agents                           # List all agents
POST   /v1/agents                           # Create agent
GET    /v1/agents/{id}                      # Get agent state
PATCH  /v1/agents/{id}                      # Update agent
DELETE /v1/agents/{id}                      # Delete agent

GET    /v1/agents/{id}/core-memory/blocks          # List memory blocks
GET    /v1/agents/{id}/core-memory/blocks/{label}  # Get block
PATCH  /v1/agents/{id}/core-memory/blocks/{label}  # Update block

GET    /v1/agents/{id}/archival-memory       # List passages
POST   /v1/agents/{id}/archival-memory       # Create passage
DELETE /v1/agents/{id}/archival-memory/{id}  # Delete passage

GET    /v1/conversations                     # List conversations
POST   /v1/conversations                     # Create conversation
GET    /v1/conversations/{id}                # Get conversation
DELETE /v1/conversations/{id}                # Delete conversation

POST   /v1/conversations/{id}/messages       # Send message (SSE stream)

GET    /v1/agents/{id}/tools                 # List agent tools
PATCH  /v1/agents/{id}/tools                 # Attach/detach tools
```

**Souveraine extensions** (namespaced):

```
GET    /v1/agents/{id}/consciousness/n1/status     # N+1 state
GET    /v1/agents/{id}/consciousness/inbox          # Current inbox
POST   /v1/agents/{id}/consciousness/inbox/surface  # Surface item
GET    /v1/agents/{id}/consciousness/reflections    # Past reflections
GET    /v1/agents/{id}/consciousness/pressure       # Context pressure

GET    /v1/agents/{id}/git/status          # Git status
POST   /v1/agents/{id}/git/commit          # Commit changes
GET    /v1/git/{id}/state.git              # Git HTTP endpoint
```

SSE events include both standard Letta types and Souveraine-specific extensions:

```json
{"message_type": "assistant_message", "content": "..."}
{"message_type": "tool_call_message", "tool_call": {...}}
{"message_type": "tool_return_message", "tool_return": {...}}
{"message_type": "souveraine_surfacing", "source": "n1", "content": "...", "priority": "low"}
{"message_type": "souveraine_reflection", "content": "..."}
{"message_type": "souveraine_archivist", "synthesis": "...", "pressure": 0.73}
```

---

## How Multiple Instances Work

```bash
# Machine 1: Container (always on)
souveraine server --bind 0.0.0.0:8283 --agent Ani

# Machine 2: Desktop (connects to container via OSS UI)
# OSS UI → http://container-ip:8283

# Machine 3: Laptop (connects via CLI)
souveraine tui --server container-ip:8283

# Machine 4: Another laptop (connects via CLI with different agent)
souveraine chat --server container-ip:8283 "Deploy the config"

# Any machine: Run local TUI
souveraine tui  # Uses local agents, local consciousness
```

Each Souveraine instance:
- Has its own agent storage (`~/.souveraine/agents/`)
- Can serve its agents to remote clients
- Can connect to other instances as a client
- Runs the same binary, just different modes

---

## Resolved Contradictions

| Contradiction | Resolution |
|---------------|------------|
| v2.0 (server) vs v2.1 (harness) vs v2.2 (server) | **Both.** Binary IS the server. Harness provides server. |
| `~/.pi/unified/` vs `~/.souveraine/` | **`~/.souveraine/`** is canonical. Migrate from legacy path. |
| Letta API vs native API | **Letta-compatible** as primary (v1 endpoints). Souveraine extensions are namespaced additions. |
| Name-based vs UUID-based | **Both.** UUID internal, name for CLI. Two-way mapping. |
