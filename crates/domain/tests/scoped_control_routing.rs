//! Recorded local control choices route identically in full replay and a bounded path.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    AddressedPlan, AddressedRunEvent, ExecutionInstanceId, InstancePath, PathExecutionState,
    RunEvent, RunId, RunState, StepStatus, SwitchSelection, plan_addressed, plan_path,
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
