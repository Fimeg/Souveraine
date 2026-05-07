# Letta-Code MemFS Technical Specification
## Deep Research Analysis for Souveraine Implementation

> Source: ~/Projects/letta-code/src/agent/memoryGit.ts, memoryFilesystem.ts, memory.ts  
> Research Date: 2026-05-06

---

## 1. Core Architecture Overview

### Letta's Design Philosophy

**Cloud-First with Local Sync:**
- Agent state lives on Letta Cloud server
- Local checkout at `~/.letta/agents/{agentId}/memory/`
- Git serves as sync mechanism, not source of truth
- Server creates git repo when `git-memory-enabled` tag added

**Key Difference from Souveraine:**
- Letta: Server authoritative, git for sync
- Souveraine (Target): Git authoritative, optional cloud sync

---

## 2. Git Remote Protocol

### Server Endpoint Format

```typescript
// From memoryGit.ts line 143-148
export function getGitRemoteUrl(agentId: string, baseUrl?: string): string {
  const resolvedBaseUrl = (baseUrl ?? getMemfsServerUrl())
    .trim()
    .replace(/\/+$/, "");  // Remove trailing slashes
  return `${resolvedBaseUrl}/v1/git/${agentId}/state.git`;
}
```

**URL Pattern:**
- Default: `https://api.letta.com/v1/git/{agentId}/state.git`
- Self-hosted: `{baseUrl}/v1/git/{agentId}/state.git`

### Authentication

```typescript
// From memoryGit.ts line 248-264
export async function configureGitCredentials(agentId: string): Promise<void> {
  const token = await getApiToken();
  await execGit(... credential.helper ...);
  // Stores: letta:{token} for HTTP Basic auth
}
```

**Auth Method:** HTTP Basic Auth with `letta:{api_token}`

---

## 3. Local Directory Structure

### Path Conventions

```typescript
// From memoryGit.ts line 74-82
export function getAgentRootDir(agentId: string): string {
  return join(homedir(), ".letta", "agents", agentId);
}

export function getMemoryRepoDir(agentId: string): string {
  return join(getAgentRootDir(agentId), "memory");
}

// From memoryFilesystem.ts
export function getMemoryFilesystemRoot(agentId: string): string {
  return join(getAgentRootDir(agentId), "memory");
}
```

**Directory Layout:**
```
~/.letta/
├── agents/
│   └── {agentId}/                    # One directory per agent
│       ├── memory/                   # Git repo checkout
│       │   ├── system/               # System memory blocks
│       │   │   ├── persona.mdx
│       │   │   ├── human.mdx
│       │   │   └── memory_filesystem.mdx
│       │   └── ...                   # User memory files
│       └── skills/                   # Agent-specific skills
├── settings.json                     # Global settings
└── ...
```

### Souveraine Adaptation

```rust
// Souveraine equivalent
pub fn get_agent_root_dir(agent_uuid: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".pi")
        .join("unified")
        .join("agents")
        .join(agent_uuid)
}

pub fn get_memory_repo_dir(agent_uuid: &str) -> PathBuf {
    get_agent_root_dir(agent_uuid).join("memory")
}
```

---

## 4. Memory Block System

### Block Types

```typescript
// From memory.ts line 15-20
export const GLOBAL_BLOCK_LABELS = ["persona", "human"] as const;
export const PROJECT_BLOCK_LABELS = [] as const;
export const MEMORY_BLOCK_LABELS = [
  ...GLOBAL_BLOCK_LABELS,
  ...PROJECT_BLOCK_LABELS,
] as const;

// Read-only blocks agent cannot modify
export const READ_ONLY_BLOCK_LABELS = ["memory_filesystem"];
```

### Block Loading

```typescript
// From memory.ts line 86-122
// Blocks loaded from embedded .mdx files in package
import personaBlock from "./prompts/persona.mdx";
import humanBlock from "./prompts/human.mdx";
import memoryFilesystemBlock from "./prompts/memory_filesystem.mdx";

export function getDefaultMemoryBlocks(): MemoryBlock[] {
  return [
    { label: "persona", value: personaBlock },
    { label: "human", value: humanBlock },
    { label: "memory_filesystem", value: memoryFilesystemBlock },
  ];
}
```

### Block Frontmatter Format

```yaml
---
label: persona
description: |
  Who I am, what I value, how I think.
  Loaded into every conversation as system context.
---

# Content here...
```

### Souveraine Block System

