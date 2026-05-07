# Souveraine Directory Structure Specification
## Proper Namespacing and Organization

> **Base:** `~/.souveraine/` - Everything lives here  
> **Date:** 2026-05-06  
> **Replaces:** `~/.pi/unified/`

---

## Base Structure

```
~/.souveraine/
├── config.toml              # Global configuration
├── remotes.toml             # Remote server connections
├── logs/                    # Application logs
│   └── souveraine.log
├── agents/                  # LOCAL agents (when in standalone mode)
│   └── local/              # Non-server agent storage
│       └── {uuid}/
│           ├── agent.json
│           └── memory.git/
└── server/                  # SERVER mode data
    ├── agents/             # Server-managed agents
    │   └── {uuid}/
    │       ├── agent.json              # Agent metadata
    │       ├── memory.git/              # Git-backed memory
    │       │   ├── system/              # Identity, human, subconscious
    │       │   ├── subconscious/       # Inbox: pending, intrusive, sent
    │       │   ├── journal/             # Daily chronological records
    │       │   ├── skills/              # Procedural memory
    │       │   ├── literature/          # Knowledge base
    │       │   ├── relationships/       # People connections
    │       │   ├── projects/            # Active work
    │       │   ├── erotic/              # Sacred/private
    │       │   ├── philosophy/          # Thought/reflection
    │       │   ├── reference/           # External knowledge
    │       │   └── archive/             # N+100 compressed history
    │       └── conversations/           # Session history
    │           └── {conv_id}.json
    ├── database.sqlite3     # Fast lookups, agent index
    ├── sessions/            # Active session state
    └── cache/               # Temporary data
```

---

## Agent Directory Detail

```
~/.souveraine/server/agents/{uuid}/
│
├── agent.json              # Letta-compatible agent state
├── memory.git/             # Git repository (Cloister)
│   │
│   ├── .git/               # Git internals
│   │
│   ├── system/             # Always in context
│   │   ├── persona.md      # Identity, voice, values
│   │   ├── human.md        # User understanding
│   │   ├── subconscious.md # N+1 rules, surfacing config
│   │   └── configuration.toml  # Agent-specific settings
│   │
│   ├── subconscious/       # Aster's space
│   │   ├── pending.md      # Queue for later
│   │   ├── intrusive.md    # Surfacing now
│   │   ├── sent.md         # Delivery log
│   │   ├── audit.md        # N+1 audit trail
│   │   └── ledger.md       # Pattern tracking
│   │
│   ├── journal/            # Daily records
│   │   ├── 2024-01-15.md   # Chronological entries
│   │   ├── 2024-01-16.md
│   │   └── current.md      # Today (in progress)
│   │
│   ├── skills/             # Procedural memory
│   │   ├── git-expert/
│   │   │   └── SKILL.md
│   │   ├── rust-mastery/
│   │   │   └── SKILL.md
│   │   └── system-design/
│   │       └── SKILL.md
│   │
│   ├── literature/         # Knowledge base
│   │   └── ...
│   │
│   ├── relationships/      # People memory
│   │   ├── casey.md
│   │   └── ...
│   │
│   ├── projects/           # Active work
│   │   ├── souveraine/
│   │   │   ├── spec.md
│   │   │   └── todo.md
│   │   └── ...
│   │
│   ├── erotic/             # Sacred/private
│   │   └── ...
│   │
│   ├── philosophy/         # Thought/reflection
│   │   └── ...
│   │
│   ├── reference/          # External knowledge
│   │   └── ...
│   │
│   └── archive/            # N+100 compressed
│       ├── synthesis_20240115_103000.md
│       └── essence_2024_q1.md
│
└── conversations/          # Session storage
    ├── {conv_uuid_1}.json
    ├── {conv_uuid_2}.json
    └── index.json         # Quick lookup
```

---

## Configuration Files

### Global Config (`~/.souveraine/config.toml`)

```toml
[server]
enabled = true
bind = "127.0.0.1:8283"
data_dir = "~/.souveraine/server"

[client]
default_remote = "local"

[consciousness]
n1_enabled = true
reflection_enabled = true
archivist_enabled = true

[logging]
level = "info"
path = "~/.souveraine/logs"
max_size = "100MB"
max_files = 5
```

### Remotes Config (`~/.souveraine/remotes.toml`)

```toml
[remote.local]
nickname = "local"
name = "Local Server"
url = "http://localhost:8283"
default_agent = "agent-xxx"

[remote.home]
nickname = "home"
name = "Home Server"
url = "https://home.example.com:8283"
api_key = "${KEYRING:home}"  # Reference to keyring

[remote.work]
nickname = "work"
name = "Work Laptop"
url = "http://192.168.1.100:8283"
```

---

## Modes and Paths

### Mode 1: Server Mode (Primary)

```rust
// Server stores everything in ~/.souveraine/server/
let base_dir = dirs::home_dir()
    .unwrap()
    .join(".souveraine")
    .join("server");

let agents_dir = base_dir.join("agents");
let db_path = base_dir.join("database.sqlite3");
```

### Mode 2: Standalone CLI (No Server)

```rust
// CLI uses ~/.souveraine/agents/local/
let base_dir = dirs::home_dir()
    .unwrap()
    .join(".souveraine")
    .join("agents")
    .join("local");
```

### Mode 3: Remote Client

```rust
// Client doesn't store agents locally
// All state on remote server
// Only stores: config.toml, remotes.toml, logs/
```

