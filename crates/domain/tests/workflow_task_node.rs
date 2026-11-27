//! The reserved node of a code run's workflow-task step (design 044, Technical Plan 2; Task 044-4): `~workflow` reads
//! back from storage, is neither derived nor forwarded, has no companion, and cannot be declared.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use sagat_domain::workflow::NodeId;

#[test]
fn the_workflow_task_step_is_neither_derived_nor_forwarded() {
    let node =
        NodeId::parse_stored("~workflow").expect("the reserved node reads back from storage");
    assert_eq!(node, NodeId::workflow_task());
    assert_eq!(node.as_str(), "~workflow");
    assert!(node.is_workflow_task());
    assert!(
        !node.is_derived(),
        "the workflow-task step is not a derived step"
    );
    assert_eq!(
        node.forward(),
        None,
        "the workflow-task step forwards to no node"
    );
    assert!(!node.is_compensation());
    assert!(!node.is_reconciliation());
}

#[test]
fn the_reserved_node_cannot_be_declared_nor_take_a_companion() {
    assert!(
        NodeId::parse("~workflow").is_err(),
        "no author can declare it"
    );
    for stored in [
        "~workflow~c",
        "~workflow~r",
        "~workflows",
        "x~workflow",
        "~Workflow",
        "~",
    ] {
        assert!(
            NodeId::parse_stored(stored).is_err(),
            "{stored} is not a stored id"
        );
    }
    let command = NodeId::parse_stored("c-12~c").expect("a command's compensation reads back");
    assert!(command.is_derived());
    assert_eq!(
        command.forward(),
        Some(NodeId::parse("c-12").expect("a command node"))
    );
    assert!(!command.is_workflow_task());
    assert!(
        !NodeId::parse("workflow")
            .expect("a declared id")
            .is_workflow_task()
    );
}
