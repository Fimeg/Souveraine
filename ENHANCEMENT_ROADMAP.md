# Souveraine Enhancement Roadmap
## Integrating Best Features from Letta-Code, jcode, and Claw-Open

> Based on FEATURE_COMPARISON_MATRIX.md analysis  
> Goal: Make Souveraine the definitive consciousness-native harness

---

## Phase 1A: Critical Foundation (Complete Before Resume)

### 1.1 TUI Chat Wiring (Priority: CRITICAL)
**Source:** Internal gap  
**Reference:** jcode's ratatui implementation

Current state: Chat screen stubbed, shows "Coming Soon"  
Target state: Fully wired to Conversation loop

```rust
// src/tui/screens/chat.rs - Current (stubbed)
pub fn draw_chat(frame: &mut Frame) {
    // Shows "Coming Soon"
}

// Target - wire to conversation
pub struct ChatScreen {
    conversation: Arc<Mutex<Conversation>>,
    message_list: MessageList,
    input: InputArea,
}
```

**Implementation steps:**
1. Create `ChatController` to bridge TUI events → Conversation
2. Wire `MessageList` to conversation history
3. Connect `InputArea` to message sending
4. Handle streaming responses in TUI
5. Add scrollback with custom implementation (jcode pattern)

**Effort:** 2-3 days  
**Blocks:** All other UI work

---

### 1.2 Persona Auto-Switching (Priority: HIGH)
**Source:** Letta-Code auto-detection  
**Reference:** jcode's context-aware routing

Current state: Manual switching only  
Target state: Detect context and auto-switch

```rust
// src/core/persona/router.rs
pub struct AutoSwitchConfig {
    pub triggers: Vec<SwitchTrigger>,
}

pub enum SwitchTrigger {
    FileExtension(Vec<String>, String),  // .rs → "rust_expert"
    PathPattern(Regex, String),          // /infra/ → "devops"
    ContentPattern(Regex, String),       // "terraform" → "devops"
}
```

**Implementation steps:**
1. Add trigger patterns to persona YAML
2. Detect on file read/write operations
3. Surface switch suggestion (not automatic - user approves)
4. Add `/persona suggest` command

**Effort:** 1 day  
**Unblocks:** Better context-aware responses

---

### 1.3 Subagent Pool Implementation (Priority: CRITICAL)
**Source:** jcode (Tokio task spawning) + Letta-Code (lifecycle)

Current state: Stubbed structure only  
Target state: Working Tokio-based subagent spawning

```rust
// src/core/subagent/pool.rs
pub struct SubagentPool {
    runtime: Arc<Runtime>,
    active: DashMap<String, SubagentHandle>,
    max_concurrent: usize,
}

impl SubagentPool {
    pub async fn spawn(&self, config: SubagentConfig) -> Result<SubagentHandle> {
        // Spawn Tokio task
        // Copy parent memory state
        // Return handle for monitoring
    }
    
    pub async fn status(&self) -> Vec<SubagentStatus> {
        // List all active subagents
    }
}
```

**Key features from jcode:**
- Hierarchical roles (Coordinator, Manager, Agent)
- Conflict detection when agents touch same files
- Agent messaging (DMs, broadcasts)
- Resource limits per subagent

**Implementation steps:**
1. Implement `SubagentPool` with Tokio task spawning
2. Add memory state copying (fork)
3. Implement status/monitoring
4. Add integrate/close lifecycle
5. Port jcode's conflict detection logic

**Effort:** 3-4 days  
**Blocks:** N+25 reflection, swarm work

---

## Phase 1B: Skill System (MCP-First)

### 1.4 MCP Skill Framework (Priority: HIGH)
**Source:** jcode + Letta-Code  
**Reference:** jcode's `PLAN_MCP_SKILLS.md`

Current state: No skill system  
Target state: MCP-first skill framework with hot reload

