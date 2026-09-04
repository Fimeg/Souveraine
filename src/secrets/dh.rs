//! OpenSession key exchange for the Secret Service D-Bus API.
//!
//! Implements `dh-ietf1024-sha256-aes128-cbc-pkcs7`: a classic (non-EC)
//! Diffie-Hellman exchange over the RFC 2409 "Second Oakley Group" (MODP,
//! 1024-bit), with the shared secret run through HKDF-SHA256 to derive a
//! 16-byte AES-128 session key. This mirrors libsecret's own
//! `secret-session.c` exactly — client and server must agree byte-for-byte
//! on the group and the KDF, or negotiation silently produces mismatched
//! keys instead of a clean failure.
//!
//! Reference: <https://gitlab.gnome.org/GNOME/libsecret/-/blob/main/libsecret/secret-session.c>

use anyhow::{Context, Result};
use hkdf::Hkdf;
use num_bigint::BigUint;
use num_traits::Num;
use sha2::Sha256;

/// RFC 2409 Second Oakley Group: 1024-bit MODP prime, generator 2.
///
/// This must be the *1024-bit* group (libsecret's `dh_group_1024_prime`,
/// 128 bytes). The daemon previously carried the RFC 3526 2048-bit group-14
/// prime under this name — both sides then computed internally-consistent
/// but mutually-worthless keys, and every negotiated session failed its
/// first decrypt. Verified byte-for-byte against libsecret `egg/egg-dh.c`.
const MODP_1024_PRIME_HEX: &str = concat!(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD1",
    "29024E088A67CC74020BBEA63B139B22514A08798E3404DD",
    "EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245",
    "E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
    "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381",
    "FFFFFFFFFFFFFFFF",
);
const GENERATOR: u64 = 2;

/// This side's half of an in-progress OpenSession exchange.
pub struct DhSession {
    private: BigUint,
    prime: BigUint,
    /// The public value we sent to the peer — kept only for logging/debugging.
    pub public: BigUint,
}

impl DhSession {
    /// Generate our private exponent and compute the public value to send back
    /// to the client as `OpenSession`'s output.
    pub fn new() -> Result<Self> {
        let prime =
            BigUint::from_str_radix(MODP_1024_PRIME_HEX, 16).context("parsing MODP-1024 prime")?;
        let generator = BigUint::from(GENERATOR);

        // 1024-bit exchange: a private exponent as wide as the group avoids
        // biasing the shared secret's bit length. `rand` fills the private
        // key; libsecret does the equivalent via gcry_mpi_randomize.
        let mut bytes = [0u8; 128];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut bytes);
        let private = BigUint::from_bytes_be(&bytes) % &prime;

        let public = generator.modpow(&private, &prime);

        Ok(Self {
            private,
            prime,
            public,
        })
    }

    /// Combine the peer's public value with our private exponent, then run
    /// the shared secret through HKDF-SHA256 (no salt, no info — matching
    /// libsecret) to derive a 16-byte AES-128 session key.
    ///
    /// The raw DH shared secret must be encoded as a fixed-width big-endian
    /// big number *without* stripping leading zero bytes — libsecret's
    /// `egg_dh_gen_secret` preserves the full modulus width. Losing that
    /// padding is the classic interop bug: it silently shifts every key
    /// derived from a shared secret that happens to start with a zero byte.
    pub fn derive_session_key(&self, peer_public: &BigUint) -> Result<[u8; 16]> {
        let shared = peer_public.modpow(&self.private, &self.prime);

        let modulus_len = (self.prime.bits() as usize + 7) / 8;
        let mut shared_bytes = vec![0u8; modulus_len];
        let raw = shared.to_bytes_be();
        anyhow::ensure!(
            raw.len() <= modulus_len,
            "DH shared secret wider than modulus — malformed peer public value"
        );
        shared_bytes[modulus_len - raw.len()..].copy_from_slice(&raw);

        let hk = Hkdf::<Sha256>::new(None, &shared_bytes);
        let mut key = [0u8; 16];
        hk.expand(&[], &mut key)
            .map_err(|_| anyhow::anyhow!("HKDF expand failed producing session key"))?;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_derive_the_same_session_key() {
        let alice = DhSession::new().unwrap();
        let bob = DhSession::new().unwrap();

        let alice_key = alice.derive_session_key(&bob.public).unwrap();
        let bob_key = bob.derive_session_key(&alice.public).unwrap();

        assert_eq!(alice_key, bob_key);
    }

    /// Pin the group to the real 1024-bit Second Oakley prime. The regression
    /// this guards: the 2048-bit group-14 prime living under this name, which
    /// broke every libsecret client's first decrypt.
    #[test]
    fn prime_is_the_1024_bit_oakley_group() {
        let prime = BigUint::from_str_radix(MODP_1024_PRIME_HEX, 16).unwrap();
        assert_eq!(prime.bits(), 1024);
        let bytes = prime.to_bytes_be();
        assert_eq!(bytes.len(), 128);
        // libsecret dh_group_1024_prime starts FF×8, C9 0F DA A2 … and ends
        // … EC E6 53 81, FF×8.
        assert_eq!(
            &bytes[..10],
            &[0xFF; 8]
                .iter()
                .chain([0xC9, 0x0F].iter())
                .copied()
                .collect::<Vec<u8>>()[..]
        );
        assert_eq!(
            &bytes[116..],
            &[0xEC, 0xE6, 0x53, 0x81, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }
}
