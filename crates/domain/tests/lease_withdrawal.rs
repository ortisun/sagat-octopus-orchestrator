//! A lease that never reached a handler is withdrawn (feature 006, decisions D006-001, D006-003, D006-007 and D006-011 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098): the step goes back to `READY`, the attempt is not consumed, the
//! generation keeps rising, and no review is opened.
#![allow(clippy::expect_used)]

mod common;

use common::{Harness, activity, build, effect, flaky, id, ms};
use sagat_domain::execution::{
    self, AddressedRunEvent, ExecutionError, ExecutionInstanceId, Fence, InstancePath,
    PathExecutionState, ResetRule, RunEvent, RunId, RunState, RunStatus, StepStatus, StopReason,
    WithdrawalCause, lease_instance, rule_of as reset_rule,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Expression, Node, NodeId, NodeKind, RetryPolicy,
    WorkflowDefinition,
};
use time::OffsetDateTime;

const fn fence(attempt: u32, generation: u64) -> Fence {
    Fence {
        attempt,
        generation,
    }
}

#[test]
fn a_withdrawn_lease_gives_the_step_back_and_the_next_lease_reuses_its_attempt() {
    for cause in [WithdrawalCause::NotSent, WithdrawalCause::NotStarted] {
        // a declared activity with an effect: a lapse of it would be a review (feature 037)
        let mut run = Harness::start(build("a", vec![effect("a", false)]));
        let first = run.lease("a");
        assert_eq!(first, fence(1, 1));
        run.withdraw("a", first, cause).expect("withdraw");

        let step = run.state.step(&id("a")).expect("step");
        assert_eq!(step.status(), StepStatus::Ready);
        assert_eq!(
            step.attempt(),
            0,
            "the attempt that never began is given back"
        );
        assert_eq!(step.generation(), 1, "the generation is kept");
        assert!(!run.names().contains(&"step.needs_review"));
        assert!(!run.names().contains(&"reconciliation.requested"));

        let second = run.lease("a");
        assert_eq!(second, fence(1, 2), "same attempt number, next generation");
        run.heartbeat("a", second).expect("started");
        run.complete("a", second).expect("complete");
        assert_eq!(run.state.status(), RunStatus::Succeeded);
        assert_eq!(
            run.names(),
            [
                "run.started",
                "step.released",
                "step.leased",
                "step.lease_withdrawn",
                "step.leased",
                "step.started",
                "step.completed",
                "step.routed",
                "run.finished",
            ]
        );
        let withdrawn = run
            .journal
            .iter()
            .find(|event| event.name() == "step.lease_withdrawn")
            .expect("withdrawal journaled");
        assert_eq!(
            withdrawn,
            &RunEvent::StepLeaseWithdrawn {
                node: id("a"),
                attempt: 1,
                generation: 1,
                cause,
            }
        );
        assert_eq!(withdrawn.attempt(), Some(1));
        assert_eq!(withdrawn.node(), Some(&id("a")));
        let shown = withdrawn.cause().expect("a cause for the timeline");
        assert_eq!(shown.kind, "withdrawal");
        assert_eq!(shown.code, cause.as_str());
        run.assert_replays();
    }
}

/// The retry budget counts the attempts that began: with three attempts allowed and one lease withdrawn, the handler still
/// runs three times before the step fails for good.
#[test]
fn a_withdrawal_does_not_spend_the_retry_budget() {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    let mut handler_runs = 0u32;
    let mut leases = Vec::new();
    loop {
        let lease = run.lease("a");
        leases.push(lease);
        if leases.len() == 2 {
            // the second lease never reaches a handler
            run.withdraw("a", lease, WithdrawalCause::NotSent)
                .expect("withdraw");
            continue;
        }
        run.heartbeat("a", lease).expect("started");
        handler_runs += 1;
        run.fail("a", lease, true).expect("fail");
        if run.status("a") == StepStatus::Failed {
            break;
        }
        run.advance(ms(100));
        run.retry_due("a").expect("due");
    }
    assert_eq!(handler_runs, 3, "the budget is three attempts that began");
    assert_eq!(
        leases,
        [fence(1, 1), fence(2, 2), fence(2, 3), fence(3, 4)],
        "the withdrawn attempt number is reused and every generation is new"
    );
    assert_eq!(run.state.status(), RunStatus::Failed);
    run.assert_replays();
}

