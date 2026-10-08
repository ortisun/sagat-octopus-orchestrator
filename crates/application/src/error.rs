//! Typed errors of the application layer and of its storage port (AGENTS.md rule 10).

use sagat_domain::identity::{IdentityError, Permission};

/// Which uniqueness rule a conflict refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    TenantSlug,
    WorkspaceSlug,
    NamespaceSlug,
}

/// Failure reported by a storage adapter. `detail` is for logs only and must never be returned to
/// API clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("uniqueness conflict")]
    Conflict(ConflictKind),
    #[error("referenced row does not exist")]
    MissingReference,
    #[error("storage unavailable: {detail}")]
    Unavailable { detail: String },
    #[error("storage failure: {detail}")]
    Internal { detail: String },
}

/// Failure of a use case, already expressed in terms the API can translate one-to-one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// No valid identity could be established. Deliberately carries no reason.
    #[error("authentication failed")]
    Unauthenticated,
    /// The caller is known but lacks `permission`.
    #[error("permission denied")]
    Forbidden { permission: Permission },
    /// The resource does not exist *for this caller*. Includes resources of other tenants.
    #[error("resource not found")]
    NotFound,
    #[error("resource already exists")]
    Conflict(ConflictKind),
    /// The request was well-formed but violates a domain rule.
    #[error("invalid input: {0}")]
    Invalid(IdentityError),
    /// A dependency is temporarily unavailable; the caller may retry.
    #[error("service temporarily unavailable")]
    Unavailable,
    #[error("internal error")]
    Internal,
}

impl AppError {
    /// Whether the same request may succeed if repeated later.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable)
    }
}

impl From<IdentityError> for AppError {
    fn from(err: IdentityError) -> Self {
        match err {
            IdentityError::PermissionDenied { permission, .. } => Self::Forbidden { permission },
            // A reference into another tenant must look exactly like a missing resource.
            IdentityError::TenantMismatch => Self::NotFound,
            other => Self::Invalid(other),
        }
    }
}

impl From<StoreError> for AppError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::Conflict(kind) => Self::Conflict(kind),
            StoreError::MissingReference => Self::NotFound,
            StoreError::Unavailable { .. } => Self::Unavailable,
            StoreError::Internal { .. } => Self::Internal,
        }
    }
}
