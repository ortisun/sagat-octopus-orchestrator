//! A1 pure lifecycle only: trusted exact membership facts, no execution or storage permission.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sagat_domain::execution::{
    CapacityGroup, CapacityGroupError, CapacityGroupId, CapacityMemberId, CapacityMemberSnapshot,
    CapacityMembership, CapacityPossession,
};
use sagat_domain::identity::TenantId;
use uuid::Uuid;

fn member(n: u128) -> CapacityMemberId {
    CapacityMemberId::new(Uuid::from_u128(n))
}

fn group() -> CapacityGroup {
    CapacityGroup::new(
        CapacityGroupId::new(Uuid::from_u128(100)),
        TenantId::new(Uuid::from_u128(200)),
        member(1),
    )
}

fn fact(
    g: &CapacityGroup,
    who: CapacityMemberId,
    state: CapacityMembership,
) -> CapacityMemberSnapshot {
    CapacityMemberSnapshot::new(g.id(), who, state)
}

fn join(g: &mut CapacityGroup, who: CapacityMemberId) {
    let snapshot = fact(g, who, CapacityMembership::NeverSeen);
    let updated = g.join(g.revision(), snapshot).expect("new membership");
    assert_eq!(updated.state(), CapacityMembership::Active);
}

fn vacate(g: &mut CapacityGroup, who: CapacityMemberId) {
    g.vacate(
        g.revision(),
        g.epoch(),
        fact(g, who, CapacityMembership::Active),
    )
    .expect("quiescent holder");
}

fn acquire(g: &mut CapacityGroup, who: CapacityMemberId) {
    g.acquire(
        g.revision(),
        g.epoch(),
        fact(g, who, CapacityMembership::Active),
    )
    .expect("eligible member acquires vacancy");
}

#[test]
fn final_holder_leaves_a_vacancy_for_surviving_detached_members() {
    let mut g = group();
    join(&mut g, member(2));
    join(&mut g, member(3));
    vacate(&mut g, member(1));
    acquire(&mut g, member(2));
    let finished = g
        .finish(
            g.revision(),
            fact(&g, member(2), CapacityMembership::Active),
        )
        .expect("intermediate B final");
    assert_eq!(finished.state(), CapacityMembership::Finished);
    assert_eq!(g.active_members(), 2);
    assert_eq!(g.possession(), CapacityPossession::Vacant);
    acquire(&mut g, member(1));
    g.finish(
        g.revision(),
        fact(&g, member(1), CapacityMembership::Active),
    )
    .expect("A final");
    acquire(&mut g, member(3));
    g.finish(
        g.revision(),
        fact(&g, member(3), CapacityMembership::Active),
    )
    .expect("C final");
    assert_eq!(g.active_members(), 0);
    assert_eq!(g.possession(), CapacityPossession::Retired);
    let before = g.clone();
    assert_eq!(
        g.finish(
            g.revision(),
            fact(&g, member(3), CapacityMembership::Finished)
        ),
        Err(CapacityGroupError::Retired)
    );
    assert_eq!(g, before);
}

#[test]
fn intermediate_nonholder_finality_preserves_the_childs_possession_epoch() {
    let mut g = group();
    join(&mut g, member(2));
    join(&mut g, member(3));
    vacate(&mut g, member(1));
    acquire(&mut g, member(3));
    let epoch = g.epoch();
    g.finish(
        g.revision(),
        fact(&g, member(2), CapacityMembership::Active),
    )
    .expect("B final");
    assert_eq!(g.epoch(), epoch);
    assert_eq!(g.possession(), CapacityPossession::Held(member(3)));
    assert_eq!(g.active_members(), 2);
}

#[test]
fn a_parked_child_can_yield_before_logical_completion() {
    let mut g = group();
    join(&mut g, member(2));
    vacate(&mut g, member(1));
    acquire(&mut g, member(2));
    vacate(&mut g, member(2));
    acquire(&mut g, member(1));
    assert_eq!(g.active_members(), 2, "yield is not logical completion");
    vacate(&mut g, member(1));
    acquire(&mut g, member(2));
    assert_eq!(g.active_members(), 2);
}

#[test]
fn finished_membership_facts_cannot_be_rejoined() {
    let mut g = group();
    join(&mut g, member(2));
    let finished = g
        .finish(
            g.revision(),
            fact(&g, member(1), CapacityMembership::Active),
        )
        .expect("A final");
    let before = g.clone();
    assert_eq!(
        g.join(g.revision(), finished),
        Err(CapacityGroupError::MemberAlreadyKnown)
    );
    assert_eq!(g, before);
}

