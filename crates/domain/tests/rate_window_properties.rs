//! Properties of the pure window decision of a rate-limit key (feature 022, task T022-071; FR-022-040, FR-022-048;
//! decisions D022-003, D022-005, D022-009, D022-014, D022-032, D022-073, D022-078): whatever sequence of arrivals,
//! passes, lease recordings and clock moves (forwards and backwards) a key sees, with acquirers of mixed limits and
//! windows, no decision leaves more than its own `limit` grants counted in its own window; the grant instants of run
//! starts never put more than `limit` in any window ending at a grant; with one bound, leases of steps never put more
//! than `limit` lease instants in any window; grants follow admission order; and the estimate of a waiter's grant is the
//! first instant the grant routine admits it.
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_domain::execution::window::{
    Admission, Bound, Grant, Waiter, Window, estimate_us, period_us,
};
use sagat_domain::workflow::RatePeriod;

const SECOND: i64 = 1_000_000;
/// A deadline no sequence reaches.
const FAR: i64 = i64::MAX / 2;

/// One step of a sequence.
#[derive(Debug, Clone)]
enum Op {
    /// An arrival of the acquirer of this index (modulo the bounds), which may queue or not (a reset).
    Arrive { acquirer: usize, may_queue: bool },
    /// One pass of the grant routine.
    Pass,
    /// The database clock moves by this many microseconds (negative: set back).
    Advance(i64),
    /// The lease of the unleased grant of this index (modulo the unleased ones) is recorded now.
    Lease(usize),
}

fn bound() -> impl Strategy<Value = Bound> {
    (1u32..=4, prop::sample::select(RatePeriod::ALL.to_vec()))
        .prop_map(|(limit, per)| Bound { limit, per })
}

fn advance(backwards: bool) -> impl Strategy<Value = i64> {
    let forward = prop::sample::select(vec![
        0,
        1,
        SECOND / 2,
        SECOND - 1,
        SECOND,
        SECOND + 1,
        30 * SECOND,
        60 * SECOND,
        61 * SECOND,
        3_600 * SECOND,
        3_601 * SECOND,
    ]);
    if backwards {
        prop_oneof![
            4 => forward,
            1 => prop::sample::select(vec![-1, -SECOND, -60 * SECOND, -3_600 * SECOND]),
        ]
        .boxed()
    } else {
        forward.boxed()
    }
}

fn ops(backwards: bool, leases: bool) -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        5 => (0usize..8, prop::bool::weighted(0.85))
            .prop_map(|(acquirer, may_queue)| Op::Arrive { acquirer, may_queue }),
        3 => Just(Op::Pass),
        3 => advance(backwards).prop_map(Op::Advance),
        if leases { 2 } else { 0 } => (0usize..8).prop_map(Op::Lease),
    ];
    prop::collection::vec(op, 0..80)
}

/// A key and what was decided on it.
struct Sim {
    window: Window,
    now: i64,
    /// The arrival number of each waiter, in queue order (parallel to `window.waiters`).
    queue: Vec<u32>,
    /// For each grant, in decision order (parallel to `window.grants`): its acquirer's bound, its decision instant and
    /// its arrival number.
    decided: Vec<(Bound, i64, u32)>,
    arrivals: u32,
    /// Whether a grant is leased at its grant (a run's start) or later by a recorded lease (a step).
    run_scope: bool,
}

impl Sim {
    fn new(run_scope: bool) -> Self {
        Self {
            window: Window::default(),
            now: 1_000_000 * SECOND,
            queue: Vec::new(),
            decided: Vec::new(),
            arrivals: 0,
            run_scope,
        }
    }

    fn grant(&self, at: i64) -> Grant {
        if self.run_scope {
            Grant::leased(at)
        } else {
            Grant::UNLEASED
        }
    }

    /// The decision-time guarantee: after a grant decided at `at` by `bound`, its own window counts at most its limit.
    fn check_decision(&self, bound: Bound, at: i64) -> Result<(), TestCaseError> {
        let counted = self.window.counted(bound.per, at);
        prop_assert!(
            counted <= bound.limit,
            "{counted} grants counted for {bound:?} at {at} after a grant"
        );
        Ok(())
    }

