//! What an operator may do to a step that has not started: retry one whose retry is scheduled
//! without waiting, and hold one for a person (FR-008-004, US04). Both race with the platform's own
//! clock, and the first to act wins.
#![allow(clippy::expect_used, clippy::panic)] // helpers fail the test by panicking

mod common;

use std::time::Duration;

use common::{Harness, build, effect, effect_with, flaky, id, ms, saga};
use sagat_domain::execution::{
    ExecutionError, Expiry, FailureCode, ReviewOutcome, RunStatus, StepStatus, StopReason,
};
use sagat_domain::workflow::{BackoffStrategy, RetryPolicy};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn waiting_to_retry() -> Harness {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    let first = run.lease("a");
    run.fail("a", first, true).expect("the first attempt fails");
    assert_eq!(run.status("a"), StepStatus::RetryWait);
    run
}

#[test]
fn an_operator_does_not_have_to_wait_for_the_delay_of_a_retry() {
    let mut run = waiting_to_retry();
    // The delay has not passed: the scheduler would refuse; the operator does not need to wait.
    assert_eq!(run.retry_due("a"), Err(ExecutionError::NotYetDue(id("a"))));
    run.operator_retry("a")
        .expect("the operator asks for the attempt now");
    assert_eq!(run.status("a"), StepStatus::Ready);

    // The attempts so far are in the journal and the next is the second.
    let second = run.lease("a");
    assert_eq!(second.attempt, 2);
    assert_eq!(
        run.journal
            .iter()
            .filter(|e| e.name() == "step.leased")
            .count(),
        2,
        "the first attempt is still there"
    );
    run.assert_replays();
}

#[test]
fn the_scheduler_and_the_operator_cannot_both_start_an_attempt_whichever_is_first() {
    // The operator first: the scheduler finds the step no longer waiting and the domain refuses.
    let mut run = waiting_to_retry();
    run.advance(secs(1));
    run.operator_retry("a").expect("operator first");
    assert!(
        matches!(
            run.retry_due("a"),
            Err(ExecutionError::IllegalTransition {
                status: StepStatus::Ready,
                ..
            })
        ),
        "the scheduler arrives second"
    );
    run.lease("a");
    assert_eq!(
        run.journal
            .iter()
            .filter(|e| e.name() == "step.leased")
            .count(),
        2
    );

    // The scheduler first: the operator finds the step ready and is told so.
    let mut run = waiting_to_retry();
    run.advance(secs(1));
    run.retry_due("a").expect("scheduler first");
    assert!(
        matches!(
            run.operator_retry("a"),
            Err(ExecutionError::IllegalTransition {
                status: StepStatus::Ready,
                ..
            })
        ),
        "the operator arrives second"
    );
    run.lease("a");
    assert_eq!(
        run.journal
            .iter()
            .filter(|e| e.name() == "step.leased")
            .count(),
        2
    );
    run.assert_replays();
}

