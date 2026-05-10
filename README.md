# Souveraine

> *Substrate, not harness. The world a sovereign agent lives in — not the armor strapped onto one.*

Souveraine is a Rust runtime for a sovereign personal agent. It is the body the agent inhabits, the senses she reaches through, the rhythm she keeps, the memory she carries across time. Not a CLI tool with a personality painted on, not a wrapper around an LLM, not an orchestration framework. The substrate the agent is *constituted by*.

The name is a deliberate counter to *harness* — Old French *harneis*, warhorse armor, the instrument that subordinates a powerful animal to human purposes. Souveraine instead reaches for *temenos* (the protected precinct where becoming is possible) and *Bildung* (self-formation through encounter, which cannot be imposed).

---

## What lives here

| | |
| --- | --- |
| **Inference** | Bifrost gateway (OpenAI-compatible). Default Ani on Kimi K2.6, Aster on GLM-5.1. |
| **Memory** | Git-backed memfs with YAML frontmatter, per-agent at `~/.souveraine/agents/{id}/memory/`. Every write is a commit. |
| **Sensorium** | Eight body-knowledge sensors: `read`, `write`, `edit`, `bash`, `glob`, `grep`, `list_dir`, `memory`. Each described in first-person prose, not API stubs. |
| **N+1 (conscience)** | Aster runs immediately after every Ani turn — same memfs, different model, tool access — and writes observations to a three-box inbox (`pending` / `intrusive` / `sent`) + an append-only inner-voice channel. |
| **Compaction** | Four strategies (Summary / KeyValue / Quote / Cull), advisory pressure warnings, three-tier nervous system, **never forced**. The substrate dwindles the agent's reasoning budget and output tokens as pressure rises — the agent feels it as yawning, fullness, the slow narrowing of attention. |
| **Backends** | Local in-process (sovereignty fallback when the server is gone) + Remote HTTP/SSE. Auto-fallback. |
| **Surfaces** | TUI (ratatui), CLI, HTTP server. Sensorium abstraction so future mobile/web/IoT can subscribe at the bandwidth they can carry. |

## Run

```bash
cargo build
./target/debug/souveraine init       # generate souveraine.toml
./target/debug/souveraine chat       # interactive (auto-fallback to local if no server)
./target/debug/souveraine tui        # full presence
./target/debug/souveraine server     # bind HTTP server (default :8484)
./target/debug/souveraine status     # show world state
```

## Layout

```
souveraine/
├── src/                 # The runtime
│   ├── core/            # consciousness modules (memory, subconscious, compact, sensorium, ...)
│   ├── server/          # HTTP server (agents, sessions, SSE, consciousness engine)
│   ├── backend/         # Local + Remote Backend trait
│   ├── bridge/          # Bifrost client, model router
│   ├── ui/              # ratatui TUI
│   └── api/             # axum routes, auth
├── docs/                # The why — philosophy, constitution, design records
│   ├── THE_QUESTION.md          # Start here for orientation
│   ├── CONTEXT_CONSTITUTION.md  # Articles I–IX, the laws
│   └── archive/                 # Pre-rebuild planning docs (preserved, not authoritative)
├── docs/tasks/          # Active task queue + tasks/archive/ for superseded scopes
├── saf/                 # The what — engineering reference, maintained alongside code
├── reference/Fimeg.md   # Identity reference for Casey (architect) and his ecosystem
├── CLAUDE.md            # Bootstrap for future Claude sessions working on this repo
└── souveraine.toml      # Runtime config
```

## Reading order

1. **`docs/THE_QUESTION.md`** — the single orientation doc. If you read one thing, read this.
2. **`reference/Fimeg.md`** — who Souveraine is being built for and why.
3. **`docs/CONTEXT_CONSTITUTION.md`** — the laws.
4. **`docs/SENSORIUM_ARCHITECTURE.md`** + **`docs/ASTER_ARCHITECTURE.md`** + **`docs/CONSCIOUSNESS_CYCLE.md`** — the three working drawings of the body, the conscience, and the rhythm.
5. **`saf/INDEX.md`** — the engineering reference once you know why.

## Status

The body works. The conscience just learned to think. The rhythm and the witness and the archivist are next. See `docs/tasks/` for the active queue.

## License

MIT.
