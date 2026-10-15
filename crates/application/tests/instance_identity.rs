#![allow(clippy::expect_used)]

use sagat_application::execution::{instance_step_id, step_id};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::workflow::NodeId;
use std::str::FromStr;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

#[test]
fn root_step_ids_remain_byte_identical_to_legacy_derivation() {
    let version =
        WorkflowVersionId::from_str("0198c5a0-0000-7000-8000-000000000011").expect("version id");
    let run = RunId::from_str("0198c5a0-0000-7000-8000-000000000001").expect("run id");
    let root = ExecutionInstanceId::root(node("reserve"));
    assert_eq!(
        instance_step_id(version, run, &root),
        step_id(version, run, root.node())
    );
    assert_eq!(
        instance_step_id(version, run, &root).to_string(),
        "e8acb1eb-11ab-8dcf-a873-e7fb35827001"
    );
}

#[test]
fn repeated_nodes_have_distinct_stable_ids_and_derived_work_keeps_its_path() {
    let version = WorkflowVersionId::generate();
    let run = RunId::generate();
    let path0 = InstancePath::root().child(node("items"), 0).expect("path");
    let path1 = InstancePath::root().child(node("items"), 1).expect("path");
    let first = ExecutionInstanceId::in_path(path0, node("reserve"));
    let second = ExecutionInstanceId::in_path(path1, node("reserve"));
    let a = instance_step_id(version, run, &first);
    assert_eq!(a, instance_step_id(version, run, &first));
    assert_ne!(a, instance_step_id(version, run, &second));
    assert_ne!(a, step_id(version, run, first.node()));
    assert_ne!(
        instance_step_id(version, run, &first.compensation()),
        instance_step_id(version, run, &second.compensation())
    );
}
