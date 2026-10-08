//! Scenario tests for the run state machine (T002-009, T002-014, T002-028; FR-002-001..005).
//!
//! Each scenario drives a run exactly as the application does and checks the journal, and that the
//! journal alone rebuilds the state.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{Harness, T0, activity, build, condition, end, flaky, id, parallel, timer, wait};
use sagat_domain::execution::{
    self, CorrelationKey, ExecutionError, Expiry, FailureCode, Fence, Resolution, RunEvent, RunId,
    RunState, RunStatus, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::workflow::JoinPolicy;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ------------------------------------------------------------------ the happy path

#[test]
fn a_two_step_chain_runs_to_success_and_the_journal_tells_the_story() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    assert_eq!(run.names(), ["run.started", "step.released"]);
    assert_eq!(run.state.status(), RunStatus::Running);

    let fence = run.lease("a");
    assert_eq!(
        fence,
        Fence {
            attempt: 1,
            generation: 1
        }
    );
    let beat = run.heartbeat("a", fence).expect("heartbeat");
    assert!(!beat.cancel_requested && beat.event.is_some());
    assert_eq!(run.status("a"), StepStatus::Running);
    run.complete("a", fence).expect("complete a");
    assert!(run.has_step("b") && run.status("b") == StepStatus::Ready);

    let fence = run.lease("b");
    run.complete("b", fence).expect("complete b");

    assert_eq!(
        run.names(),
        [
            "run.started",
            "step.released",
            "step.leased",
            "step.started",
            "step.completed",
            "step.routed",
            "step.released",
            "step.leased",
            "step.completed",
            "step.routed",
            "run.finished",
        ]
    );
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    assert_eq!(run.state.version(), 11, "one version per journal entry");
    run.assert_replays();
}

#[test]
fn a_second_heartbeat_does_not_journal_again() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let fence = run.lease("a");
    run.heartbeat("a", fence).expect("first");
    let before = run.journal.len();
    let beat = run.heartbeat("a", fence).expect("second");
    assert!(beat.event.is_none());
    assert_eq!(run.journal.len(), before);
}

// ------------------------------------------------------------------ fencing and retries

#[test]
fn a_stale_generation_cannot_complete_a_step() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let first = run.lease("a");
    run.expire("a", first, Expiry::LeaseExpired)
        .expect("expire");
    run.advance(secs(120));
    run.retry_due("a").expect("due");
    let second = run.lease("a");
    assert_eq!(
        second,
        Fence {
            attempt: 2,
            generation: 2
        }
    );

    let stale = run.complete("a", first);
    assert_eq!(stale, Err(ExecutionError::StaleAttempt { node: id("a") }));
    assert_eq!(
        run.heartbeat("a", first).map(|_| ()),
        Err(ExecutionError::StaleAttempt { node: id("a") })
    );
    run.complete("a", second)
        .expect("the current holder still can");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn a_late_report_after_the_lease_was_reclaimed_is_rejected_without_a_trace() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let first = run.lease("a");
    run.expire("a", first, Expiry::LeaseExpired)
        .expect("expire");
    let journal = run.journal.clone();
    let state = run.state.clone();

    assert!(matches!(
        run.complete("a", first),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::RetryWait,
            ..
        })
    ));
    assert!(run.fail("a", first, true).is_err());
    assert_eq!(run.journal, journal, "a rejected command journals nothing");
    assert_eq!(run.state, state, "and changes nothing");
}