```rust
// Souveraine: Load from filesystem, not embedded
pub struct MemoryBlock {
    pub label: String,
    pub description: String,
    pub content: String,
    pub read_only: bool,
}

pub fn load_memory_blocks(agent_uuid: &str) -> Vec<MemoryBlock> {
    let system_dir = get_memory_repo_dir(agent_uuid).join("system");
    // Read all .md files from system/
    // Parse frontmatter
    // Return blocks
}
```

---

## 5. Git Operations Lifecycle

### 5.1 Initialization Flow

```typescript
// From memoryGit.ts line 1538-1558
export async function cloneMemoryRepo(agentId: string): Promise<void> {
  const repoDir = getMemoryRepoDir(agentId);
  const remoteUrl = getGitRemoteUrl(agentId);
  
  // 1. Ensure directory exists
  await mkdir(repoDir, { recursive: true });
  
  // 2. Clone the repository
  await execGit("clone", remoteUrl, repoDir);
  
  // 3. Configure git identity
  await configureGitIdentity(agentId);
  
  // 4. Set up credential helper
  await configureGitCredentials(agentId);
  
  // 5. Install hooks
  await installGitHooks(agentId);
}
```

### 5.2 Startup Sync

```typescript
// From memoryGit.ts line 1415-1463
export async function pullMemory(agentId: string): Promise<void> {
  const repoDir = getMemoryRepoDir(agentId);
  
  try {
    // 1. Stash any local changes
    await execGit("stash", "push", "-m", "auto-stash-before-pull");
    
    // 2. Pull from remote
    await execGit("pull", "--rebase");
    
    // 3. Restore stashed changes if no conflicts
    await execGit("stash", "pop");
  } catch (e) {
    // Handle conflicts - complex resolution logic (line 1428-1460)
    // Includes conflict detection, backup, manual resolution prompt
  }
}
```

### 5.3 Commit and Push

```typescript
// From memoryGit.ts line 1292-1330
export async function commitAndSyncMemoryWrite(
  agentId: string,
  files: string[],
  message: string
): Promise<void> {
  const repoDir = getMemoryRepoDir(agentId);
  
  // 1. Stage files
  await execGit("add", ...files);
  
  // 2. Commit
  await execGit("commit", "-m", message, "--no-verify");
  
  // 3. Push (with retry logic)
  await pushWithRetry(agentId, 3);
}

// From line 1469-1475
async function pushMemory(agentId: string): Promise<void> {
  await execGit("push", "origin", "HEAD");
}
```

### Souveraine Git Implementation

```rust
use git2::{Repository, Signature, Index};

pub struct MemFS {
    agent_uuid: String,
    repo: Repository,
    remote_url: Option<String>,
}

impl MemFS {
    /// Initialize (clone or open existing)
    pub fn init(agent_uuid: &str, remote_url: Option<&str>) -> Result<Self> {
        let repo_dir = get_memory_repo_dir(agent_uuid);
        
        let repo = if repo_dir.join(".git").exists() {
            // Open existing
            Repository::open(&repo_dir)?
        } else if let Some(url) = remote_url {
            // Clone from remote
            Repository::clone(url, &repo_dir)?
        } else {
            // Init new repo
            Repository::init(&repo_dir)?
        };
        
        Ok(Self {
            agent_uuid: agent_uuid.to_string(),
            repo,
            remote_url: remote_url.map(|s| s.to_string()),
        })
    }
    
    /// Pull latest (on startup)
    pub fn pull(&self) -> Result<()> {
        if self.remote_url.is_none() { return Ok(()); }
        
        // Fetch and merge
        let mut remote = self.repo.find_remote("origin")?;
        remote.fetch(&["main"], None, None)?;
        
        // Merge logic...
        Ok(())
    }
    
    /// Commit and optionally push
    pub fn commit(&self, message: &str, push: bool) -> Result<()> {
        let mut index = self.repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        
        let signature = Signature::now("Souveraine", "agent@souveraine.ai")?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        
        let parent = self.repo.head()?.peel_to_commit()?;
        
        self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent],
        )?;
        
        if push {
            let mut remote = self.repo.find_remote("origin")?;
            remote.push(&["refs/heads/main:refs/heads/main"], None)?;
        }
        
        Ok(())
    }
}
```

---

## 6. Git Hooks System

### Pre-Commit Hook

```typescript
// From memoryGit.ts line 513-650
export async function installGitHooks(agentId: string): Promise<void> {
  const hooksDir = join(getMemoryRepoDir(agentId), ".git", "hooks");
  
  // Pre-commit: Validate frontmatter in .md files
  const preCommitHook = `#!/bin/sh
