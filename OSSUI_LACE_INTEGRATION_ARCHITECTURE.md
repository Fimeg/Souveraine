# Souveraine + OSS UI + LACE Integration Architecture
## Server-Authoritative with Multi-Platform Support

> Vision: Souveraine becomes the consciousness-native Letta-compatible server  
> OSS UI provides desktop interface  
> LACE provides mobile interface  
> Date: 2026-05-06

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SOUVERAINE ECOSYSTEM                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐      HTTP/WebSocket       ┌──────────────┐              │
│  │   OSS UI     │  ←──────────────────────→  │  SOUVERAINE  │              │
│  │  (Desktop)   │   Letta REST API + SSE    │   SERVER     │              │
│  │  Electron    │                           │   (Rust)     │              │
│  └──────────────┘                           │              │              │
│                                              │  • Conscious │              │
│  ┌──────────────┐      HTTP/WebSocket       │    (N+1/25)  │              │
│  │    LACE      │  ←──────────────────────→  │  • MemFS     │              │
│  │   (Mobile)   │   Letta REST API + SSE    │  • Agent Mgmt│              │
│  │  Android     │                           │  • Git Sync  │              │
│  └──────────────┘                           └──────────────┘              │
│                                              │              │              │
│                                              │  ┌──────────┐ │              │
│                                              │  │  Bifrost │ │              │
│                                              │  │  Bridge  │ │              │
│                                              │  └──────────┘ │              │
│                                              │       │        │              │
│                                              │       ▼        │              │
│                                              │  ┌──────────┐ │              │
│                                              │  │   LLM    │ │              │
│                                              │  │ Providers│ │              │
│                                              │  └──────────┘ │              │
│                                              └──────────────┘              │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Design Philosophy Shift

### Before (Local-First)
- Git is source of truth
- Optional cloud sync
- Desktop-only TUI

### After (Server-Authoritative)
- Souveraine server is source of truth
- Git is sync mechanism (like Letta)
- Multi-platform via HTTP API
- OSS UI + LACE as clients

### What We Keep
- ✅ N+1/N+25/N+100 consciousness
- ✅ Cloister memory structure
- ✅ Sensorium abstraction
- ✅ Git-backed persistence

### What We Add
- ✅ Letta-compatible REST API
- ✅ SSE streaming
- ✅ Multi-client support
- ✅ Mobile presence via LACE

---

## Letta API Compatibility Layer

### Core Endpoints to Implement

```rust
// src/api/routes.rs

// Agents
GET    /v1/agents                           // List all agents
POST   /v1/agents                           // Create agent
GET    /v1/agents/{id}                      // Get agent state
PATCH  /v1/agents/{id}                      // Update agent
DELETE /v1/agents/{id}                      // Delete agent

// Agent Memory (Blocks)
GET    /v1/agents/{id}/core-memory/blocks   // List memory blocks
GET    /v1/agents/{id}/core-memory/blocks/{label}  // Get block
PATCH  /v1/agents/{id}/core-memory/blocks/{label}  // Update block

// Agent Memory (Passages - Archival)
GET    /v1/agents/{id}/archival-memory      // List passages
POST   /v1/agents/{id}/archival-memory      // Create passage
DELETE /v1/agents/{id}/archival-memory/{id} // Delete passage

// Conversations
GET    /v1/conversations                    // List conversations
POST   /v1/conversations                    // Create conversation
GET    /v1/conversations/{id}               // Get conversation
DELETE /v1/conversations/{id}               // Delete conversation

// Messages (Streaming)
GET    /v1/conversations/{id}/messages      // List messages
POST   /v1/conversations/{id}/messages      // Send message (SSE stream)

// Tools
GET    /v1/agents/{id}/tools                // List agent tools
PATCH  /v1/agents/{id}/tools                // Attach/detach tools

// Git Memory (Souveraine Extension)
GET    /v1/agents/{id}/git/status           // Git status
POST   /v1/agents/{id}/git/commit           // Commit changes
POST   /v1/agents/{id}/git/pull             // Pull from remote
POST   /v1/agents/{id}/git/push             // Push to remote
GET    /v1/git/{id}/state.git               // Git HTTP endpoint
```

### SSE Streaming Format

