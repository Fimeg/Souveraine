use anyhow::Result;
use keyring_core::Entry;

const SERVICE: &str = "souveraine";

pub trait CredentialStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub struct KeyringStore;

impl CredentialStore for KeyringStore {
    fn get(&self, key: &str) -> Option<String> {
        let entry = Entry::new(SERVICE, key).ok()?;
        entry.get_password().ok()
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry = Entry::new(SERVICE, key)?;
        entry.set_password(value)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = Entry::new(SERVICE, key)?;
        entry.delete_credential()?;
        Ok(())
    }
}

pub fn get_bifrost_key() -> String {
    if let Ok(key) = std::env::var("BIFROST_KEY") {
        if !key.is_empty() {
            return key;
        }
    }

    if let Some(key) = KeyringStore.get("bifrost_key") {
        if !key.is_empty() {
            return key;
        }
    }

    String::new()
}

pub fn store_bifrost_key(key: &str) -> Result<()> {
    KeyringStore.set("bifrost_key", key)
}

pub fn clear_bifrost_key() -> Result<()> {
    KeyringStore.delete("bifrost_key")
}
