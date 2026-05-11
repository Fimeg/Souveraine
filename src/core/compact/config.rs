use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level compaction configuration, mirrors [compaction] in souveraine.toml.
///
/// Per-agent-type overrides use AgentType variant names as keys:
/// "primary", "subconscious", "subagent".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Default strategy for all agent types (overridden by per_type).
    #[serde(default = "default_strategy")]
    pub strategy: CompactionStrategyKind,
    /// Pressure threshold for tier-1 (warn) advisory.
    #[serde(default = "default_warn_pressure")]
    pub warn_pressure: f32,
    /// Pressure threshold for tier-2 (urgent) advisory.
    #[serde(default = "default_urgent_pressure")]
    pub urgent_pressure: f32,
    /// Pressure threshold for tier-3 (critical) advisory.
    #[serde(default = "default_critical_pressure")]
    pub critical_pressure: f32,
    /// Per-agent-type overrides (keys: "primary", "subconscious", "subagent").
    #[serde(default)]
    pub per_type: HashMap<String, AgentCompactionConfig>,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strategy: CompactionStrategyKind::Cull,
            warn_pressure: 0.80,
            urgent_pressure: 0.90,
            critical_pressure: 0.95,
            per_type: HashMap::new(),
        }
    }
}

impl CompactionConfig {
    /// Resolve the effective config for a given agent type.
    pub fn for_agent_type(&self, agent_type: &str) -> AgentCompactionConfig {
        self.per_type
            .get(agent_type)
            .cloned()
            .unwrap_or_else(|| AgentCompactionConfig {
                enabled: self.enabled,
                strategy: self.strategy.clone(),
                warn_pressure: self.warn_pressure,
                urgent_pressure: self.urgent_pressure,
                critical_pressure: self.critical_pressure,
                ..Default::default()
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCompactionConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_strategy")]
    pub strategy: CompactionStrategyKind,
    /// Pressure threshold for tier-1 (warn) advisory.
    #[serde(default = "default_warn_pressure")]
    pub warn_pressure: f32,
    /// Pressure threshold for tier-2 (urgent) advisory.
    #[serde(default = "default_urgent_pressure")]
    pub urgent_pressure: f32,
    /// Pressure threshold for tier-3 (critical) advisory.
    #[serde(default = "default_critical_pressure")]
    pub critical_pressure: f32,
    /// Max summary length in chars (Summary strategy). Generous default;
/// the model's output limit is the real bound, this is an upper guard.
    #[serde(default = "default_max_summary")]
    pub max_summary_length: usize,
    /// Target KV pair count (KeyValue strategy).
    #[serde(default = "default_kv_target")]
    pub kv_target: usize,
    /// Min messages before compaction can run.
    #[serde(default = "default_min_messages")]
    pub min_messages: usize,
}

impl Default for AgentCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strategy: CompactionStrategyKind::Cull,
            warn_pressure: 0.80,
            urgent_pressure: 0.90,
            critical_pressure: 0.95,
            max_summary_length: 2048,
            kv_target: 8,
            min_messages: 20,
        }
    }
}

/// Available compaction strategies. Synthesized from OpenHarness
/// (port of Claude Code's microCompact.ts / autoCompact.ts), hermes-agent,
/// claw-open, and jcode. See `docs/tasks/compaction-rebuild.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategyKind {
    /// Cheap pre-pass: replace old tool result contents with a placeholder,
    /// keeping recent tool results intact. No LLM. From OpenHarness/Claude
    /// Code microCompact.ts. The first response to context pressure.
    Microcompact,
    /// Keep system + last N messages, drop the middle. No LLM. Fast.
    /// Tool-pair aware: never splits a tool call from its result.
    SlidingWindow,
    /// LLM-based structured summarization of oldest messages, producing a
    /// 9-section boundary message (from OpenHarness/Claude Code autoCompact.ts).
    Summary,
    /// Drop low-value messages (greetings, acknowledgments). Role-aware:
    /// never drops System or Tool messages or tool-call carriers.
    Cull,
}

impl CompactionStrategyKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "microcompact" | "micro" | "tool_results" => Some(Self::Microcompact),
            "sliding_window" | "sliding-window" | "window" => Some(Self::SlidingWindow),
            "summarize" | "summary" => Some(Self::Summary),
            "cull" => Some(Self::Cull),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Microcompact => "microcompact",
            Self::SlidingWindow => "sliding_window",
            Self::Summary => "summary",
            Self::Cull => "cull",
        }
    }
}

// ── Default helpers ──

fn default_enabled() -> bool { true }
fn default_strategy() -> CompactionStrategyKind { CompactionStrategyKind::Cull }
fn default_warn_pressure() -> f32 { 0.80 }
fn default_urgent_pressure() -> f32 { 0.90 }
fn default_critical_pressure() -> f32 { 0.95 }
fn default_max_summary() -> usize { 32000 }
fn default_kv_target() -> usize { 16 }
fn default_min_messages() -> usize { 10 }
