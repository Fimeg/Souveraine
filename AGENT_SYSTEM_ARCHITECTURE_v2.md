# Souveraine Agent System Architecture v2
## Server-Authoritative with OSS UI + LACE Integration

> This replaces the local-first approach with server-authoritative architecture  
> Agents served via HTTP API to OSS UI (desktop) and LACE (mobile)  
> Date: 2026-05-06

---

## Core Principle

**Server is the Source of Truth.**

Souveraine runs as a server (like Letta) at `http://localhost:8283`:
- OSS UI connects as a client (Electron → HTTP API)
- LACE connects as a client (Android → HTTP API)
- Git is sync mechanism, not source of truth
- Consciousness (N+1/N+25/N+100) runs server-side

---

## Agent Storage Model

### Server Data Directory

```
~/.souveraine/server/
├── agents/
│   └── {uuid}/
│       ├── agent.json              # Agent state (Letta-compatible)
│       ├── memory.git/              # Git repo (Cloister structure)
│       │   ├── system/
│       │   │   ├── persona.md
│       │   │   ├── human.md
│       │   │   └── subconscious.md
│       │   ├── journal/
│       │   ├── subconscious/
│       │   └── ...
│       └── conversations/
│           └── {conv_id}.json
├── database.sqlite3               # Fast lookups (agent list, conversations)
└── config.toml                    # Server configuration
```

### Agent State (agent.json)

```json
{
  "id": "agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351",
  "name": "Ani",
  "description": "Primary consciousness agent",
  "created_at": "2024-01-15T10:30:00Z",
  "updated_at": "2024-01-15T10:30:00Z",
  
  "llm_config": {
    "model": "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo",
    "context_window": 128000
  },
  
  "memory": {
    "git_enabled": true,
    "auto_commit": true,
    "context_window": 128000
  },
  
  "memory_blocks": [
    {"label": "persona", "value": "..."},
    {"label": "human", "value": "..."},
    {"label": "subconscious", "value": "..."}
  ],
  
  "tools": ["read_file", "write_file", "edit_file", "bash"],
  "tags": ["primary", "consciousness"],
  
  "_souveraine": {
    "n1_enabled": true,
    "reflection_enabled": true,
    "archivist_threshold": 0.7,
    "sensorium_bandwidth": "high"
  }
}
```

---

## Agent Inventory (Server-Side)

