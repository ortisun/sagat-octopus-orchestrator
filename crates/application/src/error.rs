//! Typed errors of the application layer and of its storage port (AGENTS.md rule 10).

use sagat_domain::database::LakeRefusal;
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
    /// A schedule with that name already exists in the namespace.
    ScheduleName,
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
    #[error("database connection is unavailable or may not be used")]
    DatabaseUnavailable,
    #[error("database operation exceeds connection limits")]
    DatabaseLimit,
    /// The write sets a `pii` egress ceiling and the actor does not hold the namespace's reveal grant (FR-035-005).
    /// It is a refusal of the actor, not of the resource: the actor does manage connections, so nothing is hidden.
    #[error("setting the pii egress ceiling needs the payload reveal grant")]
    RevealGrantRequired,
    /// The settings of an Apache Iceberg connection refuse a step: a mode it does not allow, or fields above its ceiling
    /// (FR-035-005, FR-035-025). The step's author may use the connection, so nothing is hidden by saying which node.
    #[error("an Apache Iceberg write is refused by the settings of its connection")]
    LakeRefused { node: String, refusal: LakeRefusal },
    #[error("conflict with the current state: {0}")]
    StateConflict(&'static str),
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
    /// The workflow version a run is pinned to is stored but this build cannot read it (it was
    /// written by a newer build, and this one was rolled back). The run is not damaged and is not
    /// this build's to touch: callers that serve many runs skip it and go on (G460).
    #[error("a stored workflow version cannot be read by this build")]
    UnreadableDefinition,
    /// A start's business identifier is held (feature 018, FR-018-019/020): `reason` is the closed
    /// `details.reason` of the contract, `holder` the execution that holds it and `reusable_after`
    /// the instant its window ends, for a closed holder.
    #[error("business identifier held: {reason}")]
    BusinessIdHeld {
        reason: &'static str,
        holder: uuid::Uuid,
        reusable_after: Option<time::OffsetDateTime>,
    },
}

