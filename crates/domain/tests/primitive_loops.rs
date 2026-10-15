//! Loop progress is derived only from explicit journal events.
#![allow(clippy::expect_used)]

use time::OffsetDateTime;

use sagat_domain::execution::{
    AddressedRunEvent, ExecutionError, ExecutionInstanceId, FailureCode, InstancePath,
    LoopContinuation, LoopDrainReason, LoopKind, LoopOutcome, LoopPhase, Plan, RunEvent, RunId,
    RunState, StepStatus, StopReason, lease_instance, plan, start_run,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Digest, Expression, Node, NodeId, NodeKind, RetryPolicy,
    WorkflowDefinition, WorkflowLimits,
};

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

fn definition(kind: LoopKind) -> WorkflowDefinition {
    let body = BodyDefinition::new(
        node("iteration"),
        node("check"),
        vec![
            Node::new(
                node("check"),
                NodeKind::Activity {
                    activity: ActivityName::parse("loop.check").expect("activity"),
                    retry: RetryPolicy::DEFAULT,
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            )
            .with_next(node("returned")),
            Node::new(
                node("returned"),
                NodeKind::Return {
                    result: Expression::parse("steps.check.output").expect("result"),
                },
            ),
        ],
    )
    .expect("loop body");
    let loop_kind = match kind {
        LoopKind::While => NodeKind::While {
            condition: Expression::parse("carry < 2").expect("condition"),
            body: node("iteration"),
            initial: Expression::parse("0").expect("initial"),
            carry: Expression::parse("bodyResult").expect("carry"),
            max_iterations: 2,
            delay: None,
        },
        LoopKind::RepeatUntil => NodeKind::RepeatUntil {
            condition: Expression::parse("carry >= 2").expect("condition"),
            body: node("iteration"),
            initial: Expression::parse("0").expect("initial"),
            carry: Expression::parse("bodyResult").expect("carry"),
            max_iterations: 2,
            delay: None,
        },
    };
    WorkflowDefinition::new_structured(
        Slug::parse("loop-planner").expect("workflow name"),
        node("poll"),
        vec![
            Node::new(node("poll"), loop_kind)
                .with_timeout(std::time::Duration::from_secs(60))
                .with_next(node("done")),
            Node::new(node("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    )
    .expect("loop workflow")
}

#[test]
fn starting_a_loop_parks_its_owner_for_the_transactional_scheduler() {
    for kind in [LoopKind::While, LoopKind::RepeatUntil] {
        let definition = definition(kind);
        let (state, progress) =
            start_run(&definition, RunId::generate(), OffsetDateTime::UNIX_EPOCH)
                .expect("loop run starts");
        assert_eq!(progress.events.len(), 1);
        assert!(progress.pending.is_none());
        assert_eq!(
            state.step(&node("poll")).map(|step| step.status()),
            Some(StepStatus::Pending)
        );
        assert_eq!(
            plan(&definition, &state, OffsetDateTime::UNIX_EPOCH).expect("loop stays parked"),
            Plan::Idle
        );
    }
}

#[test]
fn cancellation_fails_closed_while_a_loop_scope_is_active() {
    let definition = definition(LoopKind::While);
    let (mut state, _) = start_run(&definition, RunId::generate(), OffsetDateTime::UNIX_EPOCH)
        .expect("loop run starts");
    state
        .apply(&RunEvent::LoopEntered {
            node: node("poll"),
            kind: LoopKind::While,
            max_iterations: 2,
            entered_at: at(100),
            deadline: at(200),
            initial_carry_digest: Digest::new([0x10; 32]),
            initial_condition: Some(true),
        })
        .expect("loop enters");
    state
        .apply(&RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        })
        .expect("cancel requested");
    assert_eq!(
        plan(&definition, &state, at(101)),
        Err(ExecutionError::UnsupportedStructuredNode(node("poll")))
    );
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
fn a_deadline_can_drain_a_suspended_loop_without_an_active_iteration() {
    let mut state = start(LoopKind::While, 3, Some(true));
    admit(&mut state, 0);
    return_body(&mut state, 0);
    checkpoint(&mut state, 0, true, LoopContinuation::DelayUntil(at(400)));

    state
        .apply(&RunEvent::LoopDrainStarted {
            node: node("poll"),
            reason: LoopDrainReason::TimedOut,
        })
        .expect("deadline begins drain without an active body");
    state
        .apply(&RunEvent::LoopFinished {
            node: node("poll"),
            outcome: LoopOutcome::Failed(FailureCode::run_timed_out()),
        })
        .expect("deadline finishes loop");

    let scope = state.loop_state(&loop_instance()).expect("loop state");
    assert_eq!(scope.phase(), LoopPhase::Failed);
    assert_eq!(scope.failure(), Some(&FailureCode::run_timed_out()));
    assert_eq!(scope.active_iteration(), None);
    assert_eq!(scope.delay_until(), None);
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
