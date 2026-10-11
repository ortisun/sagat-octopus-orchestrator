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
    #[error(
        "the key file {0} does not hold a key this provider wrote: delete it to start with a new one"
    )]
    Corrupt(String),
    #[error("the key file {0} is a link: name the file itself")]
    Link(String),
    #[error(
        "the directory of the key file {0} can be written to by other users: make it private (chmod go-w)"
    )]
    Shared(String),
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
    /// The file is never followed through a link, is read through the handle that was checked (not
    /// through the name again), must be readable by its owner alone, and must sit in a directory
    /// others cannot add files to or remove files from; a new key is written whole to a file of its
    /// own and only then given its name, so a crash leaves no half-written key.
    ///
    /// # Errors
    /// [`KeyError`] when the file cannot be read or written, is not a key of this provider, can be
    /// read by other users, is a link, or is in a directory others can write to.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        let shown = path.display().to_string();
        #[cfg(unix)]
        directory_is_private(path, &shown)?;
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => Err(KeyError::Link(shown)),
            Ok(_) => Self::read(path, &shown),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::create(path, &shown),
            Err(e) => Err(KeyError::File(shown, e.to_string())),
        }
    }

    fn read(path: &Path, shown: &str) -> Result<Self, KeyError> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)
            .map_err(|e| KeyError::File(shown.to_owned(), e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file
                .metadata()
                .map_err(|e| KeyError::File(shown.to_owned(), e.to_string()))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(KeyError::Open(shown.to_owned()));
            }
        }
        // A key is a few hundred bytes: a file that is not one is not read to its end.
        let mut text = String::new();
        std::io::Read::by_ref(&mut file)
            .take(8192)
            .read_to_string(&mut text)
            .map_err(|e| KeyError::File(shown.to_owned(), e.to_string()))?;
        let der = STANDARD
            .decode(text.trim())
            .map_err(|_| KeyError::Corrupt(shown.to_owned()))?;
        let signing =
            SigningKey::from_pkcs8_der(&der).map_err(|_| KeyError::Corrupt(shown.to_owned()))?;
        Ok(Self::from(signing))
    }

    fn create(path: &Path, shown: &str) -> Result<Self, KeyError> {
        let key = Self::generate()?;
        let der = key
            .signing
            .to_pkcs8_der()
            .map_err(|e| KeyError::File(shown.to_owned(), e.to_string()))?;
        let own = |e: std::io::Error| KeyError::File(shown.to_owned(), e.to_string());
        // A name of its own however many start at once, in however many containers (each of which
        // may be process 1).
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| KeyError::Random(e.to_string()))?;
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let temporary = path.with_extension(format!("tmp-{suffix}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let written = options.open(&temporary).map_err(own).and_then(|mut file| {
            file.write_all(STANDARD.encode(der.as_bytes()).as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(own)
        });
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        // Giving it its name fails if another process has just done the same: that one's key is used.
        let named = std::fs::hard_link(&temporary, path);
        let _ = std::fs::remove_file(&temporary);
        match named {
            Ok(()) => Ok(key),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Self::read(path, shown),
            Err(e) => Err(own(e)),
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

/// The directory the key lives in must not be one where another user can replace the file.
#[cfg(unix)]
fn directory_is_private(path: &Path, shown: &str) -> Result<(), KeyError> {
    use std::os::unix::fs::PermissionsExt;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mode = std::fs::metadata(parent)
        .map_err(|e| KeyError::File(shown.to_owned(), e.to_string()))?
        .permissions()
        .mode();
    // Writable by group or others, and without the sticky bit that stops them removing what is not theirs.
    if mode & 0o022 != 0 && mode & 0o1000 == 0 {
        return Err(KeyError::Shared(shown.to_owned()));
    }
    Ok(())
}
