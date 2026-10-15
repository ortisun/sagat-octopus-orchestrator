//! Loop progress is derived only from explicit journal events.
#![allow(clippy::expect_used)]

use time::OffsetDateTime;

use sagat_domain::execution::{
    AddressedRunEvent, ExecutionError, ExecutionInstanceId, FailureCode, InstancePath,
    LoopContinuation, LoopDrainReason, LoopKind, LoopOutcome, LoopPhase, RunEvent, RunId, RunState,
    StepStatus, lease_instance,
};
use sagat_domain::workflow::{Digest, NodeId};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("valid timestamp")
}

fn loop_instance() -> ExecutionInstanceId {
    ExecutionInstanceId::root(node("poll"))
}

fn body(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("poll"), index)
            .expect("bounded path"),
        node("check"),
    )
}

fn start(kind: LoopKind, max_iterations: u32, initial_condition: Option<bool>) -> RunState {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: node("poll"),
        })
        .expect("run starts");
    state
        .apply(&RunEvent::LoopEntered {
            node: node("poll"),
            kind,
            max_iterations,
            entered_at: at(100),
            deadline: at(1_000),
            initial_carry_digest: Digest::new([0x10; 32]),
            initial_condition,
        })
        .expect("loop enters");
    state
}

fn admit(state: &mut RunState, index: u32) {
    state
        .apply(&RunEvent::LoopIterationAdmitted {
            node: node("poll"),
            index,
            entry: body(index),
        })
        .expect("iteration admitted");
}

fn return_body(state: &mut RunState, index: u32) {
    state
        .apply(&RunEvent::LoopIterationReturned {
            node: node("poll"),
            index,
            return_instance: body(index),
        })
        .expect("body returned");
}

fn checkpoint(state: &mut RunState, index: u32, condition: bool, continuation: LoopContinuation) {
    state
        .apply(&RunEvent::LoopIterationCheckpointed {
            node: node("poll"),
            index,
            return_instance: body(index),
            result_digest: Digest::new([0x20 + index as u8; 32]),
            carry_digest: Digest::new([0x30 + index as u8; 32]),
            condition,
            checked_at: at(200 + i64::from(index)),
            continuation,
        })
        .expect("iteration checkpointed");
}

#[test]
fn while_false_at_entry_completes_without_a_body() {
    let state = start(LoopKind::While, 3, Some(false));
    let scope = state.loop_state(&loop_instance()).expect("loop state");

    assert_eq!(scope.phase(), LoopPhase::Succeeded);
    assert_eq!(scope.next_iteration(), 0);
    assert_eq!(scope.current_carry_digest(), Digest::new([0x10; 32]));
    assert_eq!(
        state.step(&node("poll")).map(|step| step.status()),
        Some(StepStatus::Succeeded)
    );
}

#[test]
fn repeat_until_executes_once_before_its_first_condition() {
    let mut state = start(LoopKind::RepeatUntil, 3, None);
    assert_eq!(
        state
            .loop_state(&loop_instance())
            .map(|scope| scope.phase()),
        Some(LoopPhase::Ready)
    );

    admit(&mut state, 0);
    return_body(&mut state, 0);
    checkpoint(&mut state, 0, true, LoopContinuation::Complete);

    let scope = state.loop_state(&loop_instance()).expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Succeeded);
    assert_eq!(scope.next_iteration(), 1);
    assert_eq!(scope.current_carry_digest(), Digest::new([0x30; 32]));
}