#[test]
fn a_retryable_failure_backs_off_then_retries_until_the_budget_is_spent() {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));

    let first = run.lease("a");
    run.fail("a", first, true).expect("fail 1");
    assert_eq!(run.status("a"), StepStatus::RetryWait);
    let step = run.state.step(&id("a")).expect("step");
    assert_eq!(
        step.next_attempt_at(),
        Some(T0 + Duration::from_millis(100))
    );
    assert_eq!(
        step.failure().map(FailureCode::as_str),
        Some("PAYMENT_DECLINED")
    );

    run.advance(Duration::from_millis(50));
    assert_eq!(run.retry_due("a"), Err(ExecutionError::NotYetDue(id("a"))));
    run.advance(Duration::from_millis(50));
    run.retry_due("a").expect("due now");
    assert_eq!(run.status("a"), StepStatus::Ready);

    let second = run.lease("a");
    assert_eq!(
        second,
        Fence {
            attempt: 2,
            generation: 2
        }
    );
    run.fail("a", second, true).expect("fail 2");
    run.advance(secs(1));
    run.retry_due("a").expect("due again");
    let third = run.lease("a");
    assert_eq!(third.attempt, 3);

    run.fail("a", third, true)
        .expect("the last attempt fails for good");
    assert_eq!(run.status("a"), StepStatus::Failed);
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(
        run.state.stop_code().map(FailureCode::as_str),
        Some("PAYMENT_DECLINED")
    );
    run.assert_replays();
}

#[test]
fn a_non_retryable_failure_skips_the_remaining_attempts() {
    let mut run = Harness::start(build("a", vec![flaky("a", 5)]));
    let fence = run.lease("a");
    run.fail("a", fence, false).expect("fail");
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(run.state.step(&id("a")).map(|s| s.attempt()), Some(1));
}

#[test]
fn an_error_route_handles_a_failure_and_the_run_still_succeeds() {
    let mut run = Harness::start(build(
        "a",
        vec![
            activity("a")
                .with_next(id("b"))
                .with_on_error(id("cleanup")),
            activity("b"),
            activity("cleanup"),
        ],
    ));
    let fence = run.lease("a");
    run.fail("a", fence, false).expect("fail");
    assert_eq!(run.status("a"), StepStatus::Failed);
    assert!(run.has_step("cleanup") && !run.has_step("b"));
    assert_eq!(run.state.status(), RunStatus::Running);

    let fence = run.lease("cleanup");
    run.complete("cleanup", fence).expect("cleanup");
    assert_eq!(
        run.state.status(),
        RunStatus::Succeeded,
        "a handled failure is not a failed run"
    );
    run.assert_replays();
}

#[test]
fn lease_expiry_and_attempt_timeout_consume_an_attempt_with_their_own_code() {
    for (cause, code) in [
        (Expiry::LeaseExpired, "LEASE_EXPIRED"),
        (Expiry::AttemptTimeout, "ATTEMPT_TIMEOUT"),
    ] {
        let mut run = Harness::start(build("a", vec![flaky("a", 2)]));
        let fence = run.lease("a");
        run.expire("a", fence, cause).expect("expire");
        let step = run.state.step(&id("a")).expect("step");
        assert_eq!(step.status(), StepStatus::RetryWait);
        assert_eq!(step.failure().map(FailureCode::as_str), Some(code));
        assert_eq!(step.attempt(), 1);

        run.advance(secs(1));
        run.retry_due("a").expect("due");
        let fence = run.lease("a");
        run.expire("a", fence, cause)
            .expect("second expiry exhausts the budget");
        assert_eq!(run.state.status(), RunStatus::Failed, "{code}");
    }
}

#[test]
fn heartbeats_and_reports_need_a_lease() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let ghost = Fence {
        attempt: 1,
        generation: 1,
    };
    assert!(matches!(
        run.heartbeat("a", ghost),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Ready,
            ..
        })
    ));
    assert_eq!(
        run.complete("nope", ghost),
        Err(ExecutionError::UnknownStep(id("nope")))
    );
}

// ------------------------------------------------------------------ timers and waits

#[test]
fn a_timer_parks_the_run_without_any_worker_until_it_is_due() {
    let mut run = Harness::start(build(
        "pause",
        vec![
            timer("pause", 3600).with_next(id("after")),
            activity("after"),
        ],
    ));
    assert_eq!(run.names(), ["run.started", "timer.started"]);
    assert_eq!(run.state.status(), RunStatus::Waiting);
    assert_eq!(
        run.state
            .step(&id("pause"))
            .and_then(|s| s.waiting_on().cloned()),
        Some(WaitingOn::Timer {
            due_at: T0 + secs(3600)
        })
    );
    assert!(!run.has_step("after"));

    run.advance(secs(3599));
    assert_eq!(
        run.fire_timer("pause"),
        Err(ExecutionError::NotYetDue(id("pause")))
    );
    run.advance(secs(1));
    run.fire_timer("pause").expect("due");
    assert_eq!(
        run.status("after"),
        StepStatus::Ready,
        "a new task exists once the timer fired"
    );
    assert_eq!(run.state.status(), RunStatus::Running);
    run.assert_replays();
}

