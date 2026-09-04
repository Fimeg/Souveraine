//! `org.freedesktop.Secret.Collection` — a single fixed collection at
//! `/org/freedesktop/secrets/collection/souveraine`.
//!
//! Real secret-service daemons support multiple named collections
//! (keyrings). We don't: every secret libsecret clients store goes into one
//! collection backed by `SecretStore`, encrypted under the machine-derived
//! key. Adding real multi-collection support is meaningful added complexity
//! for a feature no caller in this codebase (Chatty, keyring-core, the
//! player, culver) needs — this can grow a real implementation later if
//! something actually requires it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::error::SecretError;
use crate::item::Item;
use crate::session::Sessions;
use crate::store::SecretStore;
use crate::types::Secret;

pub const COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/souveraine";
/// The spec's well-known alias object path — libsecret talks to the default
/// collection through this path directly, so the same interface is served
/// there too.
pub const DEFAULT_ALIAS_PATH: &str = "/org/freedesktop/secrets/aliases/default";
const NULL_PATH: &str = "/";

pub struct Collection {
    pub store: Arc<Mutex<SecretStore>>,
    pub sessions: Arc<Sessions>,
}

#[interface(name = "org.freedesktop.Secret.Collection")]
impl Collection {
    /// Refuse rather than pretend — there is exactly one collection and it
    /// isn't going away while the daemon that backs libsecret is running.
    async fn delete(&self) -> zbus::fdo::Result<OwnedObjectPath> {
        Err(zbus::fdo::Error::NotSupported(
            "the souveraine collection cannot be deleted".into(),
        ))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> zbus::fdo::Result<Vec<OwnedObjectPath>> {
        let store = self.store.lock().unwrap();
        Ok(store
            .search(&attributes)
            .iter()
            .map(|id| item_path(id).try_into().unwrap())
            .collect())
    }

    async fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: Secret,
        replace: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        let label = properties
            .get("org.freedesktop.Secret.Item.Label")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default();

        let attributes: HashMap<String, String> = properties
            .get("org.freedesktop.Secret.Item.Attributes")
            .and_then(|v| HashMap::<String, String>::try_from(v.clone()).ok())
            .unwrap_or_default();

        let plaintext = self
            .sessions
            .unseal(&secret.session, &secret.parameters, &secret.value)
            .ok_or_else(|| SecretError::NoSession(format!("no such session: {}", secret.session)))?
            .map_err(|e| SecretError::failed(format!("failed to unseal secret: {e}")))?;

        let existing_id = if replace {
            let store = self.store.lock().unwrap();
            store.search(&attributes).into_iter().next()
        } else {
            None
        };
        let is_new = existing_id.is_none();
        // Hyphen-less: the id is a D-Bus object path segment, where `-` is
        // an invalid character.
        let id = existing_id.unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        {
            let mut store = self.store.lock().unwrap();
            store
                .set(
                    id.clone(),
                    label,
                    attributes,
                    secret.content_type.clone(),
                    &plaintext,
                )
                .map_err(|e| SecretError::failed(format!("failed to store item: {e}")))?;
        }

        let path = item_path(&id);
        if is_new {
            emitter
                .connection()
                .object_server()
                .at(
                    path.clone(),
                    Item {
                        id: id.clone(),
                        store: self.store.clone(),
                        sessions: self.sessions.clone(),
                    },
                )
                .await
                .map_err(|e| SecretError::failed(format!("failed to register item object: {e}")))?;
        }

        let item: OwnedObjectPath = path.try_into().unwrap();
        if is_new {
            let _ = Self::item_created(&emitter, item.clone()).await;
        } else {
            let _ = Self::item_changed(&emitter, item.clone()).await;
        }

        let prompt: OwnedObjectPath = NULL_PATH.try_into().unwrap();
        Ok((item, prompt))
    }

    #[zbus(signal)]
    pub async fn item_created(
        emitter: &SignalEmitter<'_>,
        item: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn item_deleted(
        emitter: &SignalEmitter<'_>,
        item: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn item_changed(
        emitter: &SignalEmitter<'_>,
        item: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    async fn items(&self) -> Vec<OwnedObjectPath> {
        let store = self.store.lock().unwrap();
        store
            .all_ids()
            .iter()
            .map(|id| item_path(id).try_into().unwrap())
            .collect()
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        "souveraine".to_string()
    }

    #[zbus(property)]
    async fn set_label(&self, _value: String) -> zbus::Result<()> {
        // Single fixed collection — relabeling is a no-op, not an error;
        // clients that just set the label they already read shouldn't fail.
        Ok(())
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        self.store.lock().unwrap().collection_created()
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        self.store.lock().unwrap().collection_modified()
    }
}

pub fn item_path(id: &str) -> String {
    format!("{COLLECTION_PATH}/{id}")
}
