//! Dormant immutable source-certification port, fixed ordinary-leaf V1 only.
//!
//! A dormant PostgreSQL writer is verified separately; no production caller is enabled. One exact
//! identity is the canonical
//! receipt. Current authority precedes replay; creation CAS applies only on a receipt miss.
//! Cancellation/uncertain COMMIT requires fresh same-identity reconciliation, never assumed rollback.
//! Storage must prove the pinned source facts and preserve receipt/source retention before use.

use std::future::Future;

use sagat_domain::{
    audit::AuditId,
    definition::WorkflowVersionId,
    execution::{ExecutionInstanceId, InstancePath, RunId},
    identity::{NamespaceId, TenantId},
    workflow::{CollectionSuccessShapeError, Digest, NodeId},
};
use time::OffsetDateTime;

use crate::{
    error::StoreError,
    primitives::{CollectionScopeId, SchedulerAuthority},
};

/// Canonical source-fact identity, not an execution capability or a caller's closure assertion.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeafCollectionPathIdentity {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    run_id: RunId,
    path: InstancePath,
}

impl LeafCollectionPathIdentity {
    /// # Errors
    /// Only one root-owned iteration path is supported by this profile.
    pub fn new(
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        path: InstancePath,
    ) -> Result<Self, LeafCollectionSuccessError> {
        if path.depth() != 1 {
            return Err(LeafCollectionSuccessError::UnsupportedPath);
        }
        Ok(Self {
            tenant_id,
            namespace_id,
            run_id,
            path,
        })
    }
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }
    #[must_use]
    pub const fn path(&self) -> &InstancePath {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifyLeafCollectionSuccess {
    pub identity: LeafCollectionPathIdentity,
    /// Creation CAS only; an existing canonical record replays before this comparison.
    pub expected_run_revision: u64,
    /// Required on creation and replay; this data is not a live authority proof.
    pub scheduler: SchedulerAuthority,
}

impl CertifyLeafCollectionSuccess {
    /// Structural command bounds only. Does not check current revision or scheduler ownership.
    ///
    /// # Errors
    /// Revisions are positive and must fit the platform's signed 64-bit source journal.
    pub const fn validate(&self) -> Result<(), LeafCollectionSuccessError> {
        if self.expected_run_revision == 0 || self.expected_run_revision >= (1_u64 << 63) {
            Err(LeafCollectionSuccessError::InvalidRevision)
        } else {
            Ok(())
        }
    }
}

/// Closed source rules, including definition shape contract 1 and its checked producer invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafCollectionSuccessProfile {
    OrdinaryLeafV1,
}

/// Trusted storage snapshot of a canonical source certification. Constructing this value or
/// passing validate_structure does not establish that its facts exist in a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafCollectionSuccessRecord {
    pub identity: LeafCollectionPathIdentity,
    pub profile: LeafCollectionSuccessProfile,
    pub definition_version: WorkflowVersionId,
    pub scope_id: CollectionScopeId,
    pub body: NodeId,
    pub return_instance: ExecutionInstanceId,
    pub admitted_sequence: u64,
    pub returned_sequence: u64,
    pub checkpoint_sequence: u64,
    pub checkpoint_result_digest: Digest,
    pub finished_sequence: u64,
    pub aggregate_result_digest: Digest,
    pub owner_routed_sequence: u64,
    pub certified_run_revision: u64,
    pub certified_at: OffsetDateTime,
    pub audit_id: AuditId,
}

impl LeafCollectionSuccessRecord {
    /// Check only qualified return-path binding and V1 source-reference relationships. The store
    /// must independently validate the actual pin, scope, journal, digests and complete local set.
    ///
    /// # Errors
    /// A malformed stored snapshot is contradictory, not merely unfinished source work.
    pub fn validate_structure(&self) -> Result<(), LeafCollectionSuccessError> {
        let counters = [
            self.admitted_sequence,
            self.returned_sequence,
            self.checkpoint_sequence,
            self.finished_sequence,
            self.owner_routed_sequence,
            self.certified_run_revision,
        ];
        if counters
            .into_iter()
            .any(|value| value == 0 || value >= (1_u64 << 63))
            || self.return_instance.path() != self.identity.path()
            || self.admitted_sequence >= self.returned_sequence
            || self.returned_sequence.checked_add(1) != Some(self.checkpoint_sequence)
            || self.checkpoint_sequence >= self.finished_sequence
            || self.finished_sequence.checked_add(1) != Some(self.owner_routed_sequence)
            || self.owner_routed_sequence > self.certified_run_revision
        {
            return Err(LeafCollectionSuccessError::ContradictorySource);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafCollectionSuccessResult {
    pub record: LeafCollectionSuccessRecord,
    pub replayed: bool,
}

#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeafCollectionSuccessError {
    #[error("unsupported source path")]
    UnsupportedPath,
    #[error("source resource is missing")]
    Missing,
    #[error("unsupported definition shape: {0}")]
    UnsupportedShape(#[from] CollectionSuccessShapeError),
    #[error("unsupported stored source profile")]
    UnsupportedStoredProfile,
    #[error("scheduler ownership changed")]
    SchedulerFenceLost,
    #[error("run revision changed")]
    StaleRunRevision { current: u64 },
    #[error("source is not closed")]
    SourceNotClosed,
    #[error("required retained source evidence is missing")]
    SourceEvidenceMissing,
    #[error("source evidence is contradictory")]
    ContradictorySource,
    #[error("invalid source revision")]
    InvalidRevision,
    #[error("confirmed storage failure")]
    Storage(#[from] StoreError),
    #[error("certification commit outcome is unknown")]
    CommitOutcomeUnknown {
        identity: LeafCollectionPathIdentity,
    },
}

// Preserve the diagnostic source for explicit internal inspection without including it (or the
// uncertain command identity) in ordinary error logging. Explicit source-chain formatting still
// requires redaction at the caller boundary.
impl std::fmt::Debug for LeafCollectionSuccessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Adapters must follow the reviewed exact guard,
/// authority/replay/CAS/source-proof/commit order and atomic canonical-record/audit contract.
pub trait LeafCollectionSuccessStore: Send + Sync {
    fn certify_leaf_collection_success(
        &self,
        command: &CertifyLeafCollectionSuccess,
    ) -> impl Future<Output = Result<LeafCollectionSuccessResult, LeafCollectionSuccessError>> + Send;
}