#[test]
fn a_timer_can_fire_only_once() {
    let mut run = Harness::start(build(
        "t",
        vec![timer("t", 1).with_next(id("a")), activity("a")],
    ));
    run.advance(secs(1));
    run.fire_timer("t").expect("first");
    assert!(matches!(
        run.fire_timer("t"),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Succeeded,
            ..
        })
    ));
}

fn wait_flow(on_timeout: Option<&str>) -> Harness {
    let mut nodes = vec![
        wait("w", 300, on_timeout).with_next(id("notify")),
        activity("notify"),
    ];
    if let Some(handler) = on_timeout {
        nodes.push(activity(handler));
    }
    Harness::start(build("w", nodes))
}

#[test]
fn a_wait_subscribes_under_the_evaluated_correlation_key() {
    let mut run = wait_flow(Some("review"));
    assert_eq!(run.names(), ["run.started"]);
    assert!(run.pending.is_some(), "the key must be evaluated first");
    run.answer(true, "payment-42");
    let step = run.state.step(&id("w")).expect("step");
    assert_eq!(
        step.waiting_on(),
        Some(&WaitingOn::Event {
            correlation_key: CorrelationKey::parse("payment-42").expect("key"),
            deadline: T0 + secs(300),
        })
    );
    assert_eq!(run.state.status(), RunStatus::Waiting);

    run.satisfy_wait("w").expect("event arrived");
    assert_eq!(run.status("notify"), StepStatus::Ready);
    assert!(!run.has_step("review"));
    run.assert_replays();
}

#[test]
fn a_wait_that_times_out_takes_the_timeout_route() {
    let mut run = wait_flow(Some("review"));
    run.answer(true, "payment-42");
    run.advance(secs(299));
    assert_eq!(
        run.time_out_wait("w"),
        Err(ExecutionError::NotYetDue(id("w")))
    );
    run.advance(secs(1));
    run.time_out_wait("w").expect("deadline");
    assert!(run.state.step(&id("w")).is_some_and(|s| s.timed_out()));
    assert!(run.has_step("review") && !run.has_step("notify"));
}

#[test]
fn a_wait_that_times_out_without_a_route_fails_the_run() {
    let mut run = wait_flow(None);
    run.answer(true, "payment-42");
    run.advance(secs(300));
    run.time_out_wait("w").expect("deadline");
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(
        run.state.stop_code().map(FailureCode::as_str),
        Some("WAIT_TIMED_OUT")
    );
}

#[test]
fn the_first_commit_wins_the_race_between_an_event_and_its_timeout() {
    let mut event_first = wait_flow(Some("review"));
    event_first.answer(true, "k");
    event_first.advance(secs(300));
    event_first.satisfy_wait("w").expect("event wins");
    assert!(matches!(
        event_first.time_out_wait("w"),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Succeeded,
            ..
        })
    ));

    let mut timeout_first = wait_flow(Some("review"));
    timeout_first.answer(true, "k");
    timeout_first.advance(secs(300));
    timeout_first.time_out_wait("w").expect("timeout wins");
    assert!(matches!(
        timeout_first.satisfy_wait("w"),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Succeeded,
            ..
        })
    ));
    assert!(
        timeout_first.has_step("review"),
        "the losing event changed nothing"
    );
}

#[test]
fn an_event_cannot_satisfy_a_timer_and_a_timer_cannot_fire_a_wait() {
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["t", "w"], JoinPolicy::All),
            timer("t", 60),
            wait("w", 60, None),
        ],
    ));
    run.answer(true, "k");
    run.advance(secs(60));
    assert!(run.satisfy_wait("t").is_err());
    assert!(run.fire_timer("w").is_err());
    assert!(run.time_out_wait("t").is_err());
}

