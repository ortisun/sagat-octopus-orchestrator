//! O706: possession is one prerequisite for a new worker lease, not execution authority.
#![allow(clippy::expect_used)]

use proptest::prelude::*;
use sagat_domain::{
    execution::{
        CapacityGroup, CapacityGroupError, CapacityGroupId, CapacityMemberId,
        CapacityMemberSnapshot, CapacityMembership,
    },
    identity::TenantId,
};
use uuid::Uuid;

fn fixture() -> (
    CapacityGroup,
    CapacityMemberSnapshot,
    CapacityMemberSnapshot,
) {
    let group_id = CapacityGroupId::new(Uuid::from_u128(1));
    let holder = CapacityMemberSnapshot::new(
        group_id,
        CapacityMemberId::new(Uuid::from_u128(2)),
        CapacityMembership::Active,
    );
    let mut group = CapacityGroup::new(
        group_id,
        TenantId::new(Uuid::from_u128(3)),
        holder.member_id(),
    );
    let other = group
        .join(
            group.revision(),
            CapacityMemberSnapshot::new(
                group_id,
                CapacityMemberId::new(Uuid::from_u128(4)),
                CapacityMembership::NeverSeen,
            ),
        )
        .expect("join second member");
    (group, holder, other)
}

#[test]
fn only_the_active_holder_passes_without_mutating_the_header() {
    let (group, holder, other) = fixture();
    let before = group.clone();
    assert_eq!(group.check_worker_possession(holder), Ok(()));
    assert_eq!(
        group.check_worker_possession(other),
        Err(CapacityGroupError::NotHolder)
    );
    assert_eq!(group, before);
}

#[test]
fn vacancy_blocks_both_members_until_acquire_commits() {
    let (mut group, holder, other) = fixture();
    group
        .vacate(group.revision(), group.epoch(), holder)
        .expect("vacate");
    for member in [holder, other] {
        assert_eq!(
            group.check_worker_possession(member),
            Err(CapacityGroupError::NotHolder)
        );
    }
    group
        .acquire(group.revision(), group.epoch(), other)
        .expect("acquire");
    assert_eq!(group.check_worker_possession(other), Ok(()));
    assert_eq!(
        group.check_worker_possession(holder),
        Err(CapacityGroupError::NotHolder)
    );
}

#[test]
fn unknown_or_finished_holder_facts_do_not_grant_a_lease() {
    let (group, holder, _) = fixture();
    for state in [CapacityMembership::NeverSeen, CapacityMembership::Finished] {
        assert_eq!(
            group.check_worker_possession(CapacityMemberSnapshot::new(
                group.id(),
                holder.member_id(),
                state,
            )),
            Err(CapacityGroupError::InactiveMember)
        );
    }
}

#[test]
fn foreign_group_cannot_reuse_the_holder_identity() {
    let (group, holder, _) = fixture();
    assert_eq!(
        group.check_worker_possession(CapacityMemberSnapshot::new(
            CapacityGroupId::new(Uuid::from_u128(99)),
            holder.member_id(),
            CapacityMembership::Active,
        )),
        Err(CapacityGroupError::ForeignGroup)
    );
}

#[test]
fn retired_group_refuses_even_a_stale_active_holder_fact() {
    let (mut group, holder, other) = fixture();
    group.finish(group.revision(), other).expect("finish other");
    group
        .finish(group.revision(), holder)
        .expect("finish holder");
    assert_eq!(
        group.check_worker_possession(holder),
        Err(CapacityGroupError::Retired)
    );
}

proptest! {
    #[test]
    fn arbitrary_identities_cannot_bypass_possession(seed in any::<u128>()) {
        let group_id = CapacityGroupId::new(Uuid::from_u128(seed));
        let holder_id = CapacityMemberId::new(Uuid::from_u128(seed.wrapping_add(1)));
        let other_id = CapacityMemberId::new(Uuid::from_u128(seed.wrapping_add(2)));
        let group = CapacityGroup::new(group_id, TenantId::new(Uuid::from_u128(seed)), holder_id);
        let before = group.clone();
        prop_assert_eq!(group.check_worker_possession(CapacityMemberSnapshot::new(
            group_id, holder_id, CapacityMembership::Active,
        )), Ok(()));
        prop_assert_eq!(group.check_worker_possession(CapacityMemberSnapshot::new(
            group_id, other_id, CapacityMembership::Active,
        )), Err(CapacityGroupError::NotHolder));
        prop_assert_eq!(group, before);
    }
}