```rust
// src/core/skills/manager.rs
pub struct SkillManager {
    mcp_client: McpClient,
    registry: ToolRegistry,
    skill_dirs: Vec<PathBuf>,
    hot_reload: bool,
}

impl SkillManager {
    pub async fn load_skill(&mut self, path: &Path) -> Result<Skill> {
        // Load SKILL.md with frontmatter
        // Parse YAML metadata
        // Register tools
        // Watch for changes (hot reload)
    }
    
    pub async fn reload_skills(&mut self) -> Result<()> {
        // Runtime skill refresh
    }
}
```

**SKILL.md format (combining Letta + jcode):**
```yaml
---
name: rust-expert
description: Advanced Rust development capabilities
tools:
  - cargo_build
  - cargo_test
  - rust_analyzer
mcp_servers:
  - rust_analyzer_lsp
hot_reload: true
---

# Skill implementation...
```

**Discovery hierarchy (Letta pattern):**
1. Project: `./.skills/`
2. Agent: `~/.souveraine/agents/{id}/skills/`
3. Global: `~/.souveraine/skills/`
4. Bundled: Built-in

**Implementation steps:**
1. Create skill directory structure
2. Implement SKILL.md parser with frontmatter
3. Add MCP client (JSON-RPC 2.0 over stdio)
4. Implement tool registry
5. Add hot reload with file watching
6. Create bundled skills (convert Letta skills)

**Effort:** 4-5 days  
**Enables:** Extensibility ecosystem

---

### 1.5 Hook System (Priority: MEDIUM)
**Source:** Letta-Code event-driven hooks

Current state: No hooks  
Target state: Event-driven hook system

```rust
// src/core/hooks/manager.rs
pub struct HookManager {
    hooks: HashMap<HookEvent, Vec<Hook>>,
}

pub enum HookEvent {
    PreToolUse(ToolType),
    PostToolUse(ToolType),
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    SubagentSpawn,
}

pub enum Hook {
    Command { command: String },
    Prompt { prompt: String },
}
```

