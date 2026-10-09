//! The signature of an artifact (`FR-005-001`, US02): the platform's statement that this digest is
//! what its sandbox built for this tenant from the source of a version.
//!
//! The runner runs nothing whose signature does not verify. That is what makes a row written into
//! the database by anything other than the control plane worthless: the bytes of an artifact are
//! only trusted if the platform signed their digest, so an attacker who can write to the table
//! cannot make the runner execute code of their choosing. (An attacker who holds the signing key
//! can; the key is the secret of the control plane and is never given to the runner.)
//!
//! The signature covers the tenant and the language as well as the digest, so an artifact of one
//! tenant cannot be presented as another's and the same bytes cannot be run as another language.

use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use sagat_domain::identity::TenantId;
use sagat_domain::scripts::{Digest, Language};
use sha2::{Digest as _, Sha256};

/// What a signature is made over, so it cannot be a signature of anything else the platform signs.
const CONTEXT: &str = "sagat/script-artifact/v1";

/// Why a key or a signature was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    #[error("the signing key is not an Ed25519 key in PKCS#8 form")]
    BadKey,
    #[error("the signature does not verify")]
    BadSignature,
    #[error("no key with that id is trusted")]
    UnknownKey,
}

fn message(tenant: TenantId, digest: &Digest, language: Language) -> Vec<u8> {
    format!(
        "{CONTEXT}\n{tenant}\n{}\n{}\n",
        digest.as_str(),
        language.as_str()
    )
    .into_bytes()
}

/// The key id of a public key: the first eight bytes of its SHA-256, in hexadecimal.
#[must_use]
pub fn key_id_of(public_key: &[u8]) -> String {
    Sha256::digest(public_key)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The private key of the control plane.
pub struct Signer {
    pair: Ed25519KeyPair,
    key_id: String,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key is not shown, and neither is anything derived from it but its public id.
        f.debug_struct("Signer")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl Signer {
    /// Reads a key from its PKCS#8 encoding.
    ///
    /// # Errors
    /// [`SigningError::BadKey`].
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, SigningError> {
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| SigningError::BadKey)?;
        let key_id = key_id_of(pair.public_key().as_ref());
        Ok(Self { pair, key_id })
    }

    /// A new key, as PKCS#8: what an operator stores and gives the control plane.
    ///
    /// # Errors
    /// [`SigningError::BadKey`] if the system's randomness fails.
    pub fn generate_pkcs8() -> Result<Vec<u8>, SigningError> {
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| SigningError::BadKey)
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    #[must_use]
    pub fn sign(&self, tenant: TenantId, digest: &Digest, language: Language) -> Vec<u8> {
        self.pair
            .sign(&message(tenant, digest, language))
            .as_ref()
            .to_vec()
    }
}

/// The public keys whose signatures are trusted: the one in force and the ones it replaced, so a
/// rotation does not make every artifact unverifiable.
#[derive(Debug, Clone, Default)]
pub struct Verifier {
    keys: Vec<(String, Vec<u8>)>,
}

impl Verifier {
    #[must_use]
    pub fn new(public_keys: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            keys: public_keys
                .into_iter()
                .map(|key| (key_id_of(&key), key))
                .collect(),
        }
    }

    /// The trusted keys as `(id, public key)`.
    #[must_use]
    pub fn keys(&self) -> &[(String, Vec<u8>)] {
        &self.keys
    }

    /// # Errors
    /// [`SigningError::UnknownKey`] when `key_id` is not trusted; [`SigningError::BadSignature`]
    /// when the signature is not the key's over this tenant, digest and language.
    pub fn verify(
        &self,
        key_id: &str,
        tenant: TenantId,
        digest: &Digest,
        language: Language,
        signature: &[u8],
    ) -> Result<(), SigningError> {
        let (_, key) = self
            .keys
            .iter()
            .find(|(id, _)| id == key_id)
            .ok_or(SigningError::UnknownKey)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&message(tenant, digest, language), signature)
            .map_err(|_| SigningError::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::digest_of;

    fn signer() -> Signer {
        Signer::from_pkcs8(&Signer::generate_pkcs8().expect("key")).expect("signer")
    }

    #[test]
    fn a_signature_verifies_only_for_the_tenant_digest_and_language_it_was_made_for() {
        let signer = signer();
        let verifier = Verifier::new([signer.public_key()]);
        let tenant = TenantId::generate();
        let digest = digest_of(b"artifact");
        let signature = signer.sign(tenant, &digest, Language::Python);
        assert_eq!(
            verifier.verify(
                signer.key_id(),
                tenant,
                &digest,
                Language::Python,
                &signature
            ),
            Ok(())
        );
        for (other_tenant, other_digest, other_language) in [
            (TenantId::generate(), digest.clone(), Language::Python),
            (tenant, digest_of(b"another"), Language::Python),
            (tenant, digest.clone(), Language::TypeScript),
        ] {
            assert_eq!(
                verifier.verify(
                    signer.key_id(),
                    other_tenant,
                    &other_digest,
                    other_language,
                    &signature
                ),
                Err(SigningError::BadSignature)
            );
        }
        let mut flipped = signature;
        flipped[0] ^= 1;
        assert_eq!(
            verifier.verify(signer.key_id(), tenant, &digest, Language::Python, &flipped),
            Err(SigningError::BadSignature)
        );
    }

    #[test]
    fn a_key_that_is_not_trusted_signs_nothing_the_verifier_accepts() {
        let (ours, theirs) = (signer(), signer());
        let verifier = Verifier::new([ours.public_key()]);
        let tenant = TenantId::generate();
        let digest = digest_of(b"x");
        let forged = theirs.sign(tenant, &digest, Language::Python);
        assert_eq!(
            verifier.verify(theirs.key_id(), tenant, &digest, Language::Python, &forged),
            Err(SigningError::UnknownKey)
        );
        // Claiming to be our key does not help: the signature is not ours.
        assert_eq!(
            verifier.verify(ours.key_id(), tenant, &digest, Language::Python, &forged),
            Err(SigningError::BadSignature)
        );
    }

    #[test]
    fn what_is_not_a_key_is_refused_and_a_key_is_never_printed() {
        assert!(matches!(
            Signer::from_pkcs8(b"not a key"),
            Err(SigningError::BadKey)
        ));
        let signer = signer();
        let shown = format!("{signer:?}");
        assert!(shown.contains(signer.key_id()));
        assert!(!shown.contains("pair"), "{shown}");
    }

    #[test]
    fn the_key_id_follows_the_public_key() {
        let signer = signer();
        assert_eq!(signer.key_id(), key_id_of(&signer.public_key()));
        assert_eq!(signer.key_id().len(), 16);
    }
}
