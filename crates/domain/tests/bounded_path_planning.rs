//! Bounded path planning is equivalent to the full run oracle without retaining unrelated paths.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    AddressedPlan, AddressedRunEvent, CollectionPhase, ExecutionInstanceId, InstancePath,
    PathExecutionState, RunEvent, RunId, RunState, plan_addressed, plan_path,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Expression, Node, NodeId, NodeKind, RetryPolicy,
    WorkflowDefinition,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}

fn definition() -> WorkflowDefinition {
    let body = BodyDefinition::new(
        id("body"),
        id("work"),
        vec![
            Node::new(
                id("work"),
                NodeKind::Activity {
                    activity: ActivityName::parse("act.work").expect("activity"),
                    retry: RetryPolicy::no_retry(),
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            )
            .with_next(id("return")),
            Node::new(
                id("return"),
                NodeKind::Return {
                    result: Expression::parse("item").expect("result"),
                },
            ),
        ],
    )
    .expect("body");
    WorkflowDefinition::new_structured(
        Slug::parse("bounded-path").expect("workflow"),
        id("items"),
        vec![
            Node::new(
                id("items"),
                NodeKind::ForEach {
                    items: Expression::parse("input.items").expect("items"),
                    body: id("body"),
                    result: Expression::parse("bodyResult").expect("aggregate"),
                    max_items: 10_000,
                    max_concurrency: 256,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        sagat_domain::workflow::WorkflowLimits::default(),
    )
    .expect("definition")
}

fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(id("items"), index)
        .expect("path")
}

fn member(index: u32, node: &str) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(path(index), id(node))
}

fn state(iterations: u32) -> RunState {
    let mut state = RunState::new(RunId::generate());
    for event in [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: id("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: id("items"),
            item_count: iterations,
            max_items: 10_000,
            max_concurrency: 256,
            snapshot_digest: sagat_domain::workflow::Digest::new([1; 32]),
        }),
    ] {
        state.apply_addressed(&event).expect("scope event");
    }
    for index in 0..iterations {
        state
            .apply_addressed(&AddressedRunEvent::root(
                RunEvent::CollectionIterationAdmitted {
                    node: id("items"),
                    index,
                    entry: member(index, "work"),
                    item_digest: sagat_domain::workflow::Digest::new([2; 32]),
                },
            ))
            .expect("iteration admitted");
    }
    state
}

#[test]
fn one_path_snapshot_excludes_every_unrelated_iteration() {
    let full = state(256);
    let local = PathExecutionState::from_run(&full, path(255)).expect("complete local snapshot");

    assert_eq!(local.local_instances().count(), 1);
    assert_eq!(
        local.local_instance(&id("work")),
        full.instance(&member(255, "work"))
    );
    assert_eq!(local.ancestors().len(), 1);
    assert_eq!(
        local.ancestors()[0].collection_phase(),
        Some(CollectionPhase::Open)
    );
    assert_eq!(local.ancestors()[0].iteration_index(), 255);
    assert_eq!(local.global().version(), full.version());
}

#[test]
fn bounded_planner_matches_the_full_run_oracle() {
    let definition = definition();
    let full = state(3);
    let chosen = path(1);
    let local = PathExecutionState::from_run(&full, chosen.clone()).expect("local state");

    assert_eq!(
        plan_path(&definition, &local).expect("bounded plan"),
        plan_addressed(&definition, &full, &chosen).expect("oracle plan")
    );
    assert!(matches!(
        plan_path(&definition, &local),
        Ok(AddressedPlan::Emit(_))
    ));
}

#[test]
fn global_pause_fences_both_bounded_and_oracle_planners() {
    let definition = definition();
    let mut full = state(1);
    full.apply(&RunEvent::RunPaused).expect("pause");
    let chosen = path(0);
    let local = PathExecutionState::from_run(&full, chosen.clone()).expect("local state");

    assert_eq!(plan_path(&definition, &local), Ok(AddressedPlan::Idle));
    assert_eq!(
        plan_addressed(&definition, &full, &chosen),
        Ok(AddressedPlan::Idle)
    );
}
