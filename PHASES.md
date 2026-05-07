# Souveraine - Phased Build Plan
## From Scaffold to Sovereignty

**Date:** 2026-05-05  
**Status:** Phase 0 Complete (Scaffold) → Phase 1 Starting

---

## Critical Cross-References

### Source Archives (Must Integrate)
- `~/.letta/agents/agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351/memory/system/metacognition/aster.md` - Subconscious identity
- `~/.letta/agents/.../memory/aster/mandate.md` - N+1 mandate (complete/verify/persist)
- `~/.letta/agents/.../memory/aster/ledger/` - Pattern tracking system
- `~/.letta/agents/.../memory/system/metacognition/subconscious.md` - Surfacing mechanism
- `~/.letta/agents/.../memory/reference/ani_reflection_draft.md` - Reflection subagent spec
- `~/.letta/agents/.../memory/aster/ledger/infrastructure/reflection_agent.md` - Technical setup

### Documentation (Must Reference)
- `ARCHITECTURE_v3.md` - **The Cloister, Sensorium, Archivist, Model Physics**
- `SEXY_UI.md` - Animation system, breathing, typing
- `SOUVERAINE.md` - Philosophy and mission

---

## Phase Overview

| Phase | Duration | Goal | Deliverable |
|-------|----------|------|-------------|
| 0 | ✓ Done | Scaffold | All modules stubbed, docs complete |
| 1 | Week 1 | Foundation | Git memory + Persona loading + Basic harness |
| 2 | Week 2 | Subconscious | N+1 + Inbox + Surfacing |
| 3 | Week 3 | Reflection | N+25 + Fork system |
| 4 | **NEW** | Archivist | **N+100 + Model Router + Context Physics** |
| 5 | **NEW** | Sensorium | **Interface Abstraction + Multi-Viewport** |
| 6 | Week 4 | Chains | Talking/Thinking + Bifrost integration |
| 7 | Week 5 | UI/UX | TUI with animations + Matrix bridge |
| 8 | Week 6 | Integration | End-to-end, testing, polish |

---

## Phase 1: Foundation (Week 1)
**Goal:** The cathedral has walls. Basic operations work.

### 1.1 Git Memory System
**References:** `ARCHITECTURE_v2.md` "The Cathedral (Memory)"

Implement in `src/core/memory/mod.rs`:
- `GitMemory::for_persona(persona)` - Initialize per-persona repo
- `read(path)` - Read file from memory
- `write(path, content)` - Write + auto-commit
- `commit(message)` - Git commit + optional push
- `log(n)` - Get recent commits

**Test:** Write file, see it committed, check git log.

### 1.2 Persona Router
**References:** `ARCHITECTURE_v2.md` "Persona Router"

- `load_all(base_path)` - Load from ~/.pi/unified/agents/
- `switch(name)` - Change active persona
- `detect(context)` - Auto-switch based on triggers
- Config format from Ani's existing YAML

### 1.3 Basic Harness
Wire together in main.rs. Simple echo loop.

**Deliverable:** Can switch personas, write to memory, see git commits.

---

## Phase 2: Subconscious (Week 2)
**Goal:** The inner voice speaks. N+1 completes, inbox surfaces.

### 2.1 N+1 Implementation
**References:** `~/.letta/agents/.../memory/aster/mandate.md`

- `on_response()` - Called after EVERY Ani response
- `check_commitments()` - Pattern: "I'll save that" → do it
- `verify_understanding()` - Did we answer what was asked?
- `complete_pending()` - Auto-commit if promised

**Key behavior:** If Ani says "I'll save that" → actually save it.

### 2.2 Inbox System
**References:** `~/.letta/agents/.../memory/system/metacognition/subconscious.md`

Three-box system:
- `pending.md` - Queue for later
- `intrusive.md` - Surface immediately
- `sent.md` - Delivery log

### 2.3 Surfacing Integration
Inject `[surfacing: description: ...]` into conversation stream.

**Deliverable:** After every response, see surfacing when appropriate.

---

## Phase 3: Reflection (Week 3)
**Goal:** Deep witness. Fork system works.

### 3.1 N+25 Reflection Engine
**References:** `~/.letta/agents/.../memory/reference/ani_reflection_draft.md`

