//! The cause a timeline shows next to a journal event (FR-008-002, US01-AC01).
#![allow(clippy::expect_used, clippy::panic)] // helpers fail the test by panicking

use sagat_domain::execution::{
    Cause, ChildOutcome, FailureCode, Fence, JoinOutcome, ReviewOutcome, RunEvent, RunStatus,
    StopReason,
};
use sagat_domain::saga::ReconcileVerdict;
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;

fn node(name: &str) -> NodeId {
    NodeId::parse(name).expect("a node id")
}

fn code_of(code: &str) -> FailureCode {
    FailureCode::parse(code).expect("a conforming code")
}

fn cause(kind: &'static str, code: &str) -> Option<Cause> {
    Some(Cause {
        kind,
        code: code.to_owned(),
    })
}

#[test]
fn a_failure_a_retry_and_a_review_say_which_code_caused_them() {
    let code = code_of("PAYMENT_DECLINED");
    let fence = Some(Fence {
        attempt: 2,
        generation: 1,
    });
    assert_eq!(
        RunEvent::StepFailed {
            node: node("charge"),
            code: code.clone(),
            fence
        }
        .cause(),
        cause("failure", "PAYMENT_DECLINED")
    );
    assert_eq!(
        RunEvent::StepRetryScheduled {
            node: node("charge"),
            attempt: 2,
            generation: 1,
            code: code.clone(),
            not_before: OffsetDateTime::UNIX_EPOCH,
        }
        .cause(),
        cause("retry", "PAYMENT_DECLINED")
    );
    assert_eq!(
        RunEvent::StepNeedsReview {
            node: node("charge"),
            code,
            fence
        }
        .cause(),
        cause("review", "PAYMENT_DECLINED")
    );
}

#[test]
fn a_stop_says_the_code_when_there_is_one_and_the_reason_when_there_is_not() {
    assert_eq!(
        RunEvent::RunStopRequested {
            reason: StopReason::Failed,
            code: Some(code_of("STEP_FAILED")),
        }
        .cause(),
        cause("stop", "STEP_FAILED")
    );
    assert_eq!(
        RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None
        }
        .cause(),
        cause("stop", "CANCELLED")
    );
}

#[test]
fn what_an_operator_decided_is_a_decision_and_what_they_asked_is_a_request() {
    let n = node("charge");
    assert_eq!(
        RunEvent::StepRetryRequested { node: n.clone() }.cause(),
        cause("decision", "RETRY")
    );
    assert_eq!(
        RunEvent::StepSkipped { node: n.clone() }.cause(),
        cause("decision", "SKIP")
    );
    assert_eq!(
        RunEvent::StepResolved {
            node: n.clone(),
            outcome: ReviewOutcome::Succeeded
        }
        .cause(),
        cause("decision", "SUCCEEDED")
    );
    assert_eq!(
        RunEvent::StepResolved {
            node: n,
            outcome: ReviewOutcome::Failed(code_of("ABANDONED")),
        }
        .cause(),
        cause("decision", "ABANDONED")
    );
    assert_eq!(RunEvent::RunPaused.cause(), cause("request", "PAUSE"));
    assert_eq!(RunEvent::RunResumed.cause(), cause("request", "RESUME"));
}

#[test]
fn the_clock_a_child_and_a_branch_are_causes_too() {
    assert_eq!(
        RunEvent::WaitTimedOut {
            node: node("wait"),
            handled: false
        }
        .cause(),
        cause("timeout", "WAIT_TIMED_OUT")
    );
    assert_eq!(
        RunEvent::ChildFinished {
            node: node("kid"),
            child: sagat_domain::execution::RunId::generate(),
            outcome: ChildOutcome::Failed(code_of("CHILD_FAILED")),
        }
        .cause(),
        cause("failure", "CHILD_FAILED")
    );
    assert_eq!(
        RunEvent::ParallelJoined {
            node: node("fan"),
            outcome: JoinOutcome::Failed
        }
        .cause(),
        cause("failure", "BRANCH_FAILED")
    );
    assert_eq!(
        RunEvent::RunFinished {
            status: RunStatus::Succeeded
        }
        .cause(),
        cause("outcome", "SUCCEEDED")
    );
}

#[test]
fn an_event_that_is_only_progress_has_no_cause() {
    let n = node("charge");
    for event in [
        RunEvent::StepReleased { node: n.clone() },
        RunEvent::StepLeased {
            node: n.clone(),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepStarted {
            node: n.clone(),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepCompleted {
            node: n.clone(),
            attempt: 1,
            generation: 1,
        },
        RunEvent::TimerFired { node: n.clone() },
        RunEvent::EndReached { node: n },
    ] {
        assert_eq!(event.cause(), None, "{}", event.name());
    }
}

#[test]
fn a_reconciliation_says_what_the_provider_answered() {
    let verdict = ReconcileVerdict::NotApplied;
    assert_eq!(
        RunEvent::ReconciliationConcluded {
            node: node("charge"),
            verdict,
            result: sagat_domain::execution::ReconcileResult::Succeeded,
        }
        .cause(),
        cause("reconciliation", verdict.as_str())
    );
}
