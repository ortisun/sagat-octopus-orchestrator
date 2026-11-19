//! The readiness gate of feature 022 in the domain (T022-096, FR-022-051, D022-020, invariant 11 of `data-model.md`):
//! a step of a node that declares `concurrency` or `rateLimit` is never `READY` without a grant of each gate it
//! declares, whatever made it ready; the planner asks for the decision once it has nothing else to do; the answer is
//! checked; and the journal alone reaches the state the transactions folded.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{T0, activity, build, flaky, id, sid};
use sagat_domain::execution::{
    self, DeclaredGates, Evaluation, ExecutionError, Failure, FailureCode, GateChange, GateEvent,
    GatePool, GateRef, GateRequest, GateTarget, KeyLabel, Progress, Resolution, RunEvent, RunId,
    RunState, StepStatus, WaitingOn,
};
use sagat_domain::workflow::flow_control::DEFAULT_QUEUE_TIMEOUT;
use sagat_domain::workflow::{
    ConcurrencyPolicy, Expression, FlowControl, FlowGroup, Node, RateLimitPolicy, RatePeriod,
    ResolvedGroup, WorkflowDefinition,
};

fn concurrency() -> ConcurrencyPolicy {
    ConcurrencyPolicy::new(
        FlowGroup::Derived,
        Some(Expression::parse("input.accountId").expect("key")),
        1,
        DEFAULT_QUEUE_TIMEOUT,
    )
    .expect("a policy")
}

fn rate() -> RateLimitPolicy {
    RateLimitPolicy::new(
        FlowGroup::Derived,
        None,
        10,
        RatePeriod::Second,
        DEFAULT_QUEUE_TIMEOUT,
    )
    .expect("a policy")
}

fn gated(node: Node, concurrency_gate: bool, rate_gate: bool) -> Node {
    node.with_flow_control(FlowControl::new(
        concurrency_gate.then(concurrency),
        rate_gate.then(rate),
    ))
    .expect("a gated node")
}

/// `a` (ungated) then `b`, gated as asked; a `b` with an error route fails over to `recover`.
fn two_steps(concurrency_gate: bool, rate_gate: bool, b: Node) -> WorkflowDefinition {
    let recover = b.on_error().is_some();
    let mut nodes = vec![
        activity("a").with_next(id("b")),
        gated(b, concurrency_gate, rate_gate),
    ];
    if recover {
        nodes.push(activity("recover"));
    }
    build("a", nodes)
}

fn gate(node: &str) -> GateRef {
    GateRef {
        pool: GatePool::Ordinary,
        group: ResolvedGroup::parse(&format!("=demo-flow/{node}")).expect("group"),
        key_label: KeyLabel::parse("0123456789ab").expect("label"),
    }
}

fn decision(node: &str, attempt: u32, change: GateChange) -> GateEvent {
    GateEvent {
        target: GateTarget::Step {
            node: id(node),
            attempt,
        },
        gate: gate(node),
        change,
    }
}

const DEADLINE: i64 = 600_000_000;

fn granted() -> GateChange {
    GateChange::RateGranted {
        limit: 10,
        per: RatePeriod::Second,
        granted_at_us: 1_000,
        from_queue: false,
    }
}

fn queued() -> GateChange {
    GateChange::ConcurrencyQueued {
        queue_deadline_us: DEADLINE,
    }
}

fn rate_queued() -> GateChange {
    GateChange::RateQueued {
        limit: 10,
        per: RatePeriod::Second,
        queue_deadline_us: DEADLINE,
    }
}

/// Starts a run of `def` and completes `a`: the readiness of `b` comes from a worker completion.
fn after_a(def: &WorkflowDefinition) -> (RunState, Vec<RunEvent>, Progress) {
    let (mut state, started) = execution::start_run(def, RunId::generate(), T0).expect("started");
    let mut journal = started.events;
    let (fence, leased) = execution::lease_step(&mut state, &sid("a")).expect("a leased");
    journal.push(leased);
    let done = execution::complete_step(def, &mut state, &sid("a"), fence, T0).expect("a done");
    journal.extend(done.events.iter().cloned());
    (state, journal, done)
}

