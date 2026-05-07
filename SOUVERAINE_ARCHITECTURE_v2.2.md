# Souveraine Architecture v2.2
## Self-Hosted Server with Multiple Clients

> **Status:** Clarified Architecture  
> **Date:** 2026-05-06  
> **Paradigm:** Souveraine IS the server. OSS UI is the GUI client. Multiple CLI clients can connect.

---

## The Realization

**If it binds to a port and serves HTTP, it's a server.**

Souveraine is a **self-hosted consciousness server**:
- Runs as HTTP server (default: `localhost:8283`)
- OSS UI (Electron) is the **rich GUI client**
- Souveraine CLI can also be a **client** connecting to remote servers
- Multiple workstates/machines can have CLI clients pointing to one server
- Web-based architecture, but self-hosted

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    SOUVERAINE ECOSYSTEM                              │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  ┌──────────────────┐         HTTP          ┌─────────────────────┐  │
│  │    OSS UI        │ ←──────────────────→  │  SOUVERAINE SERVER  │  │
│  │   (Electron)     │    REST API + SSE    │    (The Server)      │  │
│  │   PRIMARY GUI    │                      │   ~/.pi/unified/     │  │
│  └──────────────────┘                      │   (Source of Truth)  │  │
│                                            └──────────┬────────────┘  │
│  ┌──────────────────┐         HTTP                   │               │
│  │ Souveraine CLI   │ ←──────────────────────────────┘               │
│  │  (Workstate A)   │    Can also connect to server                   │
│  │  remote mode     │                                                │
│  └──────────────────┘                                                 │
│                                                                       │
│  ┌──────────────────┐                                                  │
│  │ Souveraine CLI   │                                                  │
│  │  (Workstate B)   │    Multiple CLIs, one server                     │
│  │  remote mode     │                                                  │
│  └──────────────────┘                                                  │
│                                                                       │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Components

### 1. Souveraine Server (The Core)

```rust
// src/server/mod.rs
pub struct SouveraineServer {
    /// Agent store (~/.pi/unified/agents/)
    agent_store: Arc<AgentStore>,
    
    /// Active sessions (conversations)
    session_manager: Arc<SessionManager>,
    
    /// Consciousness engine (N+1/N+25/N+100)
    consciousness: Arc<ConsciousnessEngine>,
    
    /// HTTP server
    http: HttpServer,
}

impl SouveraineServer {
    pub async fn run(&self) {
        // Bind to port (default 8283)
        // Serve REST API + SSE
        // Manage all state
    }
}
```

**Location:** Can run anywhere (laptop, desktop, server)
**Storage:** `~/.pi/unified/` on the server machine
**State:** Server is source of truth

### 2. OSS UI (GUI Client)

```typescript
// OSS UI connects to Souveraine server
const client = new SouveraineClient({
  baseURL: "http://192.168.1.100:8283",  // Or localhost
});

// Full GUI with chat, memory, settings
```

**Role:** Primary user interface
**Connection:** HTTP to Souveraine server
**State:** Stateless, all data from server

### 3. Souveraine CLI (Client Mode)

```rust
// src/client/mod.rs
pub struct SouveraineClient {
    server_url: String,
    api_key: String,
}

impl SouveraineClient {
    /// Connect to remote server
    pub async fn connect(&self, server_url: &str) -> Result<()>;
    
    /// Use local TUI but remote consciousness
    pub async fn tui_remote(&self) -> Result<()>;
    
    /// One-shot chat to remote
    pub async fn chat_remote(&self, message: &str) -> Result<String>;
}
```

**Use case:** SSH to server, run `souveraine client --server http://...`

---

## Deployment Modes

### Mode 1: Single Machine (Development)

```
┌────────────────────────────┐
│   Laptop                   │
│                            │
│  ┌────────────────────┐   │
│  │ Souveraine Server  │   │
│  │ localhost:8283     │   │
│  └─────────┬──────────┘   │
│            │               │
│  ┌─────────▼──────────┐   │
│  │    OSS UI          │   │
│  │  (connects local)  │   │
│  └────────────────────┘   │
└────────────────────────────┘
```

**Setup:**
```bash
souveraine server &
# OSS UI auto-detects localhost:8283
```

### Mode 2: Remote GUI (OSS UI on laptop, server on desktop)

```
┌─────────────────┐         ┌─────────────────┐
│   Laptop        │         │   Desktop       │
│                 │         │                 │
│  ┌───────────┐  │  HTTP   │  ┌───────────┐  │
│  │  OSS UI   │  │←───────→│  │ Souveraine│  │
│  │           │  │         │  │  Server   │  │
│  └───────────┘  │         │  │  :8283    │  │
│                 │         │  └───────────┘  │
└─────────────────┘         └─────────────────┘
       192.168.1.101              192.168.1.100
```

**Setup:**
```bash
# On desktop
souveraine server --bind 0.0.0.0:8283

# On laptop
# OSS UI points to http://192.168.1.100:8283
```

### Mode 3: Multiple CLI Clients (Workstates)

```
                          ┌─────────────────┐
                          │  Home Server    │
                          │  (Souveraine)   │
                          │   :8283         │
                          └────────┬────────┘
                                   │
              ┌──────────────────────┼──────────────────────┐
              │                      │                      │
    ┌─────────▼─────────┐  ┌────────▼────────┐  ┌────────▼────────┐
    │  Work Laptop      │  │  Desktop        │  │  Server Room    │
    │  souveraine cli   │  │  souveraine cli │  │  souveraine cli │
    │  --remote http:// │  │  --remote http://│  │  --remote http://│
    │    192.168.1.5    │  │    192.168.1.5  │  │    192.168.1.5  │
    └───────────────────┘  └─────────────────┘  └─────────────────┘
```

