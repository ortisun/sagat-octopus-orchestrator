//! Logical identity is independent of attempts and unambiguous in nested scopes.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    ExecutionInstanceId, InstanceIdentityError, InstancePath, RunEvent, RunId, RunState, RunStatus,
    StepState, StepStatus,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{Node, NodeId, NodeKind, WorkflowDefinition};
use std::collections::BTreeMap;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

#[test]
fn repeated_definition_node_has_distinct_typed_instances() {
    let root = ExecutionInstanceId::root(node("reserve"));
    let first_path = InstancePath::root().child(node("items"), 0).expect("index");
    let second_path = InstancePath::root().child(node("items"), 1).expect("index");
    let first = ExecutionInstanceId::in_path(first_path.clone(), node("reserve"));
    let second = ExecutionInstanceId::in_path(second_path, node("reserve"));
    assert_ne!(root, first);
    assert_ne!(first, second);
    assert_eq!(first.path(), &first_path);
    assert_eq!(first.node(), &node("reserve"));
    assert_eq!(first.compensation().path(), &first_path);
    assert_eq!(first.reconciliation().path(), &first_path);
}

#[test]
fn path_bounds_apply_at_each_nesting_boundary() {
    assert_eq!(
        InstancePath::root().child(node("items"), 10_000),
        Err(InstanceIdentityError::IterationIndexOutOfRange)
    );
    let mut path = InstancePath::root();
    for index in 0..8 {
        path = path.child(node("loop"), index).expect("within depth");
    }
    assert_eq!(path.depth(), 8);
    assert_eq!(
        path.child(node("loop"), 0),
        Err(InstanceIdentityError::ScopeDepthExceeded)
    );
}

#[test]
fn a_typed_segment_does_not_concatenate_user_node_names() {
    let a = InstancePath::root().child(node("a-b"), 12).expect("path");
    let b = InstancePath::root().child(node("a"), 12).expect("path");
    assert_ne!(a, b);
    assert_eq!(InstancePath::root().depth(), 0);
}

#[test]
fn old_journal_events_replay_as_empty_path_instances() {
    let run = RunId::generate();
    let event = RunEvent::RunStarted {
        entrypoint: node("begin"),
    };
    let state = RunState::replay(run, [&event]).expect("old event replays");
    let instances = state
        .instances()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(instances, [ExecutionInstanceId::root(node("begin"))]);
    assert!(state.step(&node("begin")).is_some());
}

#[test]
fn projection_restore_keeps_equal_definition_nodes_in_distinct_scopes() {
    let run = RunId::generate();
    let first = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("items"), 0).expect("path"),
        node("work"),
    );
    let second = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("items"), 1).expect("path"),
        node("work"),
    );
    let step =
        |status| StepState::restore(status, 0, 0, false, false, None, None, None, None, false);
    let state = RunState::restore_instances(
        run,
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([
            (first.clone(), step(StepStatus::Ready)),
            (second.clone(), step(StepStatus::Succeeded)),
        ]),
        BTreeMap::new(),
        2,
    );
    assert_eq!(
        state.instance(&first).map(StepState::status),
        Some(StepStatus::Ready)
    );
    assert_eq!(
        state.instance(&second).map(StepState::status),
        Some(StepStatus::Succeeded)
    );
    assert!(state.step(&node("work")).is_none());
    assert_eq!(state.instances().count(), 2);
    assert_eq!(state.steps().count(), 0);
}

#[test]
fn a_live_scoped_instance_prevents_run_finalization() {
    let run = RunId::generate();
    let active = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("items"), 0).expect("path"),
        node("work"),
    );
    let root_done = StepState::restore(
        StepStatus::Succeeded,
        0,
        0,
        false,
        true,
        None,
        None,
        None,
        None,
        false,
    );
    let scoped_ready = StepState::restore(
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
    );
    let state = RunState::restore_instances(
        run,
        RunStatus::Running,
        None,
        false,
        BTreeMap::from([
            (ExecutionInstanceId::root(node("done")), root_done),
            (active, scoped_ready),
        ]),
        BTreeMap::new(),
        2,
    );
    let def = WorkflowDefinition::new(
        Slug::parse("scope-guard").expect("name"),
        node("done"),
        vec![Node::new(node("done"), NodeKind::End { output: None })],
    )
    .expect("definition");
    let decision = sagat_domain::execution::plan(&def, &state, time::OffsetDateTime::UNIX_EPOCH)
        .expect("planning");
    assert!(!matches!(
        decision,
        sagat_domain::execution::Plan::Emit(RunEvent::RunFinished { .. })
    ));
}
