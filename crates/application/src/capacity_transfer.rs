//! Canonical logical identity for the two ChildRoot transfer operations.

use crate::capacity_finish::{CapacityFinishExpectations, CapacityFinishIdentity};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TransferKind {
    Vacate,
    Acquire,
}

pub(crate) fn fingerprint(
    kind: TransferKind,
    identity: CapacityFinishIdentity,
    expected: CapacityFinishExpectations,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"sagat.capacity.transfer.child-root.v1\0");
    hash.update([match kind {
        TransferKind::Vacate => 0_u8,
        TransferKind::Acquire => 1_u8,
    }]);
    for id in [
        identity.scope.tenant_id().as_uuid(),
        identity.scope.namespace_id().as_uuid(),
        identity.group_id.as_uuid(),
        identity.member_id.as_uuid(),
        identity.chain_id.as_uuid(),
        identity.head_run_id.as_uuid(),
    ] {
        hash.update(id.as_bytes());
    }
    for value in [
        expected.group_revision,
        expected.group_epoch,
        expected.chain_revision,
        expected.run_revision,
    ] {
        hash.update(value.to_be_bytes());
    }
    hash.finalize().into()
}
