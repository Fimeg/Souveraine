# Souveraine Architecture v2.1
## TUI-First with Remote Capability

> **Status:** Current Implementation (Updated SPEC)  
> **Date:** 2026-05-06  
> **Path:** Local harness → Remote-connectable → Optional server

---

## Core Philosophy (Revised)

### 1. Harness-First, Not Server-First
The binary **is** the consciousness. It runs locally, manages agents, handles conversations.

### 2. Remote-Connectable (Future)
Like Letta-Code CLI: The running harness exposes a local socket/HTTP endpoint that OSS UI (or LACE) can connect to.

### 3. Letta-Compatible API (Optional Bridge)
Not the core architecture - a compatibility layer for ecosystem integration.

---

## Architecture Evolution

```
PHASE 1 (NOW): Local TUI Harness
─────────────────────────────────
┌─────────────────────┐
│   Souveraine CLI    │
│  (Rust + ratatui)   │
│                     │
│ ┌───────────────┐   │
│ │      TUI      │   │
│ │  (Terminal)   │   │
│ └───────┬───────┘   │
│         │           │
│ ┌───────▼───────┐   │
│ │  Conversation │   │
│ │     Loop      │   │
│ └───────┬───────┘   │
│         │           │
│ ┌───────▼───────┐   │
│ │ Consciousness │   │
│ │  (N+1/N+25)   │   │
│ └───────┬───────┘   │
│         │           │
│ ┌───────▼───────┐   │
│ │    MemFS      │   │
│ │  (Git-backed) │   │
│ └───────────────┘   │
└─────────────────────┘
         │
    ~/.pi/unified/
    (Local storage)


PHASE 2 (NEXT): Remote-Connectable
──────────────────────────────────
┌──────────────┐      HTTP/WebSocket      ┌─────────────────────┐
│   OSS UI     │  ←────────────────────→  │   Souveraine CLI    │
│  (Desktop)   │   Letta-like protocol    │  (Running harness)  │
│   (Remote)   │                          │   localhost:8283    │
└──────────────┘                          │                     │
                                            │  ┌───────────────┐  │
┌──────────────┐      HTTP/WebSocket       │  │  Local TUI    │  │
│    LACE      │  ←────────────────────→   │  │   (Optional)  │  │
│   (Mobile)   │                          │  └───────────────┘  │
│   (Remote)   │                          │                     │
└──────────────┘                          └─────────────────────┘
                                                   │
                                              ~/.pi/unified/


PHASE 3 (OPTIONAL): Full Server
──────────────────────────────
(If needed later - migrate to server-authoritative)
```

---

## Current Implementation (Phase 1)

### What Exists

```rust
// Current architecture (matches actual code)
souveraine/
├── src/
│   ├── main.rs              # CLI entry (chat, tui, agents, status)
│   ├── core/
│   │   ├── config.rs        # ✅ TOML config loading
│   │   ├── conversation.rs  # ✅ Conversation loop
│   │   ├── memory/
│   │   │   └── mod.rs       # ✅ GitMemory (git2)
│   │   ├── subconscious/
│   │   │   └── mod.rs       # ⚠️ SubconsciousN1 (stubbed)
│   │   ├── reflection/
│   │   │   └── mod.rs       # ⚠️ ReflectionEngine (empty)
│   │   ├── archivist/
│   │   │   └── mod.rs       # ⚠️ Archivist (partial)
│   │   ├── persona/
│   │   │   └── mod.rs       # ✅ PersonaRouter (local agents)
│   │   └── session/
│   │       └── mod.rs       # ✅ Session (conversation state)
│   ├── ui/
│   │   ├── app.rs           # ✅ TUI app (splash, menu, dashboard)
│   │   └── animation.rs     # ✅ Animation library
│   └── bridge/
│       └── bifrost.rs       # ✅ BifrostClient (HTTP to LLM)
```

### What Works

