//! Wire types for the Secret Service D-Bus API.

use serde::{Deserialize, Serialize};
use zbus::zvariant::{OwnedObjectPath, Type};

/// The `Secret` struct: `(oayays)`.
///
/// - `session`: the object path of the session this secret was encoded under.
/// - `parameters`: algorithm-specific parameters (the AES IV, for the
///   `dh-ietf1024-sha256-aes128-cbc-pkcs7` algorithm; empty for `plain`).
/// - `value`: the secret's bytes — ciphertext under AES, or plaintext under
///   `plain`.
/// - `content_type`: a MIME type, e.g. `text/plain`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Secret {
    pub session: OwnedObjectPath,
    pub parameters: Vec<u8>,
    pub value: Vec<u8>,
    pub content_type: String,
}