    fn apply(&mut self, op: &Op, bounds: &[Bound]) -> Result<(), TestCaseError> {
        match *op {
            Op::Arrive {
                acquirer,
                may_queue,
            } => {
                let bound = bounds[acquirer % bounds.len()];
                let arrival = self.arrivals;
                self.arrivals += 1;
                match self.window.admit(bound, self.now, may_queue) {
                    Admission::Granted => {
                        self.window.grants.push(self.grant(self.now));
                        self.decided.push((bound, self.now, arrival));
                        self.check_decision(bound, self.now)?;
                    }
                    Admission::Queued => {
                        prop_assert!(may_queue, "a reset never queues");
                        self.window.waiters.push(Waiter {
                            bound,
                            deadline_us: FAR,
                            live: true,
                        });
                        self.queue.push(arrival);
                    }
                    Admission::Refused => prop_assert!(!may_queue, "only a reset is refused"),
                }
            }
            Op::Pass => {
                let before = self.window.grants.len();
                let queued = self.window.waiters.clone();
                let granted = self.window.pass(self.now, self.now, self.run_scope, 256);
                for (k, (original, at)) in granted.iter().enumerate() {
                    let bound = queued[*original].bound;
                    self.decided.push((bound, *at, self.queue[*original]));
                    // the window as this grant's own transaction left it: the grants before it and its own
                    let counted = self.window.grants[..=before + k]
                        .iter()
                        .filter(|grant| grant.counts(bound.per, *at))
                        .count();
                    prop_assert!(
                        counted <= bound.limit as usize,
                        "{counted} grants counted for {bound:?} at {at} after a grant of a pass"
                    );
                    self.now = self.now.max(at + 1);
                }
                let ids: Vec<u32> = granted
                    .iter()
                    .map(|(original, _)| self.queue[*original])
                    .collect();
                let mut positions: Vec<usize> =
                    granted.iter().map(|(original, _)| *original).collect();
                positions.sort_unstable();
                for original in positions.iter().rev() {
                    self.queue.remove(*original);
                }
                prop_assert_eq!(self.queue.len(), self.window.waiters.len());
                prop_assert!(
                    ids.windows(2).all(|w| w[0] < w[1]),
                    "a pass grants in queue order"
                );
            }
            Op::Advance(by) => self.now = self.now.saturating_add(by),
            Op::Lease(which) => {
                let unleased: Vec<usize> = (0..self.window.grants.len())
                    .filter(|i| self.window.grants[*i].leased_at_us.is_none())
                    .collect();
                if !unleased.is_empty() {
                    let index = unleased[which % unleased.len()];
                    // a lease is recorded at or after its grant: the pass copies `greatest(leased, granted_at)`
                    let at = self.now.max(self.decided[index].1);
                    self.window.grants[index] = Grant::leased(at);
                }
            }
        }
        Ok(())
    }
}

