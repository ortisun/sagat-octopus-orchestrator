#![allow(clippy::expect_used)]

use sagat_domain::execution::{AddressedRunEvent, ExecutionInstanceId, InstancePath, RunEvent};
use sagat_domain::workflow::NodeId;

use super::{
    SCOPED_EXECUTION_ADAPTER_REQUIRED, root_events_for_legacy_adapter,
    root_outputs_for_legacy_adapter,
};
use crate::error::StoreError;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("node id")
}

fn scoped_instance(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), index)
            .expect("bounded path"),
        node("work"),
    )
}

#[test]
fn root_events_retain_their_existing_shape_for_legacy_adapters() {
    let event = RunEvent::StepReleased { node: node("work") };
    let addressed = AddressedRunEvent::root(event.clone());

    assert_eq!(
        root_events_for_legacy_adapter(&[addressed]).expect("root event"),
        vec![event]
    );
}

#[test]
fn scoped_events_fail_instead_of_becoming_root_events() {
    let instance = scoped_instance(3);
    let addressed = AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::StepReleased {
            node: instance.node().clone(),
        },
    )
    .expect("step event can be addressed");

    assert_eq!(
        root_events_for_legacy_adapter(&[addressed]),
        Err(StoreError::Internal {
            detail: SCOPED_EXECUTION_ADAPTER_REQUIRED.into(),
        })
    );
}

#[test]
fn root_outputs_keep_the_existing_node_key() {
    let output = br#"{"ok":true}"#.to_vec();
    let instance = ExecutionInstanceId::root(node("work"));

    assert_eq!(
        root_outputs_for_legacy_adapter(&[(instance, output.clone())]).expect("root output"),
        vec![(node("work"), output)]
    );
}

#[test]
fn scoped_outputs_fail_instead_of_colliding_by_local_node_id() {
    let first = scoped_instance(0);
    let second = scoped_instance(1);

    assert_eq!(
        root_outputs_for_legacy_adapter(&[
            (first, br#"{"index":0}"#.to_vec()),
            (second, br#"{"index":1}"#.to_vec()),
        ]),
        Err(StoreError::Internal {
            detail: SCOPED_EXECUTION_ADAPTER_REQUIRED.into(),
        })
    );
}