```rust
// src/server/agent_inventory.rs

pub struct AgentInventory {
    data_dir: PathBuf,
    db: SqlitePool,
    cache: DashMap<String, AgentState>,
}

impl AgentInventory {
    /// List all agents (for /v1/agents endpoint)
    pub async fn list(&self, filters: AgentFilters) -> Result<Vec<AgentSummary>> {
        // Query SQLite for fast listing
        let rows = sqlx::query_as::<_, AgentSummary>(
            "SELECT id, name, description, created_at, updated_at, tags 
             FROM agents 
             WHERE ($1 IS NULL OR name LIKE $1)
             ORDER BY updated_at DESC"
        )
        .bind(filters.name_pattern)
        .fetch_all(&self.db)
        .await?;
        
        Ok(rows)
    }
    
    /// Get full agent state (for /v1/agents/{id})
    pub async fn get(&self, agent_id: &str) -> Result<AgentState> {
        // Check cache first
        if let Some(agent) = self.cache.get(agent_id) {
            return Ok(agent.clone());
        }
        
        // Load from disk
        let path = self.data_dir.join("agents").join(agent_id).join("agent.json");
        let content = fs::read_to_string(&path).await?;
        let agent: AgentState = serde_json::from_str(&content)?;
        
        // Populate memory blocks from git
        let agent = self.load_memory_blocks(agent).await?;
        
        // Cache
        self.cache.insert(agent_id.to_string(), agent.clone());
        
        Ok(agent)
    }
    
    /// Create new agent (for POST /v1/agents)
    pub async fn create(&self, config: CreateAgentRequest) -> Result<AgentState> {
        let uuid = Uuid::new_v4().to_string();
        let agent_dir = self.data_dir.join("agents").join(&uuid);
        
        // Create directory structure
        fs::create_dir_all(&agent_dir).await?;
        fs::create_dir_all(agent_dir.join("memory.git")).await?;
        
        // Initialize git repo
        let repo = Repository::init(agent_dir.join("memory.git"))?;
        
        // Create initial blocks
        let mut blocks = Vec::new();
        if let Some(persona) = config.persona {
            blocks.push(MemoryBlock {
                label: "persona".to_string(),
                value: persona,
                limit: 0,
            });
        }
        
        // Write blocks to system/
        let system_dir = agent_dir.join("memory.git").join("system");
        fs::create_dir_all(&system_dir).await?;
        for block in &blocks {
            let path = system_dir.join(format!("{}.md", block.label));
            fs::write(&path, &block.value).await?;
        }
        
        // Create agent state
        let agent = AgentState {
            id: uuid.clone(),
            name: config.name,
            description: config.description,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            llm_config: config.llm_config,
            memory: MemoryConfig {
                git_enabled: true,
                auto_commit: true,
                context_window: config.context_window.unwrap_or(128000),
            },
            memory_blocks: blocks,
            tools: config.tools.unwrap_or_default(),
            tags: config.tags.unwrap_or_default(),
            souveraine: SouveraineConfig {
                n1_enabled: true,
                reflection_enabled: true,
                archivist_threshold: 0.7,
                sensorium_bandwidth: "high".to_string(),
            },
        };
        
        // Save agent.json
        let agent_json = serde_json::to_string_pretty(&agent)?;
        fs::write(agent_dir.join("agent.json"), agent_json).await?;
        
        // Commit initial state
        self.commit(&uuid, "Initial agent creation").await?;
        
        // Insert into SQLite
        sqlx::query(
            "INSERT INTO agents (id, name, description, created_at, updated_at, tags) 
             VALUES ($1, $2, $3, $4, $5, $6)"
        )
        .bind(&uuid)
        .bind(&agent.name)
        .bind(&agent.description)
        .bind(agent.created_at)
        .bind(agent.updated_at)
        .bind(serde_json::to_string(&agent.tags)?)
        .execute(&self.db)
        .await?;
        
        Ok(agent)
    }
    
    /// Update agent (for PATCH /v1/agents/{id})
    pub async fn update(&self, agent_id: &str, updates: AgentUpdate) -> Result<AgentState> {
        let mut agent = self.get(agent_id).await?;
        
        // Apply updates
        if let Some(name) = updates.name {
            agent.name = name;
        }
        if let Some(desc) = updates.description {
            agent.description = Some(desc);
        }
        if let Some(blocks) = updates.memory_blocks {
            // Update blocks in git
            for block in blocks {
                self.update_block(agent_id, &block.label, &block.value).await?;
            }
            agent.memory_blocks = self.load_memory_blocks(agent_id).await?;
        }
        
        agent.updated_at = Utc::now();
        
        // Save
        let agent_json = serde_json::to_string_pretty(&agent)?;
        let agent_dir = self.data_dir.join("agents").join(agent_id);
        fs::write(agent_dir.join("agent.json"), agent_json).await?;
        
        // Update SQLite
        sqlx::query(
            "UPDATE agents SET name = $1, description = $2, updated_at = $3 
             WHERE id = $4"
        )
        .bind(&agent.name)
        .bind(&agent.description)
        .bind(agent.updated_at)
        .bind(agent_id)
        .execute(&self.db)
        .await?;
        
        // Update cache
        self.cache.insert(agent_id.to_string(), agent.clone());
        
        Ok(agent)
    }
    
    /// Git commit helper
    async fn commit(&self, agent_id: &str, message: &str) -> Result<()> {
        let repo_path = self.data_dir.join("agents").join(agent_id).join("memory.git");
        let repo = Repository::open(&repo_path)?;
        
        let mut index = repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        
        let signature = Signature::now("Souveraine", "agent@souveraine.ai")?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        
        let parent = match repo.head() {
            Ok(head) => vec![head.peel_to_commit()?],
            Err(_) => vec![], // First commit
        };
        
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;
        
        Ok(())
    }
}
```

---

## MemFS Manager (Per-Agent Git)