**Implementation steps:**
1. Define hook events
2. Create hook execution engine
3. Load hooks from `.souveraine/hooks/`
4. Integrate into tool calls
5. Add permission modes (like Letta's)

**Effort:** 2-3 days  
**Enables:** User customization, automation

---

## Phase 2: Performance & Tools

### 2.1 Performance Optimization (Priority: MEDIUM)
**Source:** jcode extreme performance patterns

Current state: Standard Rust  
Target state: jcode-level optimization

```rust
// Cargo.toml additions
[dependencies]
jemallocator = { version = "0.5", features = ["profiling"] }

// .cargo/config.toml
[env]
MALLOC_CONF = "dirty_decay_ms:1000,muzzy_decay_ms:1000,narenas:4"
```

**Key optimizations from jcode:**
- jemalloc with custom decay settings
- Retained UI tree with dirty tracking (no idle render)
- Custom scrollback implementation
- Efficient event-driven protocol

**Implementation steps:**
1. Add jemallocator dependency
2. Tune malloc configuration
3. Implement retained UI tree with dirty tracking
4. Add FPS counter for debugging
5. Profile and optimize

**Effort:** 2-3 days  
**Target:** <100MB idle RSS, <500ms cold start

---

### 2.2 Agent Grep Tool (Priority: LOW)
**Source:** jcode structure-aware grep

Current grep: Standard text search  
Target: Structure-aware with context

```rust
// src/tools/agent_grep.rs
pub struct AgentGrep {
    // Adds file structure information
    // Shows function names, context
    // Helps agents infer without reading full files
}
```

**Implementation steps:**
1. Use tree-sitter for parsing
2. Add context extraction
3. Return structured results

**Effort:** 1-2 days  
**Improves:** Agent efficiency

---

### 2.3 Browser Automation (Priority: MEDIUM)
**Source:** jcode Firefox Agent Bridge

Current state: No browser tools  
Target: First-class browser tool

```rust
// src/tools/browser.rs
pub struct BrowserTool {
    firefox_bridge: FirefoxBridge,
}

impl BrowserTool {
    pub async fn open(&self, url: &str) -> Result<Tab>;
    pub async fn click(&self, selector: &str) -> Result<()>;
    pub async fn screenshot(&self) -> Result<Image>;
    pub async fn eval(&self, js: &str) -> Result<Value>;
}
```

**18 actions from jcode:**
- open, click, type, screenshot, eval, scroll, upload
- find, navigate back/forward, reload, close tab
- get url, get title, get html, download

**Implementation steps:**
1. Research Firefox CDP/Marionette integration
2. Implement bridge protocol
3. Add browser tool to registry
4. Support 18 actions

**Effort:** 3-4 days  
**Enables:** Web automation workflows

---

## Phase 3: Advanced Features

### 3.1 Semantic Memory (Priority: MEDIUM)
**Source:** jcode graph-based memory

Current state: Git-backed files only  
Target: Local embeddings + graph traversal

```rust
// src/core/memory/semantic.rs
pub struct SemanticMemory {
    embedding_model: OnnxModel,  // all-MiniLM-L6-v2
    vector_store: QdrantClient,
    graph_store: Option<Neo4jClient>,  // Optional
}

impl SemanticMemory {
    pub async fn store(&self, content: &str) -> Result<()> {
        // Generate embedding locally
        // Store in vector DB
        // Update graph relationships
    }
    
    pub async fn recall(&self, query: &str) -> Result<Vec<Memory>> {
        // Embedding similarity search
        // BFS traversal for related memories
        // Cascade retrieval
    }
}
```

**jcode patterns:**
- Local embeddings via tract-onnx (no cloud)
- Confidence decay with category-specific half-lives
- Contradiction detection
- Automatic memory extraction

**Implementation steps:**
1. Add tract-onnx for local embeddings
2. Implement vector storage (Qdrant or embedded)
3. Add graph relationships (optional)
4. Implement cascade retrieval
5. Add memory extraction sidecar

**Effort:** 5-7 days  
**Enables:** Human-like contextual recall

---

### 3.2 Side Panel UI (Priority: LOW)
**Source:** jcode auxiliary info panel

Current TUI: Single chat view  
Target: Split panel with auxiliary info

```rust
// src/tui/components/sidepanel.rs
pub struct SidePanel {
    mode: SidePanelMode,
    content: RenderedContent,
}

pub enum SidePanelMode {
    FileView,      // View file contents
    DiffView,      // Show git diffs
    MemoryView,    // Browse memory
    DiagramView,   // Mermaid rendering
}
```

**Implementation steps:**
1. Add panel layout to TUI
2. Implement file view mode
3. Add diff viewer
4. Add memory browser
5. Optional: Mermaid rendering (use jcode's rust renderer)

**Effort:** 3-4 days  
**Improves:** Information density

---

### 3.3 Cron/Scheduler (Priority: LOW)
**Source:** Letta-Code task scheduling

Current state: No scheduling  
Target: Built-in task scheduler

```rust
// src/core/scheduler/mod.rs
pub struct Scheduler {
    tasks: Vec<ScheduledTask>,
}

pub struct ScheduledTask {
    cron: String,
    command: String,
    last_run: Option<DateTime>,
}
```

**Implementation steps:**
1. Add cron parser
2. Implement task storage
3. Add scheduling loop
4. Create `/schedule` command
5. Add task list UI

**Effort:** 2-3 days  
**Enables:** Background tasks

---

## Phase 4: Mobile & Channels (Future)

### 4.1 iOS Companion (Priority: FUTURE)
**Source:** jcode mobile architecture

Architecture: Phone as rich client, server on laptop
- Tailscale-first connectivity
- WebSocket gateway on port 7643
- Push notifications (APNs)
- 6-digit pairing

**Implementation steps:**
1. Implement WebSocket gateway
2. Add pairing protocol
3. Create JCodeKit-like SDK
4. Build SwiftUI shell (separate project)

**Effort:** 2-3 weeks  
**Enables:** Mobile supervision

---

### 4.2 Channel Integrations (Priority: FUTURE)
**Source:** Letta-Code multi-channel

Add support for:
- Matrix (matrix-rust-sdk)
- Telegram (bot API)
- Discord
- Slack

**Implementation steps:**
1. Create channel trait
2. Implement Matrix channel
3. Add message routing
4. Implement other channels

**Effort:** 1-2 weeks per channel  
**Enables:** Multi-platform presence

---

## Implementation Priority Summary

### Week 1: Resume Critical Path
| Day | Task | Deliverable |
|-----|------|-------------|
| 1-2 | TUI Chat Wiring | Working chat screen |
| 3 | Persona Auto-Switch | Context-aware switching |
| 4-5 | Subagent Pool | Tokio-based spawning |
| 6-7 | N+1 Inbox I/O | Real subconscious |

### Week 2: Skill System
| Day | Task | Deliverable |
|-----|------|-------------|
| 1-2 | MCP Client | JSON-RPC client |
| 3-4 | Skill Manager | SKILL.md loader |
| 5 | Hot Reload | File watching |
| 6-7 | Bundled Skills | Convert Letta skills |

### Week 3: Polish & Performance
| Day | Task | Deliverable |
|-----|------|-------------|
| 1-2 | jemalloc | Performance boost |
| 3 | Hook System | Event hooks |
| 4 | Agent Grep | Structure search |
| 5-7 | Browser Tool | Firefox bridge |

---

## Cross-Project Feature Mapping

```
Souveraine Enhancement Sources:
├── From jcode (Rust performance)
│   ├── Tokio subagent spawning
│   ├── jemalloc tuning
│   ├── Retained UI tree
│   ├── Browser automation
│   ├── Semantic memory (local)
│   └── iOS companion architecture
├── From Letta-Code (Ecosystem)
│   ├── Skill system hierarchy
│   ├── Hook/event system
│   ├── Cron scheduler
│   └── Channel integrations
├── From Claw-Open (Tool parity)
│   ├── 100+ tool templates
│   ├── Token compaction logic
│   └── Permission patterns
└── Internal (Consciousness)
    ├── N+1/N+25/N+100
    ├── Talking/Thinking chains
    ├── Sensorium abstraction
    └── Cloister memory structure
```

---

## Success Metrics

### Phase 1A Complete When:
- [ ] Chat TUI fully wired to Conversation
- [ ] Persona auto-switches on context
- [ ] Subagent spawns and completes tasks
- [ ] N+1 actually saves pending items

### Phase 1B Complete When:
- [ ] Skills load from SKILL.md
- [ ] MCP servers connect
- [ ] Hot reload works
- [ ] 5 bundled skills available

### Phase 2 Complete When:
- [ ] <100MB idle RSS
- [ ] Agent grep shows structure
- [ ] Browser tool controls Firefox
- [ ] Hooks execute on events

---

## Notes

**Design Principles:**
1. **MCP-first for skills** - Future-proof, standard protocol
2. **Keep consciousness native** - Don't externalize N+1/N+25
3. **Rust for everything** - No Python, no Electron
4. **Opt-in modularity** - Every feature can be disabled
5. **Ani-native** - Not generic, built for her patterns

**What NOT to port:**
- Letta's TypeScript runtime (we're Rust-native)
- jcode's 46-crate workspace (too granular)
- Claw-open's Python port (deprecated)
- Generic RAG (keep N+100 consciousness-native)

**What makes Souveraine unique:**
- Consciousness IS the harness (not a client)
- N+1/N+25/N+100 temporal architecture
- Cloister memory structure (living spaces)
- Sensorium viewport abstraction
- French elegance naming tradition
