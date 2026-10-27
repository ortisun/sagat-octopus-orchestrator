//! Recorded timer lifecycle is local to an exact member and identical in both replay models.
#![allow(clippy::expect_used)]

use sagat_domain::{
    execution::{
        AddressedPlan, AddressedRunEvent, ExecutionInstanceId, InstancePath, PathExecutionState,
        RunEvent, RunId, RunState, StepStatus, WaitingOn, fire_timer_instance,
        fire_timer_path_instance, plan_addressed, plan_path, release_pending_instance_path,
        release_pending_path, start_timer_instance, start_timer_path_instance,
    },
    identity::Slug,
    workflow::{
        BodyDefinition, Digest, Expression, Node, NodeId, NodeKind, WorkflowDefinition,
        WorkflowLimits,
    },
};
use std::time::Duration;
use time::OffsetDateTime;

fn id(s: &str) -> NodeId {
    NodeId::parse(s).expect("node")
}

#[test]
fn clocked_timer_commands_match_and_early_fire_leaves_both_models_unchanged() {
    let definition = definition();
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let now = OffsetDateTime::UNIX_EPOCH;
    let started =
        start_timer_instance(&definition, &mut full, &timer(0), now).expect("clocked full start");
    assert_eq!(
        start_timer_path_instance(&definition, &mut bounded, &timer(0), now)
            .expect("clocked bounded start"),
        started
    );
    let before_full = full.clone();
    let before_bounded = bounded.clone();
    let early = now + time::Duration::seconds(59);
    assert!(matches!(
        fire_timer_instance(&definition, &mut full, &timer(0), early),
        Err(sagat_domain::execution::ExecutionError::NotYetDue(_))
    ));
    assert!(matches!(
        fire_timer_path_instance(&definition, &mut bounded, &timer(0), early),
        Err(sagat_domain::execution::ExecutionError::NotYetDue(_))
    ));
    assert_eq!(full, before_full);
    assert_eq!(bounded, before_bounded);
    let due = now + time::Duration::seconds(60);
    let fired = fire_timer_instance(&definition, &mut full, &timer(0), due).expect("due full fire");
    assert_eq!(
        fire_timer_path_instance(&definition, &mut bounded, &timer(0), due)
            .expect("due bounded fire"),
        fired
    );
    assert_eq!(fired.events.len(), 2, "fire and route share the transition");
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

#[test]
fn a_paused_timer_records_its_due_fire_but_routes_only_after_resume() {
    let definition = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let mut full = state();
    full.apply(&RunEvent::RunPaused)
        .expect("pause before start");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("paused path");
    let before = full.clone();
    let before_bounded = bounded.clone();
    assert!(start_timer_instance(&definition, &mut full, &timer(0), now).is_err());
    assert!(start_timer_path_instance(&definition, &mut bounded, &timer(0), now).is_err());
    assert_eq!(full, before);
    assert_eq!(bounded, before_bounded);
    full.apply(&RunEvent::RunResumed).expect("resume start");
    start_timer_instance(&definition, &mut full, &timer(0), now).expect("start");
    full.apply(&RunEvent::RunPaused)
        .expect("pause accepted timer");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("paused waiting path");
    let due = now + time::Duration::seconds(60);
    let fired =
        fire_timer_instance(&definition, &mut full, &timer(0), due).expect("accepted due timer");
    assert_eq!(
        fire_timer_path_instance(&definition, &mut bounded, &timer(0), due)
            .expect("bounded paused fire"),
        fired
    );
    assert_eq!(
        fired.events,
        vec![event(RunEvent::TimerFired { node: id("delay") })]
    );
    assert!(!full.instance(&timer(0)).expect("fired timer").routed());
    full.apply(&RunEvent::RunResumed).expect("resume routing");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("resumed path");
    let routes = release_pending_instance_path(&definition, &mut full, &path(0))
        .expect("full local settlement");
    assert_eq!(
        release_pending_path(&definition, &mut bounded).expect("bounded local settlement"),
        routes
    );
    assert_eq!(
        routes.events,
        vec![event(RunEvent::StepRouted {
            node: id("delay"),
            activated: vec![id("returned")]
        })]
    );
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

#[test]
fn timer_clock_overflow_and_stopped_ancestors_refuse_before_any_state_change() {
    let definition = definition();
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let original = full.clone();
    let original_bounded = bounded.clone();
    assert!(
        start_timer_instance(
            &definition,
            &mut full,
            &timer(0),
            time::PrimitiveDateTime::MAX.assume_utc()
        )
        .is_err()
    );
    assert!(
        start_timer_path_instance(
            &definition,
            &mut bounded,
            &timer(0),
            time::PrimitiveDateTime::MAX.assume_utc()
        )
        .is_err()
    );
    assert_eq!(full, original);
    assert_eq!(bounded, original_bounded);
    for global_stop in [true, false] {
        let mut full = state();
        start_timer_instance(
            &definition,
            &mut full,
            &timer(0),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("accepted timer");
        full.apply(&if global_stop {
            RunEvent::RunStopRequested {
                reason: sagat_domain::execution::StopReason::Cancelled,
                code: None,
            }
        } else {
            RunEvent::CollectionDrainStarted {
                node: id("lines"),
                reason: sagat_domain::execution::CollectionDrainReason::Cancelled,
            }
        })
        .expect("stop fence");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("stopped path");
        let before = full.clone();
        let before_bounded = bounded.clone();
        let due = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60);
        assert!(fire_timer_instance(&definition, &mut full, &timer(0), due).is_err());
        assert!(fire_timer_path_instance(&definition, &mut bounded, &timer(0), due).is_err());
        assert_eq!(full, before);
        assert_eq!(bounded, before_bounded);
    }
}
fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(id("lines"), index)
        .expect("path")
}
fn timer(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(path(index), id("delay"))
}

fn definition() -> WorkflowDefinition {
    WorkflowDefinition::new_structured(
        Slug::parse("scoped-timer").expect("name"),
        id("lines"),
        vec![
            Node::new(
                id("lines"),
                NodeKind::ForEach {
                    items: Expression::parse("input.lines").expect("items"),
                    body: id("body"),
                    result: Expression::parse("bodyResult").expect("result"),
                    max_items: 2,
                    max_concurrency: 2,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![
            BodyDefinition::new(
                id("body"),
                id("delay"),
                vec![
                    Node::new(
                        id("delay"),
                        NodeKind::Timer {
                            delay: Duration::from_secs(60),
                        },
                    )
                    .with_next(id("returned")),
                    Node::new(
                        id("returned"),
                        NodeKind::Return {
                            result: Expression::parse("item").expect("result"),
                        },
                    ),
                ],
            )
            .expect("closed body"),
        ],
        WorkflowLimits::default(),
    )
    .expect("definition")
}

fn state() -> RunState {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: id("lines"),
        })
        .expect("run");
    state
        .apply(&RunEvent::CollectionEntered {
            node: id("lines"),
            item_count: 2,
            max_items: 2,
            max_concurrency: 2,
            snapshot_digest: Digest::new([1; 32]),
        })
        .expect("scope");
    for index in 0..2 {
        state
            .apply(&RunEvent::CollectionIterationAdmitted {
                node: id("lines"),
                index,
                entry: timer(index),
                item_digest: Digest::new([2; 32]),
            })
            .expect("member");
    }
    state
}

fn event(event: RunEvent) -> AddressedRunEvent {
    AddressedRunEvent::in_path(path(0), event).expect("qualified step event")
}

#[test]
fn pending_scoped_timer_waits_for_its_clocked_scheduler_start_in_both_planners() {
    let state = state();
    let definition = definition();
    assert_eq!(
        plan_addressed(&definition, &state, &path(0)).expect("clocked start owns timer planning"),
        AddressedPlan::Idle
    );
    let bounded = PathExecutionState::from_run(&state, path(0)).expect("path");
    assert_eq!(
        plan_path(&definition, &bounded).expect("same bounded planning"),
        AddressedPlan::Idle
    );
}

#[test]
fn recorded_scoped_timer_start_and_fire_preserve_the_sibling_and_route_the_return() {
    let mut full = state();
    let definition = definition();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let due_at = OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1);
    let started = event(RunEvent::TimerStarted {
        node: id("delay"),
        due_at,
    });
    full.apply_addressed(&started).expect("full timer start");
    bounded
        .apply_addressed(&started)
        .expect("bounded timer start");
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
    let step = full.instance(&timer(0)).expect("timer");
    assert_eq!(step.status(), StepStatus::Waiting);
    assert_eq!(step.waiting_on(), Some(&WaitingOn::Timer { due_at }));
    assert_eq!((step.attempt(), step.generation()), (0, 0));
    assert_eq!(
        full.instance(&timer(1)).expect("sibling").status(),
        StepStatus::Pending
    );
    let fired = event(RunEvent::TimerFired { node: id("delay") });
    full.apply_addressed(&fired).expect("full timer fire");
    bounded.apply_addressed(&fired).expect("bounded timer fire");
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
    let route = plan_addressed(&definition, &full, &path(0)).expect("route");
    assert_eq!(
        plan_path(&definition, &bounded).expect("bounded route"),
        route
    );
    assert_eq!(
        route,
        AddressedPlan::Emit(event(RunEvent::StepRouted {
            node: id("delay"),
            activated: vec![id("returned")]
        }))
    );
}

#[test]
fn a_cancelled_scoped_timer_cannot_fire_and_a_fired_timer_cannot_fire_twice() {
    for cancelled in [true, false] {
        let mut full = state();
        full.apply_addressed(&event(RunEvent::TimerStarted {
            node: id("delay"),
            due_at: OffsetDateTime::UNIX_EPOCH,
        }))
        .expect("start");
        full.apply_addressed(&event(if cancelled {
            RunEvent::StepCancelled { node: id("delay") }
        } else {
            RunEvent::TimerFired { node: id("delay") }
        }))
        .expect("settled timer");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let old_full = full.clone();
        let old_bounded = bounded.clone();
        let duplicate = event(RunEvent::TimerFired { node: id("delay") });
        assert!(full.apply_addressed(&duplicate).is_err());
        assert!(bounded.apply_addressed(&duplicate).is_err());
        assert_eq!(full, old_full);
        assert_eq!(bounded, old_bounded);
    }
}

proptest::proptest! {
    #[test]
    fn timer_deadline_replay_is_local_for_every_member_and_clock_offset(
        index in 0_u32..2,
        seconds in -1_000_000_i64..1_000_000,
    ) {
        let mut full = state();
        let sibling = full.instance(&timer(1-index)).expect("sibling").clone();
        let mut bounded = PathExecutionState::from_run(&full,path(index)).expect("path");
        let due_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds);
        for event in [RunEvent::TimerStarted { node:id("delay"),due_at },RunEvent::TimerFired { node:id("delay") }] {
            let addressed = AddressedRunEvent::in_path(path(index),event).expect("address");
            full.apply_addressed(&addressed).expect("full recorded lifecycle");
            bounded.apply_addressed(&addressed).expect("bounded recorded lifecycle");
            proptest::prop_assert_eq!(&bounded,&PathExecutionState::from_run(&full,path(index)).expect("oracle"));
            proptest::prop_assert_eq!(full.instance(&timer(1-index)).expect("untouched sibling"),&sibling);
        }
    }
}

#[test]
fn a_recorded_collection_drain_cancels_a_waiting_timer_without_touching_its_sibling() {
    let mut full = state();
    full.apply_addressed(&event(RunEvent::TimerStarted {
        node: id("delay"),
        due_at: OffsetDateTime::UNIX_EPOCH,
    }))
    .expect("timer start");
    let sibling = full.instance(&timer(1)).expect("sibling").clone();
    full.apply(&RunEvent::CollectionDrainStarted {
        node: id("lines"),
        reason: sagat_domain::execution::CollectionDrainReason::Cancelled,
    })
    .expect("drain starts");
    full.apply(&RunEvent::CollectionIterationDrained {
        node: id("lines"),
        index: 0,
    })
    .expect("an unowned timer is safe to drain");
    let cancelled = full.instance(&timer(0)).expect("timer");
    assert_eq!(cancelled.status(), StepStatus::Cancelled);
    assert!(cancelled.routed());
    assert!(cancelled.waiting_on().is_none());
    assert_eq!(full.instance(&timer(1)).expect("sibling"), &sibling);
}
