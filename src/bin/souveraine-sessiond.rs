//! souveraine-sessiond — session authority: lock-before-shell, lock-past-
//! shell-death.
//!
//! Runs on the user tier as the seat owner. Requires nothing from the seed
//! (it signs nothing yet — capability-token verification arrives with the
//! services review); it requires only a Wayland display that speaks
//! ext-session-lock-v1 and a PAM stack for its service name.
//!
//! Startup default is fail-closed: take the session lock immediately. The
//! shell later takes over via the handoff protocol (see
//! `sessiond/protocol.rs`). `--no-initial-lock` exists for bring-up and
//! laptop experiments only.

// The sessiond module tree lives in the main crate source but is compiled
// standalone into this bin, machined-style. Mounting it as `sessiond` keeps
// the modules' own `crate::sessiond::...` paths valid.
#[path = "../sessiond/mod.rs"]
pub(crate) mod sessiond;

// The somatic vocabulary, mounted the way machined mounts `identity/seed.rs`.
// It lives under `core/nervous/` because the agent side shares the types, but
// the *fields* are sessiond's — SOMATIC_NERVOUS_SYSTEM.md resolves open
// question 1 with "sessiond, period", and a second holder of the body's state
// would be the fifth blind actor. `use super::belief` inside plexus resolves
// to this crate root here and to `core::nervous` there, so one file serves
// both without a copy.
#[path = "../core/nervous/belief.rs"]
pub(crate) mod belief;
#[path = "../core/nervous/plexus.rs"]
pub(crate) mod plexus;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut initial_lock = true;
    let mut socket: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                socket =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--socket requires a path")
                    })?));
            }
            "--no-initial-lock" => initial_lock = false,
            "--help" | "-h" => {
                println!(
                    "souveraine-sessiond [--socket $XDG_RUNTIME_DIR/{}] [--no-initial-lock]",
                    sessiond::protocol::SOCKET_RELPATH,
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let socket = match socket {
        Some(p) => p,
        None => sessiond::server::socket_path()?,
    };
    sessiond::server::run(initial_lock, &socket)
}
