//! Randomness and token hashing, behind a port so tests can be deterministic.

use sha2::{Digest, Sha256};

/// Source of unpredictable values.
pub trait Entropy: Send + Sync {
    /// A uniform sample in `[0, 1)` (retry jitter; not security sensitive).
    fn unit(&self) -> f64;

    /// 32 unpredictable bytes (lease and session tokens; security sensitive).
    fn token(&self) -> [u8; 32];
}

/// The operating system's CSPRNG.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEntropy;

impl Entropy for SystemEntropy {
    #[allow(clippy::cast_precision_loss)] // 53 random bits fit a double exactly
    fn unit(&self) -> f64 {
        let mut bytes = [0u8; 8];
        // Failing to read the OS RNG leaves zeros: jitter degrades to "none", which is safe.
        let _ = getrandom::fill(&mut bytes);
        (u64::from_be_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn token(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        if getrandom::fill(&mut bytes).is_err() {
            // A token from a broken RNG would be guessable; an all-zero token is refused by
            // `Token::from_bytes`, which makes the caller fail instead of issuing it.
            bytes = [0u8; 32];
        }
        bytes
    }
}

/// A lease or session secret: handed to the holder once, stored only as a hash.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl Token {
    /// # Errors
    /// `None` for an all-zero input, which only a failed RNG produces.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Option<Self> {
        if bytes == [0u8; 32] {
            return None;
        }
        Some(Self(hex(&bytes)))
    }

    /// The text given to the holder (64 lower-case hex characters).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Hash of a presented token, as stored. Compares equal for equal tokens only.
    #[must_use]
    pub fn hash_of(presented: &str) -> Vec<u8> {
        Sha256::digest(presented.as_bytes()).to_vec()
    }

    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        Self::hash_of(&self.0)
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
