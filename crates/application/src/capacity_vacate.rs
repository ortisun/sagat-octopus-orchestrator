//! Exact ChildRoot Vacate contract. Dormant until durable callers and claim guards exist.
//! Retain the complete command for same-key reconciliation, including after cancellation.
use crate::{
    capacity_finish::{CapacityFinishExpectations, CapacityFinishIdentity},
    error::StoreError,
    primitives::SchedulerAuthority,
};
use sagat_domain::audit::AuditId;
use sagat_domain::execution::{CapacityGroupId, CapacityMemberId, CapacityPossession, RunId};
use sha2::{Digest as _, Sha256};
use std::future::Future;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityVacateCommand {
    pub operation_key: Uuid,
    pub identity: CapacityFinishIdentity,
    pub expected: CapacityFinishExpectations,
    pub scheduler: SchedulerAuthority,
}
impl CapacityVacateCommand {
    /// Domain-separated fixed-width identity; renewable authority and operation key are excluded.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"sagat.capacity.transfer.child-root.v1\0");
        hash.update([0_u8]); // VACATE. ACQUIRE has no native implementation in this slice.
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
        for value in [
            self.expected.group_revision,
            self.expected.group_epoch,
            self.expected.chain_revision,
            self.expected.run_revision,
        ] {
            hash.update(value.to_be_bytes());
        }
        hash.finalize().into()
    }
}

/// Original immutable transition receipt, never current permission to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityVacateResult {
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub head_run_id: RunId,
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub vacated_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapacityVacateError {
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
    #[error("the selected member does not hold the group")]
    NotHolder,
    #[error("the head run still has a leased or running step")]
    NotQuiescent,
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
pub trait CurrentChildRootVacateStore: Send + Sync {
    fn vacate_current_child_root(
        &self,
        command: &CapacityVacateCommand,
    ) -> impl Future<Output = Result<CapacityVacateResult, CapacityVacateError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::{
        execution::ChainScope,
        identity::{NamespaceId, TenantId},
    };
    fn command() -> CapacityVacateCommand {
        let id = Uuid::from_u128;
        let head = RunId::new(id(6));
        CapacityVacateCommand {
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
    fn canonical_vacate_fingerprint_is_independently_specified() {
        let c = command();
        // Python hashlib: prefix + VACATE byte0 + six UUID bytes + four unsigned BE64.
        let expected: [u8; 32] = [
            18, 41, 163, 222, 16, 43, 36, 218, 246, 239, 128, 73, 50, 72, 241, 56, 223, 27, 58,
            180, 59, 249, 159, 20, 14, 62, 97, 13, 2, 171, 2, 101,
        ];
        assert_eq!(c.fingerprint(), expected);
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
}