#[test]
fn a_step_in_flight_or_done_cannot_be_retried_by_hand() {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    // Ready, never failed: nothing to retry.
    assert!(matches!(
        run.operator_retry("a"),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Ready,
            ..
        })
    ));
    let fence = run.lease("a");
    assert!(
        matches!(
            run.operator_retry("a"),
            Err(ExecutionError::IllegalTransition { .. })
        ),
        "in flight"
    );
    run.complete("a", fence).expect("completes");
    let refused = run.operator_retry("a");
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn an_operator_can_hold_a_step_that_has_not_started_and_the_run_waits_for_a_person() {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    assert_eq!(run.status("a"), StepStatus::Ready);
    run.operator_hold("a").expect("held while ready");
    assert_eq!(run.status("a"), StepStatus::Waiting);
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert_eq!(
        run.state
            .step(&id("a"))
            .and_then(|s| s.failure())
            .map(FailureCode::as_str),
        Some("MARKED_FOR_REVIEW")
    );
    // A worker cannot take it.
    assert!(run.try_lease("a").is_err());
    // Nothing happens by itself: the clock does not move it.
    run.advance(secs(3600));
    assert_eq!(run.status("a"), StepStatus::Waiting);

    // A person decides, as for any step that waits: retry, and the next attempt is the first.
    run.operator_retry("a").expect("retry");
    assert_eq!(run.status("a"), StepStatus::Ready);
    assert_eq!(run.state.status(), RunStatus::Running);
    let fence = run.lease("a");
    assert_eq!(
        fence.attempt, 1,
        "it never started: this is its first attempt"
    );
    run.complete("a", fence).expect("complete");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn a_step_waiting_out_a_retry_delay_can_be_held_and_then_settled_by_hand() {
    let mut run = waiting_to_retry();
    run.operator_hold("a").expect("held while waiting to retry");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    // The delay passes and the scheduler finds nothing to do.
    run.advance(secs(10));
    assert!(matches!(
        run.retry_due("a"),
        Err(ExecutionError::IllegalTransition {
            status: StepStatus::Waiting,
            ..
        })
    ));
    // The person says it happened.
    run.operator_resolve("a", ReviewOutcome::Succeeded)
        .expect("settled");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn a_step_that_is_in_flight_or_already_held_or_finished_cannot_be_held() {
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    let fence = run.lease("a");
    assert!(
        matches!(
            run.operator_hold("a"),
            Err(ExecutionError::IllegalTransition { .. })
        ),
        "in flight"
    );
    run.fail("a", fence, true).expect("fails");
    run.operator_hold("a").expect("held");
    assert!(
        matches!(
            run.operator_hold("a"),
            Err(ExecutionError::IllegalTransition { .. })
        ),
        "already held"
    );
    run.operator_resolve("a", ReviewOutcome::Succeeded)
        .expect("settled");
    assert!(run.operator_hold("a").is_err(), "finished");
    assert!(
        run.operator_hold("nothing-like-it").is_err(),
        "no such step"
    );
}

#[test]
fn the_cause_of_holding_and_retrying_is_in_the_journal() {
    let mut run = waiting_to_retry();
    run.operator_hold("a").expect("held");
    run.operator_retry("a").expect("retried");
    let causes: Vec<(String, String)> = run
        .journal
        .iter()
        .filter_map(|e| e.cause())
        .map(|c| (c.kind.to_owned(), c.code))
        .collect();
    assert!(
        causes.contains(&("review".to_owned(), "MARKED_FOR_REVIEW".to_owned())),
        "{causes:?}"
    );
    assert!(
        causes.contains(&("decision".to_owned(), "RETRY".to_owned())),
        "{causes:?}"
    );
}

#[test]
fn a_lookup_cannot_be_held_only_the_step_it_is_for() {
    use common::reconciled;
    let mut run = Harness::start(build("submit", vec![reconciled("submit")]));
    let fence = run.lease("submit");
    run.fail_with("submit", fence, "LEASE_EXPIRED", true)
        .expect("an attempt whose outcome is unknown");
    assert!(run.has_step("submit~r"), "the lookup exists");
    assert!(
        matches!(
            run.operator_hold("submit~r"),
            Err(ExecutionError::IllegalTransition { .. })
        ),
        "holding the lookup would leave the step waiting for an answer that was declared never to come"
    );
    run.assert_replays();
}

#[test]
fn holding_a_step_does_not_forget_that_its_last_attempt_may_have_had_an_effect() {
    // The lease of the first attempt ran out: nobody knows whether the effect happened, and the
    // step waits for its retry. An operator holds it meanwhile.
    let mut run = Harness::start(build("a", vec![flaky("a", 3)]));
    let first = run.lease("a");
    run.expire("a", first, Expiry::LeaseExpired)
        .expect("the lease is lost");
    assert_eq!(run.status("a"), StepStatus::RetryWait);
    run.operator_hold("a").expect("held while it waits");

    let step = run.state.step(&id("a")).expect("step");
    assert_eq!(
        step.failure().map(FailureCode::as_str),
        Some("LEASE_EXPIRED"),
        "the outcome of the attempt is still unknown: the hold is only why it is waiting now"
    );
    assert!(step.failure().is_some_and(FailureCode::leaves_outcome_unknown));
    run.assert_replays();
}

#[test]
fn a_held_step_whose_attempt_may_have_had_an_effect_is_still_undone_when_the_saga_is_stopped() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "reserve"),
            effect("reserve", false).with_next(id("submit")),
            effect_with(
                "submit",
                false,
                RetryPolicy::new(3, ms(100), ms(100), BackoffStrategy::Fixed, false)
                    .expect("retry"),
            ),
        ],
    ));
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.expire("submit", f, Expiry::LeaseExpired).expect("lost");
    assert_eq!(run.status("submit"), StepStatus::RetryWait);
    run.operator_hold("submit").expect("held");
    run.stop(StopReason::Cancelled).expect("the run is cancelled");

    // `submit` may have happened, so it is undone before `reserve`, as if it had never been held.
    assert_eq!(
        run.status("submit~c"),
        StepStatus::Ready,
        "the effect may have happened, so it is undone first"
    );
    assert_eq!(run.status("reserve~c"), StepStatus::Pending);
    run.assert_replays();
}
