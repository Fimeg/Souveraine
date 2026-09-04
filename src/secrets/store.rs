//! On-disk encrypted item storage — the Secret Service face of the storage
//! encryption design of record (`SouveraineOS/docs/STORAGE-ENCRYPTION.md`).
//!
//! Key hierarchy, per that doc:
//!
//! ```text
//! random 32-byte store key ── AES-256-GCM seals every item (id as AAD)
//!   ├─ wrapped by the machine KEK   (HKDF of a deterministic machined
//!   │                                signature — boot-available, no prompt)
//!   └─ wrapped by the passphrase KEK (Argon2id over the user's real
//!                                    lockscreen passphrase, salted, then
//!                                    HKDF-mixed with the machine signature
//!                                    so off-device brute force needs the
//!                                    machine seed too)
//! ```
//!
//! A passphrase change re-wraps the store key; it never re-encrypts items
//! and never orphans them. The machine wrap keeps secrets available from
//! session start without prompting (TASK-03); the passphrase wrap is the
//! design-of-record slot that makes the user's real password part of the
//! at-rest story — the eviction/Personal-class arc drives it once the
//! lock-signal ingress decision (deferred in the doc) is made.
//!
//! Persistence is atomic: a temp file created 0600 in the same directory,
//! fully written, then renamed over the store. A crash mid-write leaves the
//! previous store intact instead of a truncated JSON that would silently
//! orphan every secret on the machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result};
use argon2::Argon2;
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Argon2id parameters, stored in the header so they can be raised later
/// without breaking existing stores. 64 MiB / 3 passes is aggressive for a
/// phone-class device while staying interactive at unlock.
const ARGON2_M_COST_KIB: u32 = 64 * 1024;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

const HKDF_INFO_MACHINE_KEK: &[u8] = b"souveraine-secrets/machine-kek/v2";
const HKDF_INFO_PASSPHRASE_KEK: &[u8] = b"souveraine-secrets/passphrase-kek/v2";

