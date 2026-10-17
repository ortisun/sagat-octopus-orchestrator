//! Collection and loop limits of the definition model (T014-014): `maxItems` and `maxIterations`
//! are integers in 1..10000 and `maxConcurrency` is 1..256, each checked at its first accepted and
//! first rejected value. The reducer rules (membership fixed after entry, admission bounded by
//! `maxConcurrency`, one result per index) are in `collection_reducer.rs`; the shared run and
//! tenant budgets are storage invariants and are tested against PostgreSQL.
#![allow(clippy::expect_used)]

use std::time::Duration;

use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    BodyDefinition, Expression, Node, NodeId, NodeKind, WorkflowDefinition, WorkflowError,
    WorkflowLimits,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node id")
}

fn expr(value: &str) -> Expression {
    Expression::parse(value).expect("expression")
}

fn body() -> BodyDefinition {
    BodyDefinition::new(
        id("line"),
        id("return"),
        vec![Node::new(
            id("return"),
            NodeKind::Return {
                result: expr("item"),
            },
        )],
    )
    .expect("closed body")
}

fn collection(max_items: u32, max_concurrency: u16) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new_structured(
        Slug::parse("bounded").expect("name"),
        id("items"),
        vec![
            Node::new(
                id("items"),
                NodeKind::ForEach {
                    items: expr("input.items"),
                    body: id("line"),
                    result: expr("bodyResult"),
                    max_items,
                    max_concurrency,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body()],
        WorkflowLimits::default(),
    )
}

fn looping(max_iterations: u32) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new_structured(
        Slug::parse("bounded").expect("name"),
        id("again"),
        vec![
            Node::new(
                id("again"),
                NodeKind::While {
                    condition: expr("carry < 3"),
                    body: id("line"),
                    initial: expr("0"),
                    carry: expr("bodyResult"),
                    max_iterations,
                    delay: None,
                },
            )
            .with_timeout(Duration::from_secs(60))
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body()],
        WorkflowLimits::default(),
    )
}

#[test]
fn max_items_is_an_integer_from_one_to_ten_thousand() {
    for accepted in [1, 2, 9_999, 10_000] {
        assert!(collection(accepted, 1).is_ok(), "maxItems {accepted}");
    }
    for rejected in [0, 10_001, u32::MAX] {
        assert!(collection(rejected, 1).is_err(), "maxItems {rejected}");
    }
}

#[test]
fn max_concurrency_is_an_integer_from_one_to_256() {
    for accepted in [1, 2, 255, 256] {
        assert!(
            collection(10, accepted).is_ok(),
            "maxConcurrency {accepted}"
        );
    }
    for rejected in [0, 257, u16::MAX] {
        assert!(
            collection(10, rejected).is_err(),
            "maxConcurrency {rejected}"
        );
    }
}

#[test]
fn max_iterations_is_an_integer_from_one_to_ten_thousand() {
    for accepted in [1, 2, 9_999, 10_000] {
        assert!(looping(accepted).is_ok(), "maxIterations {accepted}");
    }
    for rejected in [0, 10_001, u32::MAX] {
        assert!(looping(rejected).is_err(), "maxIterations {rejected}");
    }
}