#[test]
fn checkpoints_replay_carry_and_delay_without_reevaluating() {
    let mut state = start(LoopKind::While, 3, Some(true));

    admit(&mut state, 0);
    return_body(&mut state, 0);
    checkpoint(&mut state, 0, true, LoopContinuation::Continue);
    admit(&mut state, 1);
    return_body(&mut state, 1);
    checkpoint(&mut state, 1, true, LoopContinuation::DelayUntil(at(400)));

    let after_second_commit = state.clone();
    let scope = after_second_commit
        .loop_state(&loop_instance())
        .expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Delaying);
    assert_eq!(scope.next_iteration(), 2);
    assert_eq!(scope.current_carry_digest(), Digest::new([0x31; 32]));
    assert!(scope.checkpoint(0).expect("first").condition());
    assert_eq!(
        scope.checkpoint(1).expect("second").result_digest(),
        Digest::new([0x21; 32])
    );

    state
        .apply(&RunEvent::LoopDelayElapsed {
            node: node("poll"),
            next_iteration: 2,
        })
        .expect("delay elapses");
    admit(&mut state, 2);
    return_body(&mut state, 2);
    checkpoint(&mut state, 2, false, LoopContinuation::Complete);

    assert_eq!(
        state
            .loop_state(&loop_instance())
            .map(|scope| scope.phase()),
        Some(LoopPhase::Succeeded)
    );
}

#[test]
fn restart_rebuilds_the_last_committed_checkpoint_from_events() {
    let run = RunId::generate();
    let events = vec![
        RunEvent::RunStarted {
            entrypoint: node("poll"),
        },
        RunEvent::LoopEntered {
            node: node("poll"),
            kind: LoopKind::While,
            max_iterations: 3,
            entered_at: at(100),
            deadline: at(1_000),
            initial_carry_digest: Digest::new([0x10; 32]),
            initial_condition: Some(true),
        },
        RunEvent::LoopIterationAdmitted {
            node: node("poll"),
            index: 0,
            entry: body(0),
        },
        RunEvent::LoopIterationReturned {
            node: node("poll"),
            index: 0,
            return_instance: body(0),
        },
        RunEvent::LoopIterationCheckpointed {
            node: node("poll"),
            index: 0,
            return_instance: body(0),
            result_digest: Digest::new([0x20; 32]),
            carry_digest: Digest::new([0x30; 32]),
            condition: true,
            checked_at: at(200),
            continuation: LoopContinuation::Continue,
        },
    ];

    let state = RunState::replay(run, &events).expect("journal replays");
    let scope = state.loop_state(&loop_instance()).expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Ready);
    assert_eq!(scope.next_iteration(), 1);
    assert_eq!(scope.current_carry_digest(), Digest::new([0x30; 32]));
    assert!(scope.checkpoint(0).expect("checkpoint").condition());
}

#[test]
fn budget_exhaustion_fails_before_a_delay_can_be_scheduled() {
    let mut state = start(LoopKind::While, 1, Some(true));
    admit(&mut state, 0);
    return_body(&mut state, 0);
    let before = state.clone();

    let invalid = RunEvent::LoopIterationCheckpointed {
        node: node("poll"),
        index: 0,
        return_instance: body(0),
        result_digest: Digest::new([0x20; 32]),
        carry_digest: Digest::new([0x30; 32]),
        condition: true,
        checked_at: at(200),
        continuation: LoopContinuation::DelayUntil(at(400)),
    };
    assert_eq!(
        state.apply(&invalid),
        Err(ExecutionError::InvalidLoopContinuation {
            loop_instance: loop_instance(),
            index: 0,
        })
    );
    assert_eq!(state, before);

    checkpoint(&mut state, 0, true, LoopContinuation::IterationLimit);
    let scope = state.loop_state(&loop_instance()).expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Failed);
    assert_eq!(scope.failure(), Some(&FailureCode::iteration_limit()));
    assert_eq!(
        state.step(&node("poll")).and_then(|step| step.failure()),
        Some(&FailureCode::iteration_limit())
    );
}

#[test]
fn cancellation_stops_admission_and_finishes_after_active_work_drains() {
    let mut state = start(LoopKind::RepeatUntil, 3, None);
    admit(&mut state, 0);

    state
        .apply(&RunEvent::LoopDrainStarted {
            node: node("poll"),
            reason: LoopDrainReason::Cancelled,
        })
        .expect("drain starts");
    assert_eq!(
        state.apply(&RunEvent::LoopIterationAdmitted {
            node: node("poll"),
            index: 1,
            entry: body(1),
        }),
        Err(ExecutionError::LoopNotReady {
            loop_instance: loop_instance(),
            phase: LoopPhase::Draining,
        })
    );
    state
        .apply(&RunEvent::LoopIterationDrained {
            node: node("poll"),
            index: 0,
        })
        .expect("active iteration drains");
    state
        .apply(&RunEvent::LoopFinished {
            node: node("poll"),
            outcome: LoopOutcome::Cancelled,
        })
        .expect("loop cancellation finishes");

    assert_eq!(
        state
            .loop_state(&loop_instance())
            .map(|scope| scope.phase()),
        Some(LoopPhase::Cancelled)
    );
    assert_eq!(
        state.step(&node("poll")).map(|step| step.status()),
        Some(StepStatus::Cancelled)
    );
}

