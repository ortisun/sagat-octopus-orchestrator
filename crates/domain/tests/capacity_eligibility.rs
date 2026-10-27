//! B1c pure eligibility only: counters and a trusted effective limit, no storage or authority.
#![allow(clippy::expect_used)]

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sagat_domain::execution::{
    CapacityEligibilityError as Refused, CapacityEnablement as Mode, CapacityWork as Work,
    TenantCapacity,
};

fn counters(t: &TenantCapacity) -> (u64, u64, u64) {
    (t.retained(), t.enabled(), t.draining())
}

fn tenant(retained: u64, enabled: u64, draining: u64) -> TenantCapacity {
    TenantCapacity::restore(retained, enabled, draining).expect("consistent counters")
}

#[test]
fn a_fresh_group_needs_room_under_the_retained_bound() {
    let mut t = TenantCapacity::new();
    assert_eq!(t.allocate_group(2), Ok(Mode::Forward));
    assert_eq!(t.allocate_group(2), Ok(Mode::Forward));
    assert_eq!(counters(&t), (2, 2, 0));
    assert_eq!(t.allocate_group(2), Err(Refused::NoRoom));
    assert_eq!(counters(&t), (2, 2, 0), "a refusal changes nothing");
}

#[test]
fn a_parked_group_still_counts_against_fresh_allocation() {
    let mut t = tenant(2, 1, 0);
    assert_eq!(t.allocate_group(2), Err(Refused::NoRoom));
    assert_eq!(t.enable(Mode::Parked, 2), Ok(Mode::Forward));
    assert_eq!(counters(&t), (2, 2, 0));
}

#[test]
fn limit_zero_admits_no_group_and_enables_none() {
    let mut t = TenantCapacity::new();
    assert_eq!(t.allocate_group(0), Err(Refused::NoRoom));
    let mut t = tenant(1, 0, 0);
    assert_eq!(t.enable(Mode::Parked, 0), Err(Refused::NoRoom));
    assert_eq!(counters(&t), (1, 0, 0));
}

#[test]
fn a_reduction_revokes_nobody_but_stops_forward_work_above_the_bound() {
    let t = tenant(2, 2, 0);
    // Limit 2 → 1: both holders survive, neither may take more forward work until one parks.
    assert_eq!(
        t.claim(1, Mode::Forward, Work::Forward),
        Err(Refused::OverBound)
    );
    assert_eq!(t.claim(1, Mode::Forward, Work::ReservedDrain), Ok(()));
    assert_eq!(t.claim(2, Mode::Forward, Work::Forward), Ok(()));

    let mut t = t;
    assert_eq!(t.park(Mode::Forward), Ok(Mode::Parked));
    assert_eq!(counters(&t), (2, 1, 0));
    assert_eq!(
        t.claim(1, Mode::Forward, Work::Forward),
        Ok(()),
        "the survivor is back within the bound"
    );
    assert_eq!(
        t.enable(Mode::Parked, 1),
        Err(Refused::NoRoom),
        "the parked group cannot re-enable above the bound"
    );
}

#[test]
fn policy_zero_stops_forward_work_but_reserved_drain_still_progresses() {
    let t = tenant(1, 1, 0);
    assert_eq!(
        t.claim(0, Mode::Forward, Work::Forward),
        Err(Refused::OverBound)
    );
    assert_eq!(t.claim(0, Mode::Forward, Work::ReservedDrain), Ok(()));

    // A parked group with accepted obligations takes the single drain lane at limit zero.
    let mut t = tenant(2, 0, 0);
    assert_eq!(t.enable_for_drain(Mode::Parked, 0), Ok(Mode::Drain));
    assert_eq!(counters(&t), (2, 0, 1));
    assert_eq!(
        t.enable_for_drain(Mode::Parked, 0),
        Err(Refused::DrainLaneBusy)
    );
    assert_eq!(counters(&t), (2, 0, 1));
    assert_eq!(
        t.claim(0, Mode::Drain, Work::Forward),
        Err(Refused::DrainOnly)
    );
    assert_eq!(t.claim(0, Mode::Drain, Work::ReservedDrain), Ok(()));

    // The lane is released by parking or retiring, and the other group takes it next.
    assert_eq!(t.park(Mode::Drain), Ok(Mode::Parked));
    assert_eq!(counters(&t), (2, 0, 0));
    assert_eq!(t.enable_for_drain(Mode::Parked, 0), Ok(Mode::Drain));
    assert_eq!(t.retire(Mode::Drain), Ok(()));
    assert_eq!(counters(&t), (1, 0, 0));
}

