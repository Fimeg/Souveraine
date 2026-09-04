//! souveraine-admit — the admission executor.
//!
//! The only writer of agent Unix principals. It creates or adopts exactly one
//! account through `systemd-sysusers`, writes the root-owned agent→principal
//! mapping, and then reports the passwd facts it actually observes rather than
//! assuming its own command worked.
//!
//! What it deliberately does not do: read a human's home, install a worker
//! unit, move agent data, or recursively chown anything outside the agent's own
//! state root. Those are separate transitions. Until the worker exists, an
//! admitted agent still reads `acting-as-human` — this binary makes that state
//! reachable, it does not make it green.
//!
//! Contract: souveraine/saf/identity/02-agent-principal.md

#[path = "../core/identity/seed.rs"]
mod identity;
#[path = "../core/principal_map.rs"]
mod principal_map;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

use principal_map::*;

#[derive(Parser)]
#[command(
    name = "souveraine-admit",
    about = "Admit an agent to this node as her own Unix principal",
    long_about = "Creates or adopts one Unix account for one agent, writes the root-owned\nagent-to-principal mapping, and reports what NSS says afterwards.\n\nIntent lives in the agent record and is written by the server. Admission is\nthis binary and needs root. Neither one starts a worker: until per-agent\nworkers exist, an admitted agent still reports acting-as-human."
)]
struct Cli {
    /// Operate on an alternate filesystem root (image prep, tests)
    #[arg(long, global = true, default_value = "/")]
    root: PathBuf,
    /// Speak in data
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the admission facts for one agent. Writes nothing.
    Status { agent_id: String },
    /// Create or adopt the account and write the mapping. Idempotent.
    Apply {
        agent_id: String,
        /// Local account this agent runs as
        #[arg(long)]
        account: String,
        /// The agent's SeedID public key (hex). Recorded so a copied record
        /// cannot inherit an admitted account.
        #[arg(long, default_value = "")]
        seed_id: String,
        /// Print the plan and touch nothing
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Status { agent_id } => status(&cli, agent_id),
        Command::Apply {
            agent_id,
            account,
            seed_id,
            dry_run,
        } => apply(&cli, agent_id, account, seed_id, *dry_run),
    }
}

fn status(cli: &Cli, agent_id: &str) -> Result<()> {
    let mapping = load_node_mapping(&cli.root, agent_id);
    let account = mapping
        .as_ref()
        .and_then(|m| account_lookup(&cli.root, &m.account));

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "agent_id": agent_id,
                "mapping": mapping,
                "account_exists": account.is_some(),
                "account_uid": account.as_ref().map(|a| a.uid),
                "account_shell": account.as_ref().map(|a| a.shell.clone()),
                "worker_installed": false,
            }))?
        );
        return Ok(());
    }

    match (&mapping, &account) {
        (None, _) => println!("unadmitted — no mapping for {agent_id} on this node"),
        (Some(m), None) => println!(
            "principal-drift — mapped to `{}` (uid {}), which does not exist here",
            m.account, m.uid
        ),
        (Some(m), Some(a)) if a.uid != m.uid => println!(
            "principal-drift — mapping says uid {} but `{}` is uid {}",
            m.uid, m.account, a.uid
        ),
        (Some(m), Some(a)) => {
            println!("admitted    {} → {} (uid {})", agent_id, m.account, a.uid);
            println!("state root  {}", m.state_root);
            println!("shell       {}", a.shell);
            println!("seed        {}", glyph_or_none(&m.agent_seed_id));
            println!("worker      none — turns still run as the invoking user");
        }
    }
    Ok(())
}

