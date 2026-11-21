//! Properties of flow control in the domain (feature 022, T022-004, T022-008, T022-068): a declaration is accepted exactly
//! within its bounds, and whatever sequence of flow-control events a run is sent, the ones the reducer accepts replay to
//! the same state, a run never finishes while it waits for or holds a gate, and a step is never leased on a gate of its
//! attempt that is not held.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{T0, activity, build, id, sid};
use proptest::prelude::*;
use sagat_domain::execution::{
    self, GateChange, GateEvent, GatePool, GateRef, GateTarget, KeyLabel, RunEvent, RunId,
    RunState, StepStatus, StopReason,
};
use sagat_domain::workflow::flow_control::{MAX_CONCURRENCY_LIMIT, MAX_QUEUE_TIMEOUT};
use sagat_domain::workflow::{
    ConcurrencyPolicy, FlowControl, FlowGroup, RateLimitPolicy, RatePeriod, ResolvedGroup,
};

proptest! {
    #[test]
    fn a_rate_limit_is_accepted_exactly_within_the_ceiling_of_its_window(
        limit in 0u32..=12_000,
        per in prop::sample::select(RatePeriod::ALL.to_vec()),
        timeout_ms in 0u64..=90_000_000,
    ) {
        let accepted = RateLimitPolicy::new(
            FlowGroup::Derived, None, limit, per, Duration::from_millis(timeout_ms),
        ).is_ok();
        let within = (1..=per.max_limit()).contains(&limit)
            && timeout_ms >= 1
            && Duration::from_millis(timeout_ms) <= MAX_QUEUE_TIMEOUT;
        prop_assert_eq!(accepted, within);
    }

    #[test]
    fn a_concurrency_limit_is_accepted_exactly_within_its_ceiling(limit in 0u32..=20_000) {
        let accepted = ConcurrencyPolicy::new(
            FlowGroup::Derived, None, limit, Duration::from_secs(1),
        ).is_ok();
        prop_assert_eq!(accepted, (1..=MAX_CONCURRENCY_LIMIT).contains(&limit));
    }

    #[test]
    fn whatever_the_events_a_run_never_finishes_waiting_or_holding_and_its_journal_replays(
        script in prop::collection::vec((0usize..12, any::<bool>(), 1u32..3), 0..24),
    ) {
        let def = build("a", vec![activity("a")]).with_run_flow_control(FlowControl::new(
            Some(ConcurrencyPolicy::new(FlowGroup::Derived, None, 1, Duration::from_secs(60)).expect("policy")),
            Some(RateLimitPolicy::new(FlowGroup::Derived, None, 5, RatePeriod::Second, Duration::from_secs(60)).expect("policy")),
        ));
        let gate = GateRef {
            pool: GatePool::Ordinary,
            group: ResolvedGroup::parse("=orders").expect("group"),
            key_label: KeyLabel::parse("abcdefabcdef").expect("label"),
        };
        let changes = [
            GateChange::ConcurrencyQueued { queue_deadline_us: 1 },
            GateChange::ConcurrencyAcquired,
            GateChange::ConcurrencyReleased,
            GateChange::ConcurrencyWaitTimedOut,
            GateChange::ConcurrencyDequeued,
            GateChange::RateQueued { limit: 5, per: RatePeriod::Second, queue_deadline_us: 1 },
            GateChange::RateGranted { limit: 5, per: RatePeriod::Second, granted_at_us: 1, from_queue: false },
            GateChange::RateWaitTimedOut,
            GateChange::RateDequeued,
            GateChange::RateReturned,
        ];
        let (mut state, admitted) = execution::start_run_gated(
            &def,
            RunId::generate(),
            T0,
            vec![
                GateEvent { target: GateTarget::Run, gate: gate.clone(), change: changes[0] },
                GateEvent { target: GateTarget::Run, gate: gate.clone(), change: changes[5] },
            ],
        ).expect("admitted, waiting for both");
        let mut journal = admitted.events;
        for (pick, run_scope, attempt) in script {
            let event = match pick {
                10 => {
                    // a worker leases the step when it can; the attempt then completes
                    if let Ok((fence, leased)) = execution::lease_step(&mut state, &sid("a")) {
                        journal.push(leased);
                        match execution::complete_step(&def, &mut state, &sid("a"), fence, T0) {
                            Ok(progress) => journal.extend(progress.events),
                            Err(_) => return Ok(()),
                        }
                    }
                    continue;
                }
                11 => {
                    if let Ok(progress) = execution::stop_run(&def, &mut state, StopReason::Cancelled, None, T0) {
                        journal.extend(progress.events);
                    }
                    continue;
                }
                n => RunEvent::Gate(GateEvent {
                    target: if run_scope { GateTarget::Run } else { GateTarget::Step { node: id("a"), attempt } },
                    gate: gate.clone(),
                    change: changes[n],
                }),
            };
            let mut trial = state.clone();
            if trial.apply(&event).is_ok() {
                state = trial;
                journal.push(event);
            }
            if state.is_finished() {
                prop_assert!(!state.run_gates().is_waiting());
                prop_assert!(state.run_gates().held_permit().is_none());
            }
            if let Some(step) = state.step(&sid("a")) {
                if step.status().is_leased() {
                    prop_assert!(step.gates().admits_lease(step.attempt()));
                }
                prop_assert!(step.status() != StepStatus::Ready || !step.gates().is_waiting());
            }
        }
        prop_assert_eq!(RunState::replay(state.run_id(), &journal).expect("replays"), state);
    }
}