fn resolve(
    def: &WorkflowDefinition,
    state: &mut RunState,
    journal: &mut Vec<RunEvent>,
    decisions: Vec<GateEvent>,
    refused: Vec<(sagat_domain::workflow::NodeId, FailureCode)>,
) -> Result<Progress, ExecutionError> {
    let progress = execution::resolve(def, state, Resolution::Gates { decisions, refused }, T0)?;
    journal.extend(progress.events.iter().cloned());
    Ok(progress)
}

/// The journal alone, without the gated nodes, reaches the state the transactions folded (verify_run's check).
fn journal_alone_agrees(def: &WorkflowDefinition, state: &RunState, journal: &[RunEvent]) {
    let alone = RunState::replay(state.run_id(), journal)
        .expect("the journal replays")
        .with_gated_steps(def);
    assert_eq!(&alone, state, "the journal alone reaches the folded state");
    assert_eq!(state.flow_control_breach(), None, "invariant 11 holds");
}

#[test]
fn a_gated_step_made_ready_by_a_completion_waits_and_the_planner_asks_for_its_gates() {
    let def = two_steps(true, false, activity("b"));
    let (state, journal, done) = after_a(&def);
    let b = state.step(&sid("b")).expect("b");
    assert_eq!(
        b.status(),
        StepStatus::Waiting,
        "never READY before a grant"
    );
    assert_eq!(b.waiting_on(), Some(&WaitingOn::Gate));
    assert!(
        journal
            .iter()
            .any(|event| matches!(event, RunEvent::StepReleased { node } if node == &sid("b"))),
        "the step was released"
    );
    assert_eq!(
        done.pending,
        Some(Evaluation::Gates {
            steps: vec![GateRequest {
                node: id("b"),
                attempt: 1
            }]
        })
    );
    assert_eq!(
        state.declared_gates(&id("b")),
        Some(DeclaredGates {
            concurrency: true,
            rate: false
        })
    );
    assert_eq!(state.declared_gates(&id("a")), None);
}

#[test]
fn a_grant_of_each_declared_gate_makes_the_step_ready_and_one_alone_does_not() {
    let def = two_steps(true, true, activity("b"));
    let (mut state, mut journal, _) = after_a(&def);
    // the rate slot alone leaves it waiting for its permit: the reducer knows both are declared
    let mut partial = state.clone();
    partial
        .apply(&RunEvent::Gate(decision("b", 1, granted())))
        .expect("slot");
    assert_eq!(
        partial.step(&sid("b")).expect("b").status(),
        StepStatus::Waiting
    );
    let progress = resolve(
        &def,
        &mut state,
        &mut journal,
        vec![
            decision("b", 1, granted()),
            decision("b", 1, GateChange::ConcurrencyAcquired),
        ],
        Vec::new(),
    )
    .expect("granted");
    assert_eq!(progress.pending, None);
    assert_eq!(
        state.step(&sid("b")).expect("b").status(),
        StepStatus::Ready
    );
    let (_, leased) = execution::lease_step(&mut state, &sid("b")).expect("leased once granted");
    journal.push(leased);
    journal_alone_agrees(&def, &state, &journal);
}

#[test]
fn an_answer_that_leaves_a_declared_gate_undecided_is_refused() {
    let def = two_steps(true, true, activity("b"));
    let (state, mut journal, _) = after_a(&def);
    for decisions in [
        vec![decision("b", 1, granted())],
        vec![decision("b", 1, queued())],
        Vec::new(),
    ] {
        let mut state = state.clone();
        let refused = resolve(
            &def,
            &mut state,
            &mut journal,
            decisions.clone(),
            Vec::new(),
        );
        assert!(
            matches!(refused, Err(ExecutionError::Invariant(_))),
            "{decisions:?}: {refused:?}"
        );
    }
}

