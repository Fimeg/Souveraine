//! souveraine-agentsd — one collector for every agent's session activity.
//!
//! COLLECTOR, NOT AN AUTHORITY. It reads what the agents already write to disk
//! and offers a single projection of it. It decides nothing, actuates nothing,
//! and approves nothing. Every surface that wants to know "what are my agents
//! doing" reads this socket instead of growing its own fetch.
//!
//! ## Why this exists (TASK-69)
//!
//! Before this, the bar owned a fetch per provider: `ClaudeUsage.qml` polled an
//! OAuth endpoint on its own timer, a python collector shelled out to
//! `codex app-server`, and the substrate's own token counters were read by
//! nothing at all. Three cadences, three vocabularies, three ways to be stale,
//! and no single answer to "how much have I spent today". The bar should show
//! projections of one live stream, not run three clients.
//!
//! ## Naming — this is NOT souveraine-sessiond
//!
//! `souveraine-sessiond` is the device's proprioception: the authority for what
//! the machine believes about itself. This daemon is about *agent* sessions —
//! conversations with Claude, Codex and Souveraine. The names are close and the
//! concepts are unrelated. The socket is `souveraine-sessions.sock` to keep the
//! distinction visible at the wire.
//!
//! ## What it reads (verified against real records 2026-08-11)
//!
//! | Provider | Path | Record |
//! |---|---|---|
//! | Claude | `~/.claude/projects/<slug>/<session>.jsonl` | `type=="assistant"`, `.message.usage`, `.message.model`, `.cwd` |
//! | Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `type=="session_meta"` for `.payload.cwd`; `event_msg` with `.payload.type=="token_count"` for usage + `rate_limits` |
//! | Souveraine | `~/.souveraine/server/agents/<id>/conversations/<c>/messages.jsonl` | `role=="assistant"`, `.usage` |
//!
//! **Correction to TASK-69's table, found in the data:** the Souveraine row
//! claimed the substrate's usage counters were never written. They are. Every
//! assistant line carries `input_tokens` / `output_tokens` /
//! `cache_creation_input_tokens` / `cache_read_input_tokens`. What was missing
//! was a *consumer*, which is this. TASK-67's blindness was in the surface, not
//! in the recording.
//!
//! ## On summed input tokens
//!
//! Claude and Souveraine record usage per assistant message, and every message
//! re-sends the whole context. Summing `input_tokens` across a session
//! therefore counts the same context many times over — that number is a
//! *billing* quantity, not a measure of how much was said. It is reported as
//! `tokens.input` because that is what it is, and `last_turn` is carried beside
//! it for the honest "what did that cost" reading. Codex sidesteps this by
//! recording a running `total_token_usage` itself, which is used as-is.

use std::fs;
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Wire version. Bumped when the envelope changes shape incompatibly; surfaces
/// that read a version they do not know should degrade, not guess.
const WIRE_VERSION: u32 = 1;

/// How often the snapshot is rebuilt and pushed to every connected client.
const REFRESH: Duration = Duration::from_secs(5);

/// Files untouched for longer than this are not read at all. Without it, a
/// scan walks every conversation ever held — hundreds of files, tens of
/// megabytes — to answer a question that is only ever about today.
const SCAN_WINDOW: Duration = Duration::from_secs(60 * 60 * 24);

/// Activity more recent than this means the agent is mid-turn.
const WORKING_WITHIN: Duration = Duration::from_secs(90);

/// Beyond this a session is still listed but is plainly not in use.
const IDLE_WITHIN: Duration = Duration::from_secs(60 * 30);

// ---------------------------------------------------------------------------
// The envelope — one JSON shape for all three providers.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    /// Wrote something within `WORKING_WITHIN`.
    Working,
    /// Alive today, quiet now.
    Idle,
    /// Inside the scan window but past `IDLE_WITHIN`.
    Stale,
}

impl SessionState {
    /// Ranking used to sort the list. The surface shows the head of it, so this
    /// ordering *is* the "which session matters right now" policy.
    fn rank(&self) -> u8 {
        match self {
            SessionState::Working => 0,
            SessionState::Idle => 1,
            SessionState::Stale => 2,
        }
    }

