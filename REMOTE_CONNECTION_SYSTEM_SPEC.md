# Souveraine Remote Connection System
## Multi-Server Configuration & Nicknames

> **Vision:** `souveraine tui --server work` connects to your "work" server  
> Multiple consciousnesses, one CLI.  
> Date: 2026-05-06

---

## Core Concept

Users have **multiple Souveraine servers** they connect to:
- `home` - Home server (always on)
- `work` - Work laptop server
- `lab` - Lab workstation
- `cloud` - VPS somewhere

CLI manages these as **named connections** with full configuration.

---

## User Experience

### Configuration File

```toml
# ~/.config/souveraine/remotes.toml

[remote.home]
name = "Home Server"
url = "https://home.example.com:8283"
api_key = "souv_sk_xxx"  # Or token-based auth
nickname = "home"
default_agent = "agent-ani-xxx"

[remote.work]
name = "Work Laptop"
url = "http://192.168.1.100:8283"
# No api_key - local network trust
nickname = "work"
default_agent = "agent-work-xxx"

[remote.lab]
name = "Lab Workstation"
url = "http://10.10.20.50:8283"
nickname = "lab"
# Discover agents on connect

[remote.cloud]
name = "Cloud VPS"
url = "https://souv.example.com:443"
api_key = "souv_sk_yyy"
nickname = "cloud"
tls_verify = true
```

### CLI Commands

```bash
# List configured remotes
$ souveraine remotes
NAME    URL                              STATUS    DEFAULT_AGENT
home    https://home.example.com:8283    online    ani
work    http://192.168.1.100:8283        offline   -
lab     http://10.10.20.50:8283          online    devops
default localhost:8283                   online    ani

# Add new remote
$ souveraine remotes add
Name: staging
URL: https://staging.internal:8283
API Key: souv_sk_abc123
Default agent (leave blank to discover): 
Added "staging" remote

# Quick connect via nickname
$ souveraine tui --server home
# or
$ souveraine chat --server work "Deploy the new config"

# Switch default remote
$ souveraine remotes default work
Default remote set to "work"

# Check server health
$ souveraine remotes check home
✓ Home Server (https://home.example.com:8283)
  Status: online
  Agents: 3
  Version: souveraine 0.5.0
  Latency: 12ms

# Remove remote
$ souveraine remotes remove lab
Removed "lab" remote
```

### Interactive TUI Selector

```
┌─────────────────────────────────────────────┐
│  Souveraine - Select Remote                 │
├─────────────────────────────────────────────┤
│                                             │
│  ★ home     Home Server              [online]  │
│    work     Work Laptop              [offline] │
│    lab      Lab Workstation          [online]  │
│    cloud    Cloud VPS                [online]  │
│                                             │
│  [n] Add new  [d] Set default  [c] Check    │
│  [q] Quit                                   │
└─────────────────────────────────────────────┘
```

---

## Architecture

### Remote Registry

```rust
// src/remote/registry.rs

pub struct RemoteRegistry {
    config_path: PathBuf,
    remotes: HashMap<String, RemoteConfig>,
    default: Option<String>,
}

pub struct RemoteConfig {
    pub name: String,           # Display name
    pub nickname: String,       # Short alias (home, work, etc.)
    pub url: String,            # http://host:port
    pub api_key: Option<String>,
    pub default_agent: Option<String>,
    pub tls_verify: bool,
    pub timeout_secs: u64,
}

impl RemoteRegistry {
    /// Load from ~/.config/souveraine/remotes.toml
    pub fn load() -> Result<Self>;
    
    /// Save configuration
    pub fn save(&self) -> Result<()>;
    
    /// Add new remote
    pub fn add(&mut self, config: RemoteConfig) -> Result<()>;
    
    /// Remove remote
    pub fn remove(&mut self, nickname: &str) -> Result<()>;
    
    /// Get remote by nickname
    pub fn get(&self, nickname: &str) -> Option<&RemoteConfig>;
    
    /// Get default remote
    pub fn default(&self) -> Option<&RemoteConfig>;
    
    /// Set default
    pub fn set_default(&mut self, nickname: &str) -> Result<()>;
    
    /// Check all remote statuses
    pub async fn check_all(&self) -> Vec<RemoteStatus>;
    
    /// List with connection status
    pub async fn list_with_status(&self) -> Vec<(RemoteConfig, RemoteStatus)>;
}
```

### Client Connection

```rust
// src/remote/client.rs

pub struct RemoteClient {
    config: RemoteConfig,
    http: reqwest::Client,
    current_agent: Option<String>,
}

impl RemoteClient {
    /// Create client for remote
    pub fn new(config: RemoteConfig) -> Self;
    
    /// Check server health
    pub async fn health_check(&self) -> Result<ServerInfo>;
    
    /// List remote agents
    pub async fn list_agents(&self) -> Result<Vec<AgentSummary>>;
    
    /// Get default or discover
    pub async fn default_agent(&self) -> Result<String>;
    
    /// Start streaming session
    pub async fn stream(
        &self,
        agent_id: &str,
        message: &str,
    ) -> Result<SseStream<StreamEvent>>;
    
    /// Execute tool via remote
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        input: Value,
    ) -> Result<Value>;
}

/// Stream events from remote
pub enum StreamEvent {
    AssistantChunk { content: String },
    ToolCall { name: String, input: Value },
    ToolReturn { output: Value },
    Surfacing { source: String, content: String },
    Reflection { content: String },
    Archivist { synthesis: String, pressure: f32 },
    Done,
    Error { message: String },
}
```