#[test]
fn a_started_step_is_not_withdrawn_and_nothing_changes() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let lease = run.lease("a");
    run.heartbeat("a", lease).expect("started");
    let before = run.state.clone();
    assert!(matches!(
        run.withdraw("a", lease, WithdrawalCause::NotStarted),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Running,
            ..
        })
    ));
    assert_eq!(run.state, before);
}

#[test]
fn a_stale_fence_withdraws_nothing() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let lease = run.lease("a");
    run.withdraw("a", lease, WithdrawalCause::NotSent)
        .expect("withdraw");
    let current = run.lease("a");
    let before = run.state.clone();
    assert_eq!(
        run.withdraw("a", lease, WithdrawalCause::NotSent),
        Err(ExecutionError::StaleAttempt { node: id("a") }),
        "the first lease's fence does not reach the second lease"
    );
    assert_eq!(run.state, before);
    assert_eq!(run.status("a"), StepStatus::Leased);
    run.complete("a", current)
        .expect("the live lease is untouched");
}

#[test]
fn a_ready_step_cannot_replay_a_withdrawal() {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: id("a"),
        })
        .expect("start");
    state
        .apply(&RunEvent::StepReleased { node: id("a") })
        .expect("ready");
    assert!(
        state
            .apply(&RunEvent::StepLeaseWithdrawn {
                node: id("a"),
                attempt: 0,
                generation: 0,
                cause: WithdrawalCause::NotSent,
            })
            .is_err()
    );
}

/// A run that is being cancelled asks the leased step to stop; once its lease is withdrawn nothing ran, so the step is
/// cancelled in the same transaction and the run ends (decision D006-011).
#[test]
fn a_withdrawal_during_cancellation_cancels_the_step_without_a_review() {
    let mut run = Harness::start(build("a", vec![effect("a", false)]));
    let lease = run.lease("a");
    run.stop(StopReason::Cancelled).expect("cancel");
    assert!(run.state.step(&id("a")).expect("a").cancel_requested());
    run.withdraw("a", lease, WithdrawalCause::NotSent)
        .expect("withdraw");
    assert_eq!(run.status("a"), StepStatus::Cancelled);
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    assert!(!run.names().contains(&"step.needs_review"));
    let tail: Vec<_> = run.names().into_iter().rev().take(3).collect();
    assert_eq!(
        tail,
        ["run.finished", "step.cancelled", "step.lease_withdrawn"]
    );
    run.assert_replays();
}

#[test]
fn a_reset_copies_a_withdrawal_like_a_lease() {
    let event = RunEvent::StepLeaseWithdrawn {
        node: id("a"),
        attempt: 1,
        generation: 1,
        cause: WithdrawalCause::NotSent,
    };
    assert_eq!(reset_rule(&event), ResetRule::Copy);
    assert!(
        sagat_domain::execution::RESET_RULES
            .iter()
            .any(|(name, rule)| *name == "step.lease_withdrawn" && *rule == ResetRule::Copy)
    );
}

#[test]
fn the_cause_has_two_closed_values() {
    for cause in [WithdrawalCause::NotSent, WithdrawalCause::NotStarted] {
        assert_eq!(WithdrawalCause::parse(cause.as_str()), Some(cause));
    }
    assert_eq!(WithdrawalCause::NotSent.as_str(), "NOT_SENT");
    assert_eq!(WithdrawalCause::NotStarted.as_str(), "NOT_STARTED");
    assert_eq!(WithdrawalCause::parse("not_sent"), None);
    assert_eq!(WithdrawalCause::parse(""), None);
}

// ------------------------------------------------------------------ scoped instances

fn sid(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}