    fn from_age(age: Duration) -> Self {
        if age <= WORKING_WITHIN {
            SessionState::Working
        } else if age <= IDLE_WITHIN {
            SessionState::Idle
        } else {
            SessionState::Stale
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
    /// Total for the most recent turn only — the honest per-turn cost, free of
    /// the context-resend double count described in the module docs.
    pub last_turn: u64,
}

impl Tokens {
    /// Everything that was actually paid for, once. Cached reads are excluded
    /// because they are the cheap path and including them makes a long session
    /// look catastrophic.
    pub fn billed(&self) -> u64 {
        self.input + self.output + self.cache_write
    }
}

/// A provider-reported usage window (Codex's `rate_limits`, and later Claude's
/// OAuth utilization). Optional everywhere — a provider that does not report
/// limits is not broken.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Limit {
    pub label: String,
    pub used_percent: f64,
    /// Epoch milliseconds, or 0 when the provider did not say.
    pub resets_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentSession {
    pub id: String,
    pub provider: String,
    /// Short human label — the working directory's basename, which is what a
    /// person actually recognises a session by.
    pub label: String,
    pub cwd: String,
    pub model: String,
    pub state: SessionState,
    pub last_activity_ms: u64,
    pub turns: u32,
    pub tokens: Tokens,
}

#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    pub id: String,
    /// False when the provider's root does not exist or could not be read. The
    /// surface shows "unavailable" for this one provider and keeps the others.
    pub available: bool,
    pub error: Option<String>,
    pub sessions: Vec<AgentSession>,
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub v: u32,
    pub generated_at_ms: u64,
    pub providers: Vec<Provider>,
}

// ---------------------------------------------------------------------------
// Parsers. Each is a pure function over the file's text, so the whole
// collection logic is testable without a filesystem or a live agent.
// ---------------------------------------------------------------------------

fn iso_to_ms(s: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis().max(0) as u64)
}

fn basename(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

fn u64_at(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// Claude Code's per-project transcript.
///
/// Model is taken from the last assistant line that names a real one:
/// `<synthetic>` appears on locally-generated messages (interrupts, tool
/// plumbing) and is not the model the human is talking to.
pub fn parse_claude(id: &str, text: &str) -> Option<AgentSession> {
    let mut cwd = String::new();
    let mut model = String::new();
    let mut last_ms = 0u64;
    let mut turns = 0u32;
    let mut tokens = Tokens::default();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(c) = v.get("cwd").and_then(|c| c.as_str()) {
            if !c.is_empty() {
                cwd = c.to_string();
            }
        }
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let msg = v.get("message").unwrap_or(&serde_json::Value::Null);
        if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
            if !m.is_empty() && m != "<synthetic>" {
                model = m.to_string();
            }
        }
        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if let Some(ms) = iso_to_ms(ts) {
                last_ms = last_ms.max(ms);
            }
        }
        if let Some(u) = msg.get("usage") {
            let turn_in = u64_at(u, "input_tokens");
            let turn_out = u64_at(u, "output_tokens");
            let turn_cw = u64_at(u, "cache_creation_input_tokens");
            let turn_cr = u64_at(u, "cache_read_input_tokens");
            tokens.input += turn_in;
            tokens.output += turn_out;
            tokens.cache_write += turn_cw;
            tokens.cache_read += turn_cr;
            let turn_total = turn_in + turn_out + turn_cw;
            if turn_total > 0 {
                tokens.last_turn = turn_total;
                turns += 1;
            }
        }
    }

    if last_ms == 0 && turns == 0 {
        return None;
    }
    Some(AgentSession {
        id: id.to_string(),
        provider: "claude".into(),
        label: basename(&cwd),
        cwd,
        model,
        state: SessionState::Stale, // replaced once the clock is applied
        last_activity_ms: last_ms,
        turns,
        tokens,
    })
}

