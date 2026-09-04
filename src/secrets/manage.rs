//! `org.souveraine.Secrets.Manage` — the non-spec management face at
//! `/org/souveraine/secrets`.
//!
//! This is where the storage-encryption design of record plugs the user's
//! *real* lockscreen passphrase into the at-rest hierarchy: the PAM/stepUp
//! side (sessiond or the lock surface — the ingress choice is deliberately
//! still open in `STORAGE-ENCRYPTION.md`) calls `SetPassphrase` after a
//! successful PAM conversation, enrolling or rotating the passphrase wrap of
//! the store key. Rotation re-wraps only; items are never re-encrypted.
//!
//! Same-user session bus only — like the Secret Service itself, the trust
//! boundary is the UID. The passphrase crosses the bus exactly as secrets do
//! under the `plain` transport; callers that hold a negotiated session
//! should prefer it, but the wrap this enrolls is what protects the *disk*,
//! not the bus.

use std::sync::{Arc, Mutex};

use zbus::interface;

use crate::store::SecretStore;

pub const MANAGE_PATH: &str = "/org/souveraine/secrets";

pub struct Manage {
    pub store: Arc<Mutex<SecretStore>>,
}

#[interface(name = "org.souveraine.Secrets.Manage")]
impl Manage {
    /// Enroll or rotate the passphrase wrap. The caller is expected to have
    /// PAM-verified this passphrase — this daemon wraps, it does not
    /// authenticate.
    async fn set_passphrase(&self, passphrase: String) -> zbus::fdo::Result<()> {
        let mut store = self.store.lock().unwrap();
        store
            .set_passphrase(&passphrase)
            .map_err(|e| zbus::fdo::Error::Failed(format!("enrolling passphrase wrap: {e}")))
    }

    /// Whether the passphrase wrap is enrolled — the stepUp side uses this
    /// to know the store still carries a real-password wrap after a
    /// lockscreen password change.
    #[zbus(property)]
    async fn has_passphrase(&self) -> bool {
        self.store.lock().unwrap().has_passphrase()
    }

    /// Verify a candidate passphrase against the enrolled wrap. Lets the
    /// stepUp flow detect drift between the lockscreen credential and the
    /// store wrap (e.g. after a PAM password change) and re-enroll.
    async fn verify_passphrase(&self, passphrase: String) -> zbus::fdo::Result<bool> {
        let store = self.store.lock().unwrap();
        store
            .verify_passphrase(&passphrase)
            .map_err(|e| zbus::fdo::Error::Failed(format!("verifying passphrase wrap: {e}")))
    }
}