fn default_content_type() -> String {
    "text/plain".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredItem {
    pub label: String,
    pub attributes: HashMap<String, String>,
    /// MIME type the client stored alongside the secret — echoed back verbatim.
    #[serde(default = "default_content_type")]
    pub content_type: String,
    /// Unix seconds, set once at first store.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds, updated on every secret/label/attribute write.
    #[serde(default)]
    pub modified: u64,
    /// 12-byte GCM nonce followed by the AES-256-GCM ciphertext+tag; the
    /// item id is the AAD, so ciphertext cannot be swapped between items.
    pub sealed: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KdfParams {
    salt: Vec<u8>,
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

/// The wrap slots: the store key encrypted under each KEK. `machine` always
/// exists; `passphrase` exists once the user's passphrase has been enrolled.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Wraps {
    #[serde(default)]
    machine: Option<Vec<u8>>,
    #[serde(default)]
    passphrase: Option<Vec<u8>>,
}

/// The on-disk shape.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    kdf: Option<KdfParams>,
    #[serde(default)]
    wraps: Wraps,
    #[serde(default)]
    items: HashMap<String, StoredItem>,
}

pub struct SecretStore {
    path: PathBuf,
    /// The unwrapped store key. Present for the daemon's life today (machine
    /// wrap, session-lifetime availability); the eviction arc zeroes it.
    store_key: [u8; 32],
    /// Machine key material (the deterministic machined signature) — kept to
    /// derive KEKs when enrolling or verifying a passphrase wrap.
    machine_ikm: Vec<u8>,
    file: StoreFile,
}

impl SecretStore {
    /// Open (or create) the store. `machine_ikm` is the deterministic
    /// machine signature from `storage_key::resolve` — never the machine
    /// private key itself.
    pub fn open(machine_ikm: Vec<u8>, path: PathBuf) -> Result<Self> {
        let mut file: StoreFile = if path.exists() {
            let raw = std::fs::read(&path).context("reading secrets store file")?;
            if raw.is_empty() {
                StoreFile::default()
            } else {
                serde_json::from_slice(&raw).context("parsing secrets store file")?
            }
        } else {
            StoreFile::default()
        };

        let machine_kek = hkdf_expand(&machine_ikm, None, HKDF_INFO_MACHINE_KEK);

        let store_key = match &file.wraps.machine {
            Some(wrapped) => unwrap_key(&machine_kek, wrapped, b"machine")
                .context("unwrapping store key — wrong machine identity or corrupt store")?,
            None => {
                // Fresh store: mint the random store key and the machine wrap.
                let mut key = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut key);
                file.version = 2;
                file.created = now_unix();
                file.wraps.machine = Some(wrap_key(&machine_kek, &key, b"machine"));
                key
            }
        };

        let store = Self {
            path,
            store_key,
            machine_ikm,
            file,
        };
        // Persist a fresh store immediately so the wrap exists on disk even
        // before the first item does.
        if store.file.version == 2 && store.file.items.is_empty() && !store.path.exists() {
            store.persist()?;
        }
        Ok(store)
    }

    /// Enroll or rotate the passphrase wrap — the design-of-record slot that
    /// puts the user's real lockscreen passphrase into the at-rest hierarchy.
    /// Re-wraps the store key only; items are untouched.
    pub fn set_passphrase(&mut self, passphrase: &str) -> Result<()> {
        let kdf = match &self.file.kdf {
            Some(kdf) => kdf.clone(),
            None => {
                let mut salt = vec![0u8; 32];
                rand::thread_rng().fill_bytes(&mut salt);
                KdfParams {
                    salt,
                    m_cost_kib: ARGON2_M_COST_KIB,
                    t_cost: ARGON2_T_COST,
                    p_cost: ARGON2_P_COST,
                }
            }
        };
        let kek = self.passphrase_kek(passphrase, &kdf)?;
        self.file.wraps.passphrase = Some(wrap_key(&kek, &self.store_key, b"passphrase"));
        self.file.kdf = Some(kdf);
        self.persist()
    }

    /// Whether the passphrase wrap slot is enrolled.
    pub fn has_passphrase(&self) -> bool {
        self.file.wraps.passphrase.is_some()
    }

    /// Verify a passphrase against the enrolled wrap (unwraps the store key
    /// and compares). The PAM/stepUp side uses this to keep the wrap honest.
    pub fn verify_passphrase(&self, passphrase: &str) -> Result<bool> {
        let (Some(wrapped), Some(kdf)) = (&self.file.wraps.passphrase, &self.file.kdf) else {
            return Ok(false);
        };
        let kek = self.passphrase_kek(passphrase, kdf)?;
        Ok(match unwrap_key(&kek, wrapped, b"passphrase") {
            Ok(key) => key == self.store_key,
            Err(_) => false,
        })
    }

    /// Argon2id over the passphrase, then HKDF-mixed with the machine
    /// signature — off-device brute force of the passphrase wrap therefore
    /// also requires the machine seed.
    fn passphrase_kek(&self, passphrase: &str, kdf: &KdfParams) -> Result<[u8; 32]> {
        let params = argon2::Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(32))
            .map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?;
        let argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut derived = [0u8; 32];
        argon
            .hash_password_into(passphrase.as_bytes(), &kdf.salt, &mut derived)
            .map_err(|e| anyhow::anyhow!("argon2 derive: {e}"))?;
        Ok(hkdf_expand(
            &derived,
            Some(&self.machine_ikm),
            HKDF_INFO_PASSPHRASE_KEK,
        ))
    }

    /// Atomic write: temp file born 0600 next to the store, then renamed over
    /// it. The store never exists half-written or world-readable.
    fn persist(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("secrets store path has no parent directory")?;
        std::fs::create_dir_all(parent)?;

        let raw = serde_json::to_vec(&self.file)?;

        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options
                .open(&tmp)
                .context("creating secrets store temp file")?;
            f.write_all(&raw)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path).context("committing secrets store")?;
        Ok(())
    }

    pub fn set(
        &mut self,
        id: String,
        label: String,
        attributes: HashMap<String, String>,
        content_type: String,
        value: &[u8],
    ) -> Result<()> {
        let sealed = seal(&self.store_key, id.as_bytes(), value)?;
        let now = now_unix();
        let created = self.file.items.get(&id).map(|i| i.created).unwrap_or(now);
        self.file.items.insert(
            id,
            StoredItem {
                label,
                attributes,
                content_type,
                created,
                modified: now,
                sealed,
            },
        );
        self.persist()
    }

    pub fn get(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let Some(item) = self.file.items.get(id) else {
            return Ok(None);
        };
        let plaintext = open_sealed(&self.store_key, id.as_bytes(), &item.sealed)
            .context("decrypting stored item — wrong machine key or corrupt store")?;
        Ok(Some(plaintext))
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.file.items.remove(id);
        self.persist()
    }

    pub fn search(&self, attributes: &HashMap<String, String>) -> Vec<String> {
        self.file
            .items
            .iter()
            .filter(|(_, item)| {
                attributes
                    .iter()
                    .all(|(k, v)| item.attributes.get(k) == Some(v))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn label(&self, id: &str) -> Option<String> {
        self.file.items.get(id).map(|i| i.label.clone())
    }

    pub fn attributes(&self, id: &str) -> Option<HashMap<String, String>> {
        self.file.items.get(id).map(|i| i.attributes.clone())
    }

    pub fn content_type(&self, id: &str) -> Option<String> {
        self.file.items.get(id).map(|i| i.content_type.clone())
    }

    pub fn created(&self, id: &str) -> Option<u64> {
        self.file.items.get(id).map(|i| i.created)
    }

    pub fn modified(&self, id: &str) -> Option<u64> {
        self.file.items.get(id).map(|i| i.modified)
    }

    /// The collection's birth timestamp.
    pub fn collection_created(&self) -> u64 {
        self.file.created
    }

    /// The collection's last-modified: the newest item write, or the birth
    /// timestamp for an empty store.
    pub fn collection_modified(&self) -> u64 {
        self.file
            .items
            .values()
            .map(|i| i.modified)
            .max()
            .unwrap_or(self.file.created)
    }

    pub fn all_ids(&self) -> Vec<String> {
        self.file.items.keys().cloned().collect()
    }
}

// ---- sealing primitives --------------------------------------------------

/// AES-256-GCM: 12-byte random nonce prepended, AAD binds the context so a
/// wrap or item ciphertext cannot be replayed in another slot.
fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("AEAD seal failed"))?;
    let mut out = nonce.to_vec();
    out.extend(ciphertext);
    Ok(out)
}