#[test]
fn ordinary_room_is_used_before_the_drain_lane() {
    let mut t = tenant(2, 1, 0);
    assert_eq!(t.enable_for_drain(Mode::Parked, 2), Ok(Mode::Forward));
    assert_eq!(counters(&t), (2, 2, 0));
}

#[test]
fn a_parked_group_cannot_claim() {
    let t = tenant(1, 0, 0);
    for work in [Work::Forward, Work::ReservedDrain] {
        assert_eq!(t.claim(5, Mode::Parked, work), Err(Refused::NotEnabled));
    }
}

#[test]
fn transitions_name_the_enablement_they_apply_to() {
    let mut t = tenant(2, 1, 0);
    assert_eq!(t.park(Mode::Parked), Err(Refused::WrongEnablement));
    assert_eq!(t.enable(Mode::Forward, 9), Err(Refused::WrongEnablement));
    assert_eq!(
        t.enable_for_drain(Mode::Drain, 9),
        Err(Refused::WrongEnablement)
    );
    assert_eq!(counters(&t), (2, 1, 0));
}

#[test]
fn retiring_a_parked_group_releases_only_its_retained_unit() {
    let mut t = tenant(3, 1, 0);
    assert_eq!(t.retire(Mode::Parked), Ok(()));
    assert_eq!(counters(&t), (2, 1, 0));
    assert_eq!(t.retire(Mode::Forward), Ok(()));
    assert_eq!(counters(&t), (1, 0, 0));
}