/// Codex rollout files.
///
/// Codex maintains its own running total, so unlike the other two providers
/// nothing is summed here — the last `token_count` record is the answer.
pub fn parse_codex(id: &str, text: &str) -> Option<AgentSession> {
    let mut cwd = String::new();
    let mut model = String::new();
    let mut session_id = id.to_string();
    let mut last_ms = 0u64;
    let mut turns = 0u32;
    let mut tokens = Tokens::default();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let outer = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let payload = v.get("payload").unwrap_or(&serde_json::Value::Null);

        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if let Some(ms) = iso_to_ms(ts) {
                last_ms = last_ms.max(ms);
            }
        }

        // The record kind lives on the outer envelope for session_meta and on
        // the payload for event_msg. Checking both is not defensive
        // over-engineering; it is the actual shape.
        if outer == "session_meta" {
            if let Some(c) = payload.get("cwd").and_then(|c| c.as_str()) {
                cwd = c.to_string();
            }
            if let Some(s) = payload.get("id").and_then(|c| c.as_str()) {
                session_id = s.to_string();
            }
        }

        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        turns += 1;
        if let Some(info) = payload.get("info") {
            if let Some(t) = info.get("total_token_usage") {
                tokens.input = u64_at(t, "input_tokens");
                tokens.output = u64_at(t, "output_tokens");
                tokens.cache_read = u64_at(t, "cached_input_tokens");
                tokens.cache_write = u64_at(t, "cache_write_input_tokens");
                tokens.reasoning = u64_at(t, "reasoning_output_tokens");
            }
            if let Some(t) = info.get("last_token_usage") {
                tokens.last_turn = u64_at(t, "total_tokens");
            }
        }
        if let Some(m) = payload.get("model").and_then(|m| m.as_str()) {
            model = m.to_string();
        }
    }

    if last_ms == 0 && turns == 0 {
        return None;
    }
    Some(AgentSession {
        id: session_id,
        provider: "codex".into(),
        label: basename(&cwd),
        cwd,
        model,
        state: SessionState::Stale,
        last_activity_ms: last_ms,
        turns,
        tokens,
    })
}

/// The rate-limit windows, which belong to the *provider* rather than to any
/// one session — every Codex rollout reports the same account-wide numbers, so
/// hanging them off a session would duplicate them once per open terminal.
pub fn codex_limits(text: &str) -> Vec<Limit> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let payload = v.get("payload").unwrap_or(&serde_json::Value::Null);
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let Some(rl) = payload.get("rate_limits") else {
            continue;
        };
        let mut found = Vec::new();
        for (key, label) in [("primary", "primary"), ("secondary", "secondary")] {
            let Some(w) = rl.get(key) else { continue };
            if w.is_null() {
                continue;
            }
            found.push(Limit {
                label: label.to_string(),
                used_percent: w
                    .get("used_percent")
                    .and_then(|u| u.as_f64())
                    .unwrap_or(0.0),
                resets_at_ms: w
                    .get("resets_at")
                    .and_then(|r| r.as_u64())
                    .map(|s| s.saturating_mul(1000))
                    .unwrap_or(0),
            });
        }
        if !found.is_empty() {
            out = found;
        }
    }
    out
}

/// The substrate's own persisted conversations.
pub fn parse_souveraine(id: &str, label: &str, text: &str) -> Option<AgentSession> {
    let mut last_ms = 0u64;
    let mut turns = 0u32;
    let mut tokens = Tokens::default();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if let Some(ms) = iso_to_ms(ts) {
                last_ms = last_ms.max(ms);
            }
        }
        if v.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let Some(u) = v.get("usage") else { continue };
        if u.is_null() {
            continue;
        }
        let turn_in = u64_at(u, "input_tokens");
        let turn_out = u64_at(u, "output_tokens");
        let turn_cw = u64_at(u, "cache_creation_input_tokens");
        let turn_cr = u64_at(u, "cache_read_input_tokens");
        tokens.input += turn_in;
        tokens.output += turn_out;
        tokens.cache_write += turn_cw;
        tokens.cache_read += turn_cr;
        let turn_total = turn_in + turn_out + turn_cw;
        if turn_total > 0 {
            tokens.last_turn = turn_total;
            turns += 1;
        }
    }

    if last_ms == 0 && turns == 0 {
        return None;
    }
    Some(AgentSession {
        id: id.to_string(),
        provider: "souveraine".into(),
        label: label.to_string(),
        cwd: String::new(),
        model: String::new(),
        state: SessionState::Stale,
        last_activity_ms: last_ms,
        turns,
        tokens,
    })
}

