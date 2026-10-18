#![allow(clippy::expect_used)]

use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    Expression, Node, NodeId, NodeKind, SwitchCase, WorkflowDefinition, WorkflowError,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node id")
}

fn case(name: &str, predicate: &str, target: &str) -> SwitchCase {
    SwitchCase::new(
        id(name),
        Expression::parse(predicate).expect("predicate"),
        id(target),
    )
}

fn definition(cases: Vec<SwitchCase>) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new(
        Slug::parse("switch-example").expect("workflow name"),
        id("choose"),
        vec![
            Node::new(
                id("choose"),
                NodeKind::Switch {
                    cases,
                    default: id("fallback"),
                },
            ),
            Node::new(id("first"), NodeKind::End { output: None }),
            Node::new(id("second"), NodeKind::End { output: None }),
            Node::new(id("fallback"), NodeKind::End { output: None }),
        ],
    )
}

#[test]
fn switch_targets_are_ordered_and_part_of_the_graph() {
    let workflow = definition(vec![
        case("one", "input.amount > 0", "first"),
        case("two", "input.amount > -1", "second"),
    ])
    .expect("valid switch");
    let choose = workflow.node(&id("choose")).expect("switch node");
    let NodeKind::Switch { cases, default } = choose.kind() else {
        panic!("switch node kind");
    };
    assert_eq!(cases[0].id().as_str(), "one");
    assert_eq!(cases[1].id().as_str(), "two");
    assert_eq!(default.as_str(), "fallback");
}

#[test]
fn duplicate_case_ids_and_an_undefined_target_are_rejected() {
    let duplicate = definition(vec![
        case("one", "true", "first"),
        case("one", "false", "second"),
    ])
    .expect_err("duplicate case id");
    assert!(matches!(duplicate, WorkflowError::InvalidSwitch { .. }));

    let missing =
        definition(vec![case("one", "true", "absent")]).expect_err("case target must exist");
    assert!(matches!(missing, WorkflowError::UnknownTarget { .. }));
}

#[test]
fn a_switch_has_at_least_one_case_and_cannot_declare_an_error_route() {
    assert!(matches!(
        definition(Vec::new()),
        Err(WorkflowError::InvalidSwitch { .. })
    ));
    let switch = Node::new(
        id("choose"),
        NodeKind::Switch {
            cases: vec![case("one", "true", "first")],
            default: id("fallback"),
        },
    )
    .with_on_error(id("second"));
    let invalid = WorkflowDefinition::new(
        Slug::parse("switch-example").expect("workflow name"),
        id("choose"),
        vec![
            switch,
            Node::new(id("first"), NodeKind::End { output: None }),
            Node::new(id("second"), NodeKind::End { output: None }),
            Node::new(id("fallback"), NodeKind::End { output: None }),
        ],
    )
    .expect_err("switch cannot catch predicate errors");
    assert!(matches!(invalid, WorkflowError::InvalidSwitch { .. }));
}

#[test]
fn a_switch_predicate_reports_its_effective_expression_byte_limit() {
    let predicate = format!("'{}'", "é".repeat(600));
    assert!(matches!(
        definition(vec![case("one", &predicate, "first")]),
        Err(WorkflowError::ExpressionByteLimit { node, limit: 1024 })
            if node.as_str() == "choose"
    ));
}