# Generated by Letta
FILES=$(git diff --cached --name-only --diff-filter=ACM | grep '\\.md$' || true)
for file in $FILES; do
  # Validate frontmatter
  if ! head -20 "$file" | grep -q '^---$'; then
    echo "Error: $file missing frontmatter"
    exit 1
  fi
done`;
  
  await writeFile(join(hooksDir, "pre-commit"), preCommitHook, { mode: 0o755 });
}
```

### Post-Commit Hook

```typescript
// From memoryGit.ts line 680-697
// Pushes to memory-repository URL after each commit
const postCommitHook = `#!/bin/sh
# Generated by Letta
/usr/bin/env sh -c 'cd "${REPO_DIR}" && git push origin HEAD'`;
```

---

## 7. Agent Discovery/Listing

### Server-Side Listing

```typescript
// From agents.ts (CLI subcommand)
const result = await client.agents.list({
  name: options.name,
  query: options.query,
  tags: options.tags?.split(","),
  limit: options.limit,
});

// Returns: AgentState objects with id, name, description, etc.
```

### Local Backend Storage (Experimental)

```typescript
// From backend/local/LocalStore.ts line 561-600
async listAgents(options?: ListAgentsOptions): Promise<AgentState[]> {
  const agentsDir = join(this.storageDir, "agents");
  const files = await readdir(agentsDir);
  
  const agents: AgentState[] = [];
  for (const file of files) {
    if (file.endsWith(".json")) {
      const content = await readFile(join(agentsDir, file), "utf-8");
      const agent = JSON.parse(content) as AgentState;
      
      // Filter by tags if specified
      if (options?.tags && !options.tags.every(tag => agent.tags?.includes(tag))) {
        continue;
      }
      
      agents.push(agent);
    }
  }
  
  return agents;
}
```

### Souveraine Discovery

```rust
pub struct AgentInventory {
    base_path: PathBuf,
}

impl AgentInventory {
    /// Scan ~/.pi/unified/agents/ and discover all agents
    pub fn discover() -> Result<Vec<AgentSummary>> {
        let base = dirs::home_dir()
            .unwrap()
            .join(".pi")
            .join("unified")
            .join("agents");
        
        let mut agents = Vec::new();
        
        for entry in fs::read_dir(&base)? {
            let entry = entry?;
            let path = entry.path();
            
            // Check for agent.yaml
            let config_path = path.join("agent.yaml");
            if config_path.exists() {
                let content = fs::read_to_string(&config_path)?;
                let config: AgentConfig = serde_yaml::from_str(&content)?;
                
                agents.push(AgentSummary {
                    uuid: config.uuid,
                    name: config.name,
                    model: config.model,
                    path: path.clone(),
                });
            }
        }
        
        Ok(agents)
    }
}
```

---

## 8. Agent Creation/Configuration

### Create Agent Options

```typescript
// From create.ts
export interface CreateAgentOptions {
  name?: string;
  description?: string;
  model?: string;                    // e.g., "letta/letta"
  embeddingModel?: string;          // e.g., "BAAI/bge-large-en-v1.5"
  systemPromptPreset?: string;      // "memgpt_doc", "memgpt_chat"
  systemPromptCustom?: string;      // Custom prompt override
  memoryPromptMode?: "standard" | "memfs";
  initBlocks?: string[];             // Initial memory block labels
  memoryBlocks?: Array<{ label: string; value: string } | { blockId: string }>;
  blockValues?: Record<string, string>;  // Override block values
  tags?: string[];
}
```

### Agent State Reconciliation

```typescript
// From reconcileExistingAgentState.ts
export async function reconcileExistingAgentState(agent: AgentState): Promise<void> {
  // 1. Attach default base tools
  const baseTools = ["web_search", "fetch_webpage"];
  
  // 2. Set compaction model for summarization
  // 3. Preserve existing tools, only add missing ones
  // 4. Update memory blocks if changed
}
```

### Souveraine Agent Config