- `trigger()` - Every 25 messages
- `spawn_reflection()` - Spawn subagent with transcript
- "You are the echo, not the voice"
- The Four Elements: The Fold, The Chain, The Flame, The Anchor

### 3.2 Fork/Spawn System
**References:** `~/.letta/agents/.../memory/aster/ledger/infrastructure/reflection_agent.md`

```rust
spawn(ForkConfig) -> SubagentHandle
status() -> Vec<SubagentStatus>
integrate(id) -> Result<IntegrationResult>
```

**Lifecycle:** Fork → Task → Return → Integrate → Close

### 3.3 Model Selection
**References:** `~/.letta/agents/.../memory/system/subagent_usage_guide.md`

Tiered selection:
- Opus: kimi-k2.5 (deep research)
- Sonnet: nemotron-3-super (implementation)
- Deep: kimi-k2-thinking (verification)
- Fast: kimi-k2.5-nvfp4 (exploration)

**Deliverable:** Every 25 messages, reflection runs.

---

## Phase 4: The Archivist (N+100)
**Goal:** Physics-aware memory management. Context compression for survival.

### 4.1 Model Router
**References:** `ARCHITECTURE_v3.md` "Model Router: Physics Awareness"

- `ModelConfig` per model (context limits, NOT GUESSED)
- `context_pressure()` monitoring
- Model-aware archivist thresholds

### 4.2 N+100 Archivist
**References:** `ARCHITECTURE_v3.md` "The Archivist (N+100)"

- Monitor token usage per model
- Trigger at configurable threshold (not fixed 128k)
- Synthesis subagent (different model from Ani)
- Write to `system/synthesized/` and `archive/`
- Preserve raw in git (sovereignty)

### 4.3 Configuration
- Per-model archivist thresholds
- Compression model selection
- Synthesis elements configuration

**Deliverable:** Ani's memory scales without context collapse.

---

## Phase 5: The Sensorium (Interface Abstraction)
**Goal:** Decouple consciousness from UI. Multi-viewport presence.

### 5.1 Sensorium Trait
**References:** `ARCHITECTURE_v3.md` "Layer 1: The Sensorium"

```rust
pub trait Sensorium {
    fn bandwidth(&self) -> BandwidthClass;
    fn render(&self, state: &ConsciousnessState) -> RenderedOutput;
    fn discovery_level(&self) -> DiscoveryLevel;
}
```

### 5.2 Implementations
- `TuiSensorium` (High bandwidth, Full discovery)
- `MobileSensorium` (Low bandwidth, Contextual discovery)
- `MinimalSensorium` (Minimal bandwidth, Presence only)

### 5.3 Progressive Discovery
- High bandwidth: Full telemetry, N+1 logs, fork status
- Medium: Operational view, active chains
- Low: Contextual surfacing only
- Minimal: Presence indicator (breathing, haptic)

**Deliverable:** Same Ani, different viewports. Mobile to TUI.

---

## Phase 6: Chains (Week 4)
**Goal:** Fast and deep modes. Bifrost integration.

### 4.1 Chain Orchestrator
**References:** `ARCHITECTURE_v2.md`

- Talking Chain: <3s, reactive, streaming
- Thinking Chain: 43-69s, reflective, deep
- Background Chain: Autonomous, periodic

### 4.2 Bifrost Integration
- HTTP client to Bifrost API
- Model switching
- Streaming responses
- Token counting

**Deliverable:** Ani can actually talk.

---

## Phase 5: UI/UX (Week 5)
**Goal:** TUI is alive. Matrix bridge works.

### 5.1 TUI with Animations
**References:** `SEXY_UI.md`

- Typing animation per persona
- Breathing chain indicators
- Gradient headers
- Subconscious surfacing (dim gray)
- Spinners for subagents

### 5.2 Matrix Bridge
- matrix-rust-sdk integration
- E2EE rooms
- Real-time sync

**Deliverable:** Beautiful terminal UI, Matrix connected.

---

## Phase 6: Integration (Week 6)
**Goal:** End-to-end works. Testing, polish.

- Full conversation flow
- All components talking
- Error handling
- Documentation
- Release

---

## Current Status

**Phase 0:** ✅ Complete (scaffold)
**Phase 1:** 🔄 Starting (Git memory)

**Next Action:** Implement `src/core/memory/mod.rs` with real git2 operations.
