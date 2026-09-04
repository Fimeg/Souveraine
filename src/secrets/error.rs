//! Secret Service D-Bus error names.
//!
//! Reference: <https://specifications.freedesktop.org/secret-service/latest/errors.html>
//! Only three custom errors are defined by the spec; anything else maps to
//! a generic `org.freedesktop.DBus.Error.Failed`.

use zbus::DBusError;

#[derive(Debug, DBusError)]
#[zbus(prefix = "org.freedesktop.Secret.Error")]
pub enum SecretError {
    /// The object must be unlocked before this action can be carried out.
    #[allow(dead_code)]
    IsLocked(String),
    /// The session does not exist.
    NoSession(String),
    /// No such item or collection exists.
    NoSuchObject(String),
    #[zbus(error)]
    ZBus(zbus::Error),
}

impl SecretError {
    /// A generic failure that isn't one of the spec's three named errors —
    /// surfaces to the caller as `org.freedesktop.DBus.Error.Failed`.
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::ZBus(zbus::Error::Failure(msg.into()))
    }
}