fn open_sealed(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(sealed.len() > 12 + 16, "sealed blob too short");
    let (nonce, ciphertext) = sealed.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("AEAD open failed — wrong key or tampered data"))
}

fn wrap_key(kek: &[u8; 32], key: &[u8; 32], slot: &[u8]) -> Vec<u8> {
    seal(kek, slot, key).expect("wrapping a 32-byte key cannot fail")
}

fn unwrap_key(kek: &[u8; 32], wrapped: &[u8], slot: &[u8]) -> Result<[u8; 32]> {
    let bytes = open_sealed(kek, slot, wrapped)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("unwrapped key has wrong length"))
}

fn hkdf_expand(ikm: &[u8], salt: Option<&[u8]>, info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(salt, ikm);
    let mut key = [0u8; 32];
    hk.expand(info, &mut key)
        .expect("HKDF expand with fixed 32-byte output cannot fail");
    key
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn default_store_path(base_path: &Path) -> PathBuf {
    base_path.join("secrets").join("store.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ikm() -> Vec<u8> {
        vec![7u8; 64]
    }

    #[test]
    fn round_trips_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        let mut store = SecretStore::open(ikm(), path.clone()).unwrap();
        store
            .set(
                "id1".into(),
                "label".into(),
                HashMap::from([("app".to_string(), "test".to_string())]),
                "text/plain".into(),
                b"hunter2",
            )
            .unwrap();

        // Reopen from disk under the same machine material — the item survives.
        let store2 = SecretStore::open(ikm(), path).unwrap();
        assert_eq!(store2.get("id1").unwrap().unwrap(), b"hunter2");
        assert_eq!(store2.content_type("id1").unwrap(), "text/plain");
        assert!(store2.created("id1").unwrap() > 0);
    }

    #[test]
    fn wrong_machine_identity_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = SecretStore::open(ikm(), path.clone()).unwrap();
        store
            .set(
                "id".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"s",
            )
            .unwrap();
        drop(store);

        assert!(SecretStore::open(vec![9u8; 64], path).is_err());
    }

    #[test]
    fn item_ciphertext_is_bound_to_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = SecretStore::open(ikm(), path).unwrap();
        store
            .set(
                "a".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"secret-a",
            )
            .unwrap();
        // Grafting a's ciphertext onto id b must fail the AAD check.
        let sealed = store.file.items.get("a").unwrap().sealed.clone();
        assert!(open_sealed(&store.store_key, b"b", &sealed).is_err());
    }

    #[test]
    fn passphrase_wrap_enrolls_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = SecretStore::open(ikm(), path.clone()).unwrap();
        assert!(!store.has_passphrase());

        store
            .set_passphrase("correct horse battery staple")
            .unwrap();
        assert!(store.has_passphrase());
        assert!(store
            .verify_passphrase("correct horse battery staple")
            .unwrap());
        assert!(!store.verify_passphrase("wrong").unwrap());

        // The wrap survives a reload and still verifies.
        let store2 = SecretStore::open(ikm(), path).unwrap();
        assert!(store2
            .verify_passphrase("correct horse battery staple")
            .unwrap());
    }

    #[test]
    fn passphrase_rotation_rewraps_without_touching_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = SecretStore::open(ikm(), path).unwrap();
        store
            .set(
                "id".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"s",
            )
            .unwrap();
        let sealed_before = store.file.items.get("id").unwrap().sealed.clone();

        store.set_passphrase("first").unwrap();
        store.set_passphrase("second").unwrap();
        assert!(!store.verify_passphrase("first").unwrap());
        assert!(store.verify_passphrase("second").unwrap());
        // Rotation re-wraps the store key only; the item bytes are untouched.
        assert_eq!(store.file.items.get("id").unwrap().sealed, sealed_before);
        assert_eq!(store.get("id").unwrap().unwrap(), b"s");
    }

    #[test]
    fn created_survives_rewrite_and_modified_advances() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SecretStore::open(ikm(), dir.path().join("store.json")).unwrap();
        store
            .set(
                "id".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"a",
            )
            .unwrap();
        let created = store.created("id").unwrap();
        store
            .set(
                "id".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"b",
            )
            .unwrap();
        assert_eq!(store.created("id").unwrap(), created);
        assert!(store.modified("id").unwrap() >= created);
    }

    #[test]
    fn store_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = SecretStore::open(ikm(), path.clone()).unwrap();
        store
            .set(
                "id".into(),
                "l".into(),
                HashMap::new(),
                "text/plain".into(),
                b"s",
            )
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
