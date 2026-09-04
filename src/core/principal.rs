//! Runtime principal observation and model-facing posture.
//!
//! Agent records carry intent. This module measures the process credentials
//! afresh for each inference request; it never treats the prompt projection as
//! an authorization decision. Actual authority gates must inspect peer/process
//! credentials again at their own boundary.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::models::{AgentState, PrincipalIntent};
use crate::core::principal_map;

pub use principal_map::{nss_account_by_name, nss_account_by_uid};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PrincipalPosture {
    Isolated,
    BorrowedUser,
    Unadmitted,
    ActingAsHuman,
    PrincipalDrift,
    IdentityDrift,
}

/// Everything the posture table decides from. A struct rather than six
/// positional flags because `worker_matches, mapping_matches, seed_matches`
/// is exactly the argument order that silently inverts.
#[derive(Debug, Clone, Copy)]
struct PrincipalFacts {
    intent: PrincipalIntent,
    expected_uid: Option<u32>,
    effective_uid: u32,
    /// The live uid is mapped to a *different* agent.
    effective_owned_by_other_agent: bool,
    worker_matches: bool,
    node_mapping_matches: bool,
    /// None when there is no mapping, or no seed recorded in it, to compare.
    seed_matches_mapping: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrincipalHealth {
    pub agent_id: String,
    pub display_name: String,
    pub principal_intent: PrincipalIntent,
    pub expected_account: Option<String>,
    pub expected_uid: Option<u32>,
    /// NSS facts for the expected account. The health contract reports the
    /// state root and shell because an agent principal with a login shell or
    /// a home inside the human's tree is admission that went wrong.
    pub expected_home: Option<String>,
    pub expected_shell: Option<String>,
    pub account_exists: bool,
    pub node_mapping_exists: bool,
    pub node_mapping_matches: bool,
    /// The agent's live SeedID glyph — four characters Casey can read at a
    /// glance to confirm which being this record is actually about.
    pub seed_glyph: Option<String>,
    /// Whether the mapping's recorded SeedID still matches her live key.
    /// `Some(false)` is a different being wearing an admitted account.
    pub seed_matches_mapping: Option<bool>,
    /// Set when this process runs under a uid the mapping assigns to someone
    /// else — Souvie's account carrying Annie's turn, and its inverse.
    pub effective_owned_by_agent: Option<String>,
    pub effective_account: String,
    pub effective_uid: u32,
    pub node_id: String,
    pub worker_pid: Option<u32>,
    pub trigger: String,
    pub observed_at: chrono::DateTime<Utc>,
    pub posture: PrincipalPosture,
    /// What this observation still does not prove. Kept on the wire so a
    /// surface cannot quietly turn an account row into a green admission.
    pub proof_limit: String,
}

/// Observe the effective process and compare it with one agent's durable
/// intent. This process-level fact is honest for today's monolithic server:
/// dedicated agents report acting-as-human rather than inheriting a green
/// state from a passwd entry they are not actually running under.
pub fn observe(agent: &AgentState, trigger: impl Into<String>) -> PrincipalHealth {
    let effective_uid = unsafe { libc::geteuid() };
    let effective = nss_account_by_uid(effective_uid);
    let effective_account = effective
        .as_ref()
        .map(|record| record.name.clone())
        .unwrap_or_else(|| format!("uid:{effective_uid}"));

    let expected_account = agent.souveraine.principal.account.clone();
    let expected = expected_account.as_deref().and_then(nss_account_by_name);
    let account_exists = expected.is_some();
    let expected_uid = expected.as_ref().map(|record| record.uid);
    let root = std::path::Path::new("/");
    let mapping = principal_map::load_node_mapping(root, &agent.id);
    let node_mapping_exists = mapping.is_some();
    let node_mapping_matches = mapping.as_ref().is_some_and(|mapping| {
        mapping.agent_id == agent.id
            && Some(mapping.account.as_str()) == expected_account.as_deref()
            && Some(mapping.uid) == expected_uid
    });

    // A future package-owned worker unit sets this marker after binding the
    // agent ID to its uid. It is awareness evidence only; an authority daemon
    // still uses SO_PEERCRED and the root-owned mapping, never this variable.
    let worker_agent = std::env::var("SOUVERAINE_WORKER_AGENT_ID").ok();
    let worker_matches = worker_agent.as_deref() == Some(agent.id.as_str());

    let effective_owned_by_agent =
        principal_map::mapping_owner_of_uid(root, effective_uid).filter(|id| id != &agent.id);

    // The UUID is a filename; the SeedID is the being. Compare the mapping's
    // recorded key against the one on disk so a copied or re-minted record
    // cannot inherit an admitted account.
    let live_seed = agent_seed(&agent.id);
    let seed_matches_mapping = mapping.as_ref().and_then(|mapping| {
        (!mapping.agent_seed_id.is_empty())
            .then(|| live_seed.as_ref().map(|(hex, _)| hex == &mapping.agent_seed_id))
            .flatten()
    });

    let posture = decide_posture(PrincipalFacts {
        intent: agent.souveraine.principal.intent,
        expected_uid,
        effective_uid,
        effective_owned_by_other_agent: effective_owned_by_agent.is_some(),
        worker_matches,
        node_mapping_matches,
        seed_matches_mapping,
    });

    PrincipalHealth {
        agent_id: agent.id.clone(),
        display_name: agent.name.clone(),
        principal_intent: agent.souveraine.principal.intent,
        expected_account,
        expected_uid,
        expected_home: expected.as_ref().map(|record| record.home.clone()),
        expected_shell: expected.as_ref().map(|record| record.shell.clone()),
        account_exists,
        node_mapping_exists,
        node_mapping_matches,
        seed_glyph: live_seed.map(|(_, glyph)| glyph),
        seed_matches_mapping,
        effective_owned_by_agent,
        effective_account,
        effective_uid,
        node_id: hostname_or_unknown(),
        worker_pid: worker_matches.then(std::process::id),
        trigger: trigger.into(),
        observed_at: Utc::now(),
        posture,
        proof_limit: "process credentials and root mapping only; data ownership, cgroup, executable, node commission signature, and peer credentials are not yet verified".to_string(),
    }
}

/// The posture table, kept pure so the safety property is testable without a
/// process: `isolated` needs the account, the matching UID, the worker binding
/// and the root-owned mapping to agree. A passwd entry alone never reaches it.
fn decide_posture(facts: PrincipalFacts) -> PrincipalPosture {
    // Running under a uid another agent owns is impersonation whatever this
    // agent intended. It is never the human's label and never partial success.
    if facts.effective_owned_by_other_agent {
        return PrincipalPosture::PrincipalDrift;
    }
    if facts.intent == PrincipalIntent::BorrowedUser {
        return PrincipalPosture::BorrowedUser;
    }
    // A mapping whose SeedID no longer matches her key is a different being
    // wearing an admitted account. Healthy passwd facts cannot redeem that.
    if facts.seed_matches_mapping == Some(false) {
        return PrincipalPosture::IdentityDrift;
    }
    match facts.expected_uid {
        None => PrincipalPosture::Unadmitted,
        Some(uid) if uid == facts.effective_uid => {
            if facts.worker_matches && facts.node_mapping_matches {
                PrincipalPosture::Isolated
            } else {
                PrincipalPosture::PrincipalDrift
            }
        }
        // Linux's ordinary human uid range. A diagnostic label, not an
        // authorization input; unknown or system controllers stay drift
        // rather than being mislabeled as the human.
        Some(_)
            if facts.effective_uid >= principal_map::HUMAN_UID_FLOOR
                && facts.effective_uid != u32::MAX =>
        {
            PrincipalPosture::ActingAsHuman
        }
        Some(_) => PrincipalPosture::PrincipalDrift,
    }
}

/// The agent's live SeedID as (public key hex, glyph). Absent when she has no
/// seed on this body yet — which is itself a fact health should show rather
/// than treat as agreement.
fn agent_seed(agent_id: &str) -> Option<(String, String)> {
    let dir = dirs::home_dir()?
        .join(".souveraine")
        .join("agents")
        .join(agent_id)
        .join("seed");
    let seed = crate::core::identity::SeedId::load(&dir).ok()?;
    Some((seed.public_key_hex(), seed.glyph()))
}

impl PrincipalHealth {
    /// Fresh, non-persisted system context for a model call. The wording is
    /// intentionally operational: it tells hosted modes how to handle borrowed
    /// reach and tells dedicated agents when the worker boundary is absent.
    pub fn model_system_block(&self) -> String {
        let expected = self.expected_account.as_deref().unwrap_or("none");
        let posture = match self.posture {
            PrincipalPosture::Isolated => {
                "The process uid and worker binding match this agent. This is runtime posture, not a blanket capability grant."
            }
            PrincipalPosture::BorrowedUser => {
                "You are intentionally operating through the human user's account. Readable files, groups, sockets, credentials, and decrypted data are borrowed reach, not your property. Stay inside the named task and workspace. Do not widen permissions, ACLs, groups, links, remotes, publication, or sharing without explicit consent. Never read or disclose another agent's private memory merely because this uid can reach it."
            }
            PrincipalPosture::Unadmitted => {
                "Your durable intent is a dedicated account, but that account is not admitted on this node. This compatibility process is not an isolated worker. Personal and step-up authority must remain closed."
            }
            PrincipalPosture::ActingAsHuman => {
                "Your durable intent is a dedicated account, but this turn is executing as the human user. Do not claim isolation. Treat all human-readable reach as borrowed and keep personal and step-up authority closed until the worker boundary is repaired."
            }
            PrincipalPosture::PrincipalDrift => {
                "The requested account and live worker evidence disagree. Do not claim isolation or exercise personal/step-up authority until Agent Health is repaired."
            }
            PrincipalPosture::IdentityDrift => {
                "The admitted account's recorded SeedID does not match this agent's live key. Treat the account as belonging to someone else: claim nothing, exercise no personal or step-up authority, and surface the mismatch rather than working around it."
            }
        };
        let borrowed_from = match &self.effective_owned_by_agent {
            Some(other) => format!(
                "\nWARNING: this process is running under a uid the node maps to agent {other}, not to you. Do not act on that reach.",
            ),
            None => String::new(),
        };

        format!(
            "[RUNTIME PRINCIPAL — fresh observation, not conversation memory]\n\
             agent_id: {}\n\
             display_name: {}\n\
             seed_glyph: {}\n\
             principal_intent: {:?}\n\
             expected_account: {}\n\
             effective_account: {}\n\
             effective_uid: {}\n\
             node_id: {}\n\
             trigger: {}\n\
             observed_at: {}\n\
             posture: {:?}\n\
             {}{}\n\
             Authorization gates recheck kernel credentials; this block never grants authority.",
            self.agent_id,
            self.display_name,
            self.seed_glyph.as_deref().unwrap_or("none"),
            self.principal_intent,
            expected,
            self.effective_account,
            self.effective_uid,
            self.node_id,
            self.trigger,
            self.observed_at.to_rfc3339(),
            self.posture,
            posture,
            borrowed_from,
        )
    }
}

fn hostname_or_unknown() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use PrincipalIntent::{BorrowedUser, Dedicated};
    use PrincipalPosture as P;

