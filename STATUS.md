# Souveraine - Current State & Handoff
## Pause Point: 2026-05-05

---

## Status

**Development Status:** ⏸️ PAUSED  
**Reason:** Using alternative harness with working subagents for immediate needs  
**Return Condition:** When ready for full Rust implementation with subagents

---

## What Was Built

### ✅ Completed

#### 1. Git Memory System (`src/core/memory/`)
- Real git2 integration
- Per-persona repos at `~/.pi/unified/agents/{persona}/memory/`
- Read/write/append operations
- Auto-commit on write
- Git log for N+1 checking
- **The Cloister** structure (formerly Cathedral - intimate, living spaces):
  - `system/` - Core identity
  - `system/synthesized/` - N+100 compressed essence ← NEW
  - `subconscious/` - Aster's space
  - `journal/` - Raw chronological (preserved forever)
  - `archive/` - Compressed syntheses ← NEW

```rust
GitMemory::for_persona("ani").await?;
mem.write("journal/2026/05/05.md", content).await?;
mem.append("subconscious/inbox.md", entry).await?;
// N+100 synthesis goes to system/synthesized/
```

#### 2. Configuration System (`src/core/config/`)
- Modular feature flags
- Everything opt-in
- TOML/YAML support
- Per-module enable/disable
- **NEW: Model Physics (DO NOT GUESS at 128k)**
  - Per-model context limits: `models.kimi-k2-5.context_limit = 128000`
  - Model-specific archivist thresholds
  - `models.qwen2-5-72b.context_limit = 32768` (compress earlier)