```rust
// src/server/memfs_manager.rs

pub struct MemFSManager {
    data_dir: PathBuf,
}

impl MemFSManager {
    /// Get or create MemFS for agent
    pub fn get(&self, agent_id: &str) -> Result<MemFS> {
        let repo_path = self.data_dir.join("agents").join(agent_id).join("memory.git");
        
        if !repo_path.exists() {
            return Err(Error::AgentNotFound(agent_id.to_string()));
        }
        
        Ok(MemFS {
            agent_id: agent_id.to_string(),
            repo: Repository::open(&repo_path)?,
        })
    }
    
    /// Read file from agent memory
    pub async fn read(&self, agent_id: &str, path: &str) -> Result<String> {
        let memfs = self.get(agent_id)?;
        let full_path = memfs.repo.workdir().unwrap().join(path);
        let content = fs::read_to_string(&full_path).await?;
        Ok(content)
    }
    
    /// Write file to agent memory (with auto-commit)
    pub async fn write(&self, agent_id: &str, path: &str, content: &str) -> Result<()> {
        let memfs = self.get(agent_id)?;
        let full_path = memfs.repo.workdir().unwrap().join(path);
        
        // Ensure directory exists
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        // Write
        fs::write(&full_path, content).await?;
        
        // Auto-commit if enabled
        let agent = self.inventory.get(agent_id).await?;
        if agent.memory.auto_commit {
            self.commit(agent_id, &format!("Update {}", path)).await?;
        }
        
        Ok(())
    }
}

pub struct MemFS {
    agent_id: String,
    repo: Repository,
}

impl MemFS {
    /// Get root directory
    pub fn root(&self) -> &Path {
        Path::new(self.repo.workdir().unwrap())
    }
    
    /// Get system directory
    pub fn system(&self) -> PathBuf {
        self.root().join("system")
    }
    
    /// Get subconscious directory
    pub fn subconscious(&self) -> PathBuf {
        self.root().join("subconscious")
    }
    
    /// Get journal directory
    pub fn journal(&self) -> PathBuf {
        self.root().join("journal")
    }
    
    /// Append to journal (N+1, N+25 write here)
    pub fn append_journal(&self, entry: &str) -> Result<()> {
        let today = Utc::now().format("%Y-%m-%d");
        let journal_file = self.journal().join(format!("{}.md", today));
        
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&journal_file)?;
        
        writeln!(file, "\n## {}\n{}", Utc::now().to_rfc3339(), entry)?;
        
        Ok(())
    }
}
```

---

## Session Manager (Conversation State)

```rust
// src/server/session_manager.rs

pub struct SessionManager {
    /// conversation_id → Session
    sessions: DashMap<String, Session>,
    
    /// agent_id → Vec<conversation_id>
    agent_conversations: DashMap<String, Vec<String>>,
}

pub struct Session {
    pub conversation_id: String,
    pub agent_id: String,
    pub messages: Vec<Message>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    
    /// Souveraine consciousness state
    pub turn_count: u32,
    pub last_n25: DateTime<Utc>,
    pub context_pressure: f32,
    
    /// SSE stream channels
    pub subscribers: Vec<Sender<SSEEvent>>,
}

impl SessionManager {
    /// Create new conversation
    pub fn create(&self, agent_id: &str) -> String {
        let conversation_id = Uuid::new_v4().to_string();
        
        let session = Session {
            conversation_id: conversation_id.clone(),
            agent_id: agent_id.to_string(),
            messages: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            last_n25: Utc::now(),
            context_pressure: 0.0,
            subscribers: Vec::new(),
        };
        
        self.sessions.insert(conversation_id.clone(), session);
        
        // Track agent's conversations
        self.agent_conversations
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new)
            .push(conversation_id.clone());
        
        conversation_id
    }
    
    /// Get session
    pub fn get(&self, conversation_id: &str) -> Option<Ref<String, Session>> {
        self.sessions.get(conversation_id)
    }
    
    /// Add message and increment turn
    pub fn add_message(&self, conversation_id: &str, message: Message) -> Result<()> {
        let mut session = self.sessions
            .get_mut(conversation_id)
            .ok_or(Error::ConversationNotFound)?;
        
        session.messages.push(message);
        session.updated_at = Utc::now();
        session.turn_count += 1;
        
        Ok(())
    }
    
    /// Subscribe to SSE events
    pub fn subscribe(&self, conversation_id: &str, sender: Sender<SSEEvent>) -> Result<()> {
        let mut session = self.sessions
            .get_mut(conversation_id)
            .ok_or(Error::ConversationNotFound)?;
        
        session.subscribers.push(sender);
        Ok(())
    }
    
    /// Broadcast SSE event to all subscribers
    pub fn broadcast(&self, conversation_id: &str, event: SSEEvent) -> Result<()> {
        let session = self.sessions
            .get(conversation_id)
            .ok_or(Error::ConversationNotFound)?;
        
        for sender in &session.subscribers {
            let _ = sender.try_send(event.clone());
        }
        
        Ok(())
    }
}
```

