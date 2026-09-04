//! Cadences — the processing positions inside one agent.
//!
//! Primary speaks. Subconscious completes, a moment later. Reflection reviews
//! a window of turns. The Archivist presses lived journal into a record. Four
//! cadences, one being: they share her agent id, her seed, and her principal
//! (`saf/identity/02-agent-principal.md` — "the cadences share her principal
//! unless the human and the system later admit one as an independently
//! authorized agent").
//!
//! What they do *not* share is a memory root or a git author. Before this
//! module, reflection borrowed the subconscious's id and tool context, so its
//! ledger writes were committed as the subconscious and were indistinguishable
//! from a real N+1 pass; the archivist had no identity at all and wrote into
//! the primary's memfs as nobody. Commits are authored by `MemoryRepo`'s agent
//! id, so giving each cadence its own id makes authorship truthful without
//! moving a single file.
//!
//! The suffix is the wire format. It was already load-bearing (`-sub` was
//! tested by hand in three places); this module is the one place that knows it.

use std::fmt;

/// A processing position within one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cadence {
    Primary,
    Subconscious,
    Reflection,
    Archivist,
}

/// Cadences that are created alongside a primary and own a memory tree.
pub const PAIRED: [Cadence; 3] = [
    Cadence::Subconscious,
    Cadence::Reflection,
    Cadence::Archivist,
];

impl Cadence {
    /// Id suffix. Primary has none — its id *is* the agent id.
    pub fn suffix(self) -> &'static str {
        match self {
            Cadence::Primary => "",
            Cadence::Subconscious => "-sub",
            Cadence::Reflection => "-reflect",
            Cadence::Archivist => "-archive",
        }
    }

    /// The `agent_type` string in `agent.json`, and the key
    /// `[compaction.per_type]` is indexed by.
    pub fn type_name(self) -> &'static str {
        match self {
            Cadence::Primary => "primary",
            Cadence::Subconscious => "subconscious",
            Cadence::Reflection => "reflection",
            Cadence::Archivist => "archivist",
        }
    }

    /// Directory name holding this cadence's memory, relative to its agent
    /// dir. The subconscious's is `memory.git` for historical reasons — it is
    /// a working tree despite the name, and renaming it would strand every
    /// existing ledger. New cadences use the honest name.
    pub fn memory_dirname(self) -> &'static str {
        match self {
            Cadence::Primary => "memory",
            Cadence::Subconscious => "memory.git",
            Cadence::Reflection | Cadence::Archivist => "memory",
        }
    }

    /// Build this cadence's agent id from the primary's.
    pub fn id_for(self, primary_id: &str) -> String {
        format!("{primary_id}{}", self.suffix())
    }
}

impl fmt::Display for Cadence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.type_name())
    }
}

/// Split any agent id into its primary id and cadence.
///
/// Longest suffix wins, so a primary whose own name ends in `-sub` is still
/// resolved correctly against the paired cadences below it.
pub fn split(agent_id: &str) -> (&str, Cadence) {
    for cadence in PAIRED {
        if let Some(primary) = agent_id.strip_suffix(cadence.suffix()) {
            return (primary, cadence);
        }
    }
    (agent_id, Cadence::Primary)
}

/// The cadence an agent id names.
pub fn of(agent_id: &str) -> Cadence {
    split(agent_id).1
}

/// The primary this id belongs to — itself, when it is already primary.
pub fn primary_of(agent_id: &str) -> &str {
    split(agent_id).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_every_cadence() {
        assert_eq!(split("abc"), ("abc", Cadence::Primary));
        assert_eq!(split("abc-sub"), ("abc", Cadence::Subconscious));
        assert_eq!(split("abc-reflect"), ("abc", Cadence::Reflection));
        assert_eq!(split("abc-archive"), ("abc", Cadence::Archivist));
    }

    #[test]
    fn round_trips_through_id_for() {
        for cadence in PAIRED {
            let id = cadence.id_for("agent-1234");
            assert_eq!(split(&id), ("agent-1234", cadence));
        }
    }

    #[test]
    fn primary_id_ending_in_a_suffix_word_is_not_mistaken() {
        // "-substrate" is not "-sub": the suffix test must not match a prefix
        // of the id's own tail. `strip_suffix` gives this for free; the test
        // pins it because the hand-rolled `ends_with("-sub")` it replaces
        // would have had the same property and it is easy to lose.
        assert_eq!(split("my-substrate"), ("my-substrate", Cadence::Primary));
    }

    #[test]
    fn primary_of_is_idempotent() {
        assert_eq!(primary_of(primary_of("abc-reflect")), "abc");
    }
}