/// Invariant 10 (decision D022-032) of run starts, over decision order: for each grant `e` with instant `t_e`, the grants
/// decided up to it whose instants lie in `(t_e - per_e, t_e]` number at most `limit_e`.
fn invariant_10_in_decision_order(sim: &Sim) -> Result<(), TestCaseError> {
    for (e, (bound, t_e, _)) in sim.decided.iter().enumerate() {
        let from = t_e - period_us(bound.per);
        let within = sim.decided[..=e]
            .iter()
            .filter(|(_, t, _)| *t > from && t <= t_e)
            .count();
        prop_assert!(
            within <= bound.limit as usize,
            "{within} grants in the window of {bound:?} ending at grant {e}"
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    /// Run starts, mixed acquirers, the clock moving both ways: no decision leaves more than its own limit counted in
    /// its own window, and invariant 10 holds over the decisions made up to each grant.
    #[test]
    fn no_sequence_of_run_decisions_counts_more_than_the_deciders_limit_in_its_window(
        bounds in prop::collection::vec(bound(), 1..4),
        script in ops(true, false),
    ) {
        let mut sim = Sim::new(true);
        for op in &script {
            sim.apply(op, &bounds)?;
        }
        invariant_10_in_decision_order(&sim)?;
    }

    /// Steps, mixed acquirers, leases recorded late, the clock moving both ways: no decision leaves more than its own
    /// limit counted (an unleased grant counts) in its own window.
    #[test]
    fn no_sequence_of_step_decisions_counts_more_than_the_deciders_limit_in_its_window(
        bounds in prop::collection::vec(bound(), 1..4),
        script in ops(true, true),
    ) {
        let mut sim = Sim::new(false);
        for op in &script {
            sim.apply(op, &bounds)?;
        }
    }

    /// One bound, a clock that only moves forward: the lease instants of steps never put more than `limit` in any window
    /// `(t - per, t]` (decision D022-005: at the grant of the last of them every other was unleased or leased within
    /// the window, so it counted), and run starts and steps are granted in admission order.
    #[test]
    fn with_one_bound_leases_never_exceed_the_limit_in_any_window_and_grants_follow_admission(
        bound in bound(),
        script in ops(false, true),
        run_scope in any::<bool>(),
    ) {
        let bounds = [bound];
        let mut sim = Sim::new(run_scope);
        let script: Vec<Op> = script
            .into_iter()
            .map(|op| match op {
                Op::Arrive { acquirer, .. } => Op::Arrive { acquirer, may_queue: true },
                other => other,
            })
            .collect();
        for op in &script {
            sim.apply(op, &bounds)?;
        }
        let leases: Vec<i64> = sim.window.grants.iter().filter_map(|g| g.leased_at_us).collect();
        for t in &leases {
            let within = leases
                .iter()
                .filter(|l| **l > t - period_us(bound.per) && *l <= t)
                .count();
            prop_assert!(within <= bound.limit as usize, "{within} leases in the window ending at {t}");
        }
        let order: Vec<u32> = sim.decided.iter().map(|(_, _, arrival)| *arrival).collect();
        prop_assert!(order.windows(2).all(|w| w[0] < w[1]), "grants out of admission order: {order:?}");
    }

    /// The estimate of a waiter's grant (decision D022-014) is the first instant the grant routine admits it, the waiters
    /// ahead of it unchanged: admitted at the estimate, not one microsecond before it (when it is not now).
    #[test]
    fn the_estimate_is_the_first_instant_the_grant_routine_admits_the_waiter(
        bound in bound(),
        ages in prop::collection::vec(0i64..(2 * 3_600 * SECOND), 0..8),
        waiting in 1usize..6,
    ) {
        let now = 1_000_000 * SECOND;
        let window = Window {
            grants: ages.iter().map(|age| Grant::leased(now - age)).collect(),
            waiters: vec![Waiter { bound, deadline_us: FAR, live: true }; waiting],
        };
        let me = waiting - 1;
        let place = window.place(me, now).expect("a waiter");
        prop_assert_eq!(place.position as usize, waiting);
        match place.estimate_us {
            Some(at) => {
                prop_assert!(at >= now);
                prop_assert!(window.admits_queued(me, at, at), "not admitted at its estimate {at}");
                if at > now {
                    prop_assert!(!window.admits_queued(me, at - 1, at - 1), "admitted before its estimate {at}");
                }
            }
            // beyond one window: not admitted for one window from now
            None => prop_assert!(!window.admits_queued(me, now + period_us(bound.per) - 1, now + period_us(bound.per) - 1)),
        }
    }
}

/// A database clock set back by a window after a key filled (NFR-022-008): a grant counted ahead of the clock keeps
/// counting, so the key grants nothing until the clock passes it, and then grants again.
#[test]
fn a_clock_set_back_grants_nothing_until_it_passes_the_counted_instants() {
    let bound = Bound {
        limit: 2,
        per: RatePeriod::Second,
    };
    let t0 = 1_000_000 * SECOND;
    let mut window = Window::default();
    for _ in 0..2 {
        assert_eq!(window.admit(bound, t0, true), Admission::Granted);
        window.grants.push(Grant::leased(t0));
    }
    // set back one hour: the two grants are an hour ahead and count
    let back = t0 - 3_600 * SECOND;
    assert_eq!(window.counted(bound.per, back), 2);
    assert_eq!(window.admit(bound, back, true), Admission::Queued);
    assert_eq!(window.admit(bound, back, false), Admission::Refused);
    // up to just before `t0 + 1 s` they still count; at it they stop (the half-open window `(t - 1 s, t]`)
    assert_eq!(
        window.admit(bound, t0 + SECOND - 1, true),
        Admission::Queued
    );
    assert_eq!(window.admit(bound, t0 + SECOND, true), Admission::Granted);
}

/// Strict admission order (decision D022-073): an arrival with room queues behind an eligible waiter; a waiter refused
/// by its own lower limit is not eligible and does not hold back an arrival of a higher limit (D1196).
#[test]
fn an_arrival_queues_behind_an_eligible_waiter_and_not_behind_one_its_own_limit_refuses() {
    let t = 1_000_000 * SECOND;
    let low = Bound {
        limit: 1,
        per: RatePeriod::Minute,
    };
    let high = Bound {
        limit: 3,
        per: RatePeriod::Minute,
    };
    let mut window = Window {
        grants: vec![Grant::leased(t - SECOND)],
        waiters: vec![Waiter {
            bound: low,
            deadline_us: FAR,
            live: true,
        }],
    };
    assert_eq!(window.admit(high, t, true), Admission::Granted);
    // the same waiter with room for it (no grant) is eligible: the arrival queues behind it
    window.grants.clear();
    assert_eq!(window.admit(high, t, true), Admission::Queued);
    // a paused (not live) or past-deadline waiter is not eligible
    window.waiters[0].live = false;
    assert_eq!(window.admit(high, t, true), Admission::Granted);
    window.waiters[0].live = true;
    window.waiters[0].deadline_us = t;
    assert_eq!(window.admit(high, t, true), Admission::Granted);
}

/// The estimate's cases (decision D022-014): now with room, the k-th lease plus the window, none beyond one window, none
/// for a skipped run or at the deadline.
#[test]
fn the_estimate_follows_the_kth_counted_lease() {
    use sagat_domain::execution::window::Reading;
    let reading = Reading {
        at_us: 100,
        counted: 2,
        ahead: 1,
        limit: 2,
        kth_lease_us: Some(40),
        per: RatePeriod::Second,
        deadline_us: FAR,
        skipped: false,
    };
    assert_eq!(estimate_us(reading), Some(40 + SECOND));
    assert_eq!(
        estimate_us(Reading {
            ahead: 0,
            counted: 1,
            ..reading
        }),
        Some(100)
    );
    assert_eq!(
        estimate_us(Reading {
            ahead: 3,
            ..reading
        }),
        None
    );
    assert_eq!(
        estimate_us(Reading {
            skipped: true,
            ..reading
        }),
        None
    );
    assert_eq!(
        estimate_us(Reading {
            deadline_us: 40 + SECOND,
            ..reading
        }),
        None
    );
}
