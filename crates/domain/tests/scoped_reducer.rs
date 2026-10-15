//! Scoped journal addresses drive independent reducer state without enabling structured planning.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sagat_domain::execution::{
    AddressedRunEvent, ExecutionError, ExecutionInstanceId, Fence, InstancePath, RunEvent, RunId,
    RunState, RunStatus, StepState, StepStatus, heartbeat, heartbeat_instance, lease_instance,
    lease_step,
};
use sagat_domain::workflow::NodeId;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

fn instance(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), index)
            .expect("bounded path"),
        node("work"),
    )
}

fn ready() -> StepState {
    StepState::restore(
        StepStatus::Ready,
        0,
        0,
        false,
        false,
        None,
        None,
        None,
        None,
        false,
    )
}

fn state_with_two_iterations() -> RunState {
    RunState::restore_instances(
        RunId::generate(),
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([(instance(0), ready()), (instance(1), ready())]),
        BTreeMap::new(),
        0,
    )
}

#[test]
fn addressed_events_keep_equal_definition_nodes_independent() {
    let mut state = state_with_two_iterations();

    let (first_fence, first_lease) = lease_instance(&mut state, &instance(0)).expect("first lease");
    let (second_fence, second_lease) =
        lease_instance(&mut state, &instance(1)).expect("second lease");

    assert_eq!(
        first_fence,
        Fence {
            attempt: 1,
            generation: 1
        }
    );
    assert_eq!(second_fence, first_fence);
    assert_eq!(first_lease.instance(), Some(instance(0)));
    assert_eq!(second_lease.instance(), Some(instance(1)));
    assert_eq!(
        state.instance(&instance(0)).map(StepState::status),
        Some(StepStatus::Leased)
    );
    assert_eq!(
        state.instance(&instance(1)).map(StepState::status),
        Some(StepStatus::Leased)
    );

    let heartbeat = heartbeat_instance(&mut state, &instance(0), first_fence)
        .expect("first heartbeat starts only the addressed iteration");
    assert_eq!(
        heartbeat
            .event
            .as_ref()
            .and_then(AddressedRunEvent::instance),
        Some(instance(0))
    );
    assert_eq!(
        state.instance(&instance(0)).map(StepState::status),
        Some(StepStatus::Running)
    );
    assert_eq!(
        state.instance(&instance(1)).map(StepState::status),
        Some(StepStatus::Leased)
    );
}

#[test]
fn fence_validation_is_scoped_to_the_addressed_instance() {
    let mut state = state_with_two_iterations();
    let (first_fence, _) = lease_instance(&mut state, &instance(0)).expect("first lease");
    let _ = lease_instance(&mut state, &instance(1)).expect("second lease");
    let second_started = heartbeat_instance(&mut state, &instance(1), first_fence)
        .expect("equal counters are valid only for the explicitly addressed instance");
    assert_eq!(
        second_started.event.and_then(|event| event.instance()),
        Some(instance(1))
    );

    let stale = heartbeat_instance(
        &mut state,
        &instance(0),
        Fence {
            attempt: 1,
            generation: 2,
        },
    );
    assert_eq!(
        stale,
        Err(ExecutionError::StaleInstanceAttempt {
            instance: instance(0)
        })
    );
}

#[test]
fn unsupported_scoped_completion_is_rejected_without_mutation() {
    let mut state = state_with_two_iterations();
    let before = state.clone();
    let event = AddressedRunEvent::in_path(
        instance(0).path().clone(),
        RunEvent::StepCompleted {
            node: node("work"),
            attempt: 1,
            generation: 1,
        },
    )
    .expect("step event can be addressed");

    assert_eq!(
        state.apply_addressed(&event),
        Err(ExecutionError::UnsupportedScopedEvent("step.completed"))
    );
    assert_eq!(state, before);
}

#[test]
fn root_instance_commands_preserve_legacy_events() {
    let root = ExecutionInstanceId::root(node("work"));
    let mut legacy = RunState::restore(
        RunId::generate(),
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([(node("work"), ready())]),
        BTreeMap::new(),
        0,
    );
    let mut addressed = legacy.clone();

    let (legacy_fence, legacy_lease) = lease_step(&mut legacy, root.node()).expect("legacy lease");
    let (addressed_fence, addressed_lease) =
        lease_instance(&mut addressed, &root).expect("root addressed lease");
    assert_eq!(addressed_fence, legacy_fence);
    assert_eq!(addressed_lease, AddressedRunEvent::root(legacy_lease));

    let legacy_heartbeat =
        heartbeat(&mut legacy, root.node(), legacy_fence).expect("legacy heartbeat");
    let addressed_heartbeat = heartbeat_instance(&mut addressed, &root, addressed_fence)
        .expect("root addressed heartbeat");
    assert_eq!(
        addressed_heartbeat.event,
        legacy_heartbeat.event.map(AddressedRunEvent::root)
    );
    assert_eq!(addressed, legacy);
}

#[test]
fn addressed_transitions_preserve_paths_and_changed_instances() {
    let first = instance(0);
    let second = instance(1);
    let events = [
        AddressedRunEvent::in_path(
            first.path().clone(),
            RunEvent::StepReleased {
                node: first.node().clone(),
            },
        )
        .expect("node event can be scoped"),
        AddressedRunEvent::in_path(
            second.path().clone(),
            RunEvent::StepReleased {
                node: second.node().clone(),
            },
        )
        .expect("node event can be scoped"),
    ];
    let pending = StepState::restore(
        StepStatus::Pending,
        0,
        0,
        false,
        false,
        None,
        None,
        None,
        None,
        false,
    );
    let mut state = RunState::restore_instances(
        RunId::generate(),
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([(first.clone(), pending.clone()), (second.clone(), pending)]),
        BTreeMap::new(),
        0,
    );

    for event in &events {
        state
            .apply_addressed(event)
            .expect("addressed event applies");
    }

    assert_eq!(events[0].changed_instances(), vec![first.clone()]);
    assert_eq!(events[1].changed_instances(), vec![second.clone()]);
    assert_eq!(
        state.instance(&first).map(StepState::status),
        Some(StepStatus::Ready)
    );
    assert_eq!(
        state.instance(&second).map(StepState::status),
        Some(StepStatus::Ready)
    );
    assert!(state.step(first.node()).is_none());
}

#[test]
fn run_level_events_reject_a_scoped_address() {
    let error = AddressedRunEvent::in_path(instance(0).path().clone(), RunEvent::RunPaused)
        .expect_err("a run event has no scoped instance");
    assert_eq!(
        error.to_string(),
        "a run-level event must use the root path"
    );
}
