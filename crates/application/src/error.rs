//! Typed errors of the application layer and of its storage port (AGENTS.md rule 10).

use sagat_domain::identity::{IdentityError, Permission};
use sagat_domain::scripts::ScriptError;
use sagat_domain::versioning::{RequestId, VersioningError};

use crate::definitions::IrError;

/// Which uniqueness rule a conflict refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    TenantSlug,
    WorkspaceSlug,
    NamespaceSlug,
    /// A workflow with that name already exists in the namespace.
    WorkflowName,
    /// An endpoint (or connector, or subscription) with that name already exists in the namespace.
    EventEndpointSlug,
    /// An event id was used again for a different payload: the same name for something else.
    EventIdReused,
    /// A subscription to outbound events with that name already exists in the namespace.
    WebhookSubscriptionName,
    /// An outbound broker subscription with that name already exists in the namespace.
    BrokerSubscriptionName,
    /// A connector with that name already exists in the namespace.
    ConnectorSlug,
    /// A script with that name already exists in the namespace.
    ScriptName,
    /// A request to publish was published without the approvals it needs.
    RequestNotApproved,
}

/// Why a publication, an approval or a change of the active version was refused (feature 011).
/// Each is a state of the platform the caller can act on, not a malformed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The namespace asks for approvals: the draft goes through a request, not a direct publish.
    ApprovalRequired { required: u8 },
    /// A rule of the request's life (the author deciding, a second decision, a final request).
    Request(VersioningError),
    /// The change breaks what callers and emitters rely on, and the caller did not say they accept it.
    BreakingChange {
        /// How many findings the comparison made.
        findings: usize,
        /// The codes of the first few, so the caller knows what they would be accepting.
        codes: Vec<&'static str>,
    },
    /// The version that was active when the caller looked is not any more.
    ActiveChanged { current: Option<u32> },
    /// A request is open for the workflow already; there is one at a time.
    RequestOpen { request: RequestId },
    /// The target of a promotion has changes in its draft that were not published.
    DraftHasChanges,
}

/// Failure reported by a storage adapter. `detail` is for logs only and must never be returned to
/// API clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("uniqueness conflict")]
    Conflict(ConflictKind),
    #[error("referenced row does not exist")]
    MissingReference,
    /// The draft was changed since the caller read it; `current` is the revision now stored.
    #[error("revision conflict (current: {current})")]
    RevisionConflict { current: u64 },
    /// The `Idempotency-Key` was already used for a different request.
    #[error("idempotency key reused for a different request")]
    IdempotencyKeyReused,
    /// A limit of the tenant or of the platform was reached; `what` names it (never a value).
    #[error("limit reached: {what}")]
    QuotaExceeded { what: &'static str },
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
    /// Someone else changed the draft first; the caller should re-read and retry.
    #[error("the draft changed (current revision {current})")]
    RevisionConflict { current: u64 },
    /// The same `Idempotency-Key` was sent with a different request.
    #[error("idempotency key reused for a different request")]
    IdempotencyKeyReused,
    /// The workflow document is not publishable. The issues name paths and nodes, never payloads.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(Box<IrError>),
    /// A well-formed request that is not acceptable as asked (a limit, a feature not built yet).
    #[error("operation not allowed: {0}")]
    NotAllowed(&'static str),
    /// The worker identity is not bound to the namespace or to an activity it asked to serve
    /// (FR-009-002): it holds `worker:execute`, which is not the same as being allowed this work.
    #[error("the identity is not bound to that namespace and activity")]
    NotBound,
    /// A workflow names a secret that does not exist or that its publisher may not use. Which of
    /// the two is deliberately not said: someone who may not use a secret must not learn which
    /// names exist in the namespace.
    #[error("a secret does not exist or may not be used")]
    SecretUnavailable { name: String },
    /// The caller holds a lease that is no longer current (the attempt was reclaimed, cancelled or
    /// already finished): its result is refused. Idempotent replays are answered before this.
    #[error("the attempt is stale")]
    StaleAttempt,
    /// A request the *current state* of the resource cannot accept (cancelling a finished run).
    #[error("conflict with the current state: {0}")]
    StateConflict(&'static str),
    /// The request was well-formed but violates a domain rule.
    #[error("invalid input: {0}")]
    Invalid(IdentityError),
    /// A script, a schema or a limit of a script that the platform cannot take.
    #[error("invalid script: {0}")]
    InvalidScript(ScriptError),
    /// A limit was reached (for example the size of a child-run tree); `what` names it.
    #[error("limit reached: {0}")]
    QuotaExceeded(&'static str),
    /// Publishing, approving or activating was refused because of where the platform stands.
    #[error("refused: {0:?}")]
    Versioning(Refusal),
    /// A dependency is temporarily unavailable; the caller may retry.
    #[error("service temporarily unavailable")]
    Unavailable,
    /// What was sent is larger than the endpoint takes.
    #[error("the payload is larger than allowed: {0}")]
    PayloadTooLarge(&'static str),
    /// The platform is full of this caller's work and asks it to come back in `retry_after_secs`
    /// seconds: nothing is wrong with the request.
    #[error("busy; retry in {retry_after_secs} s")]
    Busy { retry_after_secs: u32 },
    #[error("internal error")]
    Internal,
}

impl AppError {
    /// Whether the same request may succeed if repeated later.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable | Self::Busy { .. })
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
            StoreError::RevisionConflict { current } => Self::RevisionConflict { current },
            StoreError::IdempotencyKeyReused => Self::IdempotencyKeyReused,
            StoreError::QuotaExceeded { what } => Self::QuotaExceeded(what),
            StoreError::Unavailable { .. } => Self::Unavailable,
            StoreError::Internal { .. } => Self::Internal,
        }
    }
}
