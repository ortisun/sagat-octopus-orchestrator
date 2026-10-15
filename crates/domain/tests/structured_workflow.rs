#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
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

fn returned() -> Node {
    Node::new(
        id("return"),
        NodeKind::Return {
            result: expr("item"),
        },
    )
}

fn collection(body: &str) -> Node {
    Node::new(
        id("items"),
        NodeKind::ForEach {
            items: expr("input.items"),
            body: id(body),
            result: expr("bodyResult"),
            max_items: 17,
            max_concurrency: 3,
        },
    )
    .with_next(id("done"))
}

fn root_nodes(body: &str) -> Vec<Node> {
    vec![
        collection(body),
        Node::new(id("done"), NodeKind::End { output: None }),
    ]
}

fn simple_body(name: &str) -> BodyDefinition {
    BodyDefinition::new(id(name), id("return"), vec![returned()]).expect("closed body")
}

fn build(
    nodes: Vec<Node>,
    bodies: Vec<BodyDefinition>,
    limits: WorkflowLimits,
) -> Result<WorkflowDefinition, sagat_domain::workflow::WorkflowError> {
    WorkflowDefinition::new_structured(
        Slug::parse("bounded-orders").expect("workflow name"),
        id("items"),
        nodes,
        bodies,
        limits,
    )
}

#[test]
fn closed_collection_body_keeps_definition_names_local() {
    let workflow = build(
        root_nodes("line"),
        vec![simple_body("line")],
        WorkflowLimits::default(),
    )
    .expect("structured definition");
    assert!(workflow.is_structured());
    assert_eq!(workflow.len(), 3);
    assert_eq!(workflow.bodies().count(), 1);
    assert_eq!(
        workflow.body(&id("line")).expect("body").entrypoint(),
        &id("return")
    );
    assert!(workflow.node(&id("return")).is_none());
    assert!(
        workflow
            .body(&id("line"))
            .expect("body")
            .node(&id("return"))
            .is_some()
    );
}

#[test]
fn instance_paths_resolve_nodes_in_their_declared_body() {
    let workflow = build(
        root_nodes("outer"),
        vec![nested_body("outer", "inner"), simple_body("inner")],
        WorkflowLimits::default(),
    )
    .expect("nested definition");
    let root = ExecutionInstanceId::root(id("items"));
    let outer = InstancePath::root()
        .child(id("items"), 0)
        .expect("outer path");
    let inner = outer.child(id("items"), 1).expect("inner path");

    assert_eq!(
        workflow.node_for_instance(&root).expect("root").id(),
        &id("items")
    );
    assert_eq!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(outer.clone(), id("items")))
            .expect("outer collection")
            .kind()
            .label(),
        "forEach"
    );
    assert_eq!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(inner.clone(), id("return")))
            .expect("inner return")
            .kind()
            .label(),
        "return"
    );
    assert!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(inner, id("done")))
            .is_none()
    );
    assert!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(
                InstancePath::root().child(id("done"), 0).expect("path"),
                id("return"),
            ))
            .is_none()
    );
    assert!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(
                outer.child(id("return"), 0).expect("path"),
                id("return"),
            ))
            .is_none()
    );
    assert!(
        workflow
            .node_for_instance(&ExecutionInstanceId::in_path(
                InstancePath::root().child(id("items"), 17).expect("path"),
                id("return"),
            ))
            .is_none()
    );
}

#[test]
fn legacy_constructor_remains_root_only() {
    let name = Slug::parse("legacy-orders").expect("name");
    let old = WorkflowDefinition::new(
        name.clone(),
        id("done"),
        vec![Node::new(id("done"), NodeKind::End { output: None })],
    )
    .expect("legacy definition");
    assert!(!old.is_structured());
    assert!(old.bodies().next().is_none());
    assert!(WorkflowDefinition::new(name, id("items"), root_nodes("line")).is_err());
    let root_return = Node::new(
        id("items"),
        NodeKind::Return {
            result: expr("null"),
        },
    );
    assert!(matches!(
        build(vec![root_return], vec![], WorkflowLimits::default()),
        Err(WorkflowError::InvalidScope { .. })
    ));
}