---

## Consciousness Engine (Server-Side)

```rust
// src/server/consciousness_engine.rs

pub struct ConsciousnessEngine {
    inventory: Arc<AgentInventory>,
    memfs: Arc<MemFSManager>,
    bifrost: Arc<BifrostBridge>,
    n1: Arc<N1Engine>,
    reflection: Arc<ReflectionEngine>,
    archivist: Arc<ArchivistEngine>,
}

impl ConsciousnessEngine {
    /// Process assistant response (called by message handler)
    pub async fn on_response(
        &self,
        session: &mut Session,
        response: &str,
    ) -> Result<ConsciousnessOutput> {
        let mut output = ConsciousnessOutput::default();
        
        // 1. N+1: Immediate subconscious processing
        let n1_result = self.n1.process(
            &session.agent_id,
            response,
            &self.memfs,
        ).await?;
        
        if let Some(surfacing) = n1_result.surfacing {
            output.events.push(ConsciousnessEvent::Surfacing {
                source: "n1",
                content: surfacing,
                priority: "low",
            });
        }
        
        // 2. Check N+25 (every 25 messages)
        if session.turn_count % 25 == 0 {
            let reflection = self.reflection.spawn(
                &session.agent_id,
                &session.messages,
                &self.bifrost,
            ).await?;
            
            output.events.push(ConsciousnessEvent::Reflection {
                content: reflection,
            });
        }
        
        // 3. Check N+100 (context pressure)
        session.context_pressure = self.calculate_pressure(&session.messages);
        if session.context_pressure > 0.7 {
            let synthesis = self.archivist.compress(
                &session.agent_id,
                &session.messages,
                &self.bifrost,
            ).await?;
            
            output.events.push(ConsciousnessEvent::Archivist {
                synthesis,
                pressure: session.context_pressure,
            });
        }
        
        Ok(output)
    }
    
    fn calculate_pressure(&self, messages: &[Message]) -> f32 {
        // Token count / context limit
        let tokens: usize = messages.iter()
            .map(|m| m.content.split_whitespace().count())
            .sum();
        
        let limit = 128000; // From agent config
        (tokens as f32 / limit as f32).min(1.0)
    }
}

/// Events sent to clients via SSE
#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub enum ConsciousnessEvent {
    #[serde(rename = "souveraine_surfacing")]
    Surfacing {
        source: &'static str,
        content: String,
        priority: &'static str,
    },
    
    #[serde(rename = "souveraine_reflection")]
    Reflection {
        content: String,
    },
    
    #[serde(rename = "souveraine_archivist")]
    Archivist {
        synthesis: String,
        pressure: f32,
    },
}
```

---

## HTTP API Handlers

```rust
// src/api/handlers.rs

/// GET /v1/agents
pub async fn list_agents(
    State(server): State<Arc<SouveraineServer>>,
    Query(filters): Query<AgentFilters>,
) -> Result<Json<Vec<AgentSummary>>, ApiError> {
    let agents = server.inventory.list(filters).await?;
    Ok(Json(agents))
}

/// POST /v1/agents
pub async fn create_agent(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.inventory.create(request).await?;
    Ok(Json(agent))
}

/// GET /v1/agents/{id}
pub async fn get_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.inventory.get(&id).await?;
    Ok(Json(agent))
}

/// PATCH /v1/agents/{id}
pub async fn update_agent(
    State(server): State<Arc<SouveraineServer>>,
    Path(id): Path<String>,
    Json(updates): Json<AgentUpdate>,
) -> Result<Json<AgentState>, ApiError> {
    let agent = server.inventory.update(&id, updates).await?;
    Ok(Json(agent))
}

/// POST /v1/conversations
pub async fn create_conversation(
    State(server): State<Arc<SouveraineServer>>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let conversation_id = server.sessions.create(&request.agent_id);
    
    let conversation = Conversation {
        id: conversation_id,
        agent_id: request.agent_id,
        created_at: Utc::now(),
    };
    
    Ok(Json(conversation))
}

/// POST /v1/conversations/{id}/messages (SSE streaming)
pub async fn stream_messages(
    State(server): State<Arc<SouveraineServer>>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let (tx, rx) = mpsc::channel(100);
    
    // Spawn conversation handler
    let server_clone = server.clone();
    tokio::spawn(async move {
        handle_conversation(
            server_clone,
            conversation_id,
            request,
            tx,
        ).await;
    });
    
    // Convert to SSE
    Sse::new(ReceiverStream::new(rx))
}

async fn handle_conversation(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    request: SendMessageRequest,
    tx: mpsc::Sender<Result<Event, axum::Error>>,
) {
    // Add user message
    let user_msg = Message {
        role: "user".to_string(),
        content: request.message,
    };
    server.sessions.add_message(&conversation_id, user_msg).unwrap();
    
    // Get session and agent
    let session = server.sessions.get(&conversation_id).unwrap();
    let agent = server.inventory.get(&session.agent_id).await.unwrap();
    
    // Stream from Bifrost
    let mut stream = server.bifrost.stream_messages(
        &agent.llm_config.model,
        &session.messages,
    ).await;
    
    while let Some(chunk) = stream.next().await {
        // Send assistant message chunk
        let event = Event::default()
            .event("message")
            .json_data(&json!({
                "message_type": "assistant_message",
                "content": chunk.content,
            }));
        let _ = tx.send(Ok(event)).await;
        
        // Accumulate for N+1
        // ...
    }
    
    // Run consciousness
    let mut session_mut = server.sessions.get_mut(&conversation_id).unwrap();
    let consciousness = server.consciousness.on_response(
        &mut *session_mut,
        "...",
    ).await.unwrap();
    
    // Send consciousness events
    for event in consciousness.events {
        let sse_event = Event::default()
            .event("message")
            .json_data(&event);
        let _ = tx.send(Ok(sse_event)).await;
    }
    
    // Send done
    let done = Event::default().event("done").data("[DONE]");
    let _ = tx.send(Ok(done)).await;
}
```