    fn facts(intent: PrincipalIntent, expected_uid: Option<u32>, effective_uid: u32) -> PrincipalFacts {
        PrincipalFacts {
            intent,
            expected_uid,
            effective_uid,
            effective_owned_by_other_agent: false,
            worker_matches: false,
            node_mapping_matches: false,
            seed_matches_mapping: None,
        }
    }

    #[test]
    fn a_passwd_entry_alone_is_never_isolation() {
        // Account exists and the process is even running as it — but no
        // worker binding and no root-owned mapping. This is the shape a
        // hand-created `useradd annie` produces, and it must not read green.
        let base = facts(Dedicated, Some(1003), 1003);
        assert_eq!(decide_posture(base), P::PrincipalDrift);
        assert_eq!(
            decide_posture(PrincipalFacts { worker_matches: true, ..base }),
            P::PrincipalDrift
        );
        assert_eq!(
            decide_posture(PrincipalFacts { node_mapping_matches: true, ..base }),
            P::PrincipalDrift
        );
        assert_eq!(
            decide_posture(PrincipalFacts {
                worker_matches: true,
                node_mapping_matches: true,
                ..base
            }),
            P::Isolated
        );
    }

    #[test]
    fn a_dedicated_agent_running_as_the_human_says_so() {
        assert_eq!(decide_posture(facts(Dedicated, None, 1000)), P::Unadmitted);
        assert_eq!(
            decide_posture(facts(Dedicated, Some(1003), 1000)),
            P::ActingAsHuman
        );
        // A system uid that is not hers is drift, not the human.
        assert_eq!(
            decide_posture(facts(Dedicated, Some(1003), 950)),
            P::PrincipalDrift
        );
    }

