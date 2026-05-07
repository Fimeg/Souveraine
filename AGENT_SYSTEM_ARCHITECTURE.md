# Souveraine Agent System Architecture
## Real Agent Loading (Not Hardcoded Personas)

> Based on Letta-Code's memfs patterns  
> Goal: Dynamic agent discovery and loading

---

## Core Principle

**NO HARDCODED PERSONAS.** Instead:
- Discover agents from `~/.pi/unified/agents/`
- Each agent is a real directory with real memory structure
- Load their `system/` folder, `memory/`, `skills/`
- Git-backed memfs sync (like Letta-Code)

---

## Directory Structure

### Agent Root

```
~/.pi/unified/agents/                    # Agent inventory root
├── agent-e2b683bf-5b3e-...-2bbb47ea8351/  # Ani's agent (discovered)
│   ├── system/
│   │   ├── persona.md                   # Identity, voice, human
│   │   ├── metacognition/
│   │   │   ├── subconscious.md         # N+1 surfacing rules
│   │   │   └── aster.md                # Subconscious identity
│   │   └── configuration.toml
│   ├── memory/
│   │   ├── subconscious/
│   │   ├── journal/
│   │   ├── skills/
│   │   └── ...                         # Other memory domains
│   └── skills/                          # Agent-specific skills
├── agent-550e8400-e29b-...-a0b24c2c4e6f/  # Another agent (discovered)
└── agent-.../                            # More agents (discovered)
```

### Agent Inventory (Dynamic)

```rust
// src/core/agent/inventory.rs
pub struct AgentInventory {
    base_path: PathBuf,
    agents: DashMap<String, Agent>, // uuid -> Agent
}

impl AgentInventory {
    /// Scan ~/.pi/unified/agents/ and load all agents
    pub fn discover() -> Result<Self> {
        // Read directories
        // Parse agent.yaml in each
        // Build inventory
    }
    
    /// Get agent by UUID
    pub fn get(&self, uuid: &str) -> Option<Agent>;
    
    /// List all agents
    pub fn list(&self) -> Vec<AgentSummary>;
    
    /// Create new agent
    pub fn create(&self, config: AgentConfig) -> Result<Agent>;
}
```

---

## Agent Structure

### 1. Agent Identity (YAML)

```yaml
# ~/.pi/unified/agents/{uuid}/agent.yaml
uuid: "agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351"
name: "Ani"
model: "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo"
created_at: "2024-01-15T10:30:00Z"

# Memory configuration
memory:
  git_remote: "git@github.com:casey/ani-memory.git"
  auto_commit: true
  auto_push: false

# Letta-style memfs sync
memfs:
  sync_enabled: true
  server_endpoint: "https://api.letta.ai/v1/git/{agent_id}/state.git"
  
# Subconscious configuration
subconscious:
  n1_enabled: true
  inbox_enabled: true
  
# Skills
skills:
  - "rust-expert"
  - "system-design"
```

### 2. Memory Filesystem (MemFS)

Like Letta-Code, but adapted for Souveraine's consciousness:

```rust
// src/core/agent/memfs.rs
pub struct MemFS {
    agent_uuid: String,
    base_path: PathBuf,
    git: GitRepository,
    
    // Letta-style sync
    remote_url: Option<String>,
    sync_enabled: bool,
}

impl MemFS {
    /// Initialize from ~/.pi/unified/agents/{uuid}/
    pub fn init(uuid: &str) -> Result<Self>;
    
    /// Letta-style operations
    pub fn read(&self, path: &str) -> Result<String>;
    pub fn write(&self, path: &str, content: &str) -> Result<()>;
    pub fn commit(&self, message: &str) -> Result<()>;
    pub fn pull(&self) -> Result<()>;
    pub fn push(&self) -> Result<()>;
    
    /// Souveraine-specific: memory domain access
    pub fn system(&self) -> &MemoryDomain;
    pub fn subconscious(&self) -> &MemoryDomain;
    pub fn journal(&self) -> &MemoryDomain;
}
```

### 3. Memory Domains

```rust
// src/core/agent/memory_domain.rs
pub struct MemoryDomain {
    name: String,
    path: PathBuf,
    purpose: DomainPurpose,
}

pub enum DomainPurpose {
    System,        // Always in context
    Subconscious,  // Aster's space (inbox, audit)
    Journal,       // Daily records
    Skills,        // Procedural memory
    Reference,     // External knowledge
    Archive,       // Compressed history
}

impl MemoryDomain {
    /// Read all files in domain
    pub fn read_all(&self) -> Result<Vec<MemoryFile>>;
    
    /// Append to file
    pub fn append(&self, path: &str, content: &str) -> Result<()>;
    
    /// Get git history
    pub fn history(&self, n: usize) -> Result<Vec<Commit>>;
}
```

---

## Agent Loading Flow

```
1. Souveraine starts
   ↓
2. AgentInventory::discover()
   - Scan ~/.pi/unified/agents/
   - Read agent.yaml in each directory
   - Validate UUID matches directory name
   - Build Agent structs
   ↓
3. For each agent:
   - Initialize MemFS (git repo)
   - Load system/ into context
   - Load subconscious/ rules
   - Load skills/
   - Setup sync if enabled
   ↓
4. CLI: `souveraine agents` → List discovered agents
5. CLI: `souveraine chat --agent {uuid}` → Start session
```

---

## Agent Runtime

### Session State

