//! Recorded timer lifecycle is local to an exact member and identical in both replay models.
#![allow(clippy::expect_used)]

use sagat_domain::{
    execution::{
        AddressedPlan, AddressedRunEvent, ExecutionInstanceId, InstancePath, PathExecutionState,
        RunEvent, RunId, RunState, StepStatus, WaitingOn, plan_addressed, plan_path,
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
