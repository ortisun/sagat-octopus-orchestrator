//! The signing key: ES256, as the platform's tests use, kept in a file with only the owner's
//! permissions when asked to keep it, so that tokens survive a restart of the provider.

use std::io::Write;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use jsonwebtoken::EncodingKey;
use p256::ecdsa::SigningKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("the key file {0} could not be read or written: {1}")]
    File(String, String),
    #[error("the key file {0} does not hold a key this provider wrote")]
    Corrupt(String),
    #[error(
        "the key file {0} can be read by other users: make it readable by its owner only (chmod 600)"
    )]
    Open(String),
    #[error("the system gave no randomness: {0}")]
    Random(String),
}

pub struct DevKey {
    signing: SigningKey,
    kid: String,
}

impl DevKey {
    /// A fresh key from the system's randomness.
    ///
    /// # Errors
    /// [`KeyError::Random`] when the system has none to give.
    pub fn generate() -> Result<Self, KeyError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| KeyError::Random(e.to_string()))?;
        // A 32-byte string is a valid P-256 scalar but for a chance of about 2^-128.
        let signing = SigningKey::from_slice(&seed).map_err(|e| KeyError::Random(e.to_string()))?;
        Ok(Self::from(signing))
    }

    fn from(signing: SigningKey) -> Self {
        let point = signing.verifying_key().to_encoded_point(false);
        let digest = Sha256::digest(point.as_bytes());
        let kid = format!(
            "dev-{}",
            digest[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        Self { signing, kid }
    }

    /// The key kept in `path`, or a new one kept there.
    ///
    /// # Errors
    /// [`KeyError`] when the file cannot be read or written, is not a key of this provider, or can
    /// be read by other users.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        let shown = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = std::fs::metadata(path)
                        .map_err(|e| KeyError::File(shown.clone(), e.to_string()))?
                        .permissions()
                        .mode();
                    if mode & 0o077 != 0 {
                        return Err(KeyError::Open(shown));
                    }
                }
                let der = STANDARD
                    .decode(text.trim())
                    .map_err(|_| KeyError::Corrupt(shown.clone()))?;
                let signing =
                    SigningKey::from_pkcs8_der(&der).map_err(|_| KeyError::Corrupt(shown))?;
                Ok(Self::from(signing))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate()?;
                let der = key
                    .signing
                    .to_pkcs8_der()
                    .map_err(|e| KeyError::File(shown.clone(), e.to_string()))?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options
                    .open(path)
                    .map_err(|e| KeyError::File(shown.clone(), e.to_string()))?;
                file.write_all(STANDARD.encode(der.as_bytes()).as_bytes())
                    .map_err(|e| KeyError::File(shown, e.to_string()))?;
                Ok(key)
            }
            Err(e) => Err(KeyError::File(shown, e.to_string())),
        }
    }

    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public key as a JWK.
    #[must_use]
    pub fn public_jwk(&self) -> Value {
        let point = self.signing.verifying_key().to_encoded_point(false);
        json!({
            "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": self.kid,
            "x": URL_SAFE_NO_PAD.encode(point.x().map(|x| x.to_vec()).unwrap_or_default()),
            "y": URL_SAFE_NO_PAD.encode(point.y().map(|y| y.to_vec()).unwrap_or_default()),
        })
    }

    /// # Errors
    /// [`jsonwebtoken::errors::Error`] when the claims cannot be signed.
    pub fn sign(&self, claims: &Value) -> Result<String, jsonwebtoken::errors::Error> {
        let der = self
            .signing
            .to_pkcs8_der()
            .map_err(|_| jsonwebtoken::errors::ErrorKind::InvalidEcdsaKey)?;
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.kid.clone());
        jsonwebtoken::encode(&header, claims, &EncodingKey::from_ec_der(der.as_bytes()))
    }
}
