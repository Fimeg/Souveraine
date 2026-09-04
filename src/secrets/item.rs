//! `org.freedesktop.Secret.Item` — one object per stored secret, at
//! `/org/freedesktop/secrets/collection/souveraine/<id>`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use crate::collection::{item_path, Collection, COLLECTION_PATH};
use crate::error::SecretError;
use crate::session::Sessions;
use crate::store::SecretStore;
use crate::types::Secret;

pub struct Item {
    pub id: String,
    pub store: Arc<Mutex<SecretStore>>,
    pub sessions: Arc<Sessions>,
}

/// Emit a Collection signal for this item. Item methods run with the item's
/// own signal emitter; the spec's ItemDeleted/ItemChanged live on the
/// Collection interface, so route through its registered emitter.
async fn emit_on_collection(
    emitter: &SignalEmitter<'_>,
    item_object_path: &str,
    deleted: bool,
) -> zbus::Result<()> {
    let collection = emitter
        .connection()
        .object_server()
        .interface::<_, Collection>(COLLECTION_PATH)
        .await?;
    let path = OwnedObjectPath::try_from(item_object_path)?;
    if deleted {
        Collection::item_deleted(collection.signal_emitter(), path).await
    } else {
        Collection::item_changed(collection.signal_emitter(), path).await
    }
}

#[interface(name = "org.freedesktop.Secret.Item")]
impl Item {
    async fn delete(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<OwnedObjectPath, SecretError> {
        {
            let mut store = self.store.lock().unwrap();
            store
                .delete(&self.id)
                .map_err(|e| SecretError::failed(format!("failed to delete item: {e}")))?;
        }
        let path = item_path(&self.id);
        let _: bool = emitter
            .connection()
            .object_server()
            .remove::<Item, _>(path.as_str())
            .await
            .map_err(|e| SecretError::failed(format!("failed to unregister item object: {e}")))?;
        let _ = emit_on_collection(&emitter, &path, true).await;
        Ok("/".try_into().unwrap())
    }

    async fn get_secret(&self, session: OwnedObjectPath) -> Result<Secret, SecretError> {
        let store = self.store.lock().unwrap();
        let plaintext = store
            .get(&self.id)
            .map_err(|e| SecretError::failed(format!("failed to read item: {e}")))?
            .ok_or_else(|| SecretError::NoSuchObject(self.id.clone()))?;
        let content_type = store
            .content_type(&self.id)
            .unwrap_or_else(|| "text/plain".to_string());

        let (parameters, value) = self
            .sessions
            .seal(&session, &plaintext)
            .ok_or_else(|| SecretError::NoSession(format!("no such session: {session}")))?;

        Ok(Secret {
            session,
            parameters,
            value,
            content_type,
        })
    }

    async fn set_secret(
        &self,
        secret: Secret,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), SecretError> {
        let plaintext = self
            .sessions
            .unseal(&secret.session, &secret.parameters, &secret.value)
            .ok_or_else(|| SecretError::NoSession(format!("no such session: {}", secret.session)))?
            .map_err(|e| SecretError::failed(format!("failed to unseal secret: {e}")))?;

        {
            let mut store = self.store.lock().unwrap();
            let label = store.label(&self.id).unwrap_or_default();
            let attributes = store.attributes(&self.id).unwrap_or_default();
            store
                .set(
                    self.id.clone(),
                    label,
                    attributes,
                    secret.content_type.clone(),
                    &plaintext,
                )
                .map_err(|e| SecretError::failed(format!("failed to store item: {e}")))?;
        }
        let _ = emit_on_collection(&emitter, &item_path(&self.id), false).await;
        Ok(())
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn attributes(&self) -> HashMap<String, String> {
        self.store
            .lock()
            .unwrap()
            .attributes(&self.id)
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn set_attributes(&self, attributes: HashMap<String, String>) -> zbus::Result<()> {
        let mut store = self.store.lock().unwrap();
        let label = store.label(&self.id).unwrap_or_default();
        let content_type = store
            .content_type(&self.id)
            .unwrap_or_else(|| "text/plain".to_string());
        if let Ok(Some(plaintext)) = store.get(&self.id) {
            let _ = store.set(self.id.clone(), label, attributes, content_type, &plaintext);
        }
        Ok(())
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        self.store
            .lock()
            .unwrap()
            .label(&self.id)
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn set_label(&self, value: String) -> zbus::Result<()> {
        let mut store = self.store.lock().unwrap();
        let attributes = store.attributes(&self.id).unwrap_or_default();
        let content_type = store
            .content_type(&self.id)
            .unwrap_or_else(|| "text/plain".to_string());
        if let Ok(Some(plaintext)) = store.get(&self.id) {
            let _ = store.set(self.id.clone(), value, attributes, content_type, &plaintext);
        }
        Ok(())
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        self.store.lock().unwrap().created(&self.id).unwrap_or(0)
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        self.store.lock().unwrap().modified(&self.id).unwrap_or(0)
    }
}
