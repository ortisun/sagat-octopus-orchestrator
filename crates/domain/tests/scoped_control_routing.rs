//! Recorded local control choices route identically in full replay and a bounded path.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    AddressedPlan, AddressedRunEvent, ExecutionInstanceId, InstancePath, PathExecutionState,
    Resolution, RunEvent, RunId, RunState, StepStatus, SwitchSelection, plan_addressed, plan_path,
    resolve_control_instance, resolve_control_path_instance,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Digest, Expression, Node, NodeId, NodeKind, RetryPolicy,
    SwitchCase, WorkflowDefinition, WorkflowLimits,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}

fn activity(name: &str, next: &str) -> Node {
    Node::new(
        id(name),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("line.{name}")).expect("activity"),
            retry: RetryPolicy::no_retry(),
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
    .with_next(id(next))
}

fn definition(control: NodeKind) -> WorkflowDefinition {
    let body = BodyDefinition::new(
        id("body"),
        id("work"),
        vec![
            activity("work", "choose"),
            Node::new(id("choose"), control),
            activity("accept", "returned"),
            activity("reject", "returned"),
            Node::new(
                id("returned"),
                NodeKind::Return {
                    result: Expression::parse("item").expect("result"),
                },
            ),
        ],
    )
    .expect("closed acyclic body");
    WorkflowDefinition::new_structured(
        Slug::parse("scoped-control").expect("name"),
        id("lines"),
        vec![
            Node::new(
                id("lines"),
                NodeKind::ForEach {
                    items: Expression::parse("input.lines").expect("items"),
                    body: id("body"),
                    result: Expression::parse("bodyResult").expect("aggregate"),
                    max_items: 3,
                    max_concurrency: 3,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    )
    .expect("definition")
}

fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(id("lines"), index)
        .expect("path")
}

fn state() -> RunState {
    let mut full = RunState::new(RunId::generate());
    for event in [
        RunEvent::RunStarted {
            entrypoint: id("lines"),
        },
        RunEvent::CollectionEntered {
            node: id("lines"),
            item_count: 3,
            max_items: 3,
            max_concurrency: 3,
            snapshot_digest: Digest::new([1; 32]),
        },
    ] {
        full.apply(&event).expect("root event");
    }
    for index in 0..3 {
        full.apply(&RunEvent::CollectionIterationAdmitted {
            node: id("lines"),
            index,
            entry: ExecutionInstanceId::in_path(path(index), id("work")),
            item_digest: Digest::new([2; 32]),
        })
        .expect("admitted sibling");
    }
    for event in [
        RunEvent::StepReleased { node: id("work") },
        RunEvent::StepLeased {
            node: id("work"),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepCompleted {
            node: id("work"),
            attempt: 1,
            generation: 1,
        },
        RunEvent::StepRouted {
            node: id("work"),
            activated: vec![id("choose")],
        },
    ] {
        full.apply_addressed(&AddressedRunEvent::in_path(path(1), event).expect("address"))
            .expect("pre-control state");
    }
    full
}

fn check_choice(definition: &WorkflowDefinition, choice: RunEvent, target: &str) {
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded snapshot");
    let addressed = AddressedRunEvent::in_path(path(1), choice).expect("choice address");
    full.apply_addressed(&addressed)
        .expect("full recorded choice");
    bounded
        .apply_addressed(&addressed)
        .expect("bounded recorded choice");
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(1)).expect("oracle")
    );

    let before_full = full.clone();
    let before_bounded = bounded.clone();
    assert!(
        full.apply_addressed(&addressed).is_err(),
        "choice is immutable"
    );
    assert!(bounded.apply_addressed(&addressed).is_err());
    assert_eq!(full, before_full);
    assert_eq!(bounded, before_bounded);

    for expected in [
        RunEvent::StepRouted {
            node: id("choose"),
            activated: vec![id(target)],
        },
        RunEvent::StepReleased { node: id(target) },
    ] {
        let action =
            AddressedPlan::Emit(AddressedRunEvent::in_path(path(1), expected).expect("expected"));
        assert_eq!(
            plan_addressed(definition, &full, &path(1)).expect("full plan"),
            action
        );
        assert_eq!(
            plan_path(definition, &bounded).expect("bounded plan"),
            action
        );
        let AddressedPlan::Emit(event) = action else {
            unreachable!()
        };
        full.apply_addressed(&event).expect("full route");
        bounded.apply_addressed(&event).expect("bounded route");
    }
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(1)).expect("final oracle")
    );
    for index in [0, 2] {
        assert_eq!(
            full.instance(&ExecutionInstanceId::in_path(path(index), id("work")))
                .expect("sibling")
                .status(),
            StepStatus::Pending
        );
        assert!(
            full.instance(&ExecutionInstanceId::in_path(path(index), id(target)))
                .is_none()
        );
    }
}