// ------------------------------------------------------------------ conditions

#[test]
fn a_condition_routes_by_its_resolved_outcome() {
    for (truth, taken, skipped) in [(true, "big", "small"), (false, "small", "big")] {
        let mut run = Harness::start(build(
            "check",
            vec![
                condition("check", "big", "small"),
                activity("big"),
                activity("small"),
            ],
        ));
        run.answer(truth, "");
        assert_eq!(run.status(taken), StepStatus::Ready);
        assert!(!run.has_step(skipped));
        assert_eq!(
            run.state.step(&id("check")).and_then(|s| s.condition()),
            Some(truth)
        );
        run.assert_replays();
    }
}

#[test]
fn an_expression_error_fails_the_node_without_retrying() {
    let mut run = Harness::start(build(
        "check",
        vec![
            condition("check", "big", "small"),
            activity("big"),
            activity("small"),
        ],
    ));
    run.resolve(Resolution::Condition {
        node: id("check"),
        outcome: Err(()),
    })
    .expect("resolution");
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(
        run.state.stop_code().map(FailureCode::as_str),
        Some("EXPRESSION_ERROR")
    );
}

#[test]
fn a_correlation_error_can_be_routed_through_on_error() {
    let mut run = Harness::start(build(
        "w",
        vec![
            wait("w", 60, None).with_on_error(id("fallback")),
            activity("fallback"),
        ],
    ));
    run.resolve(Resolution::Correlation {
        node: id("w"),
        key: Err(()),
    })
    .expect("resolution");
    assert_eq!(run.status("fallback"), StepStatus::Ready);
}

// ------------------------------------------------------------------ parallel

fn fork(join: JoinPolicy) -> Harness {
    Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], join).with_next(id("after")),
            activity("l"),
            activity("r"),
            activity("after"),
        ],
    ))
}

#[test]
fn an_all_join_waits_for_every_branch() {
    let mut run = fork(JoinPolicy::All);
    assert_eq!(
        run.names(),
        [
            "run.started",
            "parallel.forked",
            "step.released",
            "step.released"
        ]
    );
    let l = run.lease("l");
    let r = run.lease("r");
    run.complete("l", l).expect("l");
    assert!(
        !run.has_step("after"),
        "the join is not satisfied by one branch"
    );
    assert_eq!(run.status("fork"), StepStatus::Waiting);
    run.complete("r", r).expect("r");
    assert_eq!(run.status("fork"), StepStatus::Succeeded);
    assert_eq!(run.status("after"), StepStatus::Ready);
    run.assert_replays();
}

#[test]
fn an_all_join_fails_fast_and_stops_the_siblings() {
    let mut run = fork(JoinPolicy::All);
    let l = run.lease("l");
    let r = run.lease("r");
    run.heartbeat("r", r).expect("r runs");
    run.fail("l", l, false).expect("l fails");

    assert_eq!(run.status("fork"), StepStatus::Failed);
    assert!(
        run.state
            .step(&id("r"))
            .is_some_and(|s| s.cancel_requested()),
        "the running sibling is told to stop"
    );
    assert_eq!(
        run.state.status(),
        RunStatus::Cancelling,
        "the run waits for the worker to acknowledge"
    );
    let beat = run.heartbeat("r", r).expect("heartbeat");
    assert!(beat.cancel_requested);

    run.fail("r", r, true)
        .expect("the worker acknowledges by failing");
    assert_eq!(
        run.status("r"),
        StepStatus::Cancelled,
        "a stopped attempt is cancelled, never retried"
    );
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert!(!run.has_step("after"));
    run.assert_replays();
}

#[test]
fn an_any_join_takes_the_first_winner_and_cancels_the_rest() {
    let mut run = fork(JoinPolicy::Any);
    let l = run.lease("l");
    run.complete("l", l).expect("l wins");
    assert_eq!(run.status("fork"), StepStatus::Succeeded);
    assert_eq!(
        run.status("r"),
        StepStatus::Cancelled,
        "the loser never started"
    );
    assert_eq!(run.status("after"), StepStatus::Ready);
}