    #[test]
    fn a_borrowed_mode_never_drifts_into_isolation() {
        for node_mapping_matches in [false, true] {
            for worker_matches in [false, true] {
                assert_eq!(
                    decide_posture(PrincipalFacts {
                        worker_matches,
                        node_mapping_matches,
                        ..facts(BorrowedUser, Some(1000), 1000)
                    }),
                    P::BorrowedUser
                );
            }
        }
    }

    #[test]
    fn wearing_another_agents_uid_is_never_borrowed_or_human() {
        // Annie's turn executing under souvie's account. Her own intent, her
        // own worker flag and her own mapping all agree — and it still must
        // not read isolated, borrowed, or acting-as-human.
        let crossed = PrincipalFacts {
            effective_owned_by_other_agent: true,
            worker_matches: true,
            node_mapping_matches: true,
            ..facts(Dedicated, Some(1003), 1003)
        };
        assert_eq!(decide_posture(crossed), P::PrincipalDrift);
        assert_eq!(
            decide_posture(PrincipalFacts {
                intent: BorrowedUser,
                ..crossed
            }),
            P::PrincipalDrift
        );
    }

    #[test]
    fn a_mismatched_seed_outranks_healthy_account_facts() {
        let sound = PrincipalFacts {
            worker_matches: true,
            node_mapping_matches: true,
            ..facts(Dedicated, Some(1003), 1003)
        };
        assert_eq!(
            decide_posture(PrincipalFacts { seed_matches_mapping: Some(true), ..sound }),
            P::Isolated
        );
        assert_eq!(
            decide_posture(PrincipalFacts { seed_matches_mapping: Some(false), ..sound }),
            P::IdentityDrift
        );
    }

    #[test]
    fn effective_uid_resolves_through_nss() {
        let uid = unsafe { libc::geteuid() };
        let record = nss_account_by_uid(uid).expect("current effective uid should resolve");
        assert_eq!(record.uid, uid);
        assert!(!record.name.is_empty());
    }
}