#[test]
fn a_scoped_condition_routes_only_its_recorded_branch() {
    let definition = definition(NodeKind::Condition {
        expression: Expression::parse("item > 0").expect("predicate"),
        if_true: id("accept"),
        if_false: id("reject"),
    });
    for outcome in [false, true] {
        check_choice(
            &definition,
            RunEvent::ConditionEvaluated {
                node: id("choose"),
                outcome,
            },
            if outcome { "accept" } else { "reject" },
        );
    }
}

#[test]
fn a_scoped_switch_routes_only_its_recorded_case_or_default() {
    let definition = definition(NodeKind::Switch {
        cases: vec![SwitchCase::new(
            id("positive"),
            Expression::parse("item > 0").expect("predicate"),
            id("accept"),
        )],
        default: id("reject"),
    });
    for (selection, target) in [
        (SwitchSelection::Case(id("positive")), "accept"),
        (SwitchSelection::Default, "reject"),
    ] {
        check_choice(
            &definition,
            RunEvent::SwitchEvaluated {
                node: id("choose"),
                selection,
            },
            target,
        );
    }
}

#[test]
fn resolving_a_scoped_control_records_and_releases_the_selected_branch() {
    let definition = definition(NodeKind::Condition {
        expression: Expression::parse("item > 0").expect("predicate"),
        if_true: id("accept"),
        if_false: id("reject"),
    });
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("snapshot");
    let instance = ExecutionInstanceId::in_path(path(1), id("choose"));
    let answer = Resolution::Condition {
        node: id("choose"),
        outcome: Ok(true),
    };
    let expected = resolve_control_instance(&definition, &mut full, &instance, answer.clone())
        .expect("full control command");
    let actual =
        resolve_control_path_instance(&definition, &mut bounded, &instance, answer.clone())
            .expect("bounded control command");
    assert_eq!(actual, expected);
    assert_eq!(
        actual.events.len(),
        3,
        "decision, route, selected activity release"
    );
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(1)).expect("oracle")
    );
    assert_eq!(
        bounded
            .local_instance(&id("accept"))
            .expect("selected")
            .status(),
        StepStatus::Ready
    );
    assert!(bounded.local_instance(&id("reject")).is_none());
    let before_full = full.clone();
    let before_bounded = bounded.clone();
    assert!(resolve_control_instance(&definition, &mut full, &instance, answer.clone()).is_err());
    assert!(resolve_control_path_instance(&definition, &mut bounded, &instance, answer).is_err());
    assert_eq!(full, before_full);
    assert_eq!(bounded, before_bounded);
}

