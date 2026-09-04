//! The root-owned agent → Unix principal mapping.
//!
//! Shared verbatim between the server, which only ever reads it, and the
//! admission executor, which is its only writer. Deliberately free of
//! crate-internal types so the system-tier binary can `#[path]`-include it
//! without dragging in the model tree.
//!
//! Paths are relative and joined onto a root so `--root` can prepare an image
//! or a test tree without touching the live one.

// Two crates include this module and each uses a different half of it: the
// server reads mappings, the executor writes them.
#![allow(dead_code)]

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PRINCIPAL_MAP_DIR: &str = "etc/souveraine/agent-principals.d";
pub const SYSUSERS_DIR: &str = "etc/sysusers.d";
pub const AGENT_STATE_ROOT: &str = "var/lib/souveraine-agents";

/// Names an agent may never be attached to. `souveraine` is machined's
/// machine-tier account and `souveraine-session` is the lock authority's;
/// adopting either would collapse a tier boundary the whole design rests on.
pub const RESERVED_ACCOUNTS: &[&str] = &["root", "souveraine", "souveraine-session"];

/// The lowest uid this executor treats as a human login. An account in this
/// range that is not already mapped to the agent is a collision, never
/// something to adopt.
pub const HUMAN_UID_FLOOR: u32 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodePrincipalMapping {
    pub agent_id: String,
    pub agent_seed_id: String,
    pub account: String,
    pub uid: u32,
    pub node_id: String,
    pub state_root: String,
    pub admitted_at: DateTime<Utc>,
}

pub fn validate_account_name(account: &str) -> anyhow::Result<()> {
    if account.is_empty() {
        anyhow::bail!("dedicated principal requires an account");
    }
    let mut bytes = account.bytes();
    let first_ok = matches!(bytes.next(), Some(b'a'..=b'z' | b'_'));
    let rest_ok = bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));
    let valid = account.len() <= 31 && first_ok && rest_ok;
    if !valid {
        anyhow::bail!("invalid dedicated account `{account}`; use a lowercase Unix account name");
    }
    if RESERVED_ACCOUNTS.contains(&account) {
        anyhow::bail!("account `{account}` is reserved and cannot be an agent principal");
    }
    Ok(())
}

/// State root for an agent, keyed to her ID. Never the display name — a
/// rename must not strand her data or point two agents at one tree.
pub fn agent_state_root(root: &Path, agent_id: &str) -> PathBuf {
    root.join(AGENT_STATE_ROOT).join(agent_id)
}

pub fn node_mapping_path(root: &Path, agent_id: &str) -> PathBuf {
    root.join(PRINCIPAL_MAP_DIR).join(format!("{agent_id}.json"))
}

pub fn sysusers_drop_in_path(root: &Path, agent_id: &str) -> PathBuf {
    root.join(SYSUSERS_DIR)
        .join(format!("souveraine-agent-{agent_id}.conf"))
}

pub fn load_node_mapping(root: &Path, agent_id: &str) -> Option<NodePrincipalMapping> {
    let raw = std::fs::read_to_string(node_mapping_path(root, agent_id)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn all_mappings(root: &Path) -> Vec<NodePrincipalMapping> {
    let Ok(entries) = std::fs::read_dir(root.join(PRINCIPAL_MAP_DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect()
}

/// Which agent, if any, owns this uid. An account belongs to at most one
/// agent: a turn running under a uid mapped to someone else is impersonation,
/// not a degraded version of running as yourself.
pub fn mapping_owner_of_uid(root: &Path, uid: u32) -> Option<String> {
    all_mappings(root)
        .into_iter()
        .find(|m| m.uid == uid)
        .map(|m| m.agent_id)
}

/// Which agent, if any, already holds this account name.
pub fn mapping_owner_of_account(root: &Path, account: &str) -> Option<String> {
    all_mappings(root)
        .into_iter()
        .find(|m| m.account == account)
        .map(|m| m.agent_id)
}

/// Read one account out of a passwd file. Used only when operating on an
/// alternate root, where NSS still answers for the live system.
pub fn passwd_file_lookup(root: &Path, account: &str) -> Option<(String, u32, String, String)> {
    let raw = std::fs::read_to_string(root.join("etc/passwd")).ok()?;
    for line in raw.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[0] == account {
            return Some((
                fields[0].to_string(),
                fields[2].parse().ok()?,
                fields[5].to_string(),
                fields[6].to_string(),
            ));
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct NssAccount {
    pub name: String,
    pub uid: u32,
    pub home: String,
    pub shell: String,
}

/// Resolve an account the way the running system would. On an alternate root
/// NSS still answers for the live machine, so the passwd file is read instead.
pub fn account_lookup(root: &Path, account: &str) -> Option<NssAccount> {
    if root == Path::new("/") {
        nss_account_by_name(account)
    } else {
        passwd_file_lookup(root, account).map(|(name, uid, home, shell)| NssAccount {
            name,
            uid,
            home,
            shell,
        })
    }
}

pub fn nss_account_by_uid(uid: u32) -> Option<NssAccount> {
    let mut passwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; passwd_buffer_size()];
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut passwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    account_from_passwd(rc, result, &passwd)
}

pub fn nss_account_by_name(name: &str) -> Option<NssAccount> {
    let name = CString::new(name).ok()?;
    let mut passwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; passwd_buffer_size()];
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut passwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    account_from_passwd(rc, result, &passwd)
}

fn account_from_passwd(
    rc: libc::c_int,
    result: *mut libc::passwd,
    passwd: &libc::passwd,
) -> Option<NssAccount> {
    if rc != 0 || result.is_null() || passwd.pw_name.is_null() {
        return None;
    }
    Some(NssAccount {
        name: c_field(passwd.pw_name),
        uid: passwd.pw_uid,
        home: c_field(passwd.pw_dir),
        shell: c_field(passwd.pw_shell),
    })
}

fn c_field(value: *const libc::c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

fn passwd_buffer_size() -> usize {
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    if suggested > 0 {
        suggested as usize
    } else {
        16 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_and_malformed_accounts_are_refused() {
        assert!(validate_account_name("annie").is_ok());
        assert!(validate_account_name("agent_1-x").is_ok());
        assert!(validate_account_name("Annie").is_err());
        assert!(validate_account_name("").is_err());
        assert!(validate_account_name("1annie").is_err());
        assert!(validate_account_name("annie;rm").is_err());
        for reserved in RESERVED_ACCOUNTS {
            assert!(validate_account_name(reserved).is_err(), "{reserved}");
        }
    }

    #[test]
    fn state_root_follows_the_id_not_the_name() {
        let root = Path::new("/");
        let by_id = agent_state_root(root, "agent-e2b683bf");
        assert!(by_id.ends_with("agent-e2b683bf"));
        assert!(!by_id.to_string_lossy().contains("Annie"));
    }
}
