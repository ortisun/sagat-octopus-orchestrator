//! A continuation boundary waits for the work its run still holds (D296).
//!
//! Every obligation that makes a run non-quiescent maps to one named blocker, read from the run's
//! own state. A blocker is a reason to wait, never a terminal outcome of the chain.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sagat_domain::execution::{
    ApprovalRequestId, ChainError, CorrelationKey, FailureCode, Quiescence, QuiescenceBlocker,
    RunId, RunState, RunStatus, SagaPhase, SagaState, StepState, StepStatus, WaitingOn,
};
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("node id")
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("valid timestamp")
}

fn step(status: StepStatus, routed: bool, waiting: Option<WaitingOn>) -> StepState {
    StepState::restore(
        status, 1, 1, false, routed, None, waiting, None, None, false,
    )
}

fn boundary_step() -> StepState {
    step(StepStatus::Pending, false, None)
}

fn finished_step() -> StepState {
    step(StepStatus::Succeeded, true, None)
}

fn run(others: Vec<(&str, StepState)>, sagas: BTreeMap<NodeId, SagaState>) -> RunState {
    let mut steps = BTreeMap::new();
    steps.insert(node("rollover"), boundary_step());
    for (name, state) in others {
        steps.insert(node(name), state);
    }
    RunState::restore(
        RunId::generate(),
        RunStatus::Running,
        None,
        false,
        steps,
        sagas,
        7,
    )
}

fn blocker_of(state: &RunState) -> Option<QuiescenceBlocker> {
    Quiescence::of_run(state, &node("rollover")).blocker()
}

#[test]
fn a_run_whose_other_steps_are_finished_and_routed_is_quiescent() {
    let state = run(vec![("work", finished_step())], BTreeMap::new());
    assert_eq!(blocker_of(&state), None);
    assert_eq!(
        Quiescence::of_run(&state, &node("rollover")),
        Quiescence::ready()
    );
}

#[test]
fn every_kind_of_open_work_maps_to_its_own_blocker() {
    let now = at(1_000);
    let cases: Vec<(&str, StepState, QuiescenceBlocker)> = vec![
        (
            "ready activity",
            step(StepStatus::Ready, false, None),
            QuiescenceBlocker::SiblingWork,
        ),
        (
            "backing off",
            step(StepStatus::RetryWait, false, None),
            QuiescenceBlocker::SiblingWork,
        ),
        (
            "finished but not routed",
            step(StepStatus::Succeeded, false, None),
            QuiescenceBlocker::SiblingWork,
        ),
        (
            "leased",
            step(StepStatus::Leased, false, None),
            QuiescenceBlocker::Lease,
        ),
        (
            "running",
            step(StepStatus::Running, false, None),
            QuiescenceBlocker::Lease,
        ),
        (
            "durable timer",
            step(
                StepStatus::Waiting,
                false,
                Some(WaitingOn::Timer { due_at: now }),
            ),
            QuiescenceBlocker::Timer,
        ),
        (
            "event subscription",
            step(
                StepStatus::Waiting,
                false,
                Some(WaitingOn::Event {
                    correlation_key: CorrelationKey::parse("order-1").expect("key"),
                    deadline: now,
                }),
            ),
            QuiescenceBlocker::Subscription,
        ),
        (
            "approval",
            step(
                StepStatus::Waiting,
                false,
                Some(WaitingOn::Approval {
                    request_id: ApprovalRequestId::generate(),
                }),
            ),
            QuiescenceBlocker::Approval,
        ),
        (
            "child run",
            step(
                StepStatus::Waiting,
                false,
                Some(WaitingOn::Child {
                    run: RunId::generate(),
                }),
            ),
            QuiescenceBlocker::Child,
        ),
        (
            "unknown effect awaiting reconciliation",
            step(StepStatus::Waiting, false, Some(WaitingOn::Reconciliation)),
            QuiescenceBlocker::UnknownEffect,
        ),
        (
            "parallel branches",
            step(StepStatus::Waiting, false, Some(WaitingOn::Branches)),
            QuiescenceBlocker::SiblingWork,
        ),
        (
            "operator review",
            step(
                StepStatus::Waiting,
                false,
                Some(WaitingOn::Operator {
                    code: FailureCode::parse("MANUAL_REVIEW").expect("code"),
                }),
            ),
            QuiescenceBlocker::SiblingWork,
        ),
    ];
    for (label, open, expected) in cases {
        let state = run(vec![("other", open)], BTreeMap::new());
        assert_eq!(blocker_of(&state), Some(expected), "{label}");
    }
}

#[test]
fn a_saga_that_is_not_done_blocks_as_compensation_and_a_done_saga_does_not() {
    for (phase, blocked) in [
        (SagaPhase::Forward, true),
        (SagaPhase::Aborting, true),
        (SagaPhase::Compensating, true),
        (SagaPhase::Done, false),
    ] {
        let mut sagas = BTreeMap::new();
        sagas.insert(
            node("scope"),
            SagaState::restore(phase, None, BTreeMap::new(), Vec::new(), None, false),
        );
        let state = run(vec![("work", finished_step())], sagas);
        assert_eq!(
            blocker_of(&state),
            blocked.then_some(QuiescenceBlocker::Compensation),
            "{phase:?}"
        );
    }
}

#[test]
fn the_boundary_step_itself_never_blocks_its_own_handoff() {
    let state = run(Vec::new(), BTreeMap::new());
    assert_eq!(blocker_of(&state), None);
}

#[test]
fn compensation_outranks_other_work_so_the_reported_blocker_is_stable() {
    let mut sagas = BTreeMap::new();
    sagas.insert(
        node("scope"),
        SagaState::restore(
            SagaPhase::Compensating,
            None,
            BTreeMap::new(),
            Vec::new(),
            None,
            false,
        ),
    );
    let state = run(
        vec![
            ("a", step(StepStatus::Leased, false, None)),
            ("b", step(StepStatus::Ready, false, None)),
        ],
        sagas,
    );
    assert_eq!(blocker_of(&state), Some(QuiescenceBlocker::Compensation));
}

#[test]
fn an_unresolved_obligation_is_a_reason_to_wait_and_not_a_terminal_error() {
    for blocker in [
        QuiescenceBlocker::SiblingWork,
        QuiescenceBlocker::Child,
        QuiescenceBlocker::Lease,
        QuiescenceBlocker::Timer,
        QuiescenceBlocker::Subscription,
        QuiescenceBlocker::Approval,
        QuiescenceBlocker::UnknownEffect,
        QuiescenceBlocker::Compensation,
    ] {
        let error = ChainError::NotQuiescent(blocker);
        assert_eq!(error.awaited_blocker(), Some(blocker));
        assert!(error.is_transient(), "{blocker:?} waits");
        assert!(
            !blocker.label().is_empty()
                && blocker
                    .label()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
            "{blocker:?} has a bounded metric label"
        );
    }
    // Every other rejection is final for that attempt: nothing waits for it to change.
    for error in [
        ChainError::SegmentLimitExceeded,
        ChainError::CancellationRequested,
        ChainError::ChildCancellationRequested,
        ChainError::DeadlineChanged,
        ChainError::DefinitionVersionChanged,
    ] {
        assert_eq!(error.awaited_blocker(), None, "{error}");
        assert!(!error.is_transient(), "{error}");
    }
}

#[test]
fn blocker_labels_are_distinct() {
    let labels: std::collections::BTreeSet<_> = QuiescenceBlocker::ALL
        .into_iter()
        .map(QuiescenceBlocker::label)
        .collect();
    assert_eq!(labels.len(), QuiescenceBlocker::ALL.len());
}