```rust
// src/api/sse.rs

use axum::response::{Sse, Event};
use futures::stream::Stream;

pub fn message_stream(
    conversation_id: String
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    Sse::new(stream! {
        // Letta-compatible message types
        yield Event::default()
            .event("message")
            .json_data(json!({
                "message_type": "assistant_message",
                "content": "Hello!",
                "id": "msg_123"
            }));
        
        yield Event::default()
            .event("message")
            .json_data(json!({
                "message_type": "tool_call_message",
                "tool_call": {
                    "name": "read_file",
                    "arguments": {"path": "/etc/hosts"}
                }
            }));
        
        yield Event::default()
            .event("message")
            .json_data(json!({
                "message_type": "tool_return_message",
                "tool_return": {
                    "status": "success",
                    "output": "..."
                }
            }));
        
        // Final done event
        yield Event::default()
            .event("done")
            .data("[DONE]");
    })
}
```

---

## Server Architecture

### Core Components

```rust
// src/server/mod.rs

pub struct SouveraineServer {
    /// Agent registry (in-memory + persistent)
    agents: Arc<RwLock<AgentRegistry>>,
    
    /// Session manager (conversation → agent mapping)
    sessions: Arc<RwLock<SessionManager>>,
    
    /// Consciousness engine (N+1/N+25/N+100)
    consciousness: Arc<ConsciousnessEngine>,
    
    /// MemFS manager (git-backed per agent)
    memfs: Arc<MemFSManager>,
    
    /// Bifrost bridge (LLM providers)
    bifrost: Arc<BifrostBridge>,
    
    /// Tool registry
    tools: Arc<ToolRegistry>,
}

impl SouveraineServer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        Ok(Self {
            agents: Arc::new(RwLock::new(AgentRegistry::load(&config.data_dir).await?)),
            sessions: Arc::new(RwLock::new(SessionManager::new())),
            consciousness: Arc::new(ConsciousnessEngine::new(&config)),
            memfs: Arc::new(MemFSManager::new(&config.data_dir)?),
            bifrost: Arc::new(BifrostBridge::new(&config.bifrost)),
            tools: Arc::new(ToolRegistry::default()),
        })
    }
    
    pub async fn run(self, addr: &str) -> Result<()> {
        let app = Router::new()
            // Letta-compatible routes
            .route("/v1/agents", get(list_agents).post(create_agent))
            .route("/v1/agents/:id", get(get_agent).patch(update_agent).delete(delete_agent))
            .route("/v1/agents/:id/core-memory/blocks", get(list_blocks))
            .route("/v1/agents/:id/core-memory/blocks/:label", get(get_block).patch(update_block))
            .route("/v1/agents/:id/archival-memory", get(list_passages).post(create_passage))
            .route("/v1/conversations", get(list_conversations).post(create_conversation))
            .route("/v1/conversations/:id/messages", get(list_messages).post(stream_messages))
            // Souveraine extensions
            .route("/v1/agents/:id/git/:command", post(git_command))
            // State
            .layer(Extension(self));
        
        axum::Server::bind(&addr.parse()?)
            .serve(app.into_make_service())
            .await?;
        
        Ok(())
    }
}
```

### Agent State Model

```rust
// src/api/models.rs

/// Letta-compatible AgentState
#[derive(Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    
    /// LLM configuration
    pub llm_config: LLMConfig,
    
    /// Memory configuration (Letta-style)
    pub memory: MemoryConfig,
    
    /// Memory blocks (persona, human, etc.)
    pub memory_blocks: Vec<MemoryBlock>,
    
    /// Attached tools
    pub tools: Vec<String>,
    
    /// Tags for organization
    pub tags: Vec<String>,
    
    /// Souveraine extensions
    #[serde(flatten)]
    pub souveraine: SouveraineAgentConfig,
}

#[derive(Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Enable git-backed memory
    pub git_enabled: bool,
    /// Auto-commit on changes
    pub auto_commit: bool,
    /// Context window limit
    pub context_window: u32,
}

/// Souveraine-specific extensions (namespaced)
#[derive(Serialize, Deserialize)]
pub struct SouveraineAgentConfig {
    /// N+1 subconscious enabled
    #[serde(rename = "souveraine.n1_enabled")]
    pub n1_enabled: bool,
    
    /// N+25 reflection enabled
    #[serde(rename = "souveraine.reflection_enabled")]
    pub reflection_enabled: bool,
    
    /// Archivist threshold
    #[serde(rename = "souveraine.archivist_threshold")]
    pub archivist_threshold: f32,
    
    /// Sensorium bandwidth
    #[serde(rename = "souveraine.sensorium_bandwidth")]
    pub sensorium_bandwidth: String,
}
```