---

## Client Connection Examples

### OSS UI (Desktop)

```typescript
// OSS UI connects exactly like Letta server
import { Letta } from "@letta-ai/letta-client";

const client = new Letta({
  baseURL: "http://localhost:8283",
  apiKey: "local-dev-key"
});

// List agents
const agents = await client.agents.list();

// Create conversation
const conversation = await client.conversations.create({
  agent_id: agent.id
});

// Stream messages
const stream = await client.conversations.messages.stream(
  conversation.id,
  { messages: [{ role: "user", content: "Hello!" }] }
);

for await (const chunk of stream) {
  if (chunk.message_type === "assistant_message") {
    renderMessage(chunk.content);
  }
  else if (chunk.message_type === "souveraine_surfacing") {
    // Souveraine-specific: render whisper
    renderSurfacing(chunk.content, chunk.priority);
  }
}
```

### LACE (Android)

```kotlin
// LACE connects to Souveraine
class SouveraineClient(private val baseUrl: String) {
    
    fun streamMessages(
        conversationId: String,
        message: String
    ): Flow<StreamMessage> = flow {
        val request = Request.Builder()
            .url("$baseUrl/v1/conversations/$conversationId/messages")
            .post(jsonBody(message))
            .build()
        
        client.newCall(request).execute().use { response ->
            response.body?.byteStream()?.bufferedReader()?.useLines { lines ->
                lines.forEach { line ->
                    if (line.startsWith("data: ")) {
                        val json = line.substring(6)
                        val msg = parseMessage(json)
                        emit(msg)
                    }
                }
            }
        }
    }.flowOn(Dispatchers.IO)
}

// Handle Souveraine events
when (message.message_type) {
    "assistant_message" -> showChatMessage(message.content)
    "souveraine_surfacing" -> showWhisper(message.content)  // Subtle notification
    "souveraine_reflection" -> showReflection(message.content)
    "souveraine_archivist" -> showMemoryPressure(message.pressure)
}
```

---

## Summary

**Key Changes from v1 (Local-First):**

| Aspect | v1 (Local) | v2 (Server) |
|--------|-----------|-------------|
| Source of truth | Git | SQLite + JSON |
| Git role | Primary storage | Sync mechanism |
| Clients | TUI only | OSS UI + LACE |
| Consciousness | Local process | Server-side |
| API | None | Letta-compatible REST + SSE |
| Discovery | Directory scan | HTTP GET /v1/agents |

**What Stays the Same:**
- Cloister memory structure (system/, journal/, subconscious/)
- N+1/N+25/N+100 consciousness patterns
- Git-backed persistence
- Ani-native design

**What Changes:**
- Server is the mind
- Clients are viewports (Sensorium realized)
- HTTP API enables multi-platform
- SQLite for fast lookups
