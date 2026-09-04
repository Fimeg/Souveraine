//! Tracks negotiated `OpenSession` transports.
//!
//! libsecret always tries the AES algorithm first and falls back to `plain`
//! only if the daemon returns `NotSupported` — so supporting just these two
//! covers every real client.

use std::collections::HashMap;
use std::sync::Mutex;

use zbus::zvariant::OwnedObjectPath;

pub enum SessionCrypto {
    Plain,
    Aes { key: [u8; 16] },
}

pub struct Sessions {
    inner: Mutex<HashMap<OwnedObjectPath, SessionCrypto>>,
    next_id: Mutex<u64>,
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
        }
    }

    pub fn next_path(&self) -> OwnedObjectPath {
        let mut id = self.next_id.lock().unwrap();
        *id += 1;
        format!("/org/freedesktop/secrets/session/s{}", *id)
            .try_into()
            .expect("session path is always a valid object path")
    }

    pub fn insert(&self, path: OwnedObjectPath, crypto: SessionCrypto) {
        self.inner.lock().unwrap().insert(path, crypto);
    }

    pub fn remove(&self, path: &OwnedObjectPath) {
        self.inner.lock().unwrap().remove(path);
    }

    /// Encrypt `plaintext` per the session's negotiated transport, returning
    /// `(parameters, value)` ready to place in a `Secret` struct.
    pub fn seal(&self, path: &OwnedObjectPath, plaintext: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
        let sessions = self.inner.lock().unwrap();
        match sessions.get(path)? {
            SessionCrypto::Plain => Some((Vec::new(), plaintext.to_vec())),
            SessionCrypto::Aes { key } => {
                let (iv, ciphertext) = crate::transport::encrypt(key, plaintext);
                Some((iv, ciphertext))
            }
        }
    }

    /// Decrypt a `Secret`'s `(parameters, value)` per the session's negotiated
    /// transport.
    pub fn unseal(
        &self,
        path: &OwnedObjectPath,
        parameters: &[u8],
        value: &[u8],
    ) -> Option<anyhow::Result<Vec<u8>>> {
        let sessions = self.inner.lock().unwrap();
        match sessions.get(path)? {
            SessionCrypto::Plain => Some(Ok(value.to_vec())),
            SessionCrypto::Aes { key } => Some(crate::transport::decrypt(key, parameters, value)),
        }
    }
}
