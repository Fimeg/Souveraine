//! AES-128-CBC + PKCS7 wire encryption for secrets sent over a negotiated
//! `OpenSession`. Matches libsecret's `secret-session.c` framing: the
//! `Secret` struct on the wire is `(session_path, iv_bytes, ciphertext_bytes,
//! content_type)`, IV is 16 random bytes generated fresh per secret.

use aes::Aes128;
use anyhow::{Context, Result};
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rand::RngCore;

type Encryptor = cbc::Encryptor<Aes128>;
type Decryptor = cbc::Decryptor<Aes128>;

/// Encrypt `plaintext` under `session_key`, returning (iv, ciphertext).
pub fn encrypt(session_key: &[u8; 16], plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);

    let ciphertext = Encryptor::new(session_key.into(), &iv.into())
        .encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext);

    (iv.to_vec(), ciphertext)
}

/// Decrypt a secret's `(iv, ciphertext)` under `session_key`.
pub fn decrypt(session_key: &[u8; 16], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(iv.len() == 16, "IV must be 16 bytes, got {}", iv.len());
    let iv_arr: [u8; 16] = iv.try_into().unwrap();

    Decryptor::new(session_key.into(), &iv_arr.into())
        .decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
        .context("AES-CBC/PKCS7 decrypt failed — wrong session key or corrupt ciphertext")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let key = [7u8; 16];
        let (iv, ct) = encrypt(&key, b"hunter2");
        let pt = decrypt(&key, &iv, &ct).unwrap();
        assert_eq!(pt, b"hunter2");
    }
}
