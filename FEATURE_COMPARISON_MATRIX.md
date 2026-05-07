# Feature Comparison Matrix: Souveraine vs Letta-Code vs jcode vs Claw-Open

> Analysis Date: 2026-05-06  
> Purpose: Identify features to merge into Souveraine as the definitive harness

---

## Executive Summary

| Project | Language | Status | Primary Differentiator |
|---------|----------|--------|----------------------|
| **Souveraine** | Rust | 🔄 Phase 1 (Paused) | Consciousness-native architecture (N+1/N+25/N+100) |
| **Letta-Code** | TypeScript/Bun | ✅ Production | Persistent memory-first with skills ecosystem |
| **jcode** | Rust | 🔄 Active Dev | Extreme performance (245x faster than Claude Code) |
| **Claw-Open** | Python/Rust | 🔄 Porting | Clean-room Claude Code rewrite with tool parity |
| **Pi-Conscious** | TypeScript | 🗑️ Archived | Extension framework for Pi (concepts absorbed) |

---

## Detailed Feature Matrix

### 1. Core Architecture

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Language** | Rust | TypeScript/Bun | Rust | Python + Rust |
| **Async Runtime** | Tokio | Bun | Tokio (jemalloc) | Tokio |
| **Architecture** | Consciousness-core | Client-Server | Agent-daemon | CLI-focused |
| **Memory Model** | Git-based Cloister | Git-backed MemFS | Graph-based semantic | JSON session |
| **Config Format** | TOML | JSON | TOML | TOML |
| **Modular Design** | ✅ | ✅ | ✅ (46 crates) | ⚠️ |

### 2. Memory & Persistence

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Git Integration** | ✅ (git2) | ✅ (sync) | ✅ | ❌ |
| **Token Counting** | ✅ (tiktoken) | ✅ | ✅ | ✅ |
| **Context Compaction** | ✅ (Archivist N+100) | ✅ | ✅ | ✅ |
| **Semantic Search** | ❌ | ⚠️ | ✅ (Local embeddings) | ❌ |
| **Graph Memory** | ❌ | ❌ | ✅ (Cascade retrieval) | ❌ |
| **Cross-Device Sync** | ⚠️ (via git) | ✅ (Letta Cloud) | ❌ | ❌ |

### 3. Consciousness Features

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **N+1 Subconscious** | ✅ (Working) | ⚠️ (Reflection subagent) | ⚠️ (Ambient mode) | ❌ |
| **N+25 Reflection** | ⏸️ (Stubbed) | ⚠️ | ❌ | ❌ |
| **N+100 Archivist** | ✅ (Working) | ⚠️ | ⚠️ | ❌ |
| **Talking/Thinking Chains** | ⏸️ (Stubbed) | ❌ | ❌ | ❌ |
| **Persona Router** | ✅ (4 personas) | ✅ | ✅ | ❌ |
| **Auto Persona Switch** | ⏸️ (Stubbed) | ⚠️ | ❌ | ❌ |

### 4. Multi-Agent & Subagents

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Subagent Spawning** | ⏸️ (Stubbed) | ✅ (Built-in types) | ✅ (Swarm coord) | ❌ |
| **Parallel Execution** | ⏸️ (Tokio tasks) | ✅ | ✅ | ❌ |
| **Fork/Resume** | ❌ | ✅ | ✅ | ✅ |
| **Conflict Detection** | ❌ | ⚠️ | ✅ | ❌ |
| **Agent Messaging** | ❌ | ✅ | ✅ | ❌ |
| **Hierarchical Roles** | ❌ | ⚠️ | ✅ | ❌ |

### 5. Skill System

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Skill Framework** | ❌ | ✅ (SKILL.md) | ✅ (Hot-reload) | ❌ |
| **MCP Support** | ❌ | ⚠️ | ⚠️ | ❌ |
| **4-Tier Discovery** | ❌ | ✅ | ✅ | ❌ |
| **Hot Reload** | ❌ | ❌ | ✅ | ❌ |
| **Bundled Skills** | ❌ | ✅ | ⚠️ | ❌ |
| **Self-Development** | ❌ | ❌ | ✅ | ❌ |

### 6. UI/UX

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **TUI Framework** | ratatui | React/Ink | ratatui | ratatui |
| **Chat Screen** | ⏸️ (Stubbed) | ✅ | ✅ | ✅ |
| **Splash/Animations** | ✅ | ✅ | ✅ | ⚠️ |
| **Side Panel** | ❌ | ❌ | ✅ | ❌ |
| **Custom Scrollback** | ⚠️ | ⚠️ | ✅ (1000+ FPS) | ⚠️ |
| **Mobile App** | ❌ | ✅ | ✅ (iOS) | ❌ |
| **Desktop App** | ❌ | ✅ | ⚠️ | ❌ |

### 7. Integrations

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Multi-Provider** | ✅ (Bifrost) | ✅ (BYOK) | ✅ | ⚠️ (Anthropic) |
| **Slack** | ❌ | ✅ | ❌ | ❌ |
| **Discord** | ❌ | ✅ | ❌ | ❌ |
| **Telegram** | ❌ | ✅ | ❌ | ❌ |
| **Matrix** | ❌ | ✅ | ❌ | ⚠️ |
| **Browser Control** | ❌ | ❌ | ✅ (Firefox) | ❌ |
| **LSP Support** | ❌ | ✅ | ⚠️ | ❌ |