#[test]
fn stale_and_foreign_commands_leave_the_header_unchanged() {
    let mut g = group();
    let active = fact(&g, member(1), CapacityMembership::Active);
    let before = g.clone();
    assert!(g.vacate(g.revision() - 1, g.epoch(), active).is_err());
    assert!(g.vacate(g.revision(), g.epoch() - 1, active).is_err());
    let foreign = CapacityMemberSnapshot::new(
        CapacityGroupId::new(Uuid::from_u128(999)),
        member(1),
        CapacityMembership::Active,
    );
    assert_eq!(
        g.finish(g.revision(), foreign),
        Err(CapacityGroupError::ForeignGroup)
    );
    assert_eq!(g, before);
}

fn restored(
    g: &CapacityGroup,
    holder: Option<CapacityMemberSnapshot>,
) -> Result<CapacityGroup, CapacityGroupError> {
    CapacityGroup::restore(
        g.id(),
        g.tenant_id(),
        g.revision(),
        g.epoch(),
        g.active_members(),
        g.possession(),
        holder,
    )
}

#[test]
fn restoration_preserves_transition_produced_headers() {
    let mut g = group();
    let active = |g: &CapacityGroup, who| Some(fact(g, who, CapacityMembership::Active));
    assert_eq!(restored(&g, active(&g, member(1))), Ok(g.clone()));
    join(&mut g, member(2));
    vacate(&mut g, member(1));
    assert_eq!(restored(&g, None), Ok(g.clone()));
    acquire(&mut g, member(2));
    assert_eq!(restored(&g, active(&g, member(2))), Ok(g.clone()));
    for who in [member(1), member(2)] {
        g.finish(g.revision(), fact(&g, who, CapacityMembership::Active))
            .expect("settled member");
    }
    assert_eq!(g.possession(), CapacityPossession::Retired);
    assert_eq!(restored(&g, None), Ok(g));
}

#[test]
fn restoration_rejects_inconsistent_scalars_and_holder_facts() {
    use CapacityPossession::{Held, Retired, Vacant};
    let g = group();
    let active = Some(fact(&g, member(1), CapacityMembership::Active));
    let foreign = Some(CapacityMemberSnapshot::new(
        CapacityGroupId::new(Uuid::from_u128(999)),
        member(1),
        CapacityMembership::Active,
    ));
    let cases = [
        (0, 0, 0, Retired, None),
        (0, 1, 1, Held(member(1)), active),
        (1, 0, 1, Held(member(1)), active),
        (2, 3, 1, Held(member(1)), active),
        (1, 1, 2, Held(member(1)), active),
        (2, 2, 1, Retired, None),
        (2, 2, 0, Vacant, None),
        (1, 1, 0, Held(member(1)), active),
        (1, 1, 1, Held(member(1)), None),
        (1, 1, 1, Held(member(1)), foreign),
        (
            1,
            1,
            1,
            Held(member(1)),
            Some(fact(&g, member(2), CapacityMembership::Active)),
        ),
        (
            1,
            1,
            1,
            Held(member(1)),
            Some(fact(&g, member(1), CapacityMembership::NeverSeen)),
        ),
        (
            1,
            1,
            1,
            Held(member(1)),
            Some(fact(&g, member(1), CapacityMembership::Finished)),
        ),
        (2, 2, 1, Vacant, active),
        (2, 2, 0, Retired, active),
    ];
    for (revision, epoch, count, possession, holder) in cases {
        assert_eq!(
            CapacityGroup::restore(
                g.id(),
                g.tenant_id(),
                revision,
                epoch,
                count,
                possession,
                holder
            ),
            Err(CapacityGroupError::InconsistentSnapshot),
        );
    }
}

