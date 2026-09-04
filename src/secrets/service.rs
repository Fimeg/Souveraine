//! `org.freedesktop.Secret.Service` — the root object at
//! `/org/freedesktop/secrets`.
//!
//! Everything is permanently unlocked: the seed identity is resident in this
//! process the whole time it runs, so there is no separate "unlock the
//! keyring" step the way there is for a passphrase-based store. `Unlock`
//! always succeeds immediately with no prompt.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use num_bigint::BigUint;
use tracing::warn;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::dh::DhSession;
use crate::error::SecretError;
use crate::session::{SessionCrypto, Sessions};
use crate::session_object::Session;
use crate::store::SecretStore;
use crate::types::Secret;

const COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/souveraine";
const NULL_PATH: &str = "/";

pub struct SecretService {
    pub store: Arc<Mutex<SecretStore>>,
    pub sessions: Arc<Sessions>,
}

#[interface(name = "org.freedesktop.Secret.Service")]
impl SecretService {
    async fn open_session(
        &self,
        algorithm: String,
        input: OwnedValue,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(OwnedValue, OwnedObjectPath), SecretError> {
        let path = self.sessions.next_path();

        let out = match algorithm.as_str() {
            "plain" => {
                self.sessions.insert(path.clone(), SessionCrypto::Plain);
                OwnedValue::try_from(Value::from("")).unwrap()
            }
            "dh-ietf1024-sha256-aes128-cbc-pkcs7" => {
                let peer_public_bytes: Vec<u8> = Vec::<u8>::try_from(input).map_err(|_| {
                    SecretError::failed(
                        "OpenSession input must be the client's DH public value as bytes",
                    )
                })?;
                let peer_public = BigUint::from_bytes_be(&peer_public_bytes);

                let dh = DhSession::new()
                    .map_err(|e| SecretError::failed(format!("DH session setup failed: {e}")))?;
                let key = dh
                    .derive_session_key(&peer_public)
                    .map_err(|e| SecretError::failed(format!("DH key derivation failed: {e}")))?;

                self.sessions
                    .insert(path.clone(), SessionCrypto::Aes { key });

                let our_public_bytes = dh.public.to_bytes_be();
                OwnedValue::try_from(Value::from(our_public_bytes)).unwrap()
            }
            other => {
                return Err(SecretError::failed(format!(
                    "unsupported OpenSession algorithm: {other}"
                )));
            }
        };

        emitter
            .connection()
            .object_server()
            .at(
                path.clone(),
                Session {
                    path: path.clone(),
                    sessions: self.sessions.clone(),
                },
            )
            .await
            .map_err(|e| SecretError::failed(format!("failed to register session object: {e}")))?;

        Ok((out, path))
    }

    async fn create_collection(
        &self,
        _properties: HashMap<String, OwnedValue>,
        _alias: String,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        // Single fixed collection — see Collection docs for why.
        let path: OwnedObjectPath = COLLECTION_PATH.try_into().unwrap();
        let prompt: OwnedObjectPath = NULL_PATH.try_into().unwrap();
        Ok((path, prompt))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> zbus::fdo::Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)> {
        let store = self.store.lock().unwrap();
        let ids = store.search(&attributes);
        let unlocked: Vec<OwnedObjectPath> = ids
            .iter()
            .map(|id| item_path(id).try_into().unwrap())
            .collect();
        Ok((unlocked, Vec::new()))
    }

    /// Everything is already unlocked (the seed never locks). Report success
    /// immediately with no prompt.
    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> zbus::fdo::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        let prompt: OwnedObjectPath = NULL_PATH.try_into().unwrap();
        Ok((objects, prompt))
    }

    /// There is nothing to lock — refuse the concept rather than pretend.
    async fn lock(
        &self,
        _objects: Vec<OwnedObjectPath>,
    ) -> zbus::fdo::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        let prompt: OwnedObjectPath = NULL_PATH.try_into().unwrap();
        Ok((Vec::new(), prompt))
    }

    async fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
    ) -> Result<HashMap<OwnedObjectPath, Secret>, SecretError> {
        let store = self.store.lock().unwrap();
        let mut out = HashMap::new();

        for item_obj_path in items {
            let Some(id) = id_from_item_path(&item_obj_path) else {
                continue;
            };
            let Ok(Some(plaintext)) = store.get(id) else {
                continue;
            };
            let content_type = store
                .content_type(id)
                .unwrap_or_else(|| "text/plain".to_string());
            let Some((parameters, value)) = self.sessions.seal(&session, &plaintext) else {
                return Err(SecretError::NoSession(format!(
                    "no such session: {session}"
                )));
            };
            out.insert(
                item_obj_path,
                Secret {
                    session: session.clone(),
                    parameters,
                    value,
                    content_type,
                },
            );
        }

        Ok(out)
    }

    // The spec's collection lifecycle signals. With one fixed collection they
    // never fire, but clients that introspect for them must find them.
    #[zbus(signal)]
    pub async fn collection_created(
        emitter: &SignalEmitter<'_>,
        collection: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn collection_deleted(
        emitter: &SignalEmitter<'_>,
        collection: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn collection_changed(
        emitter: &SignalEmitter<'_>,
        collection: OwnedObjectPath,
    ) -> zbus::Result<()>;

    async fn read_alias(&self, name: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if name == "default" {
            Ok(COLLECTION_PATH.try_into().unwrap())
        } else {
            Ok(NULL_PATH.try_into().unwrap())
        }
    }

    async fn set_alias(&self, name: String, collection: OwnedObjectPath) -> zbus::fdo::Result<()> {
        if name != "default" || collection.as_str() != COLLECTION_PATH {
            warn!(%name, %collection, "set_alias: only the 'default' alias on the souveraine collection is supported, ignoring");
        }
        Ok(())
    }

    #[zbus(property)]
    async fn collections(&self) -> Vec<OwnedObjectPath> {
        vec![COLLECTION_PATH.try_into().unwrap()]
    }
}

fn item_path(id: &str) -> String {
    format!("{COLLECTION_PATH}/{id}")
}

fn id_from_item_path<'a>(path: &'a ObjectPath<'_>) -> Option<&'a str> {
    path.as_str().strip_prefix(&format!("{COLLECTION_PATH}/"))
}
