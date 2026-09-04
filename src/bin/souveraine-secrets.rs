//! Secret Service daemon (`org.freedesktop.secrets`), rooted in the
//! machine identity instead of a passphrase-unlocked keyring. Backs any
//! libsecret client (Chatty/libcmatrix, souveraine-player, culver, our own
//! `keyring-core` credential store) without gnome-keyring or KWallet.
//!
//! Identity is a precondition, never something this daemon creates. The
//! storage key derives from a deterministic machine signature — system tier
//! (souveraine-machined) first, the legacy `~/.souveraine/seed-id` as a loud
//! transitional fallback. If neither exists, startup fails with provisioning
//! instructions (`sudo souveraine machine init --fresh`); identity creation
//! is a deliberate, guarded action, not something a background service does
//! on your behalf.
//!
//! Lock semantics — a deliberate decision, not an omission: the store
//! unlocks with the *user session*, exactly as long as this daemon runs in
//! it. The screen lock does not lock the store — background clients (music
//! streaming, incoming Matrix messages) must keep their credentials while
//! the display is off, the same stance gnome-keyring takes. Per the session
//! authority doctrine, lock state belongs to the session authority; a
//! second hand-tracked copy here would be a shadow that can disagree with
//! it. `Unlock` therefore always succeeds promptless, and `Lock` refuses
//! rather than pretends.

#[path = "../core/identity/seed.rs"]
mod identity;

#[path = "../machined/protocol.rs"]
mod machined_protocol;

#[path = "../secrets/collection.rs"]
mod collection;
#[path = "../secrets/dh.rs"]
mod dh;
#[path = "../secrets/error.rs"]
mod error;
#[path = "../secrets/item.rs"]
mod item;
#[path = "../secrets/manage.rs"]
mod manage;
#[path = "../secrets/service.rs"]
mod service;
#[path = "../secrets/session.rs"]
mod session;
#[path = "../secrets/session_object.rs"]
mod session_object;
#[path = "../secrets/storage_key.rs"]
mod storage_key;
#[path = "../secrets/store.rs"]
mod store;
#[path = "../secrets/transport.rs"]
mod transport;
#[path = "../secrets/types.rs"]
mod types;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing::info;

use collection::Collection;
use item::Item;
use service::SecretService;
use session::Sessions;
use store::SecretStore;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let base = dirs::home_dir()
        .context("resolving home directory")?
        .join(".souveraine");

    let (machine_ikm, source) = storage_key::resolve(&base)
        .context("resolving the machine-rooted key material — refusing to start")?;
    info!(
        ?source,
        "machine key material resolved; the machine identity is the trust root"
    );

    let store_path = store::default_store_path(&base);
    let secret_store = Arc::new(Mutex::new(
        SecretStore::open(machine_ikm, store_path).context("opening secrets store")?,
    ));
    let sessions = Arc::new(Sessions::new());

    // Interfaces are registered via the builder BEFORE the name is claimed —
    // a client whose call races our startup finds the objects already there.
    let connection = zbus::connection::Builder::session()?
        .serve_at(
            "/org/freedesktop/secrets",
            SecretService {
                store: secret_store.clone(),
                sessions: sessions.clone(),
            },
        )?
        .serve_at(
            collection::COLLECTION_PATH,
            Collection {
                store: secret_store.clone(),
                sessions: sessions.clone(),
            },
        )?
        .serve_at(
            collection::DEFAULT_ALIAS_PATH,
            Collection {
                store: secret_store.clone(),
                sessions: sessions.clone(),
            },
        )?
        .serve_at(
            manage::MANAGE_PATH,
            manage::Manage {
                store: secret_store.clone(),
            },
        )?
        .name("org.freedesktop.secrets")?
        .build()
        .await
        .context(
            "connecting to session bus / claiming org.freedesktop.secrets — is another \
             provider (gnome-keyring, ksecretd) still holding the name?",
        )?;

    let existing_ids = secret_store.lock().unwrap().all_ids();
    for id in existing_ids {
        connection
            .object_server()
            .at(
                collection::item_path(&id),
                Item {
                    id,
                    store: secret_store.clone(),
                    sessions: sessions.clone(),
                },
            )
            .await?;
    }

    info!("souveraine-secrets: serving org.freedesktop.secrets on the session bus");
    std::future::pending::<()>().await;
    Ok(())
}