#[test]
fn restoration_preserves_exhausted_counters_without_allocating_a_registry() {
    let initial = group();
    let active = fact(&initial, member(1), CapacityMembership::Active);
    let mut g = CapacityGroup::restore(
        initial.id(),
        initial.tenant_id(),
        u64::MAX,
        u64::MAX,
        1,
        initial.possession(),
        Some(active),
    )
    .expect("exhausted but consistent header");
    assert_eq!(
        (g.revision(), g.epoch(), g.active_members()),
        (u64::MAX, u64::MAX, 1)
    );
    let before = g.clone();
    assert_eq!(
        g.vacate(g.revision(), g.epoch(), active),
        Err(CapacityGroupError::CounterExhausted)
    );
    assert_eq!(g, before);
    let mut g = CapacityGroup::restore(
        initial.id(),
        initial.tenant_id(),
        u64::from(u32::MAX),
        1,
        u32::MAX,
        initial.possession(),
        Some(active),
    )
    .expect("bounded header for maximum retained count");
    assert_eq!(
        (g.revision(), g.epoch(), g.active_members()),
        (u64::from(u32::MAX), 1, u32::MAX)
    );
    let before = g.clone();
    assert_eq!(
        g.join(
            g.revision(),
            fact(&g, member(2), CapacityMembership::NeverSeen)
        ),
        Err(CapacityGroupError::CounterExhausted),
    );
    assert_eq!(g, before);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, rng_seed: RngSeed::Fixed(3714), ..ProptestConfig::default() })]
    #[test]
    fn bounded_commands_match_an_independent_registry(
        commands in prop::collection::vec((0u8..4, 1u8..4, any::<bool>(), any::<bool>(), any::<bool>()), 0..128)
    ) {
        let mut g = group();
        let mut registry = BTreeMap::from([(member(1), CapacityMembership::Active)]);
        let mut holder = Some(member(1));
        let (mut revision, mut epoch) = (1_u64, 1_u64);
        let mut retired = false;
        for (op, who, stale_revision, stale_epoch, foreign) in commands {
            let who = member(u128::from(who));
            let status = registry.get(&who).copied().unwrap_or(CapacityMembership::NeverSeen);
            let snapshot = CapacityMemberSnapshot::new(
                if foreign { CapacityGroupId::new(Uuid::from_u128(999)) } else { g.id() }, who, status,
            );
            let expected_revision = revision - u64::from(stale_revision);
            let expected_epoch = epoch - u64::from(stale_epoch);
            let allowed = !retired && !foreign && !stale_revision && match op {
                0 => status == CapacityMembership::NeverSeen,
                1 => !stale_epoch && holder == Some(who) && status == CapacityMembership::Active,
                2 => !stale_epoch && holder.is_none() && status == CapacityMembership::Active,
                _ => status == CapacityMembership::Active,
            };
            let before = g.clone();
            let result = match op {
                0 => g.join(expected_revision, snapshot).map(Some),
                1 => g.vacate(expected_revision, expected_epoch, snapshot).map(|()| None),
                2 => g.acquire(expected_revision, expected_epoch, snapshot).map(|()| None),
                _ => g.finish(expected_revision, snapshot).map(Some),
            };
            prop_assert_eq!(result.is_ok(), allowed);
            if !allowed {
                prop_assert_eq!(&g, &before);
                continue;
            }
            revision += 1;
            match op {
                0 => { registry.insert(who, CapacityMembership::Active); }
                1 => { holder = None; epoch += 1; }
                2 => { holder = Some(who); epoch += 1; }
                _ => {
                    registry.insert(who, CapacityMembership::Finished);
                    let owner_finished = holder == Some(who);
                    if owner_finished { holder = None; }
                    retired = !registry.values().any(|s| *s == CapacityMembership::Active);
                    if owner_finished || retired { epoch += 1; }
                }
            }
            if let Some(updated) = result.expect("allowed transition") {
                prop_assert_eq!(updated.group_id(), g.id());
                prop_assert_eq!(updated.member_id(), who);
                prop_assert_eq!(Some(&updated.state()), registry.get(&who));
            }
            let active = registry.values().filter(|s| **s == CapacityMembership::Active).count();
            prop_assert_eq!(g.active_members() as usize, active);
            prop_assert_eq!(g.revision(), revision);
            prop_assert_eq!(g.epoch(), epoch);
            prop_assert_eq!(g.possession(), if retired { CapacityPossession::Retired } else { holder.map_or(CapacityPossession::Vacant, CapacityPossession::Held) });
            if let Some(owner) = holder { prop_assert_eq!(registry.get(&owner), Some(&CapacityMembership::Active)); }
            let holder_fact = holder.map(|owner| fact(&g, owner, *registry.get(&owner).expect("registered holder")));
            let round_trip = restored(&g, holder_fact).expect("model header restores");
            prop_assert_eq!(&round_trip, &g);
            g = round_trip;
        }
    }
}