fn apply(cli: &Cli, agent_id: &str, account: &str, seed_id: &str, dry_run: bool) -> Result<()> {
    let alternate_root = cli.root != Path::new("/");
    if !alternate_root && unsafe { libc::geteuid() } != 0 && !dry_run {
        bail!("admission needs root; re-run with sudo, or use --root for an image or test tree");
    }
    validate_account_name(account)?;
    if agent_id.is_empty() || agent_id.contains('/') || agent_id.contains("..") {
        bail!("refusing agent id `{agent_id}`: it becomes a filename");
    }
    if !seed_id.is_empty() && (seed_id.len() != 64 || !seed_id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        bail!("--seed-id must be a 64-character hex public key");
    }

    // An account belongs to at most one agent. Adopting one that another agent
    // already holds is the exact collapse per-agent principals exist to stop.
    if let Some(owner) = mapping_owner_of_account(&cli.root, account) {
        if owner != agent_id {
            bail!("account `{account}` is already admitted to agent {owner}");
        }
    }

    // A pre-existing mapping for a *different* account is a rename, not a
    // repair. Refuse rather than strand the old account and its data.
    let existing = load_node_mapping(&cli.root, agent_id);
    if let Some(prior) = &existing {
        if prior.account != account {
            bail!(
                "agent {agent_id} is already admitted as `{}`; decommission before admitting `{account}`",
                prior.account
            );
        }
        if !prior.agent_seed_id.is_empty() && !seed_id.is_empty() && prior.agent_seed_id != seed_id {
            bail!("mapping records a different SeedID for {agent_id}; this is identity drift, not a repair");
        }
    }

    // Adopting an existing human login would hand an agent a person's account.
    if let Some(found) = account_lookup(&cli.root, account) {
        if found.uid >= HUMAN_UID_FLOOR && existing.is_none() {
            bail!(
                "`{account}` already exists as uid {} — that is a login account, not a free agent principal",
                found.uid
            );
        }
    }

    let state_root = agent_state_root(&cli.root, agent_id);
    let state_root_abs = Path::new("/")
        .join(AGENT_STATE_ROOT)
        .join(agent_id)
        .to_string_lossy()
        .into_owned();
    let drop_in = sysusers_drop_in_path(&cli.root, agent_id);
    let declaration = format!(
        "# Generated by souveraine-admit for agent {agent_id}. Do not hand-edit:\n\
         # the mapping in /{PRINCIPAL_MAP_DIR}/{agent_id}.json is the record of\n\
         # truth and health compares this file against it.\n\
         u {account} - \"Souveraine agent {agent_id}\" {state_root_abs} /usr/bin/nologin\n"
    );

    if dry_run {
        println!("would write {}", drop_in.display());
        print!("{declaration}");
        println!("would run   systemd-sysusers {}", drop_in.display());
        println!("would write {}", node_mapping_path(&cli.root, agent_id).display());
        println!("would own   {} as {account}", state_root.display());
        return Ok(());
    }

    write_file(&drop_in, &declaration, 0o644)?;
    run_sysusers(&cli.root, &drop_in)?;

    // Report what the system says, never what the command intended.
    let created = account_lookup(&cli.root, account).ok_or_else(|| {
        anyhow::anyhow!("systemd-sysusers reported success but `{account}` still does not resolve")
    })?;
    if created.shell != "/usr/bin/nologin" {
        bail!(
            "`{account}` resolved with shell {} — refusing to record a login-capable agent principal",
            created.shell
        );
    }

    let mapping = NodePrincipalMapping {
        agent_id: agent_id.to_string(),
        agent_seed_id: if seed_id.is_empty() {
            existing.as_ref().map(|p| p.agent_seed_id.clone()).unwrap_or_default()
        } else {
            seed_id.to_string()
        },
        account: created.name.clone(),
        uid: created.uid,
        node_id: node_id(&cli.root),
        state_root: state_root_abs,
        admitted_at: existing.as_ref().map(|p| p.admitted_at).unwrap_or_else(Utc::now),
    };
    write_file(
        &node_mapping_path(&cli.root, agent_id),
        &serde_json::to_string_pretty(&mapping)?,
        0o644,
    )?;

    // Ownership is applied inside her own root only. Never a human home, never
    // a recursive pass over ~/.souveraine.
    std::fs::create_dir_all(&state_root)
        .with_context(|| format!("creating {}", state_root.display()))?;
    if unsafe { libc::geteuid() } == 0 {
        chown(&state_root, created.uid)?;
        restrict(&state_root, 0o700)?;
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&mapping)?);
    } else {
        println!("admitted    {} → {} (uid {})", agent_id, mapping.account, mapping.uid);
        println!("state root  {}", mapping.state_root);
        println!("seed        {}", glyph_or_none(&mapping.agent_seed_id));
        println!("worker      none — her turns still run as whoever invokes them");
    }
    Ok(())
}

fn run_sysusers(root: &Path, drop_in: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new("systemd-sysusers");
    if root != Path::new("/") {
        cmd.arg(format!("--root={}", root.display()));
    }
    cmd.arg(drop_in);
    let out = cmd.output().context("running systemd-sysusers")?;
    if !out.status.success() {
        bail!(
            "systemd-sysusers failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn chown(path: &Path, uid: u32) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::chown(c.as_ptr(), uid, uid) } != 0 {
        bail!(
            "chown {} to {uid}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn node_id(root: &Path) -> String {
    std::fs::read_to_string(root.join("etc/hostname"))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn glyph_or_none(seed_hex: &str) -> String {
    if seed_hex.is_empty() {
        return "none recorded".to_string();
    }
    let bytes: Vec<u8> = (0..seed_hex.len().min(4))
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&seed_hex[i..i + 2], 16).ok())
        .collect();
    format!("{}  {}", identity::glyph_from_pubkey(&bytes), &seed_hex[..16])
}