fn collection() -> WorkflowDefinition {
    let body = BodyDefinition::new(
        sid("body"),
        sid("work"),
        vec![
            Node::new(
                sid("work"),
                NodeKind::Activity {
                    activity: ActivityName::parse("act.work").expect("activity"),
                    retry: RetryPolicy::DEFAULT,
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            )
            .with_next(sid("return")),
            Node::new(
                sid("return"),
                NodeKind::Return {
                    result: Expression::parse("item").expect("result"),
                },
            ),
        ],
    )
    .expect("body");
    WorkflowDefinition::new_structured(
        Slug::parse("withdrawn-collection").expect("workflow"),
        sid("items"),
        vec![
            Node::new(
                sid("items"),
                NodeKind::ForEach {
                    items: Expression::parse("input.items").expect("items"),
                    operators: None,
                    body: sid("body"),
                    result: Expression::parse("bodyResult").expect("aggregate"),
                    max_items: 10_000,
                    max_concurrency: 256,
                },
            )
            .with_next(sid("done")),
            Node::new(sid("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        sagat_domain::workflow::WorkflowLimits::default(),
    )
    .expect("definition")
}

fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(sid("items"), index)
        .expect("path")
}

fn member(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(path(index), sid("work"))
}

fn scoped_state(iterations: u32) -> RunState {
    let mut state = RunState::new(RunId::generate());
    for event in [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: sid("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: sid("items"),
            item_count: iterations,
            max_items: 10_000,
            max_concurrency: 256,
            snapshot_digest: sagat_domain::workflow::Digest::new([1; 32]),
        }),
    ] {
        state.apply_addressed(&event).expect("scope event");
    }
    for index in 0..iterations {
        state
            .apply_addressed(&AddressedRunEvent::root(
                RunEvent::CollectionIterationAdmitted {
                    node: sid("items"),
                    index,
                    entry: member(index),
                    item_digest: sagat_domain::workflow::Digest::new([2; 32]),
                },
            ))
            .expect("iteration admitted");
        state
            .apply_addressed(
                &AddressedRunEvent::in_path(
                    path(index),
                    RunEvent::StepReleased { node: sid("work") },
                )
                .expect("address"),
            )
            .expect("release");
    }
    state
}

/// A scoped withdrawal changes only its own iteration, and the bounded path reducer agrees with the full run on every
/// case: an ordinary lease, a lease whose cancellation was asked for, and a started step that is refused.
#[test]
fn a_scoped_withdrawal_matches_the_full_run_and_touches_only_its_iteration() {
    let def = collection();
    for case in 0..3 {
        let mut full = scoped_state(2);
        let (other, _) = lease_instance(&mut full, &member(0)).expect("lease the other iteration");
        let (lease, _) = lease_instance(&mut full, &member(1)).expect("lease");
        assert_eq!(lease, fence(1, 1));
        if case == 1 {
            full.apply_addressed(
                &AddressedRunEvent::in_path(
                    path(1),
                    RunEvent::StepCancelRequested { node: sid("work") },
                )
                .expect("address"),
            )
            .expect("request cancellation");
        }
        if case == 2 {
            execution::heartbeat_instance(&mut full, &member(1), lease).expect("started");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded");
        let expected = execution::withdraw_instance(
            &def,
            &mut full,
            &member(1),
            lease,
            WithdrawalCause::NotStarted,
            0,
        );
        let actual = execution::withdraw_path_instance(
            &def,
            &mut bounded,
            &member(1),
            lease,
            WithdrawalCause::NotStarted,
            0,
        );
        assert_eq!(actual, expected, "case {case}");
        if case == 2 {
            assert!(
                matches!(
                    expected,
                    Err(ExecutionError::IllegalInstanceTransition {
                        status: StepStatus::Running,
                        ..
                    })
                ),
                "{expected:?}"
            );
            continue;
        }
        let progress = expected.expect("withdrawn");
        assert_eq!(
            progress.events.first().map(AddressedRunEvent::event),
            Some(&RunEvent::StepLeaseWithdrawn {
                node: sid("work"),
                attempt: 1,
                generation: 1,
                cause: WithdrawalCause::NotStarted,
            })
        );
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(1)).expect("oracle"),
            "case {case}"
        );
        let step = full.instance(&member(1)).expect("withdrawn instance");
        assert_eq!(step.status(), StepStatus::Ready, "case {case}");
        assert_eq!((step.attempt(), step.generation()), (0, 1));
        assert!(!step.cancel_requested(), "a withdrawal clears the request");
        let untouched = full.instance(&member(0)).expect("other instance");
        assert_eq!(untouched.status(), StepStatus::Leased);
        assert_eq!(
            (untouched.attempt(), untouched.generation()),
            (other.attempt, other.generation)
        );

        let (again, _) = lease_instance(&mut full, &member(1)).expect("lease again");
        assert_eq!(again, fence(1, 2));
    }
}

#[test]
fn a_scoped_withdrawal_with_a_stale_fence_is_refused() {
    let def = collection();
    let mut full = scoped_state(1);
    let (lease, _) = lease_instance(&mut full, &member(0)).expect("lease");
    let before = full.clone();
    let stale = fence(lease.attempt, lease.generation + 1);
    assert!(matches!(
        execution::withdraw_instance(
            &def,
            &mut full,
            &member(0),
            stale,
            WithdrawalCause::NotSent,
            0
        ),
        Err(ExecutionError::StaleInstanceAttempt { .. })
    ));
    assert_eq!(full, before);
    let _ = OffsetDateTime::UNIX_EPOCH;
}

/// One attempt is withdrawn at most four times (decision D006-031): the fourth withdrawal of an attempt is taken, the fifth
/// is refused and leaves the step `LEASED` to that lease. Once the attempt is spent, the next attempt has its own four.
#[test]
fn one_attempt_is_withdrawn_at_most_four_times() {
    assert_eq!(execution::MAX_WITHDRAWALS_PER_ATTEMPT, 4);
    for cause in [WithdrawalCause::NotSent, WithdrawalCause::NotStarted] {
        let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
        for taken in 0..4u32 {
            let lease = run.lease("a");
            assert_eq!(lease.attempt, 1, "every withdrawn lease reuses attempt 1");
            assert_eq!(run.withdrawals_of("a", 1), taken);
            run.withdraw("a", lease, cause).expect("within the cap");
        }
        let fifth = run.lease("a");
        assert_eq!(run.withdrawals_of("a", 1), 4);
        let before = run.state.clone();
        assert!(
            matches!(
                run.withdraw("a", fifth, cause),
                Err(ExecutionError::WithdrawalsSpent { attempt: 1, .. })
            ),
            "{cause:?}"
        );
        assert_eq!(run.state, before, "a refused withdrawal changes nothing");
        assert_eq!(run.status("a"), StepStatus::Leased);

        // the attempt is spent by an ordinary failure; attempt 2 starts with no withdrawal of its own
        run.heartbeat("a", fifth).expect("started");
        run.fail("a", fifth, true).expect("fail");
        run.advance(ms(100));
        run.retry_due("a").expect("due");
        let next = run.lease("a");
        assert_eq!(next.attempt, 2);
        assert_eq!(run.withdrawals_of("a", 2), 0);
        run.withdraw("a", next, cause)
            .expect("attempt 2 has its own cap");
        run.assert_replays();
    }
}

/// The cap holds in a scoped instance and in the bounded path reducer alike, and a count below it is taken.
#[test]
fn a_scoped_attempt_is_withdrawn_at_most_four_times() {
    let def = collection();
    let mut full = scoped_state(1);
    for before in 0..4u32 {
        let (lease, _) = lease_instance(&mut full, &member(0)).expect("lease");
        execution::withdraw_instance(
            &def,
            &mut full,
            &member(0),
            lease,
            WithdrawalCause::NotSent,
            before,
        )
        .expect("within the cap");
    }
    let (fifth, _) = lease_instance(&mut full, &member(0)).expect("lease");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("bounded");
    let before = full.clone();
    let expected = execution::withdraw_instance(
        &def,
        &mut full,
        &member(0),
        fifth,
        WithdrawalCause::NotStarted,
        4,
    );
    let actual = execution::withdraw_path_instance(
        &def,
        &mut bounded,
        &member(0),
        fifth,
        WithdrawalCause::NotStarted,
        4,
    );
    assert_eq!(actual, expected);
    assert!(
        matches!(
            expected,
            Err(ExecutionError::WithdrawalsSpent { attempt: 1, .. })
        ),
        "{expected:?}"
    );
    assert_eq!(full, before);
    assert_eq!(
        full.instance(&member(0)).expect("instance").status(),
        StepStatus::Leased
    );
}