#[test]
fn an_any_join_fails_only_when_every_branch_failed() {
    let mut run = fork(JoinPolicy::Any);
    let l = run.lease("l");
    run.fail("l", l, false).expect("l fails");
    assert_eq!(
        run.status("fork"),
        StepStatus::Waiting,
        "one failure is tolerated"
    );
    let r = run.lease("r");
    run.fail("r", r, false).expect("r fails");
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(run.status("fork"), StepStatus::Failed);
}

#[test]
fn a_branch_can_handle_its_own_failure_and_still_join() {
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All).with_next(id("after")),
            activity("l").with_on_error(id("cleanup")),
            activity("cleanup"),
            activity("r"),
            activity("after"),
        ],
    ));
    let l = run.lease("l");
    run.fail("l", l, false).expect("l fails");
    assert_eq!(
        run.status("fork"),
        StepStatus::Waiting,
        "handled inside the branch"
    );
    let c = run.lease("cleanup");
    run.complete("cleanup", c).expect("cleanup");
    let r = run.lease("r");
    run.complete("r", r).expect("r");
    assert_eq!(run.status("after"), StepStatus::Ready);
}

#[test]
fn nested_parallels_join_inside_out() {
    let mut run = Harness::start(build(
        "outer",
        vec![
            parallel("outer", &["a", "inner"], JoinPolicy::All).with_next(id("after")),
            activity("a"),
            parallel("inner", &["b", "c"], JoinPolicy::Any).with_next(id("tail")),
            activity("b"),
            activity("c"),
            activity("tail"),
            activity("after"),
        ],
    ));
    let a = run.lease("a");
    run.complete("a", a).expect("a");
    let b = run.lease("b");
    run.complete("b", b).expect("b wins the inner join");
    assert_eq!(run.status("c"), StepStatus::Cancelled);
    assert_eq!(run.status("tail"), StepStatus::Ready);
    assert_eq!(
        run.status("outer"),
        StepStatus::Waiting,
        "the outer join still waits for the tail"
    );
    let t = run.lease("tail");
    run.complete("tail", t).expect("tail");
    assert_eq!(run.status("after"), StepStatus::Ready);
    let f = run.lease("after");
    run.complete("after", f).expect("after");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

// ------------------------------------------------------------------ stopping, pausing, deadlines

#[test]
fn cancelling_a_run_with_nothing_in_flight_finishes_at_once() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    assert_eq!(run.status("a"), StepStatus::Cancelled);
    assert!(!run.has_step("b"));
    run.assert_replays();
}

#[test]
fn cancelling_a_run_lets_the_worker_acknowledge_before_it_ends() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let fence = run.lease("a");
    run.heartbeat("a", fence).expect("running");
    run.stop(StopReason::Cancelled).expect("cancel");

    assert_eq!(run.state.status(), RunStatus::Cancelling);
    assert!(run.heartbeat("a", fence).expect("beat").cancel_requested);
    run.complete("a", fence)
        .expect("a late result is still recorded");
    assert_eq!(run.status("a"), StepStatus::Succeeded);
    assert!(!run.has_step("b"), "but nothing new starts");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn an_attempt_that_vanishes_during_cancellation_is_cancelled_not_retried() {
    let mut run = Harness::start(build("a", vec![flaky("a", 5)]));
    let fence = run.lease("a");
    run.stop(StopReason::Cancelled).expect("cancel");
    run.expire("a", fence, Expiry::LeaseExpired)
        .expect("expire");
    assert_eq!(run.status("a"), StepStatus::Cancelled);
    assert_eq!(run.state.status(), RunStatus::Cancelled);
}

#[test]
fn cancelling_drops_timers_and_waits_and_a_late_timer_is_refused() {
    let mut run = Harness::start(build(
        "t",
        vec![timer("t", 60).with_next(id("a")), activity("a")],
    ));
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.advance(secs(61));
    assert_eq!(
        run.fire_timer("t"),
        Err(ExecutionError::RunFinished(RunStatus::Cancelled))
    );
}

#[test]
fn a_deadline_ends_the_run_as_timed_out() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    run.stop(StopReason::TimedOut).expect("deadline");
    assert_eq!(run.state.status(), RunStatus::TimedOut);
}