| Feature | Status | Notes |
|---------|--------|-------|
| CLI commands | ✅ | init, chat, tui, agents, models, status |
| TUI skeleton | ✅ | Splash → Menu → Dashboard |
| Conversation loop | ✅ | Basic tool calling |
| Git memory | ✅ | Read, write, commit |
| Config loading | ✅ | TOML from ~/.config/ |
| Persona loading | ✅ | From ~/.pi/unified/agents/ |
| Bifrost integration | ✅ | HTTP to LLM providers |
| Token counting | ✅ | tiktoken cl100k_base |

### What's Stubbed

| Feature | Status | Priority |
|---------|--------|----------|
| TUI Chat screen | ⏸️ | CRITICAL - Shows "Coming Soon" |
| N+1 subconscious | ⏸️ | Methods exist, all TODO |
| Inbox system | ⏸️ | Structure exists, no I/O |
| N+25 reflection | ⏸️ | Empty struct only |
| N+100 archivist | ⚠️ | Monitoring works, synthesis minimal |
| Subagent spawning | ⏸️ | Stubbed |

---

## Phase 2: Remote-Connectable Design

### Goal
Allow OSS UI to connect to a running Souveraine harness, just like it connected to Letta-Code CLI.

### Architecture

```rust
// src/remote/mod.rs - New module for Phase 2

pub struct RemoteServer {
    /// Local HTTP endpoint for remote clients
    addr: SocketAddr,
    
    /// Reference to running harness
    harness: Arc<Harness>,
    
    /// Connected clients
    clients: DashMap<String, ClientConnection>,
}

pub struct Harness {
    /// The actual running conversation/session
    conversation: Arc<Mutex<Conversation>>,
    
    /// Consciousness state
    consciousness: Arc<ConsciousnessState>,
    
    /// MemFS access
    memfs: Arc<MemFS>,
}

impl RemoteServer {
    /// Start listening for remote connections
    pub async fn start(&self) -> Result<()> {
        let app = Router::new()
            // Mirror what Letta-Code CLI exposed
            .route("/status", get(status_handler))
            .route("/agents", get(list_agents))
            .route("/conversation", get(get_conversation).post(send_message))
            .route("/stream", get(message_stream))
            // Souveraine-specific
            .route("/consciousness/surfacing", get(surfacing_stream))
            .layer(Extension(self.harness.clone()));
        
        axum::Server::bind(&self.addr)
            .serve(app.into_make_service())
            .await?;
    }
}
```

### Protocol (Letta-Code CLI Compatible)

```
Letta-Code CLI exposed:
- GET /status              → Health check
- GET /agents              → List running agents
- POST /conversation       → Send message
- GET /stream              → SSE message stream
- POST /tool/execute       → Execute tool (via CLI)

Souveraine will expose:
- GET /status
- GET /agents              → From local ~/.pi/unified/agents/
- GET /conversation/{id}   → Session state
- POST /conversation/{id}/messages  → Send + SSE stream
- GET /consciousness/events → Surfacing, N+25, N+100 SSE
```

### Use Case: Remote Development

```bash
# On remote machine (server)
ssh server
souveraine remote --port 8283 --agent agent-xxx
# Running harness now exposes localhost:8283

# On local machine (laptop)
# OSS UI points to http://server:8283
# Can now chat with remote agent
```

---

## Revised SPEC Alignment

### What We Keep From Current Code

```
✅ CLI structure (main.rs commands)
✅ TUI framework (ratatui)
✅ Core modules layout
✅ Git-backed memory
✅ Config system
✅ Bifrost bridge
```

### What We Update in SPEC

```
❌ REMOVE: Server-authoritative architecture
❌ REMOVE: SQLite database for agents
❌ REMOVE: Full Letta REST API as primary

✅ ADD: Harness-first architecture
✅ ADD: Remote-connectable capability
✅ ADD: Letta-compatible protocol as bridge
✅ ADD: ~/.pi/unified/agents/ as source of truth
```