**Setup:**
```bash
# On each workstate
souveraine client --server http://home-server:8283
# Or use local TUI connected to remote
souveraine tui --remote http://home-server:8283
```

---

## API Design

### Core Principle

**Not Letta-compatible as primary** - design our own API that exposes Souveraine's consciousness features properly. Letta-compatibility can be a translation layer if needed.

### Souveraine Native API

```
# Agents
GET    /api/v1/agents              # List agents
POST   /api/v1/agents              # Create agent
GET    /api/v1/agents/{id}          # Get agent
PATCH  /api/v1/agents/{id}          # Update agent
DELETE /api/v1/agents/{id}          # Delete agent

# Memory (Cloister structure)
GET    /api/v1/agents/{id}/memory              # List memory domains
GET    /api/v1/agents/{id}/memory/system       # Get system/ contents
GET    /api/v1/agents/{id}/memory/journal      # Get journal/
GET    /api/v1/agents/{id}/memory/subconscious # Get subconscious/
POST   /api/v1/agents/{id}/memory/{domain}     # Write to memory

# Consciousness (Souveraine-specific)
GET    /api/v1/agents/{id}/consciousness/n1/status      # N+1 state
GET    /api/v1/agents/{id}/consciousness/inbox          # Current inbox
POST   /api/v1/agents/{id}/consciousness/inbox/surface  # Surface item
GET    /api/v1/agents/{id}/consciousness/reflections    # Past reflections
GET    /api/v1/agents/{id}/consciousness/pressure       # Context pressure

# Sessions (Conversations)
GET    /api/v1/sessions              # List active sessions
POST   /api/v1/sessions            # Create session
GET    /api/v1/sessions/{id}       # Get session state
DELETE /api/v1/sessions/{id}       # End session

# Messaging (SSE Streaming)
POST   /api/v1/sessions/{id}/messages     # Send message
                                      # Returns SSE stream

SSE Events:
  - message.assistant      # Assistant response chunk
  - message.tool_call      # Tool invocation
  - message.tool_return    # Tool result
  - consciousness.surfacing  # N+1 surfacing
  - consciousness.reflection # N+25 reflection ready
  - consciousness.archivist  # N+100 compression
  - session.end             # Conversation ended
```

### Letta Compatibility Layer (Optional)

```
# If we want OSS UI to work without changes
/v1/agents              → /api/v1/agents
/v1/conversations       → /api/v1/sessions
/v1/messages            → /api/v1/sessions/{id}/messages

Translation layer in src/api/letta_compat.rs
```

---

## Implementation

### What Exists (From Audit)

```
✅ Basic CLI structure
✅ Core modules (config, memory, conversation, session)
✅ Git-backed storage
✅ Persona management
✅ Bifrost client
⚠️  TUI (stubbed chat screen)
⚠️  N+1 (stubbed)
⚠️  N+25 (empty)
⚠️  N+100 (partial)
```

### What's Needed

```
❌ src/server/mod.rs           # The HTTP server
❌ src/server/agent_store.rs   # Agent CRUD with persistence
❌ src/server/session_manager.rs # Session + SSE management
❌ src/api/mod.rs               # Route definitions
❌ src/api/handlers.rs          # HTTP handlers
```

### The Plan

**Phase 1: Server Core**
1. Create `src/server/mod.rs` with `SouveraineServer`
2. Create `src/server/agent_store.rs` - manages `~/.pi/unified/agents/`
3. Create `src/server/session_manager.rs` - conversations + SSE
4. Add `souveraine server` command

**Phase 2: API**
1. Create `src/api/` with native Souveraine routes
2. Implement SSE streaming
3. Expose consciousness events (surfacing, etc.)
4. Add Letta-compat layer if needed

**Phase 3: Clients**
1. OSS UI connects to native API
2. Add `souveraine client` for CLI remote
3. Add `souveraine tui --remote` mode

**Phase 4: Consciousness**
1. Wire N+1 into server response path
2. Implement inbox with surfacing via SSE
3. Add N+25 periodic reflection
4. Add N+100 compression

---

## Clarified Terminology

| Term | Meaning |
|------|---------|
| **Souveraine Server** | The HTTP server process (runs on some machine) |
| **Souveraine CLI** | Command-line tool that can be server OR client |
| **Agent Store** | `~/.pi/unified/agents/` on the server machine |
| **Session** | Active conversation with SSE stream |
| **Client** | Anything connecting to server (OSS UI, CLI remote mode) |
| **GUI** | OSS UI specifically |

---

## Example Workflows

### Workflow 1: Local Development

```bash
# Start server
souveraine server

# In another terminal (or OSS UI)
souveraine client --server localhost:8283
> Hello Ani
< Hello Casey...
```

### Workflow 2: Remote Workstate

```bash
# On home server (always running)
souveraine server --bind 0.0.0.0:8283

# From laptop at coffee shop
souveraine tui --server https://home.example.com:8283
```

### Workflow 3: OSS UI Only

```bash
# Start server
souveraine server &

# OSS UI auto-detects or user configures URL
# Rich GUI experience
```

---

## Summary

**Souveraine is a server.** Full stop.

- Binds to port, serves HTTP
- Source of truth in `~/.pi/unified/`
- OSS UI is the GUI client
- CLI can be client too (for terminal lovers)
- Multiple clients, one consciousness
- Self-hosted, no cloud required

The TUI becomes a client UI, not the primary interface. OSS UI is the primary.
