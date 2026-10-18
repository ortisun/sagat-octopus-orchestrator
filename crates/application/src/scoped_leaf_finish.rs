//! Dormant exact OrdinaryLeafV1 capacity Finish application port.
//!
//! This module defines structural checks and a stable logical-operation fingerprint only. It
//! enables no native adapter, production caller, group lifecycle, or nested execution path.

use std::future::Future;

use sagat_domain::{
    audit::AuditId,
    execution::{
        CapacityGroupId, CapacityMemberId, CapacityPossession, MAX_ITERATION_INDEX, RunId,
    },
    workflow::NodeId,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    error::StoreError, leaf_certification::LeafCollectionPathIdentity,
    primitives::SchedulerAuthority,
};

const MAX_POSTGRES_COUNTER: u64 = (1_u64 << 63) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopedLeafFinishExpectations {
    pub group_revision: u64,
    pub group_epoch: u64,
    pub run_revision: u64,
}

/// Exact ordinary-leaf member Finish command.
///
/// Deliberately has no `Debug` implementation: scheduler owner text is renewable authority data,
/// not safe ordinary diagnostic output.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedLeafFinishCommand {
    pub operation_key: Uuid,
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub identity: LeafCollectionPathIdentity,
    pub expected: ScopedLeafFinishExpectations,
    pub scheduler: SchedulerAuthority,
}

impl ScopedLeafFinishCommand {
    /// Checks only the fixed OrdinaryLeafV1 path shape and PostgreSQL counter bounds.
    ///
    /// This does not prove current scheduler ownership, source evidence, member binding, or CAS
    /// equality.
    ///
    /// # Errors
    /// Returns [`ScopedLeafFinishError::UnsupportedPath`] unless the existing identity is exactly
    /// one ordinary declared owner and an index in `0..=9999`. Returns
    /// [`ScopedLeafFinishError::InvalidRevision`] for zero expectations or an epoch newer than its
    /// group revision, and [`ScopedLeafFinishError::CounterExhausted`] for otherwise valid positive
    /// counters outside PostgreSQL's signed 64-bit range.
    pub fn validate(&self) -> Result<(), ScopedLeafFinishError> {
        let [segment] = self.identity.path().segments() else {
            return Err(ScopedLeafFinishError::UnsupportedPath);
        };
        if segment.iteration_index() > MAX_ITERATION_INDEX
            || NodeId::parse(segment.scope_node().as_str()).is_err()
        {
            return Err(ScopedLeafFinishError::UnsupportedPath);
        }

        let expected = &self.expected;
        if expected.group_revision == 0
            || expected.group_epoch == 0
            || expected.run_revision == 0
            || expected.group_epoch > expected.group_revision
        {
            return Err(ScopedLeafFinishError::InvalidRevision);
        }
        if [
            expected.group_revision,
            expected.group_epoch,
            expected.run_revision,
        ]
        .into_iter()
        .any(|value| value > MAX_POSTGRES_COUNTER)
        {
            return Err(ScopedLeafFinishError::CounterExhausted);
        }
        Ok(())
    }

    /// Hashes the reviewed versioned canonical logical-operation identity.
    ///
    /// The operation key and renewable scheduler credentials deliberately do not participate.
    ///
    /// # Errors
    /// Returns the same structural error as [`Self::validate`] before hashing any command.
    pub fn fingerprint(&self) -> Result<[u8; 32], ScopedLeafFinishError> {
        self.validate()?;
        let segment = &self.identity.path().segments()[0];
        let owner = segment.scope_node().as_str().as_bytes();
        let owner_len =
            u16::try_from(owner.len()).map_err(|_| ScopedLeafFinishError::UnsupportedPath)?;

        let mut hash = Sha256::new();
        hash.update(b"sagat.capacity.finish.scoped-leaf.v1\0");
        for id in [
            self.identity.tenant_id().as_uuid(),
            self.identity.namespace_id().as_uuid(),
            self.group_id.as_uuid(),
            self.member_id.as_uuid(),
            self.identity.run_id().as_uuid(),
        ] {
            hash.update(id.as_bytes());
        }
        hash.update(owner_len.to_be_bytes());
        hash.update(owner);
        hash.update(segment.iteration_index().to_be_bytes());
        for counter in [
            self.expected.group_revision,
            self.expected.group_epoch,
            self.expected.run_revision,
        ] {
            hash.update(counter.to_be_bytes());
        }
        Ok(hash.finalize().into())
    }
}

/// Immutable original receipt result, distinct from the ChildRoot result and current group state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedLeafFinishResult {
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub original_run_id: RunId,
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub finished_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}

#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopedLeafFinishError {
    #[error("unsupported scoped leaf path")]
    UnsupportedPath,
    #[error("invalid capacity finish revision")]
    InvalidRevision,
    #[error("capacity counter cannot advance in PostgreSQL range")]
    CounterExhausted,
    #[error("capacity group is busy")]
    Busy,
    #[error("capacity member, group, source, or run is missing")]
    Missing,
    #[error("capacity member binding does not match the requested scoped leaf")]
    BindingMismatch,
    #[error("scheduler ownership changed")]
    SchedulerFenceLost,
    #[error("unsupported original run context for scoped leaf finish")]
    UnsupportedRunContext,
    #[error("required retained source evidence is missing")]
    SourceEvidenceMissing,
    #[error("unsupported stored source profile")]
    UnsupportedStoredProfile,
    #[error("source evidence is contradictory")]
    ContradictorySource,
    #[error("operation key was reused for a different finish command")]
    FingerprintConflict,
    #[error("stored scoped receipt or capacity projection is contradictory")]
    ContradictoryProjection,
    #[error("capacity group revision changed")]
    StaleGroupRevision { current: u64 },
    #[error("capacity group epoch changed")]
    StaleGroupEpoch { current: u64 },
    #[error("run revision changed")]
    StaleRunRevision { current: u64 },
    #[error("capacity member is not active")]
    InactiveMember,
    #[error("confirmed storage failure")]
    Storage(#[from] StoreError),
    #[error("capacity finish commit outcome is unknown for operation {operation_key}")]
    CommitOutcomeUnknown { operation_key: Uuid },
}

// Preserve StoreError as the diagnostic source for explicit redacted inspection while keeping its
// raw adapter detail out of ordinary Display and Debug output.
impl std::fmt::Debug for ScopedLeafFinishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Future adapters must implement the separately reviewed authority/source/receipt/CAS transaction.
pub trait ScopedLeafFinishStore: Send + Sync {
    fn finish_scoped_leaf(
        &self,
        command: &ScopedLeafFinishCommand,
    ) -> impl Future<Output = Result<ScopedLeafFinishResult, ScopedLeafFinishError>> + Send;
}