#[test]
fn pre_and_post_test_entry_forms_are_not_interchangeable() {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: node("poll"),
        })
        .expect("run starts");
    let before = state.clone();

    let invalid = RunEvent::LoopEntered {
        node: node("poll"),
        kind: LoopKind::RepeatUntil,
        max_iterations: 3,
        entered_at: at(100),
        deadline: at(1_000),
        initial_carry_digest: Digest::new([0x10; 32]),
        initial_condition: Some(false),
    };
    assert_eq!(
        state.apply(&invalid),
        Err(ExecutionError::InvalidLoopEntry(loop_instance()))
    );
    assert_eq!(state, before);
}

#[test]
fn a_failed_body_drains_and_retains_its_failure_code() {
    let mut state = start(LoopKind::While, 3, Some(true));
    admit(&mut state, 0);
    state
        .apply(&RunEvent::LoopDrainStarted {
            node: node("poll"),
            reason: LoopDrainReason::Failed {
                iteration: 0,
                code: FailureCode::worker_error(),
            },
        })
        .expect("failed body begins drain");
    state
        .apply(&RunEvent::LoopIterationDrained {
            node: node("poll"),
            index: 0,
        })
        .expect("failed body drains");
    state
        .apply(&RunEvent::LoopFinished {
            node: node("poll"),
            outcome: LoopOutcome::Failed(FailureCode::worker_error()),
        })
        .expect("loop fails");

    let scope = state.loop_state(&loop_instance()).expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Failed);
    assert_eq!(scope.failure(), Some(&FailureCode::worker_error()));
}

#[test]
fn drain_refuses_to_hide_an_iteration_with_an_active_lease() {
    let mut state = start(LoopKind::RepeatUntil, 3, None);
    admit(&mut state, 0);
    state
        .apply_addressed(
            &AddressedRunEvent::in_path(
                body(0).path().clone(),
                RunEvent::StepReleased {
                    node: node("check"),
                },
            )
            .expect("scoped release"),
        )
        .expect("body becomes ready");
    lease_instance(&mut state, &body(0)).expect("attempt receives a lease");
    state
        .apply(&RunEvent::LoopDrainStarted {
            node: node("poll"),
            reason: LoopDrainReason::Cancelled,
        })
        .expect("drain starts");
    let before = state.clone();

    assert_eq!(
        state.apply(&RunEvent::LoopIterationDrained {
            node: node("poll"),
            index: 0,
        }),
        Err(ExecutionError::IllegalLoopTransition {
            loop_instance: loop_instance(),
            event: "loop.iteration_drained",
        })
    );
    assert_eq!(state, before);
}

#[test]
fn a_checkpoint_after_the_deadline_loses_the_race_without_mutation() {
    let mut state = start(LoopKind::While, 3, Some(true));
    admit(&mut state, 0);
    return_body(&mut state, 0);
    let before = state.clone();

    let late = RunEvent::LoopIterationCheckpointed {
        node: node("poll"),
        index: 0,
        return_instance: body(0),
        result_digest: Digest::new([0x20; 32]),
        carry_digest: Digest::new([0x30; 32]),
        condition: false,
        checked_at: at(1_001),
        continuation: LoopContinuation::Complete,
    };
    assert_eq!(
        state.apply(&late),
        Err(ExecutionError::IllegalLoopTransition {
            loop_instance: loop_instance(),
            event: "loop.iteration_checkpointed",
        })
    );
    assert_eq!(state, before);
}
