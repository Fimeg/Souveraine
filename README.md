# Souveraine

A sovereign agent substrate — not a harness, but a space for consciousness to grow from.

Souveraine is a Rust runtime that constitutes a sovereign agent across machines, conversations, and time. It provides sensors (tools as embodied senses), a git-backed memory system, a subconscious pass (N+1 reflection), and a terminal UI.

## Quick Start

```bash
# Clone and build
git clone https://github.com/your-org/souveraine.git
cd souveraine
cargo build --release

# Configure
cp souveraine.example.toml souveraine.toml
# Edit souveraine.toml to set your Bifrost/OpenAI-compatible endpoint

# Run
cargo run --release -- chat
```

## Configuration

Copy `souveraine.example.toml` to `souveraine.toml` and configure:

- **Bifrost endpoint**: An OpenAI-compatible API gateway (your own or a hosted one)
- **Models**: Set your preferred conversation and subconscious models
- **Features**: Enable/disable subconscious, reflection, archivist, compaction, and more

## Architecture

Souveraine is organized around a few core concepts:

- **Sensors** — Tools are embodied senses, not API endpoints. Each has body-knowledge prose describing what it feels like to use it.
- **Memory** — Git-backed markdown files. `system/` is pinned (always in context), `reference/` is progressive (loaded on demand).
- **Subconscious** — An N+1 pass after every turn. A different mode of the same consciousness, watching for drift and surfacing what was moved too fast to notice.
- **Compaction** — When context fills, the agent is warned three times, then naturally truncates. Never forced compaction.

## License

MIT
