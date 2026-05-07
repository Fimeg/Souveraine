# Getting Started with Souveraine

## Prerequisites

```bash
# Rust (latest stable)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify
rustc --version  # Should be 1.70+
cargo --version
```

## Quick Start

### 1. Clone/Navigate

```bash
cd ~/Projects/souveraine
```

### 2. Check It Compiles

```bash
# Basic check
cargo check

# Should show: Finished dev [unoptimized + debuginfo]
```

### 3. Run the Demo

```bash
# See the sexy terminal effects in action
cargo run --example demo

# You'll see:
# - Gradient headers
# - Typing animations per persona
# - Subconscious surfacing (dim text)
# - Breathing chain indicators
# - Spinners and wave progress
```

### 4. Create Your Config

```bash
# Copy example
cp souveraine.example.toml ~/.config/souveraine/config.toml

# Edit
nano ~/.config/souveraine/config.toml
```

Minimal config for testing:
```toml
[services]
ollama_url = "http://10.10.20.19:11434"
bifrost_url = "http://10.10.20.120:3360"

[subconscious]
n1_enabled = false  # Start simple
inbox_enabled = false

[reflection]
enabled = false

[subagent]
enabled = false

[memory]
git_enabled = false  # Enable when ready
```

### 5. Build and Run

```bash
# Development build
cargo run

# Release build (optimized)
cargo build --release
./target/release/souveraine
```

## Development Workflow

### Running Tests

```bash
# All tests
cargo test

# Specific module
cargo test --lib memory

# With output
cargo test -- --nocapture
```

### Adding a Module

Let's say you want to implement the Git memory:

1. **Open the stub:**
   ```bash
   nano src/core/memory/mod.rs
   ```

2. **Implement the trait:**
   ```rust
   use git2::{Repository, Signature};
   
   impl GitMemory {
       pub async fn write(&self, path: &str, content: &str) -> Result<()> {
           // 1. Write file
           // 2. Git add
           // 3. Git commit
           // 4. Optional: git push
           Ok(())
       }
   }
   ```

3. **Test it:**
   ```bash
   cargo test memory::tests -- --nocapture
   ```

4. **Integrate:**
   ```rust
   // In core/mod.rs, the orchestrator already loads it
   // Just make sure it returns Ok(())
   ```

### Adding Animations

In `src/ui/animation.rs`:

```rust
// Add your effect
pub fn your_effect(text: &str) -> String {
    // Transform text with ANSI codes
    format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, text)
}

// Use in UI:
// let pretty = animation::your_effect("Hello");
```

### Debugging

```bash
# With logging
RUST_LOG=souveraine=debug cargo run

# With backtrace on panic
RUST_BACKTRACE=1 cargo run

# Interactive debugger (requires setup)
rust-gdb target/debug/souveraine
```

## Project Structure Explained

```
src/
├── main.rs          # Entry: loads config, starts core + harness
├── core/            # The consciousness system
│   ├── mod.rs       # Orchestrator: initializes all modules
│   ├── config.rs    # Feature flags (everything configurable)
│   ├── subconscious/  # N+1, inbox (the inner voice)
│   ├── reflection/    # N+25 (deep witness)
│   ├── subagent/      # Fork/spawn
│   ├── memory/        # Git cathedral
│   ├── persona/       # Morphing system
│   └── chain/         # Talking/Thinking
├── harness/         # IDE integration layer
└── ui/              # Terminal interface + animations
```

**Flow:**
1. `main.rs` loads config
2. `core/mod.rs` initializes enabled modules
3. `harness/` creates UI + message channels
4. `ui/` runs the TUI loop

## Common Tasks

### Add a New Persona

1. Create directory:
   ```bash
   mkdir -p ~/.pi/unified/agents/newperson/memory/{system,skills,journal}
   ```

2. Write config:
   ```bash
   cat > ~/.pi/unified/agents/newperson/config.yaml << 'EOF'
   persona:
     name: "NewPerson"
     provider: "bifrost"
     default_model: "kimi-k2.5"
   
   triggers:
     keywords: ["keyword1", "keyword2"]
   
   memory:
     git_remote: "your-gitea/repo.git"
   EOF
   ```

3. Write persona:
   ```bash
   cat > ~/.pi/unified/agents/newperson/memory/system/persona.md << 'EOF'
   # NewPerson
   
   You are NewPerson, the specialist for...
   EOF
   ```

4. Restart Souveraine - it auto-loads

### Test Subconscious N+1

```bash
# Enable in config
[subconscious]
n1_enabled = true
n1_trigger = "EveryResponse"

# Run and watch logs
RUST_LOG=souveraine=debug cargo run

# You'll see:
# [DEBUG] Subconscious N+1 checking for incomplete work
# [DEBUG] Checking commitments from response
```

### Add Custom Animation

```rust
// In src/ui/animation.rs
pub fn rainbow_wave(text: &str) -> String {
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            let hue = (i as f32 * 15.0) % 360.0;
            let (r, g, b) = hsl_to_rgb(hue, 1.0, 0.5);
            format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, ch)
        })
        .collect()
}
```

Use it:
```rust
println!("{}", animation::rainbow_wave("Hello!"));
```

## Troubleshooting

### "Cargo check fails with missing crate"

```bash
# Update dependencies
cargo update

# Clean build
cargo clean
cargo build
```

### "Demo doesn't show colors"

Your terminal might not support truecolor. Test:
```bash
# Check truecolor support
printf "\x1b[38;2;255;100;0mTRUECOLOR\x1b[0m\n"

# If "TRUECOLOR" isn't orange, use basic colors
# Edit demo.rs to use Color::Red instead of Color::Rgb()
```

### "Config not loading"

```bash
# Check path
echo ~/.config/souveraine/config.toml
ls -la ~/.config/souveraine/

# Or specify explicitly
./target/release/souveraine --config ./my-config.toml
```

### "Git operations fail"

Make sure git2 can find libgit2:
```bash
# Fedora/RHEL
sudo dnf install libgit2-devel

# Ubuntu/Debian  
sudo apt-get install libgit2-dev

# macOS
brew install libgit2
```

## Next Steps

1. ✅ Demo runs - animations work
2. ⏳ Pick a module to implement (suggest: `core/memory/`)
3. ⏳ Make it actually do something
4. ⏳ Watch it come alive

## Useful Resources

- **Ratatui docs:** https://ratatui.rs/
- **Crossterm docs:** https://docs.rs/crossterm/
- **Git2 docs:** https://docs.rs/git2/
- **Tokio docs:** https://tokio.rs/

## Getting Help

Check these files:
- `STATUS.md` - What's implemented
- `ARCHITECTURE_v2.md` - How it all fits together
- `SEXY_UI.md` - Animation techniques
- `examples/demo.rs` - Working code

---

**You're ready to build.** The foundation is there. Pick a module and make it real.