### 8. Event System & Hooks

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Hook System** | ❌ | ✅ (Event-driven) | ❌ | ❌ |
| **Pre/Post Tool** | ❌ | ✅ | ❌ | ❌ |
| **Permission Hooks** | ❌ | ✅ | ⚠️ | ✅ |
| **Session Events** | ⚠️ | ✅ | ✅ | ⚠️ |
| **Cron/Scheduling** | ❌ | ✅ | ❌ | ❌ |

### 9. Performance & Telemetry

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Cold Start** | N/A | ~3.4s | ~48ms | N/A |
| **Memory Footprint** | N/A | ~386 MB | ~28 MB | N/A |
| **Per-Session Cost** | N/A | ~100 MB | ~10 MB | N/A |
| **Telemetry** | ❌ | ❌ | ✅ (Opt-out) | ❌ |
| **Transparent Metrics** | ✅ | ✅ | ✅ | ⚠️ |

### 10. Tool System

| Feature | Souveraine | Letta-Code | jcode | Claw-Open |
|---------|:----------:|:----------:|:-----:|:---------:|
| **Tool Count** | 5 (basic) | 40+ | 30+ | 100+ |
| **Parallel Execution** | ❌ | ✅ | ✅ | ❌ |
| **Model-Specific Sets** | ❌ | ✅ | ⚠️ | ❌ |
| **Custom Tools** | ❌ | ✅ | ✅ | ✅ |
| **Agent Grep** | ❌ | ❌ | ✅ | ❌ |

---

## Unique Strengths by Project

### Souveraine (Base)
- ✅ **Consciousness-native architecture** - N+1/N+25/N+100 pattern is unique
- ✅ **Modular TOML config** - Everything opt-in
- ✅ **Sensorium abstraction** - Interface decoupling
- ✅ **French elegance naming** - Coquette tradition

### Letta-Code
- ✅ **Mature skill ecosystem** - 4-tier discovery, declarative skills
- ✅ **Production-ready** - Desktop, mobile, multi-channel
- ✅ **Memory-first identity** - Persistent agents across sessions
- ✅ **Hook system** - Event-driven automation

### jcode
- ✅ **Extreme performance** - 245x faster than Claude Code
- ✅ **Human-like memory** - Automatic contextual recall
- ✅ **Swarm coordination** - True multi-agent with conflict detection
- ✅ **Self-development mode** - Can modify own source
- ✅ **Browser automation** - First-class Firefox bridge

### Claw-Open
- ✅ **Tool parity** - 100+ tools matching Claude Code
- ✅ **Clean-room rewrite** - Ethical reimplementation
- ✅ **Token compaction** - Sophisticated context management
- ✅ **Compat-harness** - TypeScript analysis for parity

---

## Enhancement Priority for Souveraine

### 🔴 Critical (Blocking Full Use)

| Priority | Feature | Source | Effort |
|----------|---------|--------|--------|
| 1 | Wire TUI chat to Conversation | Internal | Medium |
| 2 | Implement subagent spawning | jcode/Letta | Medium |
| 3 | Complete N+1 with inbox I/O | Internal | Medium |
| 4 | Skill system (MCP-first) | jcode + Letta | Large |

### 🟠 High Impact

| Priority | Feature | Source | Effort |
|----------|---------|--------|--------|
| 5 | Hook/event system | Letta | Medium |
| 6 | Hot-reload skills | jcode | Medium |
| 7 | Persona auto-switching | Internal | Small |
| 8 | Browser automation | jcode | Large |

### 🟡 Medium Priority

| Priority | Feature | Source | Effort |
|----------|---------|--------|--------|
| 9 | Local embeddings | jcode | Medium |
| 10 | Agent grep tool | jcode | Small |
| 11 | Side panel UI | jcode | Medium |
| 12 | Cron/scheduler | Letta | Medium |

### 🟢 Future/Nice-to-Have

| Priority | Feature | Source | Effort |
|----------|---------|--------|--------|
| 13 | Channel integrations | Letta | Large |
| 14 | Mobile companion | jcode | Large |
| 15 | Swarm coordination | jcode | Large |
| 16 | Self-development mode | jcode | Large |

---

## Recommended Architecture for Enhanced Souveraine

```
souveraine/
├── src/
│   ├── core/
│   │   ├── consciousness/       # N+1/N+25/N+100 (existing)
│   │   ├── chains/              # Talking/Thinking (complete stub)
│   │   ├── skills/              # NEW: MCP-first skill system
│   │   ├── subagents/           # NEW: Tokio-based spawning
│   │   └── hooks/               # NEW: Event system
│   ├── bridge/
│   │   ├── bifrost.rs           # Existing
│   │   ├── mcp.rs               # NEW: MCP client
│   │   └── embeddings.rs        # NEW: Local embeddings
│   ├── ui/
│   │   ├── chat.rs              # NEW: Wire to conversation
│   │   ├── sidepanel.rs         # NEW: Auxiliary info panel
│   │   └── components/          # Enhanced widgets
│   └── tools/
│       ├── agent_grep.rs        # NEW: Structure-aware grep
│       └── browser.rs             # NEW: Firefox bridge
├── skills/                      # NEW: Skill directory
├── docs/
└── Cargo.toml
```

---

## Conclusion

**Souveraine** has the strongest **conceptual foundation** (consciousness-native) but needs:
1. **jcode's** performance patterns and subagent architecture
2. **Letta-code's** skill ecosystem and hook system
3. **Claw-open's** comprehensive tool parity

The path forward is completing Phase 1 foundation, then layering in skills (MCP-first), subagents, and hooks while maintaining the unique consciousness architecture.
