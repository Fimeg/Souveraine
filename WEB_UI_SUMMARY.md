# Souveraine Web UI

## What's Been Built

### 1. Web Server Integration
- **Modified**: `Cargo.toml` — Added `fs` feature to `tower-http`
- **Modified**: `src/api/mod.rs` — Added `ServeDir` for static file serving
- **Location**: `web/dist/index.html` — Single-file web UI

### 2. Web UI Features
- **Peonia-inspired aesthetic**: Generative canvas background with flowing curves and particles
- **Dark theme**: `#0a0a0f` background with rose/coral accents (`#c77` → `#e9b`)
- **Agent sidebar**: List, select, create agents
- **Chat interface**: Message bubbles, streaming, consciousness events
- **Pressure indicator**: Context pressure visualization
- **Composer**: Auto-resizing textarea with send button
- **Real-time**: SSE streaming from Souveraine's API

### 3. Tauri Desktop App (Configured)
- **Feature flag**: `tauri-desktop` in `Cargo.toml`
- **Config**: `tauri/tauri.conf.json` — Window, tray, bundle settings
- **Build targets**: Linux (deb/rpm/appimage), macOS (dmg), Windows (nsis)

### 4. API Compatibility
- **Web UI** → `http://localhost:8484` (Souveraine server)
- **Desktop** → Embedded web view → same API
- **Mobile/LACE** → Can connect to `http://<server>:8484` if on same network

## Running It

### Web Mode (Browser)
```bash
cd /home/casey/Projects/souveraine
cargo run -- server
# Open http://localhost:8484 in browser
```

### Desktop Mode (Tauri)
```bash
cd /home/casey/Projects/souveraine
cargo run --features tauri-desktop -- server
```

### For LACE/Android
- Ensure Souveraine server binds to `0.0.0.0` not `127.0.0.1`
- Android app connects to `http://<server-ip>:8484`
- CORS is already permissive (`tower_http::cors::CorsLayer::permissive()`)

## Architecture

```
┌─────────────────────────────────────────┐
│           SOUVERAINE (Rust)             │
│  ┌─────────────┐    ┌───────────────┐  │
│  │ Axum Server │────│ Web UI (dist) │  │ ← Served at /
│  │  /v1/*      │    │  index.html   │  │
│  └─────────────┘    └───────────────┘  │
│         ↑                               │
│    ┌────┴────┐                         │
│    │  Tauri  │ ← Optional desktop     │
│    │  Shell  │   wrapper              │
│    └─────────┘                         │
└─────────────────────────────────────────┘
         ↑
    ┌────┴────┐
    │  LACE   │ ← Android (if same network)
    │ Android │
    └─────────┘
```

## Next Steps

1. **Build & Test**: `cargo build` to verify no errors
2. **Tauri Icons**: Create `tauri/icons/` (32x32.png, 128x128.png, icon.icns, icon.ico)
3. **Agent Creation**: Wire up "New Agent" button
4. **Settings Panel**: Server config, model selection
5. **Mobile Responsiveness**: Add media queries for LACE

## Design Tokens

From the CSS custom properties in `index.html`:
- Background: `#0a0a0f` (primary), `#12121a` (secondary)
- Accent: `#c77` (rose) → `#e9b` (pink)
- Font: `'Courier New', monospace` + `'Georgia', serif` for display
- Animation: `breathe` (3s), `pulse` (3s), `gradientShift` (8s)