#[test]
fn invalid_or_paused_control_answers_fail_without_a_partial_transition() {
    let definition = definition(NodeKind::Switch {
        cases: vec![SwitchCase::new(
            id("positive"),
            Expression::parse("item > 0").expect("predicate"),
            id("accept"),
        )],
        default: id("reject"),
    });
    let instance = ExecutionInstanceId::in_path(path(1), id("choose"));
    for (paused, answer) in [
        (
            false,
            Resolution::Switch {
                node: id("choose"),
                selection: Ok(SwitchSelection::Case(id("unknown"))),
            },
        ),
        (
            false,
            Resolution::Switch {
                node: id("wrong"),
                selection: Ok(SwitchSelection::Default),
            },
        ),
        (
            false,
            Resolution::Condition {
                node: id("choose"),
                outcome: Ok(true),
            },
        ),
        (
            true,
            Resolution::Switch {
                node: id("choose"),
                selection: Ok(SwitchSelection::Default),
            },
        ),
    ] {
        let mut full = state();
        if paused {
            full.apply(&RunEvent::RunPaused).expect("pause");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("snapshot");
        let before_full = full.clone();
        let before_bounded = bounded.clone();
        assert!(
            resolve_control_instance(&definition, &mut full, &instance, answer.clone()).is_err()
        );
        assert!(
            resolve_control_path_instance(&definition, &mut bounded, &instance, answer).is_err()
        );
        assert_eq!(full, before_full);
        assert_eq!(bounded, before_bounded);
    }
}

#[test]
fn an_expression_failure_records_an_unhandled_local_failure_in_both_models() {
    let definition = definition(NodeKind::Condition {
        expression: Expression::parse("item > 0").expect("predicate"),
        if_true: id("accept"),
        if_false: id("reject"),
    });
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("snapshot");
    let instance = ExecutionInstanceId::in_path(path(1), id("choose"));
    let answer = Resolution::Condition {
        node: id("choose"),
        outcome: Err(()),
    };
    assert_eq!(
        resolve_control_instance(&definition, &mut full, &instance, answer.clone())
            .expect("full expression failure"),
        resolve_control_path_instance(&definition, &mut bounded, &instance, answer)
            .expect("bounded expression failure")
    );
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(1)).expect("oracle")
    );
    let step = bounded
        .local_instance(&id("choose"))
        .expect("failed control");
    assert_eq!(step.status(), StepStatus::Failed);
    assert_eq!(
        step.failure(),
        Some(&sagat_domain::execution::FailureCode::expression_error())
    );
    assert!(bounded.local_instance(&id("accept")).is_none());
    assert!(bounded.local_instance(&id("reject")).is_none());
}

#[test]
fn a_control_answer_cannot_consume_nonpristine_pending_metadata() {
    use sagat_domain::execution::{FailureCode, StepState, WaitingOn};
    use std::collections::BTreeMap;
    use time::OffsetDateTime;
    let definition = definition(NodeKind::Condition {
        expression: Expression::parse("item > 0").expect("predicate"),
        if_true: id("accept"),
        if_false: id("reject"),
    });
    let instance = ExecutionInstanceId::in_path(path(1), id("choose"));
    for malformed in [
        "attempt",
        "generation",
        "cancel",
        "routed",
        "retry",
        "waiting",
        "failure",
        "condition",
        "timeout",
        "switch",
    ] {
        let base = PathExecutionState::from_run(&state(), path(1)).expect("base snapshot");
        let step = StepState::restore(
            StepStatus::Pending,
            u32::from(malformed == "attempt"),
            u64::from(malformed == "generation"),
            malformed == "cancel",
            malformed == "routed",
            (malformed == "retry").then_some(OffsetDateTime::UNIX_EPOCH),
            (malformed == "waiting").then_some(WaitingOn::Reconciliation),
            (malformed == "failure").then_some(FailureCode::expression_error()),
            (malformed == "condition").then_some(true),
            malformed == "timeout",
        )
        .with_restored_switch_selection(
            (malformed == "switch").then_some(SwitchSelection::Default),
        );
        let mut steps = base
            .local_instances()
            .map(|(node, step)| (node.clone(), step.clone()))
            .collect::<BTreeMap<_, _>>();
        steps.insert(id("choose"), step);
        let mut bounded = PathExecutionState::restore(
            base.global().clone(),
            path(1),
            base.ancestors().to_vec(),
            steps,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("exact path");
        let before = bounded.clone();
        let answer = Resolution::Condition {
            node: id("choose"),
            outcome: Ok(true),
        };
        assert!(
            resolve_control_path_instance(&definition, &mut bounded, &instance, answer).is_err(),
            "{malformed}"
        );
        assert_eq!(
            bounded, before,
            "{malformed}: no recorded choice or successor"
        );
    }
}