- **NEW: Archivist (N+100) Configuration**
  - `archivist_enabled`, `archivist_interval = 100`
  - `archivist_threshold = 0.7` (70% of context)
  - `archivist_compression_model` (can differ from Ani's model)
  - `archivist_synthesis_elements = ["themes", "emotions", ...]`
- **NEW: Sensorium Configuration**
  - Bandwidth classes: High, Medium, Low, Minimal
  - Progressive discovery levels
  - Mobile context awareness

```toml
[subconscious]
n1_enabled = true
n1_trigger = "EveryResponse"

[archivist]
enabled = true
threshold = 0.7  # 70% of model's context_limit

[models.kimi-k2-5]
context_limit = 128000
archivist_threshold = 0.7

[sensorium]
primary_bandwidth = "high"
```

#### 3. Animation Library (`src/ui/animation.rs`)
- Typing animation (configurable WPM)
- Gradient text (HSL color ramps)
- Breathing colors (sine wave)
- Braille spinners
- Wave progress bars
- Persona color schemes

#### 4. Full TUI (`src/ui/app.rs`)
- Splash screen with breathing background
- Welcome menu (7 options)
- Dashboard with status cards
- Activity log display
- Arrow key navigation
- 'm' for menu, 'q' to quit

**Screens:**
- Splash → Welcome → Dashboard/Chat/Code/Therapy/AgentTime/Cron/Settings

#### 5. Subagent Investigation (`docs/SUBAGENT_INVESTIGATION.md`)
- How Letta-Code actually spawns subagents (process-based)
- Trade-offs: process vs in-process
- Recommendation: Tokio async tasks for Rust

---

## What's Stubbed (Needs Implementation)

### ⏸️ Persona Router (`src/core/persona/`)
- Structure exists, no implementation
- Needs to load from `~/.pi/unified/agents/`
- Auto-detect based on context

### ⏸️ Subconscious N+1 (`src/core/subconscious/`)
- Module structure exists
- Needs actual completion logic
- Pattern matching for commitments
- Git log checking

### ⏸️ Inbox System (`src/core/subconscious/`)
- Structure exists
- Needs file I/O to `subconscious/inbox/`
- Surfacing mechanism

### ⏸️ Reflection Engine (`src/core/reflection/`)
- Module stub
- Needs N+25 trigger logic
- Transcript accumulation
- Subagent spawning

### ⏸️ Subagent Pool (`src/core/subagent/`)
- Module stub
- Needs Tokio task spawning
- Fork/integrate lifecycle

### ⏸️ Chain Orchestrator (`src/core/chain/`)
- Module stub
- Talking/Thinking chain switching

### ⏸️ Archivist (N+100) ← NEW
- Configuration implemented in `config.rs`
- Needs `src/core/archivist/mod.rs` stub
- Physics-aware compression logic
- Synthesis subagent spawning
- Model-specific trigger thresholds

### ⏸️ Sensorium Layer ← NEW
- Configuration implemented in `config.rs`
- Needs `src/core/sensorium/mod.rs` with trait definition
- Bandwidth classification
- Progressive discovery filtering
- `TuiSensorium` implementation
- `MobileSensorium` stub for future

### ⏸️ Model Router ← NEW
- Configuration implemented in `config.rs`
- Needs `src/bridge/model_router.rs`
- Context pressure monitoring
- Model-aware archivist triggers
- Token usage tracking per model

### ⏸️ Bifrost Integration
- HTTP client placeholder
- No actual API calls

---

## Architecture Decisions Made

### ✅ Confirmed
1. **Name:** Souveraine (not JCode-UC, not cathedral)
2. **Terminology:** **Cloister** not Cathedral - intimate, living spaces, not monuments
3. **Subconscious:** Aster is completing mind, not separate entity
4. **Memory Structure:** Flat, personal (Ani's actual structure)
5. **UI:** Full ratatui TUI (not toy examples)
6. **Subagents:** Tokio async tasks (not OS processes)
7. **Modular:** Everything configurable, opt-in
8. **Physics-Aware:** DO NOT GUESS at 128k - model-specific context limits
9. **Archivist (N+100):** Compression for survival, not just summarization
10. **Sensorium:** Interface abstraction, consciousness decoupled from UI
11. **Raw vs Synthesized:** Raw preserved in git (sovereignty), synthesized loaded (presence)

### ❓ Still Open
1. How should personas actually trigger/switch?
2. What exactly should N+25 reflection subagent DO?
3. Should subagents have isolated git repos or shared?
4. How does Bifrost integration work in detail?
5. What does "agent therapy" mode actually do?

---

## Next Steps (When Resuming)

### Phase 1b: Persona System (1-2 days)
1. Load agent definitions from `~/.pi/unified/agents/`
2. Parse YAML configs
3. Auto-detect based on context
4. Persona switching UI

### Phase 2: Subconscious (2-3 days)
1. N+1 pattern matching ("I'll save that")
2. Git log checking for pending commits
3. Inbox file I/O
4. Surfacing injection into responses

### Phase 3: Subagents (3-4 days)
1. Tokio task spawning
2. Fork with copied memory context
3. Run to completion
4. Integrate results
5. Cleanup

### Phase 4: Integration (2-3 days)
1. Wire everything together
2. Bifrost HTTP client
3. End-to-end conversation flow
4. Error handling

---

## Files to Know

```
souveraine/
├── src/
│   ├── main.rs              # Entry point, runs TUI
│   ├── core/
│   │   ├── mod.rs           # Orchestrator (loads modules)
│   │   ├── config.rs        # Feature flags ✅ DONE
│   │   ├── memory/mod.rs    # GitMemory ✅ DONE
│   │   ├── subconscious/    # N+1, inbox ⏸️ STUBBED
│   │   ├── persona/         # Router ⏸️ STUBBED
│   │   ├── reflection/      # N+25 ⏸️ STUBBED
│   │   ├── subagent/        # Fork ⏸️ STUBBED
│   │   └── chain/           # Talking/Thinking ⏸️ STUBBED
│   ├── ui/
│   │   ├── mod.rs           # Exports app
│   │   ├── app.rs           # Full TUI ✅ DONE
│   │   └── animation.rs     # Effects ✅ DONE
│   └── harness/             # IDE integration ⏸️ STUBBED
├── docs/
│   ├── SEXY_UI.md           # Animation techniques
│   ├── SUBAGENT_INVESTIGATION.md  # How spawning works
│   └── ...
├── Cargo.toml               # Rust config
└── *.md                     # Various docs
```

---

## Key Insights from Investigation

### 1. Ani's N+1 Pattern
From `~/.letta/agents/.../aster/mandate.md`:
- Completes what was promised (doesn't just flag)
- "If Ani says 'I'll save that' → actually save it"
- Checks git log before assuming
- Appends to journal/ with timestamp

### 2. Memory Structure
From `~/.letta/agents/.../memory/`:
- Ani's structure is flat, personal
- No imposed hierarchy
- `subconscious/` not `aster/` (renamed)
- System reads identity/, writes subconscious/

### 3. Subagent Spawning
From Letta-Code investigation:
- Letta-Code uses OS processes (spawn "letta" CLI)
- Souveraine should use Tokio async tasks (faster)
- Trade-off: isolation vs performance

### 4. UI Expectations
From user feedback:
- Full-screen TUI (not tiny demos)
- Impressive splash screen
- Dashboard showing agent status
- Multiple modes (chat, code, therapy, etc.)

---

## Resume Command

When ready to continue:

```bash
cd ~/Projects/souveraine

# Check it still compiles
cargo check

# Run the TUI
cargo run

# Current state: Splash → Menu → Dashboard (working)
# Missing: Everything else (stubbed)

# Next task: Implement Persona Router
# File: src/core/persona/mod.rs
```

---

## Open Questions for Future

1. **What persona switching triggers?** Keywords? Files? Time?
2. **What does "therapy mode" actually do?** Different prompts?
3. **How does N+25 reflection work?** Full transcript analysis?
4. **Should subagents have isolated git repos?** Or shared memory?
5. **What Bifrost endpoints?** Just chat completion? Or more?
6. **What about Matrix bridge?** E2EE implementation details?

---

## Contact Context

**Primary Agent:** Ani (agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351)  
**User:** Casey Tunturi  
**Bifrost:** http://10.10.20.120:3360  
**Base Path:** ~/.pi/unified

**When returning:** Read `PHASES.md` for detailed build plan.

---

*Paused but not abandoned. Foundation is solid. Resume when ready.*