#[test]
fn a_queued_step_waits_spends_no_attempt_and_is_not_leased() {
    let def = two_steps(true, true, activity("b"));
    let (mut state, mut journal, _) = after_a(&def);
    resolve(
        &def,
        &mut state,
        &mut journal,
        vec![decision("b", 1, rate_queued()), decision("b", 1, queued())],
        Vec::new(),
    )
    .expect("queued");
    let b = state.step(&sid("b")).expect("b");
    assert_eq!(b.status(), StepStatus::Waiting);
    assert_eq!(b.attempt(), 0, "waiting spends no attempt");
    assert!(execution::lease_step(&mut state.clone(), &sid("b")).is_err());
    assert_eq!(
        execution::plan(&def, &state, T0).expect("plan"),
        execution::Plan::Idle,
        "decided: the planner does not ask again"
    );
    journal_alone_agrees(&def, &state, &journal);
}

#[test]
fn a_key_that_does_not_evaluate_fails_the_step_through_its_route_with_no_attempt() {
    let def = two_steps(false, true, activity("b").with_on_error(id("recover")));
    let (mut state, mut journal, _) = after_a(&def);
    resolve(
        &def,
        &mut state,
        &mut journal,
        Vec::new(),
        vec![(id("b"), FailureCode::flow_control_key_invalid())],
    )
    .expect("failed");
    let b = state.step(&sid("b")).expect("b");
    assert_eq!(b.status(), StepStatus::Failed);
    assert_eq!(b.failure(), Some(&FailureCode::flow_control_key_invalid()));
    assert_eq!(b.attempt(), 0, "no attempt spent");
    assert_eq!(
        state.step(&sid("recover")).expect("routed").status(),
        StepStatus::Ready,
        "the failure route takes it"
    );
    journal_alone_agrees(&def, &state, &journal);
}

#[test]
fn a_retry_of_a_gated_step_waits_again_for_the_next_attempt_after_its_permit_is_given_back() {
    let def = two_steps(true, false, flaky("b", 3));
    let (mut state, mut journal, _) = after_a(&def);
    resolve(
        &def,
        &mut state,
        &mut journal,
        vec![decision("b", 1, GateChange::ConcurrencyAcquired)],
        Vec::new(),
    )
    .expect("granted");
    let (fence, leased) = execution::lease_step(&mut state, &sid("b")).expect("leased");
    journal.push(leased);
    let failed = execution::fail_step(
        &def,
        &mut state,
        &sid("b"),
        Failure {
            fence,
            code: FailureCode::worker_error(),
            retryable: true,
            jitter_sample: 0.0,
        },
        T0,
    )
    .expect("retry scheduled");
    journal.extend(failed.events.iter().cloned());
    assert!(
        failed.events.iter().any(|event| matches!(
            event,
            RunEvent::Gate(GateEvent {
                change: GateChange::ConcurrencyReleased,
                ..
            })
        )),
        "the attempt's permit is given back when it ends"
    );
    let due = execution::retry_due(&def, &mut state, &sid("b"), T0 + Duration::from_secs(1))
        .expect("due");
    journal.extend(due.events.iter().cloned());
    let b = state.step(&sid("b")).expect("b");
    assert_eq!(
        b.status(),
        StepStatus::Waiting,
        "the retry pass makes it wait"
    );
    assert_eq!(
        due.pending,
        Some(Evaluation::Gates {
            steps: vec![GateRequest {
                node: id("b"),
                attempt: 2
            }]
        })
    );
    resolve(
        &def,
        &mut state,
        &mut journal,
        vec![decision("b", 2, GateChange::ConcurrencyAcquired)],
        Vec::new(),
    )
    .expect("granted again");
    assert_eq!(
        state.step(&sid("b")).expect("b").status(),
        StepStatus::Ready
    );
    journal_alone_agrees(&def, &state, &journal);
}