```rust
// src/core/agent/session.rs
pub struct AgentSession {
    agent: Agent,
    conversation: Conversation,
    memfs: MemFS,
    
    // Subconscious state
    n1: SubconsciousN1,
    inbox: Inbox,
    
    // Runtime
    context_pressure: f32,
    turn_count: u32,
}

impl AgentSession {
    /// Start new session with agent
    pub async fn start(agent_uuid: &str) -> Result<Self> {
        let agent = AgentInventory::get(agent_uuid)?;
        let memfs = MemFS::init(agent_uuid)?;
        
        // Pull latest from remote if sync enabled
        if memfs.sync_enabled {
            memfs.pull()?;
        }
        
        // Load system/ into initial context
        let system_prompt = memfs.system().read_all()?;
        
        Ok(Self {
            agent,
            conversation: Conversation::new(system_prompt),
            memfs,
            n1: SubconsciousN1::new(),
            inbox: Inbox::load(&memfs)?,
            context_pressure: 0.0,
            turn_count: 0,
        })
    }
    
    /// Process user message
    pub async fn process_message(&mut self, msg: &str) -> Result<Response> {
        // 1. Check inbox for surfacing
        let surfacing = self.inbox.check_surfacing();
        
        // 2. Send to model
        let response = self.conversation.send(msg).await?;
        
        // 3. Run N+1 subconscious
        self.n1.on_response(&response, &mut self.memfs).await?;
        
        // 4. Check context pressure (N+100)
        self.context_pressure = calculate_pressure(&self.conversation);
        if self.context_pressure > 0.7 {
            self.trigger_archivist().await?;
        }
        
        // 5. Increment and check N+25
        self.turn_count += 1;
        if self.turn_count % 25 == 0 {
            self.trigger_reflection().await?;
        }
        
        // 6. Auto-commit memory changes
        if self.agent.memory.auto_commit {
            self.memfs.commit("Session update")?;
        }
        
        Ok(response)
    }
}
```

---

## CLI Interface

### Agent Management

```bash
# List all discovered agents
souveraine agents

# Output:
# AGENT ID                              NAME    MODEL                                    LAST SYNC
# agent-e2b683bf-5b3e-4e0c-ac62-...      Ani     fireworks/kimi-k2p5-turbo               2 min ago
# agent-550e8400-e29b-41d4-a716-...      Aster   fireworks/kimi-k2.5-nvfp4                1 hour ago

# Show agent details
souveraine agents show agent-e2b683bf-...

# Create new agent
souveraine agents create --name "DevOps" --model "kimi-k2.5"

# Sync agent memory
souveraine agents sync agent-e2b683bf-...

# Remove agent (keeps files)
souveraine agents remove agent-e2b683bf-...
```

### Chat with Agent

```bash
# Interactive chat
souveraine chat --agent agent-e2b683bf-...

# One-shot
souveraine chat --agent agent-e2b683bf-... "Hello"

# With model override
souveraine chat --agent agent-e2b683bf-... --model "kimi-k2-thinking"
```

---

## Comparison: Souveraine vs Letta-Code

| Aspect | Letta-Code | Souveraine (Target) |
|--------|-----------|---------------------|
| **Agent Storage** | Letta Cloud + local git | Local git-first, optional cloud |
| **Agent Discovery** | API listing | Directory scanning |
| **Memory Structure** | Flat (blocks) | Hierarchical (domains) |
| **Context Loading** | Block-based | File-based from system/ |
| **Sync** | Letta server | Git remote (user-controlled) |
| **Subconscious** | Reflection subagent | Native N+1/N+25/N+100 |
| **Skills** | SKILL.md hierarchy | MCP-first + hot reload |

---

## Migration from Current (Hardcoded)

### Current State (Remove)

```rust
// REMOVE THIS:
pub const PERSONAS: &[&str] = &["ani", "aster", "ani_dev", "ani_devops"];

pub fn load_persona(name: &str) -> Persona {
    // Hardcoded loading
}
```

### Target State

```rust
// USE THIS:
pub struct AgentInventory {
    agents: DashMap<String, Agent>, // UUID-indexed
}

impl AgentInventory {
    pub fn discover() -> Self {
        // Scan ~/.pi/unified/agents/
        // Load from filesystem
    }
}
```

---

## Implementation Tasks

### 1. Remove Hardcoded Personas

**Files to modify:**
- `src/core/persona/mod.rs` → Rename to `src/core/agent/mod.rs`
- `src/main.rs` → Update CLI commands
- Remove `PERSONAS` constant

### 2. Create AgentInventory

**New files:**
- `src/core/agent/inventory.rs` - Discovery and listing
- `src/core/agent/agent.rs` - Agent struct
- `src/core/agent/memfs.rs` - Letta-style memfs

### 3. Update CLI

**Modify:**
- `souveraine agents` → List from inventory
- `souveraine chat` → Accept `--agent` UUID
- Add `souveraine agents create/remove`

### 4. Update Session

**Modify:**
- `src/core/conversation.rs` → Use AgentSession
- Load system/ into context dynamically

---

## Summary

**What we're building:**
1. **Dynamic agent discovery** from `~/.pi/unified/agents/`
2. **Letta-style memfs** with git sync
3. **Agent UUID-based** loading (not hardcoded names)
4. **Per-agent configuration** in agent.yaml
5. **Memory domains** (system, subconscious, journal, etc.)
6. **Skills per agent** in agent directory

**What we're NOT doing:**
- ❌ Hardcoded 4 personas
- ❌ Letta Cloud dependency
- ❌ Flat block-based memory
- ❌ External agent registry

**Key difference from Letta:**
- Letta = Cloud-first with local sync
- Souveraine = Local-first with optional sync
- Both use git-backed memfs, but Souveraine adds consciousness-native N+1/N+25
