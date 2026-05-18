//! Matrix transport — `matrix-sdk` client construction, session
//! persistence, and the sync loop.
//!
//! This is the Rust equivalent of letta-code's `matrix/client.ts`, but
//! almost none of that file survives the port. His `client.ts` is a
//! transport *shim*: an undici dispatcher and a fetch-backed request
//! function that work around Bun's socket pooling and `matrix-bot-sdk`'s
//! deprecated `request` library. `matrix-sdk` owns its own HTTP transport,
//! so all of that pain is simply gone here. What remains — and what this
//! file actually does — is the genuine work: build a client against a
//! homeserver, restore or establish a session, and drive `/sync`.
//!
//! Credentials live next to an encrypted SQLite store under
//! `~/.souveraine/sensorium/matrix/<account>/`. The store holds crypto
//! keys and room state; the session JSON holds the access token. Phase 6
//! moves the access token and the store passphrase into the OS keyring
//! (`src/core/credentials.rs`); until then they sit in the account dir.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use matrix_sdk::{
    authentication::matrix::MatrixSession, config::SyncSettings, Client,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// A persisted Matrix session — everything needed to reconstruct a client
/// across restarts without a fresh login. Written to `session.json` in the
/// account directory after the first successful login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixSessionRecord {
    /// Homeserver base URL, e.g. `https://matrix.org`.
    pub homeserver: String,
    /// Path to the encrypted SQLite store (crypto keys + room state).
    pub db_path: PathBuf,
    /// The logged-in user session — access token, device id, user id.
    pub session: MatrixSession,
    /// Last `/sync` batch token. Lets a restart resume warm instead of
    /// re-syncing the whole account cold.
    #[serde(default)]
    pub sync_token: Option<String>,
}

/// How a [`build_client`] call should obtain its session.
pub enum MatrixAuth {
    /// Restore a session saved by a previous run. The common path.
    Restore(MatrixSessionRecord),
    /// First-time login with username + password. Run once; the resulting
    /// session is persisted so subsequent runs take the `Restore` path.
    Password {
        homeserver: String,
        user_id: String,
        password: String,
        device_name: String,
    },
}

/// The directory where Matrix state lives for a given account.
///
/// `account` is a filesystem-safe slug — typically the localpart of the
/// Matrix user id. Each account gets its own store so several identities
/// can coexist.
pub fn account_dir(store_root: &Path, account: &str) -> PathBuf {
    store_root.join("sensorium").join("matrix").join(account)
}

/// Load a persisted session record from an account directory, if one exists.
pub fn load_session_record(account_dir: &Path) -> Option<MatrixSessionRecord> {
    let path = account_dir.join("session.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Persist a session record to its account directory.
pub fn save_session_record(account_dir: &Path, record: &MatrixSessionRecord) -> Result<()> {
    std::fs::create_dir_all(account_dir)
        .with_context(|| format!("creating matrix account dir {}", account_dir.display()))?;
    let path = account_dir.join("session.json");
    let raw = serde_json::to_string_pretty(record).context("serializing matrix session")?;
    std::fs::write(&path, raw)
        .with_context(|| format!("writing matrix session to {}", path.display()))?;
    debug!("matrix: session record saved to {}", path.display());
    Ok(())
}

/// Build a `matrix-sdk` [`Client`] and bring it to a logged-in state.
///
/// Returns the live client alongside a fresh [`MatrixSessionRecord`] — the
/// caller persists that record so the next run can take the `Restore`
/// branch. For `Restore`, the returned record echoes the input.
pub async fn build_client(
    auth: MatrixAuth,
    account_dir: &Path,
) -> Result<(Client, MatrixSessionRecord)> {
    std::fs::create_dir_all(account_dir)
        .with_context(|| format!("creating matrix account dir {}", account_dir.display()))?;
    let db_path = account_dir.join("store.sqlite");

    match auth {
        MatrixAuth::Restore(record) => {
            info!(
                "matrix: restoring session for {} against {}",
                record.session.meta.user_id, record.homeserver
            );
            // Phase 3: in-memory store (no sqlite). Phase 6 resolves the
            // libsqlite3-sys conflict and adds `sqlite_store()` back.
            #[allow(deprecated)]
            let client = Client::builder()
                .homeserver_url(&record.homeserver)
                .build()
                .await
                .context("building matrix client (restore)")?;

            // NOTE (matrix-sdk 0.17 verification point): top-level
            // `Client::restore_session` auto-detects the auth kind. If a
            // future SDK bump moves this, the equivalent is
            // `client.matrix_auth().restore_session(record.session.clone())`.
            client
                .restore_session(record.session.clone())
                .await
                .context("restoring matrix session")?;

            Ok((client, record))
        }
        MatrixAuth::Password {
            homeserver,
            user_id,
            password,
            device_name,
        } => {
            info!("matrix: fresh login for {user_id} against {homeserver}");
            // Phase 3: in-memory store (no sqlite). See Restore branch note.
            #[allow(deprecated)]
            let client = Client::builder()
                .homeserver_url(&homeserver)
                .build()
                .await
                .context("building matrix client (login)")?;

            client
                .matrix_auth()
                .login_username(&user_id, &password)
                .initial_device_display_name(&device_name)
                .await
                .context("matrix password login failed")?;

            let session = client
                .matrix_auth()
                .session()
                .context("matrix client has no session after login")?;

            let record = MatrixSessionRecord {
                homeserver,
                db_path,
                session,
                sync_token: None,
            };
            save_session_record(account_dir, &record)?;
            Ok((client, record))
        }
    }
}

/// Drive `/sync` until cancelled.
///
/// Event handlers must be registered on `client` *before* this is called —
/// `matrix-sdk` dispatches inbound events on the task running the sync.
/// Does an initial [`Client::sync_once`] to catch up, then enters the
/// continuous loop. Returns `Ok(())` when `cancel` fires; `Err` if the
/// homeserver connection fails unrecoverably.
pub async fn sync_forever(client: Client, cancel: CancellationToken) -> Result<()> {
    let response = client
        .sync_once(SyncSettings::default())
        .await
        .context("matrix initial sync failed")?;
    debug!("matrix: initial sync complete");

    let settings = SyncSettings::default().token(response.next_batch);
    tokio::select! {
        _ = cancel.cancelled() => {
            debug!("matrix: sync loop cancelled");
            Ok(())
        }
        res = client.sync(settings) => {
            res.context("matrix sync loop ended unexpectedly")
        }
    }
}
