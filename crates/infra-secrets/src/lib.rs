//! Sealing secrets with AES-256-GCM under a ring of rotatable keys.
//!
//! * One key is **active** and seals new values; every key in the ring can still open values sealed
//!   under it, so rotating is: add the new key first in the ring, deploy, re-seal at leisure, and
//!   only then retire the old one.
//! * Every value gets a fresh random 96-bit nonce. With random nonces a single key must stay far
//!   below 2^32 encryptions; secrets are written rarely and keys are rotated, so that is a limit in
//!   principle, not in practice.
//! * The ciphertext is authenticated together with the associated data the caller derives from the
//!   secret's tenant, namespace, name, version and key id, so it opens only where it was written.
//! * Key material is wiped from memory when the ring is dropped, never printed, and parsed from a
//!   `id:base64` list (`SAGAT_SECRETS_KEYS`).

pub mod openbao;

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sagat_domain::secrets::{SealedSecret, SecretCipher, SecretValue};
use zeroize::Zeroizing;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const MAX_KEY_ID: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the key list is empty")]
    Empty,
    #[error(
        "a key entry must be `id:base64`, with an id of 1 to {MAX_KEY_ID} characters [a-z0-9_-]"
    )]
    Malformed,
    #[error("a key must be exactly {KEY_BYTES} bytes (base64 of 32 random bytes)")]
    WrongLength,
    #[error("a key id appears twice")]
    Duplicate,
    #[error("the system random number generator failed")]
    Entropy,
}

pub use sagat_domain::secrets::CipherError;

/// The keys, the active one first.
pub struct KeyRing {
    active: String,
    keys: BTreeMap<String, Aes256Gcm>,
    // Kept only so the raw bytes are wiped when the ring is dropped.
    _material: Vec<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRing")
            .field("active", &self.active)
            .field("keys", &self.keys.len())
            .finish_non_exhaustive()
    }
}

