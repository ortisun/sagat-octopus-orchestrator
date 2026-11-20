//! Exact ChildRoot Acquire contract. Dormant; possession is not an execution permit.
//! Retain the complete command for same-key reconciliation after COMMIT uncertainty.

use crate::{
    capacity_finish::{CapacityFinishExpectations, CapacityFinishIdentity},
    error::StoreError,
    primitives::SchedulerAuthority,
};
use sagat_domain::audit::AuditId;
use sagat_domain::execution::{CapacityGroupId, CapacityMemberId, CapacityPossession, RunId};
use std::future::Future;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityAcquireCommand {
    pub operation_key: Uuid,
    pub identity: CapacityFinishIdentity,
    pub expected: CapacityFinishExpectations,
    pub scheduler: SchedulerAuthority,
}

impl CapacityAcquireCommand {
    /// Domain-separated identity; excludes renewable authority and the receipt lookup key.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        crate::capacity_transfer::fingerprint(
            crate::capacity_transfer::TransferKind::Acquire,
            self.identity,
            self.expected,
        )
    }
}

/// Original immutable transition receipt; replay is not evidence of current possession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityAcquireResult {
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub head_run_id: RunId,
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub acquired_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapacityAcquireError {
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
    #[error("capacity group is already held")]
    Occupied,
    #[error("the head run cannot move in its current status")]
    NotMovable,
    #[error("the current child chain is final: finish the member instead")]
    ChainFinal,
    #[error("stored execution or capacity projection is contradictory")]
    ContradictoryProjection,
    #[error("capacity member is not active")]
    InactiveMember,
    #[error("operation key was reused for a different transfer command")]
    FingerprintConflict,
    #[error("capacity counter cannot advance in PostgreSQL range")]
    CounterExhausted,
    #[error("confirmed storage failure: {0}")]
    Storage(#[from] StoreError),
    #[error("commit outcome is unknown for operation {operation_key}")]
    CommitOutcomeUnknown { operation_key: Uuid },
}

pub trait CurrentChildRootAcquireStore: Send + Sync {
    fn acquire_current_child_root(
        &self,
        command: &CapacityAcquireCommand,
    ) -> impl Future<Output = Result<CapacityAcquireResult, CapacityAcquireError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity_vacate::CapacityVacateCommand;
    use sagat_domain::{
        execution::ChainScope,
        identity::{NamespaceId, TenantId},
    };

    fn command() -> CapacityAcquireCommand {
        let id = Uuid::from_u128;
        let head = RunId::new(id(6));
        CapacityAcquireCommand {
            operation_key: id(11),
            identity: CapacityFinishIdentity::new(
                ChainScope::new(TenantId::new(id(1)), NamespaceId::new(id(2))),
                CapacityGroupId::new(id(3)),
                CapacityMemberId::new(id(4)),
                RunId::new(id(5)),
                head,
            ),
            expected: CapacityFinishExpectations::new(7, 3, 9, 10),
            scheduler: SchedulerAuthority::for_run("old", head, 12),
        }
    }

    #[test]
    fn canonical_acquire_fingerprint_is_independently_specified() {
        // Python hashlib: prefix + ACQUIRE byte1 + six UUID bytes + four unsigned BE64.
        let expected = [
            57, 169, 32, 180, 60, 97, 100, 12, 96, 114, 199, 38, 237, 130, 201, 176, 101, 144, 18,
            142, 220, 241, 14, 250, 135, 185, 108, 84, 121, 40, 120, 86,
        ];
        assert_eq!(command().fingerprint(), expected);
    }

    #[test]
    fn renewal_and_receipt_key_do_not_change_logical_identity() {
        let a = command();
        let mut b = a.clone();
        b.operation_key = Uuid::from_u128(100);
        b.scheduler = SchedulerAuthority::for_run("renewed", a.identity.head_run_id, 99);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.expected.run_revision += 1;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn every_binding_and_cas_counter_changes_the_fingerprint() {
        let a = command();
        let fresh = Uuid::from_u128(100);
        let mut changed = Vec::new();
        for n in 0..10 {
            let mut b = a.clone();
            match n {
                0 => {
                    b.identity.scope =
                        ChainScope::new(TenantId::new(fresh), a.identity.scope.namespace_id())
                }
                1 => {
                    b.identity.scope =
                        ChainScope::new(a.identity.scope.tenant_id(), NamespaceId::new(fresh))
                }
                2 => b.identity.group_id = CapacityGroupId::new(fresh),
                3 => b.identity.member_id = CapacityMemberId::new(fresh),
                4 => b.identity.chain_id = RunId::new(fresh),
                5 => b.identity.head_run_id = RunId::new(fresh),
                6 => b.expected.group_revision += 1,
                7 => b.expected.group_epoch += 1,
                8 => b.expected.chain_revision += 1,
                9 => b.expected.run_revision += 1,
                _ => unreachable!(),
            }
            assert_ne!(a.fingerprint(), b.fingerprint(), "binding/counter {n}");
            changed.push(b.fingerprint());
        }
        changed.sort();
        changed.dedup();
        assert_eq!(changed.len(), 10);
    }

    #[test]
    fn acquire_and_vacate_have_distinct_operation_domains() {
        let a = command();
        let v = CapacityVacateCommand {
            operation_key: a.operation_key,
            identity: a.identity,
            expected: a.expected,
            scheduler: a.scheduler.clone(),
        };
        assert_ne!(a.fingerprint(), v.fingerprint());
    }
}