#[test]
fn body_rejects_missing_multiple_or_escaping_returns_and_workflow_end() {
    assert!(
        BodyDefinition::new(
            id("line"),
            id("done"),
            vec![Node::new(id("done"), NodeKind::End { output: None })]
        )
        .is_err()
    );
    assert!(
        BodyDefinition::new(
            id("line"),
            id("return"),
            vec![returned().with_next(id("outside"))]
        )
        .is_err()
    );
    let choose = Node::new(
        id("choose"),
        NodeKind::Condition {
            expression: expr("true"),
            if_true: id("return"),
            if_false: id("other"),
        },
    );
    let other = Node::new(
        id("other"),
        NodeKind::Return {
            result: expr("null"),
        },
    );
    assert!(
        BodyDefinition::new(id("line"), id("choose"), vec![choose, returned(), other]).is_err()
    );
    let choose = Node::new(
        id("choose"),
        NodeKind::Condition {
            expression: expr("true"),
            if_true: id("return"),
            if_false: id("dead"),
        },
    );
    let dead = Node::new(
        id("dead"),
        NodeKind::Timer {
            delay: std::time::Duration::from_secs(1),
        },
    );
    assert!(BodyDefinition::new(id("line"), id("choose"), vec![choose, returned(), dead]).is_err());
}

fn nested_body(name: &str, child: &str) -> BodyDefinition {
    let nested = Node::new(
        id("items"),
        NodeKind::ForEach {
            items: expr("input.items"),
            body: id(child),
            result: expr("bodyResult"),
            max_items: 2,
            max_concurrency: 1,
        },
    )
    .with_next(id("return"));
    BodyDefinition::new(id(name), id("items"), vec![nested, returned()])
        .expect("closed local graph")
}

#[test]
fn body_reference_graph_rejects_missing_duplicate_unused_and_recursive_bodies() {
    assert!(build(root_nodes("missing"), vec![], WorkflowLimits::default()).is_err());
    assert!(
        build(
            root_nodes("line"),
            vec![simple_body("line"), simple_body("line")],
            WorkflowLimits::default()
        )
        .is_err()
    );
    assert!(
        build(
            root_nodes("line"),
            vec![simple_body("line"), simple_body("unused")],
            WorkflowLimits::default()
        )
        .is_err()
    );
    assert!(
        build(
            root_nodes("line"),
            vec![nested_body("line", "line")],
            WorkflowLimits::default()
        )
        .is_err()
    );
    assert!(
        build(
            root_nodes("first"),
            vec![
                nested_body("first", "second"),
                nested_body("second", "first")
            ],
            WorkflowLimits::default()
        )
        .is_err()
    );
}

#[test]
fn shared_limits_bound_depth_and_total_definition_nodes() {
    assert!(WorkflowLimits::new(0, 1).is_err());
    assert!(WorkflowLimits::new(100_001, 1).is_err());
    assert!(WorkflowLimits::new(1, 0).is_err());
    assert!(WorkflowLimits::new(1, 9).is_err());
    let bodies = vec![nested_body("outer", "inner"), simple_body("inner")];
    assert!(
        build(
            root_nodes("outer"),
            bodies.clone(),
            WorkflowLimits::new(100_000, 1).expect("limits")
        )
        .is_err()
    );
    assert!(
        build(
            root_nodes("outer"),
            bodies,
            WorkflowLimits::new(100_000, 2).expect("limits")
        )
        .is_ok()
    );
    let mut nodes = Vec::new();
    for index in 0..9998 {
        nodes.push(
            Node::new(
                id(&format!("step{index}")),
                NodeKind::Timer {
                    delay: std::time::Duration::from_millis(1),
                },
            )
            .with_next(id(&format!("step{}", index + 1))),
        );
    }
    nodes.push(Node::new(
        id("step9998"),
        NodeKind::Return {
            result: expr("item"),
        },
    ));
    let large =
        BodyDefinition::new(id("large"), id("step0"), nodes).expect("body within local bound");
    assert!(build(root_nodes("large"), vec![large], WorkflowLimits::default()).is_err());
}

#[test]
fn collection_limits_and_effective_expression_bytes_are_validated() {
    for (items, concurrency) in [(0, 1), (10001, 1), (1, 0), (1, 257)] {
        let mut nodes = root_nodes("line");
        nodes[0] = Node::new(
            id("items"),
            NodeKind::ForEach {
                items: expr("input.items"),
                body: id("line"),
                result: expr("bodyResult"),
                max_items: items,
                max_concurrency: concurrency,
            },
        )
        .with_next(id("done"));
        assert!(build(nodes, vec![simple_body("line")], WorkflowLimits::default()).is_err());
    }
    let oversized =
        Expression::parse(&"é".repeat(513)).expect("legacy wrapper permits up to 2048 bytes");
    let invalid = Node::new(id("return"), NodeKind::Return { result: oversized });
    assert!(BodyDefinition::new(id("line"), id("return"), vec![invalid]).is_err());
    let mut nodes = root_nodes("line");
    nodes[0] = Node::new(
        id("items"),
        NodeKind::ForEach {
            items: expr("input.items"),
            body: id("line"),
            result: expr("bodyResult"),
            max_items: 10000,
            max_concurrency: 256,
        },
    )
    .with_next(id("done"));
    assert!(build(nodes, vec![simple_body("line")], WorkflowLimits::default()).is_ok());
}