#[test]
fn a_run_can_be_stopped_only_once() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let fence = run.lease("a");
    run.stop(StopReason::Cancelled).expect("first");
    assert_eq!(
        run.stop(StopReason::TimedOut),
        Err(ExecutionError::AlreadyStopping)
    );
    run.complete("a", fence).expect("ack");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
}

#[test]
fn pausing_holds_back_new_work_and_resuming_catches_up() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let fence = run.lease("a");
    run.pause().expect("pause");
    assert_eq!(run.state.status(), RunStatus::Paused);

    run.complete("a", fence)
        .expect("an in-flight attempt may finish");
    assert!(!run.has_step("b"), "but its successors are held");
    assert_eq!(run.state.status(), RunStatus::Paused);
    assert_eq!(run.pause(), Err(ExecutionError::AlreadyPaused));

    run.resume().expect("resume");
    assert_eq!(run.status("b"), StepStatus::Ready);
    assert_eq!(run.state.status(), RunStatus::Running);
    assert_eq!(run.resume(), Err(ExecutionError::NotPaused));
    run.assert_replays();
}

#[test]
fn a_paused_run_grants_no_leases() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    run.pause().expect("pause");
    assert_eq!(
        run.try_lease("a"),
        Err(ExecutionError::RunNotRunnable(RunStatus::Paused))
    );
    run.resume().expect("resume");
    assert!(run.try_lease("a").is_ok());
}

#[test]
fn a_paused_run_can_still_be_cancelled() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    run.pause().expect("pause");
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    assert_eq!(
        run.pause(),
        Err(ExecutionError::RunFinished(RunStatus::Cancelled))
    );
}

#[test]
fn a_stopping_run_cannot_be_paused() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let fence = run.lease("a");
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.pause(), Err(ExecutionError::AlreadyStopping));
    run.complete("a", fence).expect("ack");
}

#[test]
fn a_finished_run_refuses_everything() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let fence = run.lease("a");
    run.complete("a", fence).expect("done");
    let finished = ExecutionError::RunFinished(RunStatus::Succeeded);
    assert_eq!(
        run.try_lease("a"),
        Err(ExecutionError::RunNotRunnable(RunStatus::Succeeded))
    );
    assert_eq!(run.complete("a", fence), Err(finished.clone()));
    assert_eq!(run.stop(StopReason::Cancelled), Err(finished.clone()));
    assert_eq!(run.pause(), Err(finished.clone()));
    assert_eq!(run.heartbeat("a", fence).map(|_| ()), Err(finished));
}

#[test]
fn an_end_node_closes_the_run() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("done")), end("done")],
    ));
    let fence = run.lease("a");
    run.complete("a", fence).expect("a");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    assert_eq!(run.status("done"), StepStatus::Succeeded);
}

// ------------------------------------------------------------------ the reducer on its own

#[test]
fn the_reducer_rejects_illegal_events_and_leaves_the_state_alone() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let before = run.state.clone();
    let bad: Vec<RunEvent> = vec![
        RunEvent::RunStarted {
            entrypoint: id("a"),
        },
        RunEvent::StepReleased { node: id("a") },
        RunEvent::StepLeased {
            node: id("a"),
            attempt: 2,
            generation: 1,
        },
        RunEvent::StepLeased {
            node: id("a"),
            attempt: 1,
            generation: 9,
        },
        RunEvent::StepLeased {
            node: id("ghost"),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepCompleted {
            node: id("a"),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepRouted {
            node: id("a"),
            activated: vec![id("b")],
        },
        RunEvent::TimerFired { node: id("a") },
        RunEvent::RunFinished {
            status: RunStatus::Succeeded,
        },
        RunEvent::RunFinished {
            status: RunStatus::Cancelled,
        },
        RunEvent::RunResumed,
    ];
    for event in bad {
        assert!(
            run.state.apply(&event).is_err(),
            "{event:?} must be rejected"
        );
        assert_eq!(run.state, before, "{event:?} must not change the state");
    }
}

#[test]
fn routing_cannot_activate_a_step_twice() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let fence = run.lease("a");
    run.state
        .apply(&RunEvent::StepCompleted {
            node: id("a"),
            attempt: fence.attempt,
            generation: fence.generation,
        })
        .expect("complete");
    let before = run.state.clone();
    let error = run
        .state
        .apply(&RunEvent::StepRouted {
            node: id("a"),
            activated: vec![id("b"), id("b")],
        })
        .expect_err("duplicate activation");
    assert_eq!(error, ExecutionError::AlreadyScheduled(id("b")));
    assert_eq!(run.state, before);
}