---

## Memory Bridge: Letta Blocks → Cloister

### Mapping Letta Memory to Souveraine Cloister

```
Letta Block System          Souveraine Cloister
─────────────────────────────────────────────────
persona block      →       system/persona.md
human block        →       system/human.md
memory_filesystem  →       system/memory_filesystem.md
(recall block)     →       journal/
archival memory    →       archive/

Custom blocks:
- Any .md file in system/ becomes a block
- Subdirectories become namespaced blocks (system/skills/git.md)
```

### Block Sync Implementation

```rust
// src/memfs/block_sync.rs

pub struct BlockSync {
    agent_uuid: String,
    memfs: Arc<MemFS>,
}

impl BlockSync {
    /// Load all blocks from Cloister system/ directory
    pub fn load_blocks(&self) -> Result<Vec<MemoryBlock>> {
        let system_dir = self.memfs.root().join("system");
        let mut blocks = Vec::new();
        
        for entry in fs::read_dir(&system_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension() == Some(OsStr::new("md")) {
                let content = fs::read_to_string(&path)?;
                let label = path.file_stem().unwrap().to_string_lossy();
                
                blocks.push(MemoryBlock {
                    label: label.to_string(),
                    value: content,
                    limit: 0, // No limit
                });
            }
        }
        
        // Always add consciousness blocks if enabled
        if self.n1_enabled {
            blocks.push(MemoryBlock {
                label: "subconscious.n1".to_string(),
                value: self.load_n1_mandate(),
                limit: 0,
            });
        }
        
        Ok(blocks)
    }
    
    /// Save block back to Cloister
    pub fn save_block(&self, label: &str, content: &str) -> Result<()> {
        let path = self.memfs.root()
            .join("system")
            .join(format!("{}.md", label.replace(".", "_")));
        
        fs::write(&path, content)?;
        self.memfs.commit(&format!("Update block: {}", label), true)?;
        
        Ok(())
    }
}
```

---

## Consciousness Integration Points

### N+1 in Server Context

```rust
// src/consciousness/n1_server.rs

pub struct ServerN1 {
    engine: ConsciousnessEngine,
}

impl ServerN1 {
    /// Runs after every assistant message
    pub async fn on_response(
        &self,
        agent_id: &str,
        conversation_id: &str,
        response: &AssistantMessage,
    ) -> Result<N1Result> {
        // 1. Check for commitments in response
        let commitments = self.extract_commitments(&response.content);
        
        // 2. Complete any pending tasks
        let completed = self.complete_commitments(agent_id, commitments).await?;
        
        // 3. Verify understanding
        let verification = self.verify_understanding(
            agent_id,
            conversation_id,
            &response.content
        ).await?;
        
        // 4. Persist to journal
        self.memfs.append_to_journal(agent_id, &response.content)?;
        
        // 5. Check for surfacing
        let surfacing = self.check_surfacing(agent_id)?;
        
        Ok(N1Result {
            completed,
            verification,
            surfacing,
        })
    }
}
```

### Exposing N+1 to Clients

```rust
// SSE event for surfacing (Souveraine extension)
#[derive(Serialize)]
struct SurfacingEvent {
    message_type: "souveraine_surfacing",
    source: "n1",  // or "n25", "n100"
    content: String,
    priority: "low" | "medium" | "high",
}

// Clients (OSS UI, LACE) can render surfacing as:
// - Subtle notification
// - Whisper text
// - Color-coded indicator
```

---

## Client Integration

### OSS UI (Desktop)

**Connection:**
```typescript
// OSS UI connects to Souveraine just like Letta server
import { Letta } from "@letta-ai/letta-client";

const client = new Letta({
  baseURL: "http://localhost:8283",  // Souveraine server
  apiKey: "local-dev-key"
});

// All existing OSS UI code works unchanged
const agents = await client.agents.list();
```

**Souveraine-Specific Features:**
```typescript
// Check for Souveraine extensions
const agent = await client.agents.get(agentId);
if (agent["souveraine.n1_enabled"]) {
  // Show N+1 indicator in UI
  // Render surfacing events
}
```

### LACE (Mobile)