// ---------------------------------------------------------------------------
// Collection — the filesystem half.
// ---------------------------------------------------------------------------

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Recursively gather `*.jsonl` under `root` that were modified inside the scan
/// window. Depth-limited: these trees are shallow by construction, and an
/// unbounded walk over a symlinked home is a hazard, not a feature.
fn recent_jsonl(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let cutoff = SystemTime::now() - SCAN_WINDOW;
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            recent_jsonl(&path, depth - 1, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("jsonl")
            && meta.modified().map(|m| m >= cutoff).unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn collect_claude() -> Provider {
    let root = home().join(".claude/projects");
    let mut p = Provider {
        id: "claude".into(),
        available: root.is_dir(),
        error: None,
        sessions: Vec::new(),
        limits: Vec::new(),
    };
    if !p.available {
        p.error = Some("no ~/.claude/projects".into());
        return p;
    }
    let mut files = Vec::new();
    recent_jsonl(&root, 3, &mut files);
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        if let Some(s) = parse_claude(&stem(&f), &text) {
            p.sessions.push(s);
        }
    }
    p
}

fn collect_codex() -> Provider {
    let root = home().join(".codex/sessions");
    let mut p = Provider {
        id: "codex".into(),
        available: root.is_dir(),
        error: None,
        sessions: Vec::new(),
        limits: Vec::new(),
    };
    if !p.available {
        p.error = Some("no ~/.codex/sessions".into());
        return p;
    }
    let mut files = Vec::new();
    // year/month/day/file — four levels below the root.
    recent_jsonl(&root, 5, &mut files);
    files.sort();
    for f in &files {
        let Ok(text) = fs::read_to_string(f) else {
            continue;
        };
        if let Some(s) = parse_codex(&stem(f), &text) {
            p.sessions.push(s);
        }
        let l = codex_limits(&text);
        if !l.is_empty() {
            // Files are sorted by name, which for rollout-<ISO>-<uuid> is
            // chronological, so the last non-empty reading wins.
            p.limits = l;
        }
    }
    p
}

fn collect_souveraine() -> Provider {
    let root = home().join(".souveraine/server/agents");
    let mut p = Provider {
        id: "souveraine".into(),
        available: root.is_dir(),
        error: None,
        sessions: Vec::new(),
        limits: Vec::new(),
    };
    if !p.available {
        p.error = Some("no ~/.souveraine/server/agents".into());
        return p;
    }
    let Ok(agents) = fs::read_dir(&root) else {
        p.available = false;
        p.error = Some("unreadable".into());
        return p;
    };
    for agent in agents.flatten() {
        let convs = agent.path().join("conversations");
        if !convs.is_dir() {
            continue;
        }
        let agent_label = agent.file_name().to_string_lossy().to_string();
        let mut files = Vec::new();
        recent_jsonl(&convs, 2, &mut files);
        for f in files {
            let Ok(text) = fs::read_to_string(&f) else {
                continue;
            };
            // The conversation id is the directory, not the file — every file
            // here is called messages.jsonl.
            let conv = f
                .parent()
                .map(|d| {
                    d.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .unwrap_or_default();
            if let Some(s) = parse_souveraine(&conv, &agent_label, &text) {
                p.sessions.push(s);
            }
        }
    }
    p
}

/// Apply the clock and the ordering. Kept separate from parsing so the ranking
/// policy can be tested against a fixed `now` instead of against real time.
pub fn finish(providers: &mut [Provider], now: u64) {
    for p in providers.iter_mut() {
        for s in p.sessions.iter_mut() {
            let age = Duration::from_millis(now.saturating_sub(s.last_activity_ms));
            s.state = SessionState::from_age(age);
        }
        p.sessions.sort_by(|a, b| {
            a.state
                .rank()
                .cmp(&b.state.rank())
                .then(b.last_activity_ms.cmp(&a.last_activity_ms))
        });
    }
}

pub fn snapshot() -> Envelope {
    let mut providers = vec![collect_claude(), collect_codex(), collect_souveraine()];
    let now = now_ms();
    finish(&mut providers, now);
    Envelope {
        v: WIRE_VERSION,
        generated_at_ms: now,
        providers,
    }
}

// ---------------------------------------------------------------------------
// The socket.
// ---------------------------------------------------------------------------

fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // Safe fallback rather than a panic: a daemon that refuses to start
        // because an env var is missing is a daemon that silently never runs.
        format!("/run/user/{}", unsafe { libc_getuid() })
    });
    PathBuf::from(dir).join("souveraine-sessions.sock")
}

/// The one libc call needed, declared locally rather than pulling a dependency
/// in for a single uid lookup.
unsafe fn libc_getuid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

