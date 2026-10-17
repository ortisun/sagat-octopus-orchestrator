#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::{Harness, activity, build, id};
use sagat_domain::execution::{Evaluation, FailureCode, Resolution, RunStatus, SwitchSelection};
use sagat_domain::workflow::{Expression, Node, NodeKind, SwitchCase};

fn definition() -> sagat_domain::workflow::WorkflowDefinition {
    build(
        "choose",
        vec![
            Node::new(
                id("choose"),
                NodeKind::Switch {
                    cases: vec![
                        SwitchCase::new(
                            id("first_case"),
                            Expression::parse("true").unwrap(),
                            id("first"),
                        ),
                        SwitchCase::new(
                            id("second_case"),
                            Expression::parse("true").unwrap(),
                            id("second"),
                        ),
                    ],
                    default: id("fallback"),
                },
            ),
            activity("first"),
            activity("second"),
            activity("fallback"),
        ],
    )
}

#[test]
fn one_recorded_case_routes_once_and_replays_without_reevaluation() {
    let mut run = Harness::start(definition());
    let Some(Evaluation::Switch { cases, .. }) = &run.pending else {
        panic!("switch awaits ordered predicate evaluation");
    };
    assert_eq!(
        cases
            .iter()
            .map(|case| case.id().as_str())
            .collect::<Vec<_>>(),
        ["first_case", "second_case"]
    );
    run.resolve(Resolution::Switch {
        node: id("choose"),
        selection: Ok(SwitchSelection::Case(id("first_case"))),
    })
    .expect("case resolution");
    assert!(run.has_step("first"));
    assert!(!run.has_step("second"));
    assert!(!run.has_step("fallback"));
    run.assert_replays();
}

#[test]
fn no_matching_case_uses_the_recorded_default() {
    let mut run = Harness::start(definition());
    run.resolve(Resolution::Switch {
        node: id("choose"),
        selection: Ok(SwitchSelection::Default),
    })
    .expect("default resolution");
    assert!(run.has_step("fallback"));
    assert!(!run.has_step("first"));
    assert!(!run.has_step("second"));
    run.assert_replays();
}

#[test]
fn predicate_error_fails_instead_of_trying_another_case() {
    let mut run = Harness::start(definition());
    run.resolve(Resolution::Switch {
        node: id("choose"),
        selection: Err(()),
    })
    .expect("error resolution");
    assert_eq!(run.state.status(), RunStatus::Failed);
    assert_eq!(
        run.state.stop_code().map(FailureCode::as_str),
        Some("EXPRESSION_ERROR")
    );
    assert!(!run.has_step("first"));
    assert!(!run.has_step("second"));
    assert!(!run.has_step("fallback"));
}