#[test]
fn a_paused_run_decides_nothing_and_its_resume_asks_for_the_gates() {
    let def = two_steps(true, false, activity("b"));
    let (mut state, mut journal, done) = after_a(&def);
    assert!(done.pending.is_some());
    let paused = execution::pause_run(&mut state).expect("paused");
    journal.push(paused);
    assert_eq!(
        execution::plan(&def, &state, T0).expect("plan"),
        execution::Plan::Idle
    );
    let resumed = execution::resume_run(&def, &mut state, T0).expect("resumed");
    journal.extend(resumed.events.iter().cloned());
    assert!(
        matches!(resumed.pending, Some(Evaluation::Gates { .. })),
        "{:?}",
        resumed.pending
    );
}

#[test]
fn a_step_left_ready_without_a_grant_is_a_breach_of_invariant_11() {
    let def = two_steps(true, false, activity("b"));
    let (_, journal, _) = after_a(&def);
    // the journal alone (no gated node known) leaves `b` READY: a commit that made it ready journaled no decision
    let alone = RunState::replay(RunId::generate(), &journal)
        .expect("replays")
        .with_gated_steps(&def);
    assert_eq!(
        alone.step(&sid("b")).expect("b").status(),
        StepStatus::Ready
    );
    assert!(
        alone
            .flow_control_breach()
            .is_some_and(|(node, _)| node == id("b")),
        "{:?}",
        alone.flow_control_breach()
    );
    // and the folded state waits without a decision: a breach too, until the gate answers
    let (folded, _, _) = after_a(&def);
    assert!(folded.flow_control_breach().is_some());
}

#[test]
fn a_gate_event_for_a_node_that_declares_none_is_refused() {
    let def = two_steps(true, false, activity("b"));
    let (mut state, _, _) = after_a(&def);
    let wrong_kind = RunEvent::Gate(decision("b", 1, granted()));
    assert!(matches!(
        state.clone().apply(&wrong_kind),
        Err(ExecutionError::Invariant(_))
    ));
    let ungated = RunEvent::Gate(decision("a", 1, GateChange::ConcurrencyAcquired));
    assert!(matches!(
        state.apply(&ungated),
        Err(ExecutionError::Invariant(_))
    ));
}

#[test]
fn an_ungated_definition_folds_as_before() {
    let def = build("a", vec![activity("a").with_next(id("b")), activity("b")]);
    let (state, journal, done) = after_a(&def);
    assert_eq!(
        state.step(&sid("b")).expect("b").status(),
        StepStatus::Ready
    );
    assert_eq!(done.pending, None);
    assert_eq!(
        RunState::replay(state.run_id(), &journal).expect("replays"),
        state,
        "an ungated state is exactly the journal's"
    );
}

#[test]
fn a_stopped_run_takes_its_waiting_steps_out_of_their_queues_before_it_ends() {
    let def = two_steps(true, true, activity("b"));
    let (mut state, mut journal, _) = after_a(&def);
    resolve(
        &def,
        &mut state,
        &mut journal,
        vec![decision("b", 1, rate_queued()), decision("b", 1, queued())],
        Vec::new(),
    )
    .expect("queued");
    let stopped = execution::stop_run(
        &def,
        &mut state,
        sagat_domain::execution::StopReason::Cancelled,
        None,
        T0,
    )
    .expect("cancelled");
    journal.extend(stopped.events.iter().cloned());
    let names: Vec<_> = stopped.events.iter().map(RunEvent::name).collect();
    let position = |name: &str| {
        names
            .iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("{name} in {names:?}"))
    };
    assert!(
        position("rate_limit.dequeued") < position("concurrency.dequeued"),
        "rate first: {names:?}"
    );
    assert!(
        position("concurrency.dequeued") < position("step.cancelled"),
        "{names:?}"
    );
    assert_eq!(names.last(), Some(&"run.finished"), "{names:?}");
    journal_alone_agrees(&def, &state, &journal);
}
