use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Hardware-bound (or file-bound) cryptographic identity for a Souveraine instance.
///
/// Generated once at `souveraine init`. The private key never leaves the primary
/// machine. Forked instances carry the public key and authenticate through the
/// primary via signed commissions.
///
/// The seed_id on SensorEvent references this — `None` means local,
/// `Some(pubkey_hex)` means the event originated from a peer with this identity.
pub struct SeedId {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedIdPublic {
    pub public_key_hex: String,
    pub instance: String,
    pub created_at: String,
}

impl SeedId {
    /// Generate a new seed identity (first init).
    pub fn generate() -> Self {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Load from disk, or generate and save if not present.
    pub fn load_or_generate(seed_dir: &Path) -> Result<Self> {
        let private_path = seed_dir.join("private.key");
        let public_path = seed_dir.join("public.key");

        if private_path.exists() {
            let bytes = std::fs::read(&private_path)
                .context("reading seed private key")?;
            if bytes.len() != 32 {
                anyhow::bail!("seed private key has wrong length: {} (expected 32)", bytes.len());
            }
            let mut key_bytes = [0u8; 32];
            key_bytes.copy_from_slice(&bytes);
            let signing_key = SigningKey::from_bytes(&key_bytes);
            let verifying_key = signing_key.verifying_key();
            return Ok(Self {
                signing_key,
                verifying_key,
            });
        }

        std::fs::create_dir_all(seed_dir)
            .context("creating seed-id directory")?;

        let seed = Self::generate();

        std::fs::write(&private_path, seed.signing_key.to_bytes())
            .context("writing seed private key")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&private_path, perms)
                .context("restricting seed private key permissions")?;
        }

        std::fs::write(&public_path, seed.verifying_key.to_bytes())
            .context("writing seed public key")?;

        Ok(seed)
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Deterministic 4-glyph rendering of the public key. Used as a
    /// terminal-friendly per-agent badge in the manager view. Drawn from
    /// a 16-symbol palette indexed by 4-bit nibbles of the first two
    /// bytes of the public key.
    pub fn glyph(&self) -> String {
        glyph_from_pubkey(&self.public_key_bytes())
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        self.signing_key.sign(data)
    }

    pub fn verify(&self, data: &[u8], signature: &Signature) -> bool {
        self.verifying_key.verify(data, signature).is_ok()
    }

    /// Verify using only the public key (for remote peers).
    pub fn verify_with_pubkey(
        pubkey_bytes: &[u8; 32],
        data: &[u8],
        signature: &Signature,
    ) -> bool {
        let key = VerifyingKey::from_bytes(pubkey_bytes);
        match key {
            Ok(vk) => vk.verify(data, signature).is_ok(),
            Err(_) => false,
        }
    }

    /// Default seed directory under the souveraine base path.
    pub fn default_dir(base_path: &Path) -> PathBuf {
        base_path.join("seed-id")
    }
}

/// Standalone glyph renderer — also usable on remote agents we only know
/// the pubkey bytes for. Reads the first two bytes of `pubkey` and emits
/// four glyphs from the geometric-shapes palette.
pub fn glyph_from_pubkey(pubkey: &[u8]) -> String {
    const PALETTE: [char; 16] = [
        '◇', '◆', '○', '●', '△', '▲', '▽', '▼',
        '□', '■', '◐', '◑', '◒', '◓', '☆', '★',
    ];
    let mut out = String::with_capacity(4);
    for byte in pubkey.iter().take(2) {
        out.push(PALETTE[(byte >> 4) as usize]);
        out.push(PALETTE[(byte & 0x0f) as usize]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let seed = SeedId::generate();
        let data = b"hello sovereign world";
        let sig = seed.sign(data);
        assert!(seed.verify(data, &sig));
        assert!(!seed.verify(b"tampered", &sig));
    }

    #[test]
    fn test_verify_with_pubkey() {
        let seed = SeedId::generate();
        let data = b"federation event payload";
        let sig = seed.sign(data);
        let pubkey = seed.public_key_bytes();
        assert!(SeedId::verify_with_pubkey(&pubkey, data, &sig));
    }

    #[test]
    fn test_load_or_generate() {
        let dir = tempfile::tempdir().unwrap();
        let seed1 = SeedId::load_or_generate(dir.path()).unwrap();
        let seed2 = SeedId::load_or_generate(dir.path()).unwrap();
        assert_eq!(seed1.public_key_hex(), seed2.public_key_hex());
    }

    #[test]
    fn test_glyph_is_deterministic_and_four_chars() {
        let seed = SeedId::generate();
        let g1 = seed.glyph();
        let g2 = seed.glyph();
        assert_eq!(g1, g2);
        assert_eq!(g1.chars().count(), 4);
    }

    #[test]
    fn test_glyph_known_input() {
        // pubkey bytes [0x00, 0xff, ...] → nibbles 0,0,f,f → ◇◇★★
        let g = glyph_from_pubkey(&[0x00, 0xff]);
        assert_eq!(g, "◇◇★★");
    }
}
