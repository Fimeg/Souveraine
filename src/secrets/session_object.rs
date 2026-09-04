//! `org.freedesktop.Secret.Session` — the object `OpenSession` hands back.
//! Its only job is `Close`, so a client can tell us to forget the negotiated
//! key instead of it living for the daemon's whole lifetime.

use std::sync::Arc;

use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use crate::error::SecretError;
use crate::session::Sessions;

pub struct Session {
    pub path: OwnedObjectPath,
    pub sessions: Arc<Sessions>,
}

#[interface(name = "org.freedesktop.Secret.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), SecretError> {
        self.sessions.remove(&self.path);
        emitter
            .connection()
            .object_server()
            .remove::<Session, _>(self.path.clone())
            .await
            .map_err(|e| {
                SecretError::failed(format!("failed to unregister session object: {e}"))
            })?;
        Ok(())
    }
}
