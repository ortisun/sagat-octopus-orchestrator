//! Dormant exact ChildRoot Finish port. Possession is not execution eligibility.
//!
//! No production caller is enabled. A caller must retain the complete original command for
//! same-key reconciliation. Cancellation after invocation can race with COMMIT dispatch and
//! produces no return value: it is never evidence that the operation rolled back. A production
//! caller needs durable intent or deterministic command reconstruction before using this port.

use std::future::Future;

use sagat_domain::audit::AuditId;
use sagat_domain::execution::{
    CapacityGroupId, CapacityMemberId, CapacityPossession, ChainScope, RunId,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::StoreError, primitives::SchedulerAuthority};

/// Exact logical member and physical head observed by the caller. Never forwards to a successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityFinishIdentity {
    pub scope: ChainScope,
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub chain_id: RunId,
    pub head_run_id: RunId,
}
impl CapacityFinishIdentity {
    #[must_use]
    pub const fn new(
        scope: ChainScope,
        group_id: CapacityGroupId,
        member_id: CapacityMemberId,
        chain_id: RunId,
        head_run_id: RunId,
    ) -> Self {
        Self {
            scope,
            group_id,
            member_id,
            chain_id,
            head_run_id,
        }
    }
}

/// Independent CAS/fencing counters; current scheduler credentials may be renewed on retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityFinishExpectations {
    pub group_revision: u64,
    pub group_epoch: u64,
    pub chain_revision: u64,
    pub run_revision: u64,
}
impl CapacityFinishExpectations {
    #[must_use]
    pub const fn new(
        group_revision: u64,
        group_epoch: u64,
        chain_revision: u64,
        run_revision: u64,
    ) -> Self {
        Self {
            group_revision,
            group_epoch,
            chain_revision,
            run_revision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityFinishCommand {
    pub operation_key: Uuid,
    pub identity: CapacityFinishIdentity,
    pub expected: CapacityFinishExpectations,
    pub scheduler: SchedulerAuthority,
}
impl CapacityFinishCommand {
    #[must_use]
    pub const fn new(
        operation_key: Uuid,
        identity: CapacityFinishIdentity,
        expected: CapacityFinishExpectations,
        scheduler: SchedulerAuthority,
    ) -> Self {
        Self {
            operation_key,
            identity,
            expected,
            scheduler,
        }
    }

    /// Versioned fixed-width canonical identity. Renewable authority and trace metadata are not
    /// part of the logical operation. The operation key scopes the separate receipt lookup.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"sagat.capacity.finish.child-root.v1\0");
        for id in [
            self.identity.scope.tenant_id().as_uuid(),
            self.identity.scope.namespace_id().as_uuid(),
            self.identity.group_id.as_uuid(),
            self.identity.member_id.as_uuid(),
            self.identity.chain_id.as_uuid(),
            self.identity.head_run_id.as_uuid(),
        ] {
            hash.update(id.as_bytes());
        }
        for counter in [
            self.expected.group_revision,
            self.expected.group_epoch,
            self.expected.chain_revision,
            self.expected.run_revision,
        ] {
            hash.update(counter.to_be_bytes());
        }
        hash.finalize().into()
    }
}

/// Original immutable receipt result, not the group's current execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityFinishResult {
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub head_run_id: RunId,
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub finished_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapacityFinishError {
    #[error("capacity group is busy")]
    Busy,
    #[error("capacity member, group, chain or run is missing")]
    Missing,
    #[error("capacity member binding does not match the requested child root")]
    BindingMismatch,
    #[error("execution chain head changed")]
    StaleHead { current: RunId },
    #[error("capacity group revision changed")]
    StaleGroupRevision { current: u64 },
    #[error("capacity group epoch changed")]
    StaleGroupEpoch { current: u64 },
    #[error("execution chain revision changed")]
    StaleChainRevision { current: u64 },
    #[error("run revision changed")]
    StaleRunRevision { current: u64 },
    #[error("scheduler ownership changed")]
    SchedulerFenceLost,
    #[error("current child root is not final")]
    NotFinal,
    #[error("stored child-root finality or capacity projection is contradictory")]
    ContradictoryProjection,
    #[error("capacity member is not active")]
    InactiveMember,
    #[error("operation key was reused for a different finish command")]
    FingerprintConflict,
    #[error("capacity counter cannot advance in PostgreSQL range")]
    CounterExhausted,
    #[error("confirmed storage failure: {0}")]
    Storage(#[from] StoreError),
    #[error("commit outcome is unknown for operation {operation_key}")]
    CommitOutcomeUnknown { operation_key: Uuid },
}

pub trait CurrentChildRootFinishStore: Send + Sync {
    fn finish_current_child_root(
        &self,
        command: &CapacityFinishCommand,
    ) -> impl Future<Output = Result<CapacityFinishResult, CapacityFinishError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::identity::{NamespaceId, TenantId};

    #[test]
    fn canonical_fingerprint_vector_survives_scheduler_renewal() {
        let id = |n| Uuid::from_u128(n);
        let head = RunId::new(id(6));
        let mut command = CapacityFinishCommand::new(
            id(11),
            CapacityFinishIdentity::new(
                ChainScope::new(TenantId::new(id(1)), NamespaceId::new(id(2))),
                CapacityGroupId::new(id(3)),
                CapacityMemberId::new(id(4)),
                RunId::new(id(5)),
                head,
            ),
            CapacityFinishExpectations::new(7, 3, 9, 10),
            SchedulerAuthority::for_run("old", head, 12),
        );
        // Independently generated Python hashlib SHA-256 vector, UUID bytes and unsigned BE64.
        let expected = [
            0xe1, 0x4d, 0x30, 0xd3, 0x8f, 0xc3, 0xb8, 0x88, 0x7a, 0x7c, 0x23, 0x53, 0x4d, 0xd3,
            0x16, 0x0e, 0xad, 0x01, 0x29, 0x83, 0xa4, 0x01, 0x9e, 0x41, 0x3f, 0xf5, 0x8b, 0x98,
            0xc0, 0xbd, 0xd7, 0xba,
        ];
        assert_eq!(command.fingerprint(), expected);
        command.scheduler = SchedulerAuthority::for_run("renewed", head, 13);
        assert_eq!(command.fingerprint(), expected);
    }
}