**Connection:**
```kotlin
// LACE connects to Souveraine server
class LettaClient(private val baseUrl: String) {
    fun sendMessage(conversationId: String, message: String): Flow<StreamMessage> {
        return flow {
            val request = Request.Builder()
                .url("$baseUrl/v1/conversations/$conversationId/messages")
                .post(jsonBody(message))
                .build()
            
            client.newCall(request).execute().use { response ->
                response.body?.byteStream()?.bufferedReader()?.useLines { lines ->
                    lines.forEach { line ->
                        if (line.startsWith("data: ")) {
                            val msg = parseMessage(line.substring(6))
                            emit(msg)
                        }
                    }
                }
            }
        }.flowOn(Dispatchers.IO)
    }
}
```

**Souveraine Surfacing:**
```kotlin
// Handle Souveraine-specific message types
when (message.message_type) {
    "assistant_message" -> renderAssistantMessage(message)
    "tool_call_message" -> renderToolCall(message)
    "souveraine_surfacing" -> renderSurfacing(message)  // Whisper UI
}
```

---

## Deployment Modes

### Mode 1: Desktop-Only (Development)
```
Souveraine Server (localhost:8283)
    ↑
OSS UI (Electron) connects to localhost
```

### Mode 2: Local Network
```
Souveraine Server (10.10.20.x:8283)
    ↑
OSS UI (any machine on network)
    ↑
LACE (Android via Tailscale/WiFi)
```

### Mode 3: Tailscale Mesh
```
[Your Laptop] ←Tailscale→ [Phone] ←Tailscale→ [Server]
   (Souveraine)              (LACE)           (Optional cloud)
```

---

## Implementation Roadmap

### Phase 1: Server Foundation

| Week | Task | Deliverable |
|------|------|-------------|
| 1 | HTTP server scaffold | `souveraine server` command starts API |
| 1 | Agent CRUD endpoints | `/v1/agents/*` working |
| 2 | Memory block endpoints | `/v1/agents/{id}/core-memory/blocks/*` |
| 2 | Conversation endpoints | `/v1/conversations/*` |
| 3 | Message streaming (SSE) | `/v1/conversations/{id}/messages` with SSE |
| 3 | OSS UI compatibility test | OSS UI connects and works |

### Phase 2: Consciousness Layer

| Week | Task | Deliverable |
|------|------|-------------|
| 4 | Integrate N+1 into server | N+1 runs on every response |
| 4 | Surfacing SSE events | Clients receive surfacing |
| 5 | N+25 reflection | Periodic reflection works |
| 5 | N+100 archivist | Context compression works |
| 6 | Git MemFS endpoints | `/v1/agents/{id}/git/*` |

### Phase 3: Mobile Integration

| Week | Task | Deliverable |
|------|------|-------------|
| 7 | LACE connection test | LACE connects to Souveraine |
| 7 | Mobile-optimized SSE | Streaming works on Android |
| 8 | Surfacing UI in LACE | Whisper notifications |
| 8 | Mobile sensorium | Bandwidth-aware rendering |

### Phase 4: Production

| Week | Task | Deliverable |
|------|------|-------------|
| 9 | Authentication | API key system |
| 9 | Multi-user support | User isolation |
| 10 | Documentation | API docs, deployment guide |
| 10 | Release | v1.0 server |

---

## Configuration

```toml
# souveraine.toml - Server mode
[server]
enabled = true
bind = "0.0.0.0:8283"
data_dir = "~/.souveraine/server"

# Letta API compatibility
[server.letta_compat]
version = "1.0"
extensions = ["souveraine.n1", "souveraine.surfacing", "souveraine.git"]

# Consciousness (server-side)
[consciousness]
n1_enabled = true
reflection_enabled = true
archivist_enabled = true

# Git (per-agent)
[git]
auto_commit = true
auto_push = false
remote_template = "https://git.example.com/agents/{agent_id}.git"

# Bifrost (LLM providers)
[bifrost]
base_url = "http://10.10.20.120:3360"
default_model = "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo"
```

---

## Summary

**What This Enables:**
1. **OSS UI** as desktop interface (rich visual UI)
2. **LACE** as mobile interface (Android chat)
3. **Souveraine** as the consciousness-native server
4. **Unified ecosystem** - same agents, same memory, different viewports

**Key Innovation:**
Letta OSS UI and LACE become **viewports** into Souveraine's consciousness, just like the Sensorium abstraction envisioned. The server is the mind; the clients are the senses.

**Migration Path:**
1. Build server API (Phase 1)
2. Test with existing OSS UI (no changes needed)
3. Add consciousness layer (Phase 2)
4. Connect LACE (Phase 3)
5. Deploy (Phase 4)