fn serve_client(mut stream: UnixStream, latest: Arc<RwLock<String>>) {
    // Push immediately so a surface renders on connect rather than after the
    // first tick — a bar that is blank for five seconds after login reads as
    // broken.
    let mut last_sent = String::new();
    loop {
        let payload = match latest.read() {
            Ok(g) => g.clone(),
            Err(_) => return,
        };
        if payload != last_sent {
            if stream.write_all(payload.as_bytes()).is_err() {
                return;
            }
            if stream.write_all(b"\n").is_err() {
                return;
            }
            if stream.flush().is_err() {
                return;
            }
            last_sent = payload;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn main() {
    let path = socket_path();
    // A stale socket from an unclean exit refuses bind with EADDRINUSE, which
    // reads exactly like "already running" and is not.
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            eprintln!("souveraine-agentsd: already running at {}", path.display());
            std::process::exit(1);
        }
        let _ = fs::remove_file(&path);
    }

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("souveraine-agentsd: cannot bind {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    eprintln!("souveraine-agentsd: listening on {}", path.display());

    let latest = Arc::new(RwLock::new(
        serde_json::to_string(&snapshot()).unwrap_or_else(|_| "{}".into()),
    ));

    {
        let latest = Arc::clone(&latest);
        std::thread::spawn(move || loop {
            std::thread::sleep(REFRESH);
            let json = match serde_json::to_string(&snapshot()) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("souveraine-agentsd: serialize failed: {e}");
                    continue;
                }
            };
            if let Ok(mut g) = latest.write() {
                *g = json;
            }
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let latest = Arc::clone(&latest);
                std::thread::spawn(move || serve_client(s, latest));
            }
            Err(e) => eprintln!("souveraine-agentsd: accept failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests. Fixtures are trimmed copies of real records, not invented shapes —
// a parser tested against a fixture I made up only proves I am consistent with
// myself.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = r#"
{"type":"user","cwd":"/srv/project","timestamp":"2026-08-11T12:00:00.000Z"}
{"type":"assistant","timestamp":"2026-08-11T12:00:05.000Z","cwd":"/srv/project","sessionId":"abc","message":{"model":"claude-opus-5","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":900}}}
{"type":"assistant","timestamp":"2026-08-11T12:01:00.000Z","cwd":"/srv/project","sessionId":"abc","message":{"model":"<synthetic>","usage":{"input_tokens":200,"output_tokens":20,"cache_creation_input_tokens":0,"cache_read_input_tokens":1000}}}
"#;

    const CODEX: &str = r#"
{"ordinal":1,"type":"session_meta","timestamp":"2026-08-11T11:18:17.000Z","payload":{"cwd":"/srv","id":"019ff167","model_provider":"openai"}}
{"ordinal":2,"type":"event_msg","timestamp":"2026-08-11T11:20:00.000Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":23083358,"cached_input_tokens":22425856,"cache_write_input_tokens":0,"output_tokens":70558,"reasoning_output_tokens":26913,"total_tokens":23153916},"last_token_usage":{"total_tokens":97139}},"rate_limits":{"primary":{"used_percent":13.0,"window_minutes":10080,"resets_at":1787066336},"secondary":null}}}
"#;

    const SOUVERAINE: &str = r#"
{"role":"user","timestamp":"2026-08-11T17:50:00.000000000Z","blocks":[]}
{"role":"assistant","timestamp":"2026-08-11T17:53:04.004516645Z","usage":{"input_tokens":413961,"output_tokens":7177,"cache_creation_input_tokens":60480,"cache_read_input_tokens":353465}}
{"role":"system","timestamp":null,"usage":null}
"#;

    #[test]
    fn claude_sums_usage_and_ignores_the_synthetic_model() {
        let s = parse_claude("abc", CLAUDE).expect("a session");
        assert_eq!(s.provider, "claude");
        assert_eq!(s.label, "souveraine", "label is the cwd basename");
        // The synthetic line still costs tokens, so it counts toward usage —
        // it just must not be mistaken for the model in use.
        assert_eq!(s.model, "claude-opus-5");
        assert_eq!(s.tokens.input, 300);
        assert_eq!(s.tokens.output, 70);
        assert_eq!(s.tokens.cache_write, 10);
        assert_eq!(s.tokens.cache_read, 1900);
        assert_eq!(s.turns, 2);
        assert_eq!(s.tokens.last_turn, 220);
    }

    #[test]
    fn codex_takes_its_own_running_total_rather_than_summing() {
        let s = parse_codex("file-stem", CODEX).expect("a session");
        assert_eq!(s.id, "019ff167", "session_meta id beats the filename");
        assert_eq!(s.cwd, "/srv");
        assert_eq!(s.tokens.input, 23_083_358);
        assert_eq!(s.tokens.reasoning, 26_913);
        assert_eq!(s.tokens.last_turn, 97_139);
    }

    #[test]
    fn codex_rate_limit_seconds_become_milliseconds_at_the_boundary() {
        let l = codex_limits(CODEX);
        assert_eq!(l.len(), 1, "a null secondary window is not a window");
        assert_eq!(l[0].label, "primary");
        assert_eq!(l[0].used_percent, 13.0);
        assert_eq!(l[0].resets_at_ms, 1_787_066_336_000);
    }

    #[test]
    fn souveraine_usage_is_recorded_despite_what_task_69_claimed() {
        let s = parse_souveraine("conv-1", "agent-x", SOUVERAINE).expect("a session");
        assert_eq!(s.tokens.output, 7177);
        assert_eq!(s.tokens.cache_write, 60480);
        assert_eq!(s.turns, 1);
        // A null-usage system line must not be counted as a turn.
        assert_eq!(s.tokens.last_turn, 481_618);
    }

    #[test]
    fn a_transcript_with_no_activity_yields_no_session() {
        assert!(parse_claude("x", "").is_none());
        assert!(parse_codex("x", "not json at all\n").is_none());
        assert!(parse_souveraine("x", "y", "{}").is_none());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let mixed = format!("garbage not json\n{}\n\n", CLAUDE.trim());
        let s = parse_claude("abc", &mixed).expect("still parses the good lines");
        assert_eq!(s.turns, 2);
    }

    #[test]
    fn working_outranks_idle_outranks_stale() {
        let now = 1_000_000_000u64;
        let mk = |id: &str, ago_s: u64| AgentSession {
            id: id.into(),
            provider: "claude".into(),
            label: id.into(),
            cwd: String::new(),
            model: String::new(),
            state: SessionState::Stale,
            last_activity_ms: now - ago_s * 1000,
            turns: 1,
            tokens: Tokens::default(),
        };
        let mut ps = vec![Provider {
            id: "claude".into(),
            available: true,
            error: None,
            sessions: vec![mk("stale", 60 * 60), mk("working", 10), mk("idle", 60 * 10)],
            limits: Vec::new(),
        }];
        finish(&mut ps, now);
        let order: Vec<&str> = ps[0].sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, vec!["working", "idle", "stale"]);
        assert_eq!(ps[0].sessions[0].state, SessionState::Working);
        assert_eq!(ps[0].sessions[2].state, SessionState::Stale);
    }

    #[test]
    fn equally_stale_sessions_fall_back_to_most_recent_first() {
        let now = 1_000_000_000u64;
        let mk = |id: &str, ago_s: u64| AgentSession {
            id: id.into(),
            provider: "codex".into(),
            label: id.into(),
            cwd: String::new(),
            model: String::new(),
            state: SessionState::Stale,
            last_activity_ms: now - ago_s * 1000,
            turns: 1,
            tokens: Tokens::default(),
        };
        let mut ps = vec![Provider {
            id: "codex".into(),
            available: true,
            error: None,
            sessions: vec![mk("older", 20), mk("newer", 5)],
            limits: Vec::new(),
        }];
        finish(&mut ps, now);
        assert_eq!(ps[0].sessions[0].id, "newer");
    }

    #[test]
    fn billed_excludes_cache_reads() {
        let t = Tokens {
            input: 100,
            output: 50,
            cache_read: 900_000,
            cache_write: 10,
            reasoning: 0,
            last_turn: 0,
        };
        assert_eq!(
            t.billed(),
            160,
            "a cheap cached read must not read as spend"
        );
    }

    #[test]
    fn the_envelope_serialises_with_a_version_surfaces_can_check() {
        let env = Envelope {
            v: WIRE_VERSION,
            generated_at_ms: 42,
            providers: vec![Provider {
                id: "claude".into(),
                available: false,
                error: Some("no ~/.claude/projects".into()),
                sessions: Vec::new(),
                limits: Vec::new(),
            }],
        };
        let json = serde_json::to_string(&env).expect("serialises");
        assert!(json.contains("\"v\":1"));
        // An unavailable provider still appears, carrying its reason. Dropping
        // it would make "no Claude installed" and "collector broken"
        // indistinguishable at the surface.
        assert!(json.contains("\"available\":false"));
        assert!(json.contains("no ~/.claude/projects"));
    }
}