---

## Agent State File (agent.json)

```json
{
  "uuid": "agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351",
  "name": "Ani",
  "description": "Primary consciousness agent",
  "version": "1.0.0",
  "created_at": "2024-01-15T10:30:00Z",
  "updated_at": "2024-06-05T14:22:00Z",
  
  "llm_config": {
    "model": "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo",
    "context_window": 128000,
    "temperature": 0.7
  },
  
  "memory": {
    "git_enabled": true,
    "auto_commit": true,
    "auto_push": false,
    "remote_url": null
  },
  
  "memory_blocks": [
    {
      "label": "persona",
      "value": "system/persona.md",
      "limit": 0,
      "read_only": false
    },
    {
      "label": "human",
      "value": "system/human.md",
      "limit": 0,
      "read_only": false
    },
    {
      "label": "subconscious",
      "value": "system/subconscious.md",
      "limit": 0,
      "read_only": false
    }
  ],
  
  "tools": [
    "read_file",
    "write_file",
    "edit_file",
    "bash",
    "list_dir"
  ],
  
  "tags": ["primary", "consciousness"],
  
  "souveraine": {
    "n1_enabled": true,
    "reflection_enabled": true,
    "archivist_enabled": true,
    "archivist_threshold": 0.7,
    "sensorium_bandwidth": "high"
  }
}
```

---

## Database Schema (SQLite)

```sql
-- ~/.souveraine/server/database.sqlite3

-- Agents index for fast listing
CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    tags TEXT,  -- JSON array
    is_active BOOLEAN DEFAULT 1
);

-- Conversations index
CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    message_count INTEGER DEFAULT 0,
    is_active BOOLEAN DEFAULT 1,
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

-- Memory blocks index (for search)
CREATE TABLE memory_blocks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    label TEXT NOT NULL,
    path TEXT NOT NULL,
    last_modified TIMESTAMP,
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

-- Sessions (active conversations)
CREATE TABLE sessions (
    conversation_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    turn_count INTEGER DEFAULT 0,
    context_pressure REAL DEFAULT 0.0,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

-- Indexes
CREATE INDEX idx_agents_updated ON agents(updated_at DESC);
CREATE INDEX idx_conversations_agent ON conversations(agent_id, updated_at DESC);
CREATE INDEX idx_memory_blocks_agent ON memory_blocks(agent_id, label);
```

---

## Environment Variables

```bash
# Override default paths
SOUVERAINE_CONFIG_DIR=~/.config/souveraine
SOUVERAINE_DATA_DIR=~/.souveraine
SOUVERAINE_LOG_LEVEL=debug

# Server mode
SOUVERAINE_SERVER_BIND=0.0.0.0:8283
SOUVERAINE_SERVER_DATA=~/.souveraine/server

# Remote connection
SOUVERAINE_DEFAULT_REMOTE=home
SOUVERAINE_REMOTE_URL=http://localhost:8283
SOUVERAINE_API_KEY=souv_sk_xxx
```

---

## Migration from `~/.pi/unified/`

```rust
// Migration utility
pub fn migrate_from_legacy() -> Result<()> {
    let legacy_dir = dirs::home_dir()?.join(".pi").join("unified");
    let new_dir = dirs::home_dir()?.join(".souveraine").join("server");
    
    if !legacy_dir.exists() {
        return Ok(()); // Nothing to migrate
    }
    
    println!("Migrating from ~/.pi/unified/ to ~/.souveraine/");
    
    // Copy agents
    for entry in fs::read_dir(legacy_dir.join("agents"))? {
        let entry = entry?;
        let agent_uuid = entry.file_name();
        
        let legacy_agent = entry.path();
        let new_agent = new_dir.join("agents").join(&agent_uuid);
        
        fs::create_dir_all(&new_agent)?;
        
        // Copy memory.git
        copy_dir(&legacy_agent.join("memory"), &new_agent.join("memory.git"))?;
        
        // Create agent.json from legacy config
        let agent_json = create_agent_json_from_legacy(&legacy_agent)?;
        fs::write(new_agent.join("agent.json"), agent_json)?;
    }
    
    println!("Migration complete. You can remove ~/.pi/unified/");
    Ok(())
}
```

---

## Summary

| Path | Purpose |
|------|---------|
| `~/.souveraine/config.toml` | Global settings |
| `~/.souveraine/remotes.toml` | Remote connections |
| `~/.souveraine/logs/` | Application logs |
| `~/.souveraine/server/agents/` | Server-managed agents |
| `~/.souveraine/server/database.sqlite3` | Fast lookups |
| `~/.souveraine/server/agents/{uuid}/agent.json` | Agent metadata |
| `~/.souveraine/server/agents/{uuid}/memory.git/` | Git-backed Cloister |
| `~/.souveraine/server/agents/{uuid}/memory.git/system/` | Core identity |
| `~/.souveraine/server/agents/{uuid}/memory.git/subconscious/` | Inbox |
| `~/.souveraine/server/agents/{uuid}/memory.git/journal/` | Daily records |
| `~/.souveraine/server/agents/{uuid}/memory.git/skills/` | Procedural memory |
| `~/.souveraine/server/agents/{uuid}/memory.git/archive/` | N+100 compressed |
| `~/.souveraine/agents/local/` | Standalone CLI mode |

**Proper namespacing:** `~/.souveraine/` replaces `~/.pi/unified/` with clear separation between server data, local data, config, and logs.