impl StoreError {
    /// The SQLSTATE of a failure that a store reported by it and by nothing else of the error: five upper-case letters and digits
    /// (`57014` a statement over its timeout, `40P01` a deadlock), written by the adapter as `sqlstate:XXXXX` in `detail`
    /// (G2094, D2203). It is the one thing of the error of a selection round that may be logged: the text of a database error can
    /// quote a value. `None` for every other error, whatever its text, and for a failure that was no database error.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        match self {
            Self::Unavailable { detail } | Self::Internal { detail } => {
                detail.strip_prefix("sqlstate:").filter(|code| {
                    code.len() == 5
                        && code
                            .bytes()
                            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
                })
            }
            _ => None,
        }
    }
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
    /// A connection (feature 035) whose settings are not acceptable: `reason` is the closed `details.reason` of the
    /// contract (`CREDENTIAL_IN_URI`, ...). It names a rule and never a value of the request.
    #[error("the connection settings are not acceptable: {reason}")]
    InvalidConnection { reason: &'static str },
    /// A schedule (feature 018) that is not acceptable: `reason` is the closed `details.reason`
    /// of the contract and `field` the place of the request it concerns. Never a payload.
    #[error("invalid schedule: {reason}")]
    InvalidSchedule {
        reason: &'static str,
        field: Option<String>,
    },
    /// A schedule that is in a state the request cannot be taken in (`details.reason` of the
    /// contract): owned by a cron trigger, or waiting for the trigger handover.
    #[error("schedule conflict: {reason}")]
    ScheduleConflict { reason: &'static str },
    /// A batch operation (feature 018) that is not acceptable: `reason` is the closed
    /// `details.reason` of the contract and `field` the place of the request it concerns. Never a
    /// payload.
    #[error("invalid batch operation: {reason}")]
    InvalidBatch {
        reason: &'static str,
        field: Option<String>,
    },
    /// A batch job that is in a state the request cannot be taken in (`details.reason` of the
    /// contract): a job that already ended cannot be cancelled.
    #[error("batch conflict: {reason}")]
    BatchConflict { reason: &'static str },
    /// A start option (feature 018) that is not acceptable: `reason` is the closed `details.reason`
    /// of the contract (`BUSINESS_ID_INVALID`) and `field` the place of the request it concerns.
    /// Never the value.
    #[error("invalid start option: {reason}")]
    InvalidStart {
        reason: &'static str,
        field: Option<String>,
    },
    /// A signal that a with-start request cannot deliver (feature 018, `FR-018-024`): `reason` is the
    /// closed `details.reason` of the contract, `NOT_WAITING` (the open run does not wait for it now)
    /// or `SIGNAL_NOT_ACCEPTED_AT_START` (nothing in the new run waits for it, so nothing was
    /// created). A signal is never buffered.
    #[error("signal refused: {reason}")]
    SignalRefused { reason: &'static str },
    /// A reset to a point that cannot be made (feature 018, `FR-018-035`..`FR-018-039`): `reason` is the
    /// closed `details.reason` of the contract (`RESET_POINT_NOT_ELIGIBLE`, `RESET_RULE_MISSING`,
    /// `RESET_REEXECUTION_NOT_ACKNOWLEDGED`, `RESET_SOURCE_NOT_TERMINAL`, ...) and `event_type` names the type
    /// without a rule when that is the reason. Nothing was created.
    #[error("reset refused: {reason}")]
    ResetRefused {
        reason: &'static str,
        event_type: Option<String>,
    },
    /// A declared Update that an Update-with-Start request carried was refused before the run was
    /// admitted (feature 018, `FR-018-025`): an undeclared operation, arguments the declaration
    /// does not accept, a key reused with another request, a run that no longer admits updates.
    /// Nothing was created. The API translates it as the declared-operations routes do.
    #[error("declared update refused: {0}")]
    DeclaredUpdate(#[from] crate::declared_operations::DeclaredOperationStoreError),
    /// A business identifier that an open or remembered execution holds (`BUSINESS_ID_IN_USE`,
    /// `BUSINESS_ID_REUSE_REJECTED`). `holder` names the execution; the API shows it only to a caller
    /// who may read runs. `reusable_after` is when a remembered id is free again.
    #[error("business identifier conflict: {reason}")]
    BusinessIdConflict {
        reason: &'static str,
        holder: sagat_domain::execution::RunId,
        reusable_after: Option<time::OffsetDateTime>,
    },
    /// Another attempt was asked of a step whose last outcome is unknown; it must be settled
    /// (`RESOLVE_STEP`) before anything is repeated, by any role (feature 037).
    #[error("the step's last outcome is unknown and must be settled first")]
    OutcomeUnresolved,
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

/// Why a caller was refused with a `403` / `PERMISSION_DENIED` (gap G9): what an audit record of the
/// denial says. Never carries a value the caller chose (a secret's name, an identifier).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDenial {
    /// The caller's role does not grant the permission.
    MissingPermission(Permission),
    /// The identity holds the permission but may not do this: a closed code (`NOT_BOUND`,
    /// `SECRET_NOT_USABLE`, `AUTHOR_CANNOT_DECIDE`, `NOT_THE_AUTHOR`, or `FORBIDDEN` for any other).
    Refused(&'static str),
}

impl AccessDenial {
    /// The text stored as the reason of the audit record.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::MissingPermission(permission) => {
                format!("missing permission {}", permission.as_str())
            }
            Self::Refused(code) => format!("refused: {code}"),
        }
    }
}

impl AppError {
    /// The denial this error is answered with, when it is answered with `403` /
    /// `PERMISSION_DENIED`. A `404` that hides a foreign resource is not one: it has its own
    /// cross-tenant record.
    #[must_use]
    pub const fn denial(&self) -> Option<AccessDenial> {
        match self {
            Self::Forbidden { permission } => Some(AccessDenial::MissingPermission(*permission)),
            Self::NotBound => Some(AccessDenial::Refused("NOT_BOUND")),
            Self::SecretUnavailable { .. } => Some(AccessDenial::Refused("SECRET_NOT_USABLE")),
            Self::Versioning(Refusal::Request(VersioningError::AuthorCannotDecide)) => {
                Some(AccessDenial::Refused("AUTHOR_CANNOT_DECIDE"))
            }
            Self::Versioning(Refusal::Request(VersioningError::NotTheAuthor)) => {
                Some(AccessDenial::Refused("NOT_THE_AUTHOR"))
            }
            _ => None,
        }
    }

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
            StoreError::DatabaseUnavailable => Self::NotFound,
            StoreError::DatabaseLimit => {
                Self::NotAllowed("database operation exceeds connection limits")
            }
            StoreError::RevealGrantRequired => Self::Forbidden {
                permission: Permission::PayloadRevealPii,
            },
            StoreError::LakeRefused { node, refusal } => {
                Self::InvalidDefinition(Box::new(IrError::LakeWriteRefused { node, refusal }))
            }
            StoreError::StateConflict(reason) => Self::StateConflict(reason),
            StoreError::Conflict(kind) => Self::Conflict(kind),
            StoreError::MissingReference => Self::NotFound,
            StoreError::RevisionConflict { current } => Self::RevisionConflict { current },
            StoreError::IdempotencyKeyReused => Self::IdempotencyKeyReused,
            StoreError::QuotaExceeded { what } => Self::QuotaExceeded(what),
            StoreError::Unavailable { .. } => Self::Unavailable,
            StoreError::Internal { .. } | StoreError::UnreadableDefinition => Self::Internal,
            StoreError::BusinessIdHeld {
                reason,
                holder,
                reusable_after,
            } => Self::BusinessIdConflict {
                reason,
                holder: sagat_domain::execution::RunId::new(holder),
                reusable_after,
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::StoreError;

    /// The one thing of the error of a selection round that may be logged is its SQLSTATE, and only as the closed code the adapter
    /// writes: five upper-case letters and digits after `sqlstate:`. Any other text, however it is dressed, is nothing (G2094, D2203).
    /// Catches: a reader that hands back a longer or looser text (a message that quotes a value would pass for a code).
    #[test]
    fn only_a_closed_sqlstate_is_read_from_an_error_and_never_its_text() {
        let unavailable = |detail: &str| StoreError::Unavailable {
            detail: detail.to_owned(),
        };
        let internal = |detail: &str| StoreError::Internal {
            detail: detail.to_owned(),
        };
        assert_eq!(unavailable("sqlstate:57014").sqlstate(), Some("57014"));
        assert_eq!(internal("sqlstate:40P01").sqlstate(), Some("40P01"));
        for text in [
            "sqlstate:none",
            "sqlstate:",
            "sqlstate:5701",
            "sqlstate:570144",
            "sqlstate:57o14",
            "sqlstate:57014 and a value that was quoted",
            "error returned from database: the statement over canary-1 was cancelled",
            "",
        ] {
            assert_eq!(unavailable(text).sqlstate(), None, "{text:?}");
            assert_eq!(internal(text).sqlstate(), None, "{text:?}");
        }
        // an error that is no failure of the store says no code, whatever it is
        assert_eq!(StoreError::MissingReference.sqlstate(), None);
        assert_eq!(StoreError::StateConflict("sqlstate:57014").sqlstate(), None);
    }
}