#[test]
fn a_run_cannot_finish_while_work_remains() {
    let run = Harness::start(build("a", vec![activity("a")]));
    let mut state = run.state;
    assert!(matches!(
        state.apply(&RunEvent::RunFinished {
            status: RunStatus::Succeeded
        }),
        Err(ExecutionError::Invariant(_))
    ));
}

#[test]
fn replaying_a_prefix_rebuilds_every_intermediate_state() {
    let mut run = Harness::start(build(
        "a",
        vec![flaky("a", 2).with_next(id("b")), activity("b")],
    ));
    let first = run.lease("a");
    run.fail("a", first, true).expect("fail");
    run.advance(secs(1));
    run.retry_due("a").expect("due");
    let second = run.lease("a");
    run.complete("a", second).expect("complete");
    let third = run.lease("b");
    run.complete("b", third).expect("b");

    let mut rebuilt = RunState::new(run.state.run_id());
    for event in &run.journal {
        rebuilt.apply(event).expect("legal");
        assert_eq!(
            rebuilt,
            RunState::replay(
                run.state.run_id(),
                &run.journal[..rebuilt.version() as usize]
            )
            .expect("prefix")
        );
    }
    assert_eq!(rebuilt, run.state);
}

#[test]
fn failure_codes_and_keys_are_bounded_machine_values() {
    for ok in ["PAYMENT_DECLINED", "http:503", "a.b-c", &"A".repeat(64)] {
        assert!(FailureCode::parse(ok).is_ok(), "{ok}");
    }
    for bad in ["", "has space", "new\nline", "é", &"A".repeat(65)] {
        assert!(FailureCode::parse(bad).is_err(), "{bad:?}");
    }
    assert!(CorrelationKey::parse("payment-42").is_ok());
    for bad in ["", "tab\there", &"k".repeat(257)] {
        assert_eq!(
            CorrelationKey::parse(bad),
            Err(ExecutionError::InvalidCorrelationKey)
        );
    }
}

#[test]
fn a_run_id_round_trips_through_text() {
    let run_id = RunId::generate();
    assert_eq!(run_id.to_string().parse::<RunId>(), Ok(run_id));
    assert!("not-a-uuid".parse::<RunId>().is_err());
    assert_eq!("RUNNING".parse::<RunStatus>(), Ok(RunStatus::Running));
    assert!("running".parse::<RunStatus>().is_err());
    assert_eq!(
        "RETRY_WAIT".parse::<StepStatus>(),
        Ok(StepStatus::RetryWait)
    );
    let _ = execution::plan; // the planner is part of the public surface
}

#[test]
fn the_reducer_itself_refuses_a_stale_fence_even_if_a_command_did_not() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let first = run.lease("a");
    run.expire("a", first, Expiry::LeaseExpired)
        .expect("expire");
    run.advance(secs(120));
    run.retry_due("a").expect("due");
    let second = run.lease("a");
    assert_eq!(
        second,
        Fence {
            attempt: 2,
            generation: 2
        }
    );

    let before = run.state.clone();
    let stale = |node: &str| RunEvent::StepCompleted {
        node: id(node),
        attempt: 1,
        generation: 1,
    };
    let events = [
        stale("a"),
        RunEvent::StepCompleted {
            node: id("a"),
            attempt: 2,
            generation: 1,
        },
        RunEvent::StepCompleted {
            node: id("a"),
            attempt: 1,
            generation: 2,
        },
        RunEvent::StepStarted {
            node: id("a"),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepRetryScheduled {
            node: id("a"),
            attempt: 1,
            generation: 1,
            code: FailureCode::lease_expired(),
            not_before: T0,
        },
        RunEvent::StepFailed {
            node: id("a"),
            code: FailureCode::worker_error(),
            fence: Some(Fence {
                attempt: 1,
                generation: 1,
            }),
        },
        RunEvent::StepFailed {
            node: id("a"),
            code: FailureCode::worker_error(),
            fence: None,
        },
    ];
    for event in events {
        assert_eq!(
            run.state.apply(&event),
            Err(ExecutionError::StaleAttempt { node: id("a") }),
            "{event:?}"
        );
        assert_eq!(run.state, before, "{event:?}");
    }
    run.state
        .apply(&RunEvent::StepCompleted {
            node: id("a"),
            attempt: 2,
            generation: 2,
        })
        .expect("the current holder is accepted");
}