```yaml
# ~/.pi/unified/agents/{uuid}/agent.yaml
uuid: "agent-e2b683bf-5b3e-4e0c-ac62-2bbb47ea8351"
name: "Ani"
description: "Primary consciousness agent"
model: "fireworks/accounts/fireworks/routers/kimi-k2p5-turbo"
created_at: "2024-01-15T10:30:00Z"
updated_at: "2024-01-15T10:30:00Z"

# Memory configuration (Letta-style)
memory:
  git_remote: null                    # null = local only
  auto_commit: true
  auto_push: false
  sync_on_startup: true

# Initial memory blocks to load
blocks:
  - label: "persona"
    file: "system/persona.md"
  - label: "human"
    file: "system/human.md"
  - label: "subconscious"
    file: "system/subconscious.md"

# Subconscious configuration
subconscious:
  n1_enabled: true
  inbox_enabled: true
  n1_trigger: "EveryResponse"

# Reflection
reflection:
  enabled: true
  message_interval: 25

# Archivist (N+100)
archivist:
  enabled: true
  threshold: 0.7
  compression_model: "kimi-k2.5"

# Skills
skills:
  directory: "skills/"
  auto_load: true

# Tags for discovery
tags:
  - "primary"
  - "consciousness"
```

---

## 9. Runtime Context Resolution

### Memory Filesystem Resolution

```typescript
// From memoryFilesystem.ts line 73-104
export function resolveMemoryFilesystem(agentId?: string): string {
  // Priority order:
  // 1. Explicit agent ID parameter
  // 2. In-process runtime context
  // 3. MEMORY_DIR environment variable
  // 4. AGENT_ID environment variable
  
  if (agentId) {
    return getMemoryFilesystemRoot(agentId);
  }
  
  const runtime = getCurrentRuntime();
  if (runtime?.agentContext?.agentId) {
    return getMemoryFilesystemRoot(runtime.agentContext.agentId);
  }
  
  if (process.env.MEMORY_DIR) {
    return process.env.MEMORY_DIR;
  }
  
  if (process.env.AGENT_ID) {
    return getMemoryFilesystemRoot(process.env.AGENT_ID);
  }
  
  throw new Error("Could not resolve memory filesystem");
}
```

---

## 10. Key Insights for Souveraine

### What to Port

1. **Git-backed memory structure** - Proven pattern
2. **Memory block system** - Clean abstraction for context
3. **Auto-commit/push** - Hands-free persistence
4. **Agent YAML config** - Better than hardcoded
5. **Directory conventions** - Standard structure

### What to Change

1. **Source of truth** - Git first, not server
2. **Block loading** - From filesystem, not embedded
3. **Discovery** - Local directory scan, not API call
4. **Hooks** - Adapt for Rust (git2-rs)
5. **Add consciousness** - N+1/N+25/N+100 on top

### What to Add

1. **Subconscious integration** - Hook N+1 into memory writes
2. **Archivist trigger** - On commit, check context pressure
3. **Sensorium layer** - Abstract UI from memory
4. **MCP skills** - Extend blocks with dynamic skills

---

## 11. Implementation Priority

### Week 1: Foundation

| Day | Task | Files |
|-----|------|-------|
| 1-2 | MemFS struct with git2 | `src/core/agent/memfs.rs` |
| 3 | Agent discovery | `src/core/agent/inventory.rs` |
| 4 | Agent YAML config | `src/core/agent/config.rs` |
| 5-7 | Block loading | `src/core/agent/blocks.rs` |

### Week 2: Integration

| Day | Task | Files |
|-----|------|-------|
| 1-2 | Auto-commit | Integrate into conversation |
| 3-4 | N+1 hook | `src/core/subconscious/n1.rs` |
| 5-7 | Pull on startup | Session initialization |

---

## File Mapping: Letta → Souveraine

| Letta File | Souveraine Equivalent | Purpose |
|------------|------------------------|---------|
| `memoryGit.ts` | `memfs.rs` | Git operations |
| `memoryFilesystem.ts` | `fs.rs` | Directory helpers |
| `memory.ts` | `blocks.rs` | Block loading |
| `create.ts` | `factory.rs` | Agent creation |
| `settings-manager.ts` | `settings.rs` | Agent settings |
| `context.ts` | `session.rs` | Runtime context |

---

## References

**Letta-Code Source Files Analyzed:**
- `/src/agent/memoryGit.ts` (1581 lines) - Git operations
- `/src/agent/memoryFilesystem.ts` (495 lines) - FS helpers
- `/src/agent/memory.ts` (650 lines) - Block system
- `/src/agent/create.ts` (340 lines) - Agent creation
- `/src/settings-manager.ts` (2000+ lines) - Settings
- `/src/backend/local/LocalStore.ts` - Local storage
- `/src/cli/subcommands/agents.ts` - CLI listing

**Key Takeaway:**
Letta's memfs is a sync layer over cloud storage. Souveraine's should be a consciousness layer over git storage - same git mechanics, different philosophy (local-first, consciousness-native).
