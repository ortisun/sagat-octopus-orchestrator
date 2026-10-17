//! Secrets (`FR-009-002`, `FR-009-003`): named, versioned values a tenant stores once and lets its
//! workers use without the value ever appearing in a definition, the journal, a log or an API
//! response.
//!
//! The domain knows what a secret *is* (its name, its limits, how its ciphertext is bound to where
//! it belongs) and nothing about how it is stored or sealed.

use std::fmt;

use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::identity::{NamespaceId, Permission, Role, TenantId};

/// Longest secret value accepted. A secret is a credential, not a document.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;
/// Longest secret name.
pub const MAX_SECRET_NAME: usize = 64;
/// Most secrets one activity may declare.
pub const MAX_SECRETS_PER_ACTIVITY: usize = 16;
/// Most distinct secrets one workflow definition may declare in all. Publishing looks each one up,
/// so this bounds the cost of a document, which is otherwise bounded only by its size.
pub const MAX_DECLARED_SECRETS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error(
        "a secret name is 1 to {MAX_SECRET_NAME} lower-case letters, digits, '.', '_' or '-', starting with a letter"
    )]
    InvalidName,
    #[error("a secret value is 1 to {MAX_SECRET_BYTES} bytes")]
    InvalidValue,
}

/// The name of a secret inside a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretName(String);

impl SecretName {
    /// # Errors
    /// [`SecretError::InvalidName`].
    pub fn parse(value: &str) -> Result<Self, SecretError> {
        let ok = !value.is_empty()
            && value.len() <= MAX_SECRET_NAME
            && value.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && value.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            });
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(SecretError::InvalidName)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A secret's plaintext. It prints as `<redacted>`, has no `Display`, and wipes its memory when
/// dropped.
#[derive(Clone)]
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// # Errors
    /// [`SecretError::InvalidValue`] for an empty or oversized value.
    pub fn new(bytes: Vec<u8>) -> Result<Self, SecretError> {
        if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
            return Err(SecretError::InvalidValue);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// The plaintext. Call it only where it is handed to its legitimate user.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

impl PartialEq for SecretValue {
    /// Compared without an early exit, so the comparison does not leak how much matched.
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

impl Eq for SecretValue {}

/// A secret as stored: sealed, and bound by its associated data to exactly this tenant, namespace,
/// name and row. A ciphertext copied to another row (or another tenant) does not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedSecret {
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// What may be shown about a secret: never the value, never the ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMeta {
    /// The row; part of the associated data, so a ciphertext cannot be moved to another row.
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub name: SecretName,
    pub version: u32,
    /// Local encryption key, absent for provider-backed values.
    pub key_id: Option<String>,
    pub created_by: String,
    pub created_at: OffsetDateTime,
}

/// Why sealing or opening failed. Carries no detail: a failure to open is the same whether the key
/// is unknown, the data was tampered with, or the ciphertext was moved to another row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CipherError {
    #[error("the secret could not be sealed")]
    Seal,
    #[error("the secret could not be opened")]
    Open,
}

/// Seals and opens secret values. The domain only needs the shape; `sagat-infra-secrets` is the
/// implementation (AES-256-GCM under a ring of rotatable keys).
pub trait SecretCipher: Send + Sync {
    /// The id of the key that seals new values.
    fn active_key_id(&self) -> &str;

    /// Whether a value sealed under `key_id` can still be opened.
    fn knows(&self, key_id: &str) -> bool;

    /// Seals `value` under the active key, authenticating `aad` with it.
    ///
    /// # Errors
    /// [`CipherError::Seal`].
    fn seal(&self, value: &SecretValue, aad: &[u8]) -> Result<SealedSecret, CipherError>;

    /// Opens a sealed value, checking that `aad` is what it was sealed with.
    ///
    /// # Errors
    /// [`CipherError::Open`].
    fn open(&self, sealed: &SealedSecret, aad: &[u8]) -> Result<SecretValue, CipherError>;
}

/// Whether a member who holds `role` may reference a secret from a workflow, given whether the
/// secret's own access list names them. Both are needed: the role says people of that kind may use
/// secrets at all (`secret:use`), the list says this secret is theirs to use. Administrators are
/// not subject to the list.
#[must_use]
pub fn may_use(role: Role, granted: bool) -> bool {
    role.grants(Permission::SecretUse) && (role == Role::Admin || granted)
}