#[test]
fn counters_that_contradict_the_enablement_are_refused_without_change() {
    // No parked group exists, so a "parked" group cannot be enabled or retired.
    let mut t = tenant(1, 1, 0);
    assert_eq!(
        t.enable(Mode::Parked, 9),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(
        t.enable_for_drain(Mode::Parked, 9),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(t.retire(Mode::Parked), Err(Refused::InconsistentCounters));
    // Nobody is enabled or draining, so they cannot be parked or retired.
    let mut t = tenant(1, 0, 0);
    assert_eq!(t.park(Mode::Forward), Err(Refused::InconsistentCounters));
    assert_eq!(t.park(Mode::Drain), Err(Refused::InconsistentCounters));
    assert_eq!(t.retire(Mode::Forward), Err(Refused::InconsistentCounters));
    assert_eq!(t.retire(Mode::Drain), Err(Refused::InconsistentCounters));
    assert_eq!(counters(&t), (1, 0, 0));
    // Nothing is retained at all.
    let mut t = TenantCapacity::new();
    assert_eq!(t.retire(Mode::Parked), Err(Refused::InconsistentCounters));
}

#[test]
fn restoring_validates_the_projection() {
    assert!(TenantCapacity::restore(0, 0, 0).is_ok());
    assert!(TenantCapacity::restore(3, 2, 1).is_ok());
    assert_eq!(
        TenantCapacity::restore(3, 2, 2),
        Err(Refused::InconsistentCounters),
        "one drain lane"
    );
    assert_eq!(
        TenantCapacity::restore(2, 2, 1),
        Err(Refused::InconsistentCounters),
        "enabled plus draining exceed retained"
    );
    assert_eq!(
        TenantCapacity::restore(u64::MAX, u64::MAX, 1),
        Err(Refused::InconsistentCounters),
        "the sum cannot wrap into a valid projection"
    );
    assert!(TenantCapacity::restore(u64::MAX, u64::MAX - 1, 1).is_ok());
}

#[test]
fn the_largest_representable_state_refuses_instead_of_wrapping() {
    let mut t = tenant(u64::MAX, 0, 0);
    assert_eq!(t.allocate_group(u64::MAX), Err(Refused::NoRoom));
    assert_eq!(counters(&t), (u64::MAX, 0, 0));
    assert_eq!(t.enable(Mode::Parked, u64::MAX), Ok(Mode::Forward));
    assert_eq!(counters(&t), (u64::MAX, 1, 0));
}

#[test]
fn claims_name_counters_that_hold_no_such_group() {
    let t = tenant(1, 0, 0);
    for work in [Work::Forward, Work::ReservedDrain] {
        assert_eq!(
            t.claim(5, Mode::Forward, work),
            Err(Refused::InconsistentCounters)
        );
        assert_eq!(
            t.claim(5, Mode::Drain, work),
            Err(Refused::InconsistentCounters)
        );
    }
    let t = tenant(1, 1, 0);
    assert_eq!(
        t.claim(5, Mode::Parked, Work::Forward),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(
        t.claim(5, Mode::Drain, Work::ReservedDrain),
        Err(Refused::InconsistentCounters)
    );
}

#[test]
fn operations_from_the_extreme_projection_never_wrap() {
    let mut t = tenant(u64::MAX, u64::MAX - 1, 1);
    let extreme = counters(&t);
    // Nothing is parked, so nothing parked can enable, drain, retire or claim.
    assert_eq!(
        t.enable(Mode::Parked, u64::MAX),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(
        t.enable_for_drain(Mode::Parked, u64::MAX),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(t.retire(Mode::Parked), Err(Refused::InconsistentCounters));
    assert_eq!(
        t.claim(u64::MAX, Mode::Parked, Work::Forward),
        Err(Refused::InconsistentCounters)
    );
    assert_eq!(t.allocate_group(u64::MAX), Err(Refused::NoRoom));
    assert_eq!(counters(&t), extreme);
    // Leaving the lane makes one parked group, which only the lane (limit == enabled) can serve.
    assert_eq!(t.park(Mode::Drain), Ok(Mode::Parked));
    assert_eq!(
        t.enable_for_drain(Mode::Parked, u64::MAX - 1),
        Ok(Mode::Drain)
    );
    assert_eq!(t.retire(Mode::Drain), Ok(()));
    assert_eq!(counters(&t), (u64::MAX - 1, u64::MAX - 1, 0));
}

#[test]
fn the_drain_lane_adds_exactly_one_executing_group_above_the_limit() {
    let mut t = tenant(4, 2, 0);
    assert_eq!(t.enable_for_drain(Mode::Parked, 2), Ok(Mode::Drain));
    assert_eq!(
        t.enable_for_drain(Mode::Parked, 2),
        Err(Refused::DrainLaneBusy)
    );
    assert_eq!(counters(&t), (4, 2, 1), "limit 2 executes 3 groups at most");
}

#[test]
fn a_lane_holder_is_not_promoted_it_parks_and_enables_again() {
    let mut t = tenant(2, 0, 1);
    assert_eq!(t.enable(Mode::Drain, 9), Err(Refused::WrongEnablement));
    assert_eq!(t.park(Mode::Drain), Ok(Mode::Parked));
    assert_eq!(t.enable(Mode::Parked, 9), Ok(Mode::Forward));
    assert_eq!(counters(&t), (2, 1, 0));
}

/// Independent model: the multiset of group enablements. The unit under test keeps anonymous
/// counters, so a transition naming an enablement moves the first group that has it.
#[derive(Default, Clone)]
struct Oracle {
    groups: Vec<Mode>,
}

impl Oracle {
    fn count(&self, mode: Mode) -> u64 {
        self.groups.iter().filter(|g| **g == mode).count() as u64
    }
    fn counts(&self) -> (u64, u64, u64) {
        (
            self.groups.len() as u64,
            self.count(Mode::Forward),
            self.count(Mode::Drain),
        )
    }
    fn first(&self, mode: Mode) -> Option<usize> {
        self.groups.iter().position(|g| *g == mode)
    }
}

fn mode_of(n: u8) -> Mode {
    match n % 3 {
        0 => Mode::Parked,
        1 => Mode::Forward,
        _ => Mode::Drain,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, rng_seed: RngSeed::Fixed(3715), ..ProptestConfig::default() })]
    /// Checks the bookkeeping (counters against a recomputed multiset, no change on refusal,
    /// invariants) from arbitrary restored starting states, with arbitrary (also false) stated
    /// enablements. The policy itself is pinned by the scenario tests above, because an oracle
    /// cannot independently re-derive a rule it was written from.
    #[test]
    fn counters_follow_an_independent_per_group_model(
        initial in prop::collection::vec(0u8..3, 0..8),
        ops in prop::collection::vec((0u8..8, any::<u8>(), 0u64..5), 0..160)
    ) {
        let mut m = Oracle::default();
        for n in initial {
            let mode = mode_of(n);
            m.groups.push(if mode == Mode::Drain && m.count(Mode::Drain) == 1 { Mode::Forward } else { mode });
        }
        let (r0, e0, d0) = m.counts();
        let mut t = TenantCapacity::restore(r0, e0, d0).expect("a model state is consistent");
        let mut limit = 2_u64;
        for (op, pick, new_limit) in ops {
            let (retained, enabled, draining) = m.counts();
            let before = counters(&t);
            let from = mode_of(pick);
            let present = m.count(from) > 0;
            let result: Result<(), Refused> = match op {
                0 => {
                    let result = t.allocate_group(limit);
                    if retained < limit {
                        prop_assert_eq!(result, Ok(Mode::Forward));
                        m.groups.push(Mode::Forward);
                    } else {
                        prop_assert_eq!(result, Err(Refused::NoRoom));
                    }
                    result.map(|_| ())
                }
                1 | 2 => {
                    let result = if op == 1 { t.enable(from, limit) } else { t.enable_for_drain(from, limit) };
                    let expected = if from != Mode::Parked {
                        Err(Refused::WrongEnablement)
                    } else if !present {
                        Err(Refused::InconsistentCounters)
                    } else if enabled < limit {
                        Ok(Mode::Forward)
                    } else if op == 2 && draining == 0 {
                        Ok(Mode::Drain)
                    } else if op == 2 {
                        Err(Refused::DrainLaneBusy)
                    } else {
                        Err(Refused::NoRoom)
                    };
                    prop_assert_eq!(result, expected);
                    if let Ok(to) = expected {
                        let i = m.first(Mode::Parked).expect("presence checked");
                        m.groups[i] = to;
                    }
                    result.map(|_| ())
                }
                3 => {
                    let result = t.park(from);
                    let expected = if from == Mode::Parked {
                        Err(Refused::WrongEnablement)
                    } else if !present {
                        Err(Refused::InconsistentCounters)
                    } else {
                        Ok(Mode::Parked)
                    };
                    prop_assert_eq!(result, expected);
                    if expected.is_ok() {
                        let i = m.first(from).expect("presence checked");
                        m.groups[i] = Mode::Parked;
                    }
                    result.map(|_| ())
                }
                4 => {
                    let result = t.retire(from);
                    if present {
                        prop_assert_eq!(result, Ok(()));
                        let i = m.first(from).expect("presence checked");
                        m.groups.remove(i);
                    } else {
                        prop_assert_eq!(result, Err(Refused::InconsistentCounters));
                    }
                    result
                }
                5 | 6 => {
                    let work = if op == 5 { Work::Forward } else { Work::ReservedDrain };
                    let expected = match (from, work) {
                        _ if !present => Err(Refused::InconsistentCounters),
                        (Mode::Parked, _) => Err(Refused::NotEnabled),
                        (Mode::Drain, Work::Forward) => Err(Refused::DrainOnly),
                        (Mode::Forward, Work::Forward) if enabled > limit => Err(Refused::OverBound),
                        _ => Ok(()),
                    };
                    let result = t.claim(limit, from, work);
                    prop_assert_eq!(result, expected);
                    result
                }
                _ => {
                    limit = new_limit;
                    Ok(())
                }
            };
            let after = counters(&t);
            prop_assert_eq!(after, m.counts());
            prop_assert!(after.2 <= 1, "one drain lane");
            prop_assert!(after.1 + after.2 <= after.0, "enabled plus draining within retained");
            if result.is_err() || op >= 5 {
                prop_assert_eq!(after, before, "a refusal, a claim or a limit change never moves a counter");
            }
        }

        // Leaving and reserved-drain claims are never blocked by the limit, and a free lane
        // always serves a parked group, whatever the limit (including zero).
        for probe_limit in 0..5_u64 {
            for group in [Mode::Forward, Mode::Drain] {
                if m.count(group) > 0 {
                    prop_assert_eq!(t.claim(probe_limit, group, Work::ReservedDrain), Ok(()));
                    let mut c = t;
                    prop_assert_eq!(c.park(group), Ok(Mode::Parked));
                    let mut c = t;
                    prop_assert_eq!(c.retire(group), Ok(()));
                }
            }
            if m.count(Mode::Parked) > 0 && t.draining() == 0 {
                let mut c = t;
                prop_assert!(c.enable_for_drain(Mode::Parked, probe_limit).is_ok());
            }
        }
    }
}