### TUI Remote Mode

```rust
// src/ui/remote_mode.rs

pub struct RemoteTuiApp {
    client: RemoteClient,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    input: String,
    streaming: bool,
}

impl RemoteTuiApp {
    /// Connect to remote and start TUI
    pub async fn run(client: RemoteClient) -> Result<()> {
        // Same TUI as local, but all operations go to remote
        // - Messages → POST /api/v1/sessions/{id}/messages
        // - Surfacing → SSE events
        // - Tool calls → Remote executes, returns result
    }
}
```

---

## Connection Discovery

### Auto-Discover Local Servers

```rust
// src/remote/discovery.rs

pub struct LocalDiscovery;

impl LocalDiscovery {
    /// Scan network for Souveraine servers
    pub async fn scan_network() -> Vec<DiscoveredServer> {
        // mDNS/Bonjour discovery
        // Or scan common ports on local subnet
    }
    
    /// Check if localhost:8283 has server
    pub async fn check_local() -> Option<ServerInfo>;
}

// On first run, if no remotes configured:
// 1. Check localhost:8283
// 2. If found, add as "default"
// 3. Prompt user to confirm
```

### Server Advertisement

```rust
// Server can advertise itself via mDNS

pub struct ServerAdvertisement {
    name: String,
    version: String,
    port: u16,
    agents: Vec<String>,
}

// Clients discover: "Souveraine home-server on 192.168.1.100:8283"
```

---

## Security

### Authentication Options

```rust
pub enum AuthMethod {
    /// No auth (local network)
    None,
    
    /// API key in header: X-API-Key: souv_sk_xxx
    ApiKey { key: String },
    
    /// Bearer token: Authorization: Bearer eyJ...
    Bearer { token: String },
    
    /// Client certificates (mTLS)
    MutualTLS {
        cert_path: PathBuf,
        key_path: PathBuf,
    },
}
```

### Key Storage

```rust
// API keys stored in system keyring
use keyring::Entry;

pub fn store_api_key(remote: &str, key: &str) -> Result<()> {
    let entry = Entry::new("souveraine", remote)?;
    entry.set_password(key)?;
    Ok(())
}

pub fn get_api_key(remote: &str) -> Result<String> {
    let entry = Entry::new("souveraine", remote)?;
    entry.get_password()
}
```

---

## Workflows

### Workflow 1: Setup New Remote

```bash
# User adds work laptop
$ souveraine remotes add
Name: work-laptop
URL: http://192.168.1.50:8283
Save API key? (y/n): n
Discover agents? (y/n): y

Discovered agents:
  1. ani (primary)
  2. dev-helper
Set default: 1

Added "work-laptop" with default agent "ani"

# Use it
$ souveraine tui --server work-laptop
```

### Workflow 2: Switch Context

```bash
# At home, use home server
$ souveraine chat "What's the weather?"
# → Uses default (home)

# At coffee shop, connect to work
$ souveraine remotes default work
Default remote set to "work"

$ souveraine tui
# → Connects to work server
```

### Workflow 3: Multi-Server Awareness

```bash
# Check all your servers
$ souveraine remotes status
home           ● online  3 agents  12ms
work           ○ offline 0 agents  -
lab            ● online  1 agent   45ms
cloud          ● online  2 agents  120ms  ← slow

# Work laptop is offline (maybe suspended)
# Auto-fallback? Or prompt?
```

---

## Implementation Phases

### Phase 1: Basic Remote Support

```rust
// src/main.rs additions
#[derive(Args)]
struct Cli {
    #[arg(long, short)]
    server: Option<String>,  // URL or nickname
}

// If --server provided:
// - Parse as URL or look up in registry
// - Create RemoteClient
// - Run in remote mode
```

### Phase 2: Registry & Management

```rust
// Add subcommands:
// souveraine remotes list
// souveraine remotes add
// souveraine remotes remove
// souveraine remotes default
```

### Phase 3: TUI Remote Selector

```rust
// Interactive remote picker
// Shows status, latency, agent count
// Visual connection manager
```

### Phase 4: Advanced Features

```rust
// - Auto-discovery
// - Connection pooling
// - Offline queue (queue messages when offline)
// - Sync between servers (agent migration)
```

---

## Summary

**The Vision:**

```
┌──────────────────────────────────────────────┐
│  souveraine CLI                              │
│                                              │
│  Multiple remote consciousness servers         │
│  Managed by nickname, easy switching         │
│                                              │
│  $ souveraine tui --server home              │
│  $ souveraine chat --server work "deploy"    │
│  $ souveraine remotes status                 │
└──────────────────────────────────────────────┘
```

**Key Features:**
1. Named remotes (home, work, lab, cloud)
2. URL or nickname resolution
3. Per-remote configuration (default agent, auth)
4. Health checking & status
5. Secure credential storage
6. Auto-discovery of local servers
7. Interactive TUI selector

**One CLI, Multiple Consciousnesses.**
