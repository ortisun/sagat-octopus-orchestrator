//! Definition-level invariants for explicit continuation boundaries.
#![allow(clippy::expect_used)]

use std::time::Duration;

use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    BodyDefinition, ContinuationPolicy, Expression, Node, NodeId, NodeKind, WorkflowDefinition,
    WorkflowError, WorkflowLimits,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node id")
}

fn continuation() -> Node {
    Node::new(
        id("continue"),
        NodeKind::ContinueAsNew {
            input: Expression::parse("result.nextInput").expect("input expression"),
            state: Some(Expression::parse("state").expect("state expression")),
        },
    )
}

fn policy(max_segments: u16, timeout: Duration) -> ContinuationPolicy {
    ContinuationPolicy::new(max_segments, timeout).expect("continuation policy")
}

#[test]
fn continuation_policy_and_terminal_shape_are_bounded() {
    assert!(ContinuationPolicy::new(0, Duration::from_secs(1)).is_err());
    assert!(ContinuationPolicy::new(10_001, Duration::from_secs(1)).is_err());
    assert!(ContinuationPolicy::new(2, Duration::ZERO).is_err());
    assert!(ContinuationPolicy::new(2, Duration::from_secs(366 * 24 * 60 * 60 + 1)).is_err());

    let definition = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("continuable").expect("workflow name"),
        id("continue"),
        vec![continuation()],
        Vec::new(),
        WorkflowLimits::default(),
        policy(3, Duration::from_secs(86_400)),
    )
    .expect("continuable definition");
    assert_eq!(
        definition
            .continuation_policy()
            .expect("policy")
            .max_segments(),
        3
    );
    assert_eq!(
        definition.continuation_policy().expect("policy").timeout(),
        Duration::from_secs(86_400)
    );

    let routed = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("routed-continuation").expect("workflow name"),
        id("continue"),
        vec![
            continuation().with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        Vec::new(),
        WorkflowLimits::default(),
        policy(3, Duration::from_secs(60)),
    );
    assert!(matches!(routed, Err(WorkflowError::InvalidScope { .. })));
}

#[test]
fn ordinary_structured_definitions_and_named_bodies_reject_continuation() {
    let ordinary = WorkflowDefinition::new_structured(
        Slug::parse("ordinary").expect("workflow name"),
        id("continue"),
        vec![continuation()],
        Vec::new(),
        WorkflowLimits::default(),
    );
    assert!(matches!(ordinary, Err(WorkflowError::InvalidScope { .. })));

    let body = BodyDefinition::new(id("body"), id("continue"), vec![continuation()]);
    assert!(matches!(body, Err(WorkflowError::InvalidScope { .. })));
}

#[test]
fn continuation_is_forbidden_inside_parallel_and_saga_regions() {
    let parallel = Node::new(
        id("fork"),
        NodeKind::Parallel {
            branches: vec![id("continue"), id("other")],
            join: sagat_domain::workflow::JoinPolicy::All,
        },
    );
    let parallel_definition = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("parallel-continuation").expect("workflow name"),
        id("fork"),
        vec![
            parallel,
            continuation(),
            Node::new(id("other"), NodeKind::End { output: None }),
        ],
        Vec::new(),
        WorkflowLimits::default(),
        policy(3, Duration::from_secs(60)),
    );
    assert!(matches!(
        parallel_definition,
        Err(WorkflowError::InvalidScope { .. })
    ));

    let saga = Node::new(
        id("scope"),
        NodeKind::Saga {
            begin: id("continue"),
            on_compensation_failure: sagat_domain::saga::OnCompensationFailure::Fail,
            cancel_policy: sagat_domain::saga::SagaCancelPolicy::CompensateCompleted,
        },
    );
    let saga_definition = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("saga-continuation").expect("workflow name"),
        id("scope"),
        vec![saga, continuation()],
        Vec::new(),
        WorkflowLimits::default(),
        policy(3, Duration::from_secs(60)),
    );
    assert!(matches!(
        saga_definition,
        Err(WorkflowError::InvalidScope { .. })
    ));
}
