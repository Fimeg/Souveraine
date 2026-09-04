//! souveraine-machined — system-tier machine identity daemon.
//!
//! Owns the machine's Ed25519 seed at `/var/lib/souveraine/seed-id` and
//! serves pubkey/sign requests over `/run/souveraine/machined.sock`. The
//! private key never crosses the socket; user-tier processes (agents, the
//! federation bridge, the shell) get signatures, not key material.
//!
//! The seed is a precondition, never something this daemon creates. If no
//! seed exists, startup fails loudly — identity creation is a deliberate,
//! guarded action that lives in `souveraine machine init`, not something a
//! background service does on your behalf. Same doctrine as
//! `souveraine-secrets`; see `SeedId::load` in `core/identity/seed.rs`.

#[path = "../core/identity/seed.rs"]
mod identity;
#[path = "../machined/protocol.rs"]
mod protocol;
#[path = "../machined/server.rs"]
mod server;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut seed_dir = PathBuf::from(protocol::DEFAULT_SEED_DIR);
    let mut socket = PathBuf::from(protocol::DEFAULT_SOCKET_PATH);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed-dir" => {
                seed_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--seed-dir requires a path"))?,
                );
            }
            "--socket" => {
                socket = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--socket requires a path"))?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "souveraine-machined [--seed-dir {}] [--socket {}]",
                    protocol::DEFAULT_SEED_DIR,
                    protocol::DEFAULT_SOCKET_PATH
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let seed = identity::SeedId::load(&seed_dir).map_err(|e| {
        anyhow::anyhow!(
            "{e:#}\n\nsouveraine-machined refuses to start without a machine identity.\n\
             Provision one: sudo souveraine machine init --fresh\n\
             Or carry an existing identity over: sudo souveraine machine init \
             --migrate-from ~/.souveraine/seed-id"
        )
    })?;

    server::run(seed, &socket)
}