/// The bytes the cipher authenticates along with the ciphertext (AEAD associated data).
///
/// Everything that identifies the secret is in it, length-prefixed so two different tuples can never
/// produce the same bytes. Opening a ciphertext under other coordinates fails.
#[must_use]
pub fn associated_data(
    tenant: TenantId,
    namespace: NamespaceId,
    name: &SecretName,
    row: Uuid,
    key_id: &str,
) -> Vec<u8> {
    let mut out = b"sagat/secret/v1".to_vec();
    for part in [
        tenant.as_uuid().as_bytes().as_slice(),
        namespace.as_uuid().as_bytes().as_slice(),
        name.as_str().as_bytes(),
        row.as_bytes().as_slice(),
        key_id.as_bytes(),
    ] {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn using_a_secret_needs_the_role_and_the_secrets_own_list() {
        // Administrators are not subject to the list.
        assert!(may_use(Role::Admin, false));
        assert!(may_use(Role::Admin, true));
        // A developer needs to be on the list.
        assert!(may_use(Role::Developer, true));
        assert!(!may_use(Role::Developer, false));
        // Roles that cannot reference secrets at all are not helped by a list entry.
        for role in [Role::Operator, Role::Viewer, Role::Worker] {
            assert!(!may_use(role, true), "{role}");
            assert!(!may_use(role, false), "{role}");
        }
    }

    #[test]
    fn names_are_lower_case_and_bounded() {
        for ok in ["stripe", "stripe-key", "a.b_c-1", &"a".repeat(64)] {
            assert!(SecretName::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "Stripe",
            "1abc",
            "-a",
            "a b",
            "a/b",
            "ü",
            &"a".repeat(65),
            "a\n",
        ] {
            assert_eq!(
                SecretName::parse(bad),
                Err(SecretError::InvalidName),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_value_is_bounded_redacted_and_compared_in_full() {
        assert_eq!(SecretValue::new(vec![]), Err(SecretError::InvalidValue));
        assert_eq!(
            SecretValue::new(vec![1; MAX_SECRET_BYTES + 1]),
            Err(SecretError::InvalidValue)
        );
        let value = SecretValue::new(b"sk_live_abc123".to_vec()).expect("value");
        assert_eq!(value.len(), 14);
        let shown = format!("{value:?}");
        assert!(
            !shown.contains("sk_live") && shown.contains("redacted"),
            "{shown}"
        );
        assert_eq!(
            value,
            SecretValue::new(b"sk_live_abc123".to_vec()).expect("same")
        );
        assert_ne!(
            value,
            SecretValue::new(b"sk_live_abc124".to_vec()).expect("other")
        );
        assert_ne!(
            value,
            SecretValue::new(b"sk_live_abc12".to_vec()).expect("shorter")
        );
        assert_eq!(value.expose(), b"sk_live_abc123");
    }

    #[test]
    fn associated_data_binds_every_coordinate_without_ambiguity() {
        let (t1, t2) = (TenantId::generate(), TenantId::generate());
        let (n1, n2) = (NamespaceId::generate(), NamespaceId::generate());
        let (r1, r2) = (Uuid::now_v7(), Uuid::now_v7());
        let name = SecretName::parse("stripe").expect("name");
        let other_name = SecretName::parse("stripe2").expect("name");
        let base = associated_data(t1, n1, &name, r1, "k1");
        for other in [
            associated_data(t2, n1, &name, r1, "k1"),
            associated_data(t1, n2, &name, r1, "k1"),
            associated_data(t1, n1, &other_name, r1, "k1"),
            associated_data(t1, n1, &name, r2, "k1"),
            associated_data(t1, n1, &name, r1, "k2"),
        ] {
            assert_ne!(base, other);
        }
        // Moving a boundary between two adjacent parts must not produce the same bytes.
        let a = SecretName::parse("ab").expect("name");
        let b = SecretName::parse("a").expect("name");
        assert_ne!(
            associated_data(t1, n1, &a, r1, "c"),
            associated_data(t1, n1, &b, r1, "bc")
        );
    }
}
