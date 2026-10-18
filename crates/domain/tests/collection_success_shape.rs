//! Static definition shape only: no pinned version, source settlement or execution authority.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::identity::Slug;
use sagat_domain::saga::EffectPolicy;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Expression, LeafCollectionSuccessShape, Node, NodeId, NodeKind,
    RetryPolicy, WorkflowDefinition, WorkflowLimits,
};
use std::time::Duration;

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}
fn activity(name: &str, next: &str) -> Node {
    Node::new(
        id(name),
        NodeKind::Activity {
            activity: ActivityName::parse("work.item").expect("activity"),
            retry: RetryPolicy::no_retry(),
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
    .with_next(id(next))
}
fn returned() -> Node {
    Node::new(
        id("returned"),
        NodeKind::Return {
            result: Expression::parse("item").expect("expression"),
        },
    )
}
fn body(name: &str, count: usize, first: impl FnOnce(Node) -> Node) -> BodyDefinition {
    let mut nodes = (0..count)
        .map(|index| {
            activity(
                &format!("work-{}", count - index),
                if index + 1 == count {
                    "returned".to_owned()
                } else {
                    format!("work-{}", count - index - 1)
                }
                .as_str(),
            )
        })
        .collect::<Vec<_>>();
    if let Some(node) = nodes.first_mut() {
        *node = first(node.clone());
    }
    nodes.push(returned());
    BodyDefinition::new(
        id(name),
        if count == 0 {
            id("returned")
        } else {
            id(&format!("work-{count}"))
        },
        nodes,
    )
    .expect("body")
}
fn collection(name: &str, body_name: &str, next: &str) -> Node {
    Node::new(
        id(name),
        NodeKind::ForEach {
            items: Expression::parse("input.items").expect("items"),
            body: id(body_name),
            result: Expression::parse("bodyResult").expect("result"),
            max_items: 10_000,
            max_concurrency: 256,
        },
    )
    .with_next(id(next))
}
fn definition(entry: &str, nodes: Vec<Node>, bodies: Vec<BodyDefinition>) -> WorkflowDefinition {
    WorkflowDefinition::new_structured(
        Slug::parse("leaf-shape").expect("name"),
        id(entry),
        nodes,
        bodies,
        WorkflowLimits::default(),
    )
    .expect("valid broader domain definition")
}
fn plain(body: BodyDefinition) -> WorkflowDefinition {
    definition(
        "items",
        vec![
            collection("items", body.name().as_str(), "done"),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
    )
}
fn owner() -> ExecutionInstanceId {
    ExecutionInstanceId::root(id("items"))
}

#[test]
fn accepts_one_and_eight_in_actual_route_order_without_version_or_run_authority() {
    for count in [1, 8] {
        let d = plain(body("line", count, |node| node));
        let shape =
            LeafCollectionSuccessShape::check(&d, &owner()).expect("supported static shape");
        assert_eq!(shape.contract_version(), 1);
        assert_eq!(shape.owner(), &owner());
        assert_eq!(shape.body(), &id("line"));
        assert_eq!(
            shape.activities(),
            (1..=count)
                .rev()
                .map(|n| id(&format!("work-{n}")))
                .collect::<Vec<_>>()
        );
        assert_eq!(shape.return_node(), &id("returned"));
    }
}

#[test]
fn rejects_empty_and_over_eight_bodies() {
    for count in [0, 9] {
        assert!(
            LeafCollectionSuccessShape::check(&plain(body("line", count, |node| node)), &owner())
                .is_err()
        );
    }
}

#[test]
fn activity_representation_does_not_hide_scripts_companions_or_effect_policy() {
    for variant in [
        "compensation",
        "reconciliation",
        "script",
        "high-risk",
        "irreversible",
        "budget",
    ] {
        let d = plain(body("line", 1, |node| {
            let NodeKind::Activity {
                activity,
                retry,
                idempotency_template,
                secrets,
                ..
            } = node.kind().clone()
            else {
                unreachable!()
            };
            let kind = NodeKind::Activity {
                activity: if variant == "script" {
                    ActivityName::parse(&format!("script.sha256-{}", "a".repeat(64)))
                        .expect("lowered script")
                } else {
                    activity
                },
                retry,
                compensate_with: (variant == "compensation")
                    .then(|| ActivityName::parse("undo.item").expect("compensation")),
                reconcile_with: (variant == "reconciliation")
                    .then(|| ActivityName::parse("lookup.item").expect("reconciliation")),
                idempotency_template,
                secrets,
            };
            Node::new(node.id().clone(), kind)
                .with_next(id("returned"))
                .with_effect(EffectPolicy {
                    high_risk: variant == "high-risk",
                    irreversible: variant == "irreversible",
                    compensation_timeout: (variant == "budget").then_some(Duration::from_secs(1)),
                    ..EffectPolicy::NONE
                })
        }));
        assert!(
            LeafCollectionSuccessShape::check(&d, &owner()).is_err(),
            "{variant}"
        );
    }
}

#[test]
fn ordinary_prefix_is_accepted_but_lowered_script_prefix_is_not() {
    for (prefix, accepted) in [
        (activity("fetch", "items"), true),
        (
            Node::new(
                id("fetch"),
                NodeKind::Activity {
                    activity: ActivityName::parse(&format!("script.sha256-{}", "b".repeat(64)))
                        .expect("script"),
                    retry: RetryPolicy::no_retry(),
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: vec![],
                },
            )
            .with_next(id("items")),
            false,
        ),
    ] {
        let d = definition(
            "fetch",
            vec![
                prefix,
                collection("items", "line", "done"),
                Node::new(id("done"), NodeKind::End { output: None }),
            ],
            vec![body("line", 1, |node| node)],
        );
        assert_eq!(
            LeafCollectionSuccessShape::check(&d, &owner()).is_ok(),
            accepted
        );
    }
}

#[test]
fn scoped_or_noncollection_owner_cannot_alias_the_root_owner() {
    let d = plain(body("line", 1, |node| node));
    let nested = ExecutionInstanceId::in_path(
        InstancePath::root().child(id("items"), 0).expect("path"),
        id("items"),
    );
    for wrong in [
        nested,
        ExecutionInstanceId::root(id("done")),
        ExecutionInstanceId::root(id("missing")),
    ] {
        assert!(LeafCollectionSuccessShape::check(&d, &wrong).is_err());
    }
}

#[test]
fn broader_body_controls_and_error_routes_fail_closed() {
    let timed = BodyDefinition::new(
        id("line"),
        id("timer"),
        vec![
            Node::new(
                id("timer"),
                NodeKind::Timer {
                    delay: Duration::from_secs(1),
                },
            )
            .with_next(id("returned")),
            returned(),
        ],
    )
    .expect("broader body");
    assert!(LeafCollectionSuccessShape::check(&plain(timed), &owner()).is_err());
    let error = BodyDefinition::new(
        id("line"),
        id("work"),
        vec![
            activity("work", "returned").with_on_error(id("recover")),
            activity("recover", "returned"),
            returned(),
        ],
    )
    .expect("error edge");
    assert!(LeafCollectionSuccessShape::check(&plain(error), &owner()).is_err());
}

#[test]
fn a_root_activity_after_a_collection_is_outside_the_frozen_chain_shape() {
    let d = definition(
        "items",
        vec![
            collection("items", "line", "after"),
            activity("after", "done"),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body("line", 1, |node| node)],
    );
    assert!(LeafCollectionSuccessShape::check(&d, &owner()).is_err());
}

#[test]
fn equal_local_ids_and_another_scopes_effects_do_not_change_selected_body_qualification() {
    let other = body("other", 1, |node| {
        Node::new(
            node.id().clone(),
            NodeKind::Activity {
                activity: ActivityName::parse("other.work").expect("activity"),
                retry: RetryPolicy::no_retry(),
                compensate_with: Some(ActivityName::parse("other.undo").expect("undo")),
                reconcile_with: None,
                idempotency_template: None,
                secrets: vec![],
            },
        )
        .with_next(id("returned"))
    });
    let d = definition(
        "items",
        vec![
            collection("items", "line", "later"),
            collection("later", "other", "done"),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body("line", 1, |node| node), other],
    );
    let shape = LeafCollectionSuccessShape::check(&d, &owner()).expect("ordinary selected body");
    assert_eq!(shape.body(), &id("line"));
    assert_eq!(shape.activities(), &[id("work-1")]);
    assert!(
        LeafCollectionSuccessShape::check(&d, &ExecutionInstanceId::root(id("later"))).is_err()
    );
}
