//! Scoped activity lifecycle remains isolated by complete execution-instance identity.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sagat_domain::execution::{
    AddressedRunEvent, ExecutionError, ExecutionInstanceId, Fence, InstancePath, RunEvent, RunId,
    RunState, RunStatus, StepState, StepStatus,
};
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

fn instance(index: u32, local: &str) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), index)
            .expect("bounded path"),
        node(local),
    )
}

fn ready(attempt: u32, generation: u64) -> StepState {
    StepState::restore(
        StepStatus::Ready,
        attempt,
        generation,
        false,
        false,
        None,
        None,
        None,
        None,
        false,
    )
}

fn state() -> RunState {
    RunState::restore_instances(
        RunId::generate(),
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([
            (instance(0, "work"), ready(1, 1)),
            (instance(1, "work"), ready(0, 0)),
        ]),
        BTreeMap::new(),
        0,
    )
}

fn addressed(index: u32, event: RunEvent) -> AddressedRunEvent {
    AddressedRunEvent::in_path(instance(index, "work").path().clone(), event)
        .expect("step event can be scoped")
}

fn lease(index: u32, attempt: u32, generation: u64) -> AddressedRunEvent {
    addressed(
        index,
        RunEvent::StepLeased {
            node: node("work"),
            attempt,
            generation,
        },
    )
}

fn start(index: u32, attempt: u32, generation: u64) -> AddressedRunEvent {
    addressed(
        index,
        RunEvent::StepStarted {
            node: node("work"),
            attempt,
            generation,
        },
    )
}

#[test]
fn completion_and_routing_keep_equal_local_nodes_in_separate_iterations() {
    let mut state = state();
    for event in [lease(0, 2, 2), start(0, 2, 2), lease(1, 1, 1)] {
        state.apply_addressed(&event).expect("lifecycle transition");
    }
    let before_stale = state.clone();
    let stale = addressed(
        1,
        RunEvent::StepCompleted {
            node: node("work"),
            attempt: 2,
            generation: 2,
        },
    );
    assert_eq!(
        state.apply_addressed(&stale),
        Err(ExecutionError::StaleInstanceAttempt {
            instance: instance(1, "work")
        })
    );
    assert_eq!(state, before_stale);

    for (index, attempt, generation) in [(0, 2, 2), (1, 1, 1)] {
        state
            .apply_addressed(&addressed(
                index,
                RunEvent::StepCompleted {
                    node: node("work"),
                    attempt,
                    generation,
                },
            ))
            .expect("addressed completion");
        state
            .apply_addressed(&addressed(
                index,
                RunEvent::StepRouted {
                    node: node("work"),
                    activated: vec![node("return")],
                },
            ))
            .expect("addressed routing");
    }

    for index in 0..=1 {
        assert_eq!(
            state
                .instance(&instance(index, "work"))
                .map(StepState::status),
            Some(StepStatus::Succeeded)
        );
        assert!(
            state
                .instance(&instance(index, "work"))
                .is_some_and(StepState::routed)
        );
        assert_eq!(
            state
                .instance(&instance(index, "return"))
                .map(StepState::status),
            Some(StepStatus::Pending)
        );
    }
}

#[test]
fn a_fenced_failure_changes_only_its_addressed_iteration() {
    let mut state = state();
    for event in [
        lease(0, 2, 2),
        start(0, 2, 2),
        lease(1, 1, 1),
        start(1, 1, 1),
    ] {
        state.apply_addressed(&event).expect("lifecycle transition");
    }

    state
        .apply_addressed(&addressed(
            0,
            RunEvent::StepFailed {
                node: node("work"),
                code: sagat_domain::execution::FailureCode::worker_error(),
                fence: Some(Fence {
                    attempt: 2,
                    generation: 2,
                }),
            },
        ))
        .expect("addressed failure");

    assert_eq!(
        state.instance(&instance(0, "work")).map(StepState::status),
        Some(StepStatus::Failed)
    );
    assert_eq!(
        state.instance(&instance(1, "work")).map(StepState::status),
        Some(StepStatus::Running)
    );
    let before_stale = state.clone();
    assert_eq!(
        state.apply_addressed(&addressed(
            1,
            RunEvent::StepFailed {
                node: node("work"),
                code: sagat_domain::execution::FailureCode::worker_error(),
                fence: Some(Fence {
                    attempt: 2,
                    generation: 2,
                }),
            },
        )),
        Err(ExecutionError::StaleInstanceAttempt {
            instance: instance(1, "work")
        })
    );
    assert_eq!(state, before_stale);
}

#[test]
fn retry_schedule_and_due_are_isolated_by_path_and_fence() {
    let mut state = state();
    for event in [
        lease(0, 2, 2),
        start(0, 2, 2),
        lease(1, 1, 1),
        start(1, 1, 1),
    ] {
        state.apply_addressed(&event).expect("lifecycle transition");
    }
    let first_due = OffsetDateTime::from_unix_timestamp(100).expect("timestamp");
    state
        .apply_addressed(&addressed(
            0,
            RunEvent::StepRetryScheduled {
                node: node("work"),
                attempt: 2,
                generation: 2,
                code: sagat_domain::execution::FailureCode::worker_error(),
                not_before: first_due,
            },
        ))
        .expect("first retry scheduled");
    assert_eq!(
        state.instance(&instance(0, "work")).map(StepState::status),
        Some(StepStatus::RetryWait)
    );
    assert_eq!(
        state
            .instance(&instance(0, "work"))
            .and_then(StepState::next_attempt_at),
        Some(first_due)
    );
    assert_eq!(
        state.instance(&instance(1, "work")).map(StepState::status),
        Some(StepStatus::Running)
    );

    let before_stale = state.clone();
    assert_eq!(
        state.apply_addressed(&addressed(
            1,
            RunEvent::StepRetryScheduled {
                node: node("work"),
                attempt: 2,
                generation: 2,
                code: sagat_domain::execution::FailureCode::worker_error(),
                not_before: first_due,
            },
        )),
        Err(ExecutionError::StaleInstanceAttempt {
            instance: instance(1, "work")
        })
    );
    assert_eq!(state, before_stale);

    let second_due = OffsetDateTime::from_unix_timestamp(200).expect("timestamp");
    state
        .apply_addressed(&addressed(
            1,
            RunEvent::StepRetryScheduled {
                node: node("work"),
                attempt: 1,
                generation: 1,
                code: sagat_domain::execution::FailureCode::worker_error(),
                not_before: second_due,
            },
        ))
        .expect("second retry scheduled");
    state
        .apply_addressed(&addressed(0, RunEvent::StepRetryDue { node: node("work") }))
        .expect("first retry becomes ready");

    assert_eq!(
        state.instance(&instance(0, "work")).map(StepState::status),
        Some(StepStatus::Ready)
    );
    assert_eq!(
        state
            .instance(&instance(0, "work"))
            .and_then(StepState::next_attempt_at),
        None
    );
    assert_eq!(
        state.instance(&instance(1, "work")).map(StepState::status),
        Some(StepStatus::RetryWait)
    );
    assert_eq!(
        state
            .instance(&instance(1, "work"))
            .and_then(StepState::next_attempt_at),
        Some(second_due)
    );
}
