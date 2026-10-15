//! Logical identity is independent of attempts and unambiguous in nested scopes.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    ExecutionInstanceId, InstanceIdentityError, InstancePath, RunEvent, RunId, RunState,
};
use sagat_domain::workflow::NodeId;

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