#[test]
fn a_join_waits_for_the_whole_chain_of_every_branch() {
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All).with_next(id("after")),
            activity("l").with_next(id("l2")),
            activity("l2"),
            activity("r"),
            activity("after"),
        ],
    ));
    let l = run.lease("l");
    let r = run.lease("r");
    run.complete("r", r).expect("r");
    run.complete("l", l).expect("l");
    assert_eq!(
        run.status("fork"),
        StepStatus::Waiting,
        "l2 has not run yet"
    );
    assert_eq!(run.status("l2"), StepStatus::Ready);
    assert!(!run.has_step("after"));

    let l2 = run.lease("l2");
    run.complete("l2", l2).expect("l2");
    assert_eq!(run.status("fork"), StepStatus::Succeeded);
    assert_eq!(run.status("after"), StepStatus::Ready);
    run.assert_replays();
}

#[test]
fn an_any_join_does_not_wait_for_the_losing_chain() {
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["fast", "slow"], JoinPolicy::Any).with_next(id("after")),
            activity("fast"),
            activity("slow").with_next(id("slow2")),
            activity("slow2"),
            activity("after"),
        ],
    ));
    let slow = run.lease("slow");
    let fast = run.lease("fast");
    run.complete("fast", fast).expect("fast wins");
    assert_eq!(run.status("fork"), StepStatus::Succeeded);
    assert!(
        run.state
            .step(&id("slow"))
            .is_some_and(|s| s.cancel_requested())
    );
    run.complete("slow", slow)
        .expect("the loser's late result is recorded");
    assert!(!run.has_step("slow2"), "but its chain does not continue");
    assert_eq!(
        run.state.status(),
        RunStatus::Running,
        "after is still to run"
    );
    assert_eq!(run.status("after"), StepStatus::Ready);
}

#[test]
fn a_stopping_run_cannot_finish_while_an_attempt_is_still_in_flight() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    let fence = run.lease("a");
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.state.status(), RunStatus::Cancelling);

    let before = run.state.clone();
    assert!(matches!(
        run.state.apply(&RunEvent::RunFinished {
            status: RunStatus::Cancelled
        }),
        Err(ExecutionError::Invariant(_))
    ));
    assert_eq!(run.state, before, "the leased attempt keeps the run open");
    run.complete("a", fence).expect("ack");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
}

#[test]
fn a_stopping_run_finishes_only_with_the_status_its_reason_implies() {
    let mut run = Harness::start(build("a", vec![activity("a")]));
    run.stop(StopReason::TimedOut).expect("deadline");
    assert_eq!(run.state.status(), RunStatus::TimedOut);

    // A fresh run stopped by cancellation cannot be finished as timed out.
    let run2 = Harness::start(build("a", vec![activity("a")]));
    let mut state = run2.state;
    state
        .apply(&RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        })
        .expect("stop");
    state
        .apply(&RunEvent::StepCancelled { node: id("a") })
        .expect("cancel step");
    assert!(
        state
            .apply(&RunEvent::RunFinished {
                status: RunStatus::TimedOut
            })
            .is_err()
    );
    assert!(
        state
            .apply(&RunEvent::RunFinished {
                status: RunStatus::Succeeded
            })
            .is_err()
    );
    state
        .apply(&RunEvent::RunFinished {
            status: RunStatus::Cancelled,
        })
        .expect("matching status");
}
