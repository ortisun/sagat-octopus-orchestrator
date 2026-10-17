//! Async secret-provider boundary. Provider paths and credentials never belong to wire callers.
use std::{future::Future, pin::Pin};

use sagat_domain::{
    identity::{NamespaceId, TenantId},
    secrets::SecretValue,
};
use uuid::Uuid;

/// Identity allocated by a committed provisioning intent. Each logical version has its own
/// immutable provider object; the provider's version is always one, not the logical version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReference {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub operation: Uuid,
}

/// Closed, credential-free failures. Provider diagnostics must never be attached to these errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("secret provider configuration is invalid")]
    Configuration,
    #[error("secret provider is unavailable")]
    Unavailable,
    #[error("secret provider access was denied")]
    Denied,
    #[error("secret provider operation already exists")]
    Conflict,
    #[error("secret provider rejected the operation")]
    Rejected,
    #[error("secret provider response is invalid")]
    InvalidResponse,
}

pub type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

/// A bounded provider adapter, not a wildcard proxy. Authorization/audit and durable activation
/// belong to the application service. None of these methods changes a logical secret's authority.
pub trait SecretProvider: Send + Sync {
    /// CAS create only. A conflict requires reading and verifying the existing receipt; it must
    /// never be interpreted as a successful write of the caller's current value.
    fn create<'a>(
        &'a self,
        reference: &'a ProviderReference,
        value: &'a SecretValue,
    ) -> ProviderFuture<'a, ()>;
    fn read<'a>(
        &'a self,
        reference: &'a ProviderReference,
    ) -> ProviderFuture<'a, Option<SecretValue>>;
    /// Permanently destroy unreferenced value bytes and fence future creates at this identity.
    /// Provider metadata may remain as a noncredential tombstone. Safe to repeat after lost replies.
    fn delete<'a>(&'a self, reference: &'a ProviderReference) -> ProviderFuture<'a, ()>;
}