impl KeyRing {
    /// Parses `id:base64,id:base64,...`; the first entry is the active key.
    ///
    /// # Errors
    /// [`KeyError`]; the message never contains key material.
    pub fn parse(spec: &str) -> Result<Self, KeyError> {
        let mut active = None;
        let mut keys = BTreeMap::new();
        let mut material = Vec::new();
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (id, encoded) = entry.split_once(':').ok_or(KeyError::Malformed)?;
            let id_ok = !id.is_empty()
                && id.len() <= MAX_KEY_ID
                && id.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-')
                });
            if !id_ok {
                return Err(KeyError::Malformed);
            }
            let raw = Zeroizing::new(
                STANDARD
                    .decode(encoded.trim())
                    .map_err(|_| KeyError::Malformed)?,
            );
            if raw.len() != KEY_BYTES {
                return Err(KeyError::WrongLength);
            }
            let cipher = Aes256Gcm::new_from_slice(&raw).map_err(|_| KeyError::WrongLength)?;
            if keys.insert(id.to_owned(), cipher).is_some() {
                return Err(KeyError::Duplicate);
            }
            active.get_or_insert_with(|| id.to_owned());
            material.push(raw);
        }
        Ok(Self {
            active: active.ok_or(KeyError::Empty)?,
            keys,
            _material: material,
        })
    }

    /// The id of the key that seals new values.
    #[must_use]
    pub fn active_key(&self) -> &str {
        &self.active
    }

    /// Whether a value sealed under `key_id` can still be opened.
    #[must_use]
    pub fn knows(&self, key_id: &str) -> bool {
        self.keys.contains_key(key_id)
    }

    /// Seals `value` under the active key, authenticating `aad` with it.
    ///
    /// # Errors
    /// [`CipherError::Seal`].
    pub fn seal(&self, value: &SecretValue, aad: &[u8]) -> Result<SealedSecret, CipherError> {
        let cipher = self.keys.get(&self.active).ok_or(CipherError::Seal)?;
        let mut nonce = [0u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| CipherError::Seal)?;
        let ciphertext = cipher
            .encrypt(
                &Nonce::try_from(nonce.as_slice()).map_err(|_| CipherError::Seal)?,
                Payload {
                    msg: value.expose(),
                    aad,
                },
            )
            .map_err(|_| CipherError::Seal)?;
        Ok(SealedSecret {
            key_id: self.active.clone(),
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    /// Opens a sealed value, checking that `aad` is what it was sealed with.
    ///
    /// # Errors
    /// [`CipherError::Open`] for an unknown key, a tampered ciphertext or other associated data.
    pub fn open(&self, sealed: &SealedSecret, aad: &[u8]) -> Result<SecretValue, CipherError> {
        let cipher = self.keys.get(&sealed.key_id).ok_or(CipherError::Open)?;
        if sealed.nonce.len() != NONCE_BYTES {
            return Err(CipherError::Open);
        }
        let plain = cipher
            .decrypt(
                &Nonce::try_from(sealed.nonce.as_slice()).map_err(|_| CipherError::Open)?,
                Payload {
                    msg: &sealed.ciphertext,
                    aad,
                },
            )
            .map_err(|_| CipherError::Open)?;
        SecretValue::new(plain).map_err(|_| CipherError::Open)
    }
}

impl SecretCipher for KeyRing {
    fn active_key_id(&self) -> &str {
        self.active_key()
    }

    fn knows(&self, key_id: &str) -> bool {
        Self::knows(self, key_id)
    }

    fn seal(&self, value: &SecretValue, aad: &[u8]) -> Result<SealedSecret, CipherError> {
        Self::seal(self, value, aad)
    }

    fn open(&self, sealed: &SealedSecret, aad: &[u8]) -> Result<SecretValue, CipherError> {
        Self::open(self, sealed, aad)
    }
}

/// The key ring a process was configured with. It prints as its active key id and nothing else, so
/// a configuration struct holding it can be logged without ever showing key material.
#[derive(Clone)]
pub struct SecretsConfig {
    ring: std::sync::Arc<KeyRing>,
}

impl SecretsConfig {
    /// Parses the value of `SAGAT_SECRETS_KEYS`.
    ///
    /// # Errors
    /// [`KeyError`], whose message never contains the value.
    pub fn parse(spec: &str) -> Result<Self, KeyError> {
        Ok(Self {
            ring: std::sync::Arc::new(KeyRing::parse(spec)?),
        })
    }

    /// The ring, as the domain's port.
    #[must_use]
    pub fn cipher(&self) -> std::sync::Arc<dyn SecretCipher> {
        self.ring.clone()
    }
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("active_key", &self.ring.active_key())
            .field("keys", &"<redacted>")
            .finish()
    }
}

/// A random key, base64-encoded, for `SAGAT_SECRETS_KEYS` (what an operator generates once).
///
/// # Errors
/// [`KeyError::Entropy`].
pub fn generate_key() -> Result<String, KeyError> {
    let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|_| KeyError::Entropy)?;
    Ok(STANDARD.encode(bytes.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(spec: &[(&str, &str)]) -> KeyRing {
        let text = spec
            .iter()
            .map(|(id, key)| format!("{id}:{key}"))
            .collect::<Vec<_>>()
            .join(",");
        KeyRing::parse(&text).expect("ring")
    }

    fn key() -> String {
        generate_key().expect("key")
    }

    fn value(text: &str) -> SecretValue {
        SecretValue::new(text.as_bytes().to_vec()).expect("value")
    }

    #[test]
    fn a_sealed_value_opens_only_with_the_same_associated_data() {
        let ring = ring(&[("k1", &key())]);
        let sealed = ring
            .seal(&value("sk_live_123"), b"coordinates-A")
            .expect("seal");
        assert_eq!(sealed.key_id, "k1");
        assert_eq!(sealed.nonce.len(), 12);
        assert!(!String::from_utf8_lossy(&sealed.ciphertext).contains("sk_live"));
        assert_eq!(
            ring.open(&sealed, b"coordinates-A").expect("open"),
            value("sk_live_123")
        );
        // Another tenant, namespace, name or version means other associated data: it must not open.
        assert_eq!(ring.open(&sealed, b"coordinates-B"), Err(CipherError::Open));
        assert_eq!(ring.open(&sealed, b""), Err(CipherError::Open));
    }

    #[test]
    fn tampering_with_any_part_is_detected() {
        let ring = ring(&[("k1", &key())]);
        let sealed = ring.seal(&value("secret"), b"aad").expect("seal");
        let mut flipped = sealed.clone();
        flipped.ciphertext[0] ^= 1;
        assert_eq!(ring.open(&flipped, b"aad"), Err(CipherError::Open));
        let mut nonce = sealed.clone();
        nonce.nonce[0] ^= 1;
        assert_eq!(ring.open(&nonce, b"aad"), Err(CipherError::Open));
        let mut short = sealed.clone();
        short.nonce.pop();
        assert_eq!(ring.open(&short, b"aad"), Err(CipherError::Open));
        let mut truncated = sealed;
        truncated.ciphertext.pop();
        assert_eq!(ring.open(&truncated, b"aad"), Err(CipherError::Open));
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce_so_equal_values_look_different() {
        let ring = ring(&[("k1", &key())]);
        let a = ring.seal(&value("same"), b"aad").expect("seal");
        let b = ring.seal(&value("same"), b"aad").expect("seal");
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn rotation_keeps_old_values_readable_and_seals_new_ones_under_the_new_key() {
        let (old, new) = (key(), key());
        let before = ring(&[("k1", &old)]);
        let sealed_old = before.seal(&value("v"), b"aad").expect("seal");
        // The new key goes first (active); the old one stays so existing values still open.
        let after = ring(&[("k2", &new), ("k1", &old)]);
        assert_eq!(after.active_key(), "k2");
        assert_eq!(
            after.open(&sealed_old, b"aad").expect("old value"),
            value("v")
        );
        assert_eq!(after.seal(&value("v"), b"aad").expect("seal").key_id, "k2");
        // Once the old key is retired, what was sealed under it is gone.
        let retired = ring(&[("k2", &new)]);
        assert!(!retired.knows("k1"));
        assert_eq!(retired.open(&sealed_old, b"aad"), Err(CipherError::Open));
    }

    #[test]
    fn a_ring_is_parsed_strictly_and_never_prints_key_material() {
        let good = key();
        for (spec, error) in [
            ("", KeyError::Empty),
            (" , ", KeyError::Empty),
            ("nocolon", KeyError::Malformed),
            ("K1:AAAA", KeyError::Malformed),
            ("k 1:AAAA", KeyError::Malformed),
            (":AAAA", KeyError::Malformed),
            ("k1:not base64!!", KeyError::Malformed),
            ("k1:AAAA", KeyError::WrongLength),
        ] {
            assert_eq!(KeyRing::parse(spec).err(), Some(error), "{spec:?}");
        }
        assert_eq!(
            KeyRing::parse(&format!("k1:{good},k1:{good}")).err(),
            Some(KeyError::Duplicate)
        );
        let ring = KeyRing::parse(&format!("k1:{good}")).expect("ring");
        let shown = format!("{ring:?}");
        assert!(!shown.contains(&good), "{shown}");
        // Whitespace around entries and a trailing comma are tolerated; inside an entry it is not.
        assert!(KeyRing::parse(&format!(" k1:{good} , ")).is_ok());
        assert_eq!(
            KeyRing::parse(&format!("k1 : {good}")).err(),
            Some(KeyError::Malformed)
        );
    }
}
