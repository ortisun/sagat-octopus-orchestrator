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

fn state_with_local_collection(iterations: u32) -> RunState {
    let mut state = RunState::new(RunId::generate());
    let owner_path = path(0);
    for event in [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: id("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: id("items"),
            item_count: 1,
            max_items: 10_000,
            max_concurrency: 256,
            snapshot_digest: sagat_domain::workflow::Digest::new([1; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: id("items"),
            index: 0,
            entry: ExecutionInstanceId::in_path(owner_path.clone(), id("inner")),
            item_digest: sagat_domain::workflow::Digest::new([2; 32]),
        }),
    ] {
        state.apply_addressed(&event).expect("outer scope event");
    }
    state
        .apply_addressed(
            &AddressedRunEvent::in_path(
                owner_path.clone(),
                RunEvent::CollectionEntered {
                    node: id("inner"),
                    item_count: iterations,
                    max_items: 10_000,
                    max_concurrency: 256,
                    snapshot_digest: sagat_domain::workflow::Digest::new([3; 32]),
                },
            )
            .expect("nested collection address"),
        )
        .expect("nested collection entered");
    for index in 0..iterations {
        let nested_path = owner_path
            .child(id("inner"), index)
            .expect("nested member path");
        state
            .apply_addressed(
                &AddressedRunEvent::in_path(
                    owner_path.clone(),
                    RunEvent::CollectionIterationAdmitted {
                        node: id("inner"),
                        index,
                        entry: ExecutionInstanceId::in_path(nested_path, id("work")),
                        item_digest: sagat_domain::workflow::Digest::new([4; 32]),
                    },
                )
                .expect("nested iteration address"),
            )
            .expect("nested iteration admitted");
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
fn local_collection_is_a_scalar_projection_without_member_history() {
    let full = state_with_local_collection(256);
    let local = PathExecutionState::from_run(&full, path(0)).expect("complete local snapshot");
    let collection = local.local_collection(&id("inner")).expect("local scope");

    assert_eq!(collection.admitted_count(), 256);
    assert_eq!(collection.completed_count(), 0);
    assert_eq!(collection.active_count(), 256);
    assert_eq!(local.local_instances().count(), 1);
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