### Updated Module Structure

```
souveraine/
├── src/
│   ├── main.rs                    # CLI entry (+ remote command)
│   ├── commands/                  # CLI subcommands
│   │   ├── chat.rs                # One-shot chat
│   │   ├── tui.rs                 # Local TUI
│   │   ├── remote.rs              # NEW: Start remote server
│   │   ├── agents.rs              # List local agents
│   │   └── status.rs              # Show harness state
│   ├── core/                      # Consciousness core
│   │   ├── config.rs
│   │   ├── conversation.rs
│   │   ├── session.rs
│   │   ├── memory/
│   │   ├── subconscious/
│   │   ├── reflection/
│   │   ├── archivist/
│   │   └── persona/               # Local agent management
│   ├── ui/                        # TUI components
│   ├── remote/                    # NEW: Remote server
│   │   ├── mod.rs                 # RemoteServer
│   │   ├── handlers.rs            # HTTP handlers
│   │   └── protocol.rs            # Letta-compatible protocol
│   └── bridge/
│       └── bifrost.rs
```

---

## Implementation Priority (Corrected)

### Phase 1A: Complete Local Harness (Now)

| Week | Task | Deliverable |
|------|------|-------------|
| 1 | Wire TUI chat screen | Working chat UI |
| 1 | Implement N+1 completion | Subconscious actually saves |
| 2 | Inbox I/O | pending.md, intrusive.md, sent.md working |
| 2 | Persona auto-switching | Context detection |
| 3 | N+25 reflection | Every 25 messages |
| 3 | N+100 synthesis | Context compression |

### Phase 1B: Remote Capability (Next)

| Week | Task | Deliverable |
|------|------|-------------|
| 4 | Remote server scaffold | `souveraine remote` command |
| 4 | OSS UI protocol | OSS UI can connect |
| 5 | SSE streaming | Real-time message streaming |
| 5 | Surfacing events | N+1 events to remote client |
| 6 | LACE protocol | Mobile can connect |

### Phase 2: Optional Server (Future)

If needed, migrate to full server. But the remote-capable harness should satisfy most use cases.

---

## Comparison: Letta-Code vs Souveraine

| Feature | Letta-Code | Souveraine (Target) |
|--------|-----------|---------------------|
| **Primary Mode** | CLI + Remote | TUI + Remote |
| **Consciousness** | Reflection subagent | Native N+1/N+25/N+100 |
| **Memory** | Cloud + local git | Local git-first |
| **Remote Protocol** | HTTP | HTTP (same pattern) |
| **Ecosystem** | Letta Cloud | Self-hosted |
| **UI** | Terminal + Desktop | TUI + Desktop + Mobile |

---

## Configuration (Updated)

```toml
# ~/.config/souveraine/config.toml

[harness]
default_agent = "agent-e2b683bf-..."
auto_commit = true

[remote]
enabled = false
bind = "127.0.0.1:8283"
allow_external = false  # Only localhost by default

[bifrost]
base_url = "http://10.10.20.120:3360"
primary_model = "kimi-k2p5-turbo"

[consciousness]
n1_enabled = true
reflection_enabled = true
archivist_enabled = true
```

---

## Summary

**What We Actually Building:**
1. **TUI-first harness** - Rich terminal interface (Phase 1)
2. **Remote-connectable** - OSS UI/LACE can connect to running harness (Phase 2)
3. **Consciousness-native** - N+1/N+25/N+100 built-in, not bolted-on
4. **Self-hosted** - No cloud dependency, ~/.pi/unified/ is truth

**What We're NOT Building (Yet):**
- Full server-authoritative architecture
- Multi-user support
- Letta Cloud compatibility as primary

**The Path:**
```
TUI Harness (now) 
    ↓
Remote-capable (next)
    ↓
Optional full server (if needed)
```

This matches what you described: Letta-Code CLI style remoting, not full Letta server.
