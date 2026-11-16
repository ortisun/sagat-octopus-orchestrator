//! Flow control of feature 022 in the domain (release 0.3.0 slice, foundation): the declarations and their bounds
//! (T022-004, T022-068, FR-022-017, FR-022-039), and the ten journal events with their reducer rules, one replay
//! test per event of `contracts/journal-events.md` (T022-008, T022-068, FR-022-045).
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{T0, activity, build, id, sid};
use sagat_domain::execution::{
    self, ExecutionError, FailureCode, GateChange, GateEvent, GatePhase, GatePool, GateRef,
    GateTarget, KeyLabel, RunEvent, RunId, RunState, RunStatus, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::flow_control::{
    DEFAULT_QUEUE_TIMEOUT, MAX_CONCURRENCY_LIMIT, MAX_NODE_DECLARATIONS, MAX_QUEUE_TIMEOUT,
};
use sagat_domain::workflow::{
    ConcurrencyPolicy, DefinitionWarning, Expression, FlowControl, FlowControlRule, FlowGroup,
    GateKind, GroupName, Node, NodeKind, RateLimitPolicy, RatePeriod, ResolvedGroup,
    WorkflowDefinition, WorkflowError,
};

fn concurrency(limit: u32) -> ConcurrencyPolicy {
    ConcurrencyPolicy::new(
        FlowGroup::Derived,
        Some(Expression::parse("input.accountId").expect("key")),
        limit,
        DEFAULT_QUEUE_TIMEOUT,
    )
    .expect("a policy")
}

fn rate(limit: u32, per: RatePeriod) -> RateLimitPolicy {
    RateLimitPolicy::new(FlowGroup::Derived, None, limit, per, DEFAULT_QUEUE_TIMEOUT)
        .expect("a policy")
}

// ------------------------------------------------------------------------------------------- the declarations

#[test]
fn a_concurrency_limit_is_between_one_and_its_ceiling() {
    let make = |limit| {
        ConcurrencyPolicy::new(FlowGroup::Derived, None, limit, DEFAULT_QUEUE_TIMEOUT)
            .map(|policy| policy.limit())
    };
    assert_eq!(make(1), Ok(1));
    assert_eq!(make(MAX_CONCURRENCY_LIMIT), Ok(MAX_CONCURRENCY_LIMIT));
    let over = FlowControlRule::Limit {
        max: MAX_CONCURRENCY_LIMIT,
    };
    assert_eq!(make(0), Err(over), "zero is not unlimited: it refuses");
    assert_eq!(make(MAX_CONCURRENCY_LIMIT + 1), Err(over));
}

#[test]
fn a_rate_limit_is_bounded_by_the_ceiling_of_its_window() {
    for (per, max) in [
        (RatePeriod::Second, 100),
        (RatePeriod::Minute, 6_000),
        (RatePeriod::Hour, 10_000),
    ] {
        let make = |limit| {
            RateLimitPolicy::new(FlowGroup::Derived, None, limit, per, DEFAULT_QUEUE_TIMEOUT)
                .map(|policy| policy.limit())
        };
        assert_eq!(make(max), Ok(max), "{per:?} at its ceiling");
        assert_eq!(
            make(max + 1),
            Err(FlowControlRule::Limit { max }),
            "{per:?} over it"
        );
        assert_eq!(make(0), Err(FlowControlRule::Limit { max }), "{per:?} zero");
        assert_eq!(per.max_limit(), max);
        assert_eq!(RatePeriod::from_millis(per.millis()), Some(per));
        assert_eq!(RatePeriod::parse(per.as_str()), Some(per));
    }
    assert_eq!(RatePeriod::parse("day"), None, "the window is a closed set");
    assert_eq!(RatePeriod::from_millis(37_000), None);
}

#[test]
fn a_queue_timeout_is_between_one_millisecond_and_one_day() {
    let make = |timeout| {
        RateLimitPolicy::new(FlowGroup::Derived, None, 1, RatePeriod::Second, timeout).is_ok()
    };
    assert!(make(Duration::from_millis(1)));
    assert!(make(MAX_QUEUE_TIMEOUT));
    assert!(!make(Duration::ZERO));
    assert!(!make(MAX_QUEUE_TIMEOUT + Duration::from_millis(1)));
    assert!(
        ConcurrencyPolicy::new(FlowGroup::Derived, None, 1, Duration::ZERO).is_err(),
        "the same bound for a concurrency queue"
    );
}

#[test]
fn a_group_name_follows_its_pattern_and_a_derived_one_cannot_be_written() {
    assert!(GroupName::parse("orders-by-account").is_ok());
    assert!(GroupName::parse("provider.starts").is_ok());
    assert!(GroupName::parse(&"a".repeat(64)).is_ok());
    for bad in ["", "Orders", "9orders", "a_b", "=orders", &"a".repeat(65)] {
        assert_eq!(GroupName::parse(bad), Err(FlowControlRule::Group), "{bad}");
    }
    let workflow = Slug::parse("provider-sync").expect("slug");
    let run = FlowGroup::Derived.resolve(&workflow, None);
    let step = FlowGroup::Derived.resolve(&workflow, Some(&id("fetch")));
    assert_eq!(run.as_str(), "=provider-sync");
    assert_eq!(step.as_str(), "=provider-sync/fetch");
    assert!(run.is_derived() && step.is_derived());
    for stored in [&run, &step] {
        assert_eq!(
            ResolvedGroup::parse(stored.as_str()).as_ref(),
            Ok(stored),
            "a stored group reads back"
        );
    }
    let explicit = FlowGroup::Explicit(GroupName::parse("provider-starts").expect("name"))
        .resolve(&workflow, Some(&id("fetch")));
    assert_eq!(explicit.as_str(), "provider-starts");
    assert!(!explicit.is_derived());
    for bad in ["=", "=Bad", "=wf/", "=wf/a/b", "Upper"] {
        assert!(ResolvedGroup::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn only_a_declared_activity_may_declare_a_gate() {
    let gated = FlowControl::new(Some(concurrency(2)), Some(rate(10, RatePeriod::Second)));
    let node = activity("fetch")
        .with_flow_control(gated.clone())
        .expect("an activity may");
    assert_eq!(node.flow_control(), &gated);
    let timer = Node::new(
        id("wait"),
        NodeKind::Timer {
            delay: Duration::from_secs(1),
        },
    );
    assert_eq!(
        timer.with_flow_control(FlowControl::new(None, Some(rate(1, RatePeriod::Minute)))),
        Err(WorkflowError::InvalidFlowControl {
            kind: GateKind::RateLimit,
            node: Some("wait".to_owned()),
            rule: FlowControlRule::NodeKind,
        })
    );
    assert!(
        activity("plain")
            .with_flow_control(FlowControl::NONE)
            .is_ok_and(|node| !node.flow_control().is_gated()),
        "declaring nothing is always allowed"
    );
}

#[test]
fn a_level_with_both_gates_waits_once_until_the_earlier_deadline() {
    let short = ConcurrencyPolicy::new(FlowGroup::Derived, None, 1, Duration::from_millis(5_000))
        .expect("policy");
    let both = FlowControl::new(Some(short), Some(rate(10, RatePeriod::Second)));
    assert_eq!(both.queue_timeout(), Some(Duration::from_millis(5_000)));
    assert_eq!(FlowControl::NONE.queue_timeout(), None);
}

fn gated_definition(name: &str, nodes: Vec<Node>, run: FlowControl) -> WorkflowDefinition {
    WorkflowDefinition::new(
        Slug::parse(name).expect("slug"),
        nodes[0].id().clone(),
        nodes,
    )
    .expect("a definition")
    .with_run_flow_control(run)
}

#[test]
fn a_gate_beside_a_continuation_is_refused_at_publication() {
    let def = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("loop-forever").expect("slug"),
        id("work"),
        vec![
            activity("work").with_next(id("again")),
            Node::new(
                id("again"),
                NodeKind::ContinueAsNew {
                    input: Expression::parse("input").expect("input"),
                    state: None,
                },
            ),
        ],
        Vec::new(),
        sagat_domain::workflow::WorkflowLimits::default(),
        sagat_domain::workflow::ContinuationPolicy::new(3, Duration::from_secs(60))
            .expect("policy"),
    )
    .expect("a continuing definition");
    assert!(def.check_flow_control().is_ok(), "ungated, it is fine");
    let gated = def.with_run_flow_control(FlowControl::new(Some(concurrency(1)), None));
    assert_eq!(
        gated.check_flow_control(),
        Err(WorkflowError::FlowControlCombination {
            kind: GateKind::Concurrency,
            node: None,
            with: "continueAsNew",
        })
    );
}

#[test]
fn a_definition_holds_at_most_its_ceiling_of_node_declarations() {
    let nodes = |count: usize| {
        (0..count)
            .map(|i| {
                let node = activity(&format!("n{i}"))
                    .with_flow_control(FlowControl::new(None, Some(rate(1, RatePeriod::Second))))
                    .expect("gated");
                if i + 1 < count {
                    node.with_next(id(&format!("n{}", i + 1)))
                } else {
                    node
                }
            })
            .collect::<Vec<_>>()
    };
    let at = gated_definition("at-bound", nodes(MAX_NODE_DECLARATIONS), FlowControl::NONE);
    assert!(at.check_flow_control().is_ok());
    assert_eq!(at.gated_nodes().count(), MAX_NODE_DECLARATIONS);
    assert!(at.declares_gates());
    let over = gated_definition(
        "over-bound",
        nodes(MAX_NODE_DECLARATIONS + 1),
        FlowControl::NONE,
    );
    assert_eq!(
        over.check_flow_control(),
        Err(WorkflowError::InvalidFlowControl {
            kind: GateKind::RateLimit,
            node: None,
            rule: FlowControlRule::TooManyDeclarations {
                max: MAX_NODE_DECLARATIONS
            },
        })
    );
}

#[test]
fn an_ungated_definition_declares_no_gate_and_its_old_shape_is_unchanged() {
    let def = build("a", vec![activity("a").with_next(id("b")), activity("b")]);
    assert!(!def.declares_gates());
    assert!(!def.run_flow_control().is_gated());
    assert_eq!(def.gated_nodes().count(), 0);
    assert!(def.check_flow_control().is_ok());
    assert!(def.warnings().is_empty());
}

#[test]
fn a_child_of_the_same_workflow_under_a_run_permit_is_warned_about() {
    let child = Node::new(
        id("again"),
        NodeKind::Child {
            workflow: Slug::parse("orders").expect("slug"),
            version: None,
            input: None,
            cancel_policy: sagat_domain::workflow::CancelPolicy::Propagate,
        },
    );
    let def = gated_definition(
        "orders",
        vec![child],
        FlowControl::new(Some(concurrency(1)), None),
    );
    assert!(
        def.warnings()
            .contains(&DefinitionWarning::ConcurrencySelfChild { node: id("again") }),
        "{:?}",
        def.warnings()
    );
}

// --------------------------------------------------------------------------------------------- the journal

fn gate() -> GateRef {
    GateRef {
        pool: GatePool::Ordinary,
        group: ResolvedGroup::parse("=orders").expect("group"),
        key_label: KeyLabel::parse("0123456789ab").expect("label"),
    }
}

fn run_event(change: GateChange) -> GateEvent {
    GateEvent {
        target: GateTarget::Run,
        gate: gate(),
        change,
    }
}

fn step_event(node: &str, attempt: u32, change: GateChange) -> GateEvent {
    GateEvent {
        target: GateTarget::Step {
            node: id(node),
            attempt,
        },
        gate: gate(),
        change,
    }
}

const DEADLINE: i64 = 600_000_000;

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

fn rate_granted(from_queue: bool) -> GateChange {
    GateChange::RateGranted {
        limit: 10,
        per: RatePeriod::Second,
        granted_at_us: 1_000,
        from_queue,
    }
}

fn names(events: &[RunEvent]) -> Vec<&'static str> {
    events.iter().map(RunEvent::name).collect()
}

fn chain() -> WorkflowDefinition {
    build("a", vec![activity("a")])
}

/// `chain()` with the run-level gates an admission decides.
fn gated_chain(concurrency_gate: bool, rate_gate: bool) -> WorkflowDefinition {
    chain().with_run_flow_control(FlowControl::new(
        concurrency_gate.then(|| concurrency(1)),
        rate_gate.then(|| rate(10, RatePeriod::Second)),
    ))
}

/// Replays `events` and checks that the journal alone rebuilds `state`.
fn replays(state: &RunState, events: &[RunEvent]) {
    assert_eq!(
        &RunState::replay(state.run_id(), events).expect("the journal replays"),
        state,
        "replay of {:?}",
        names(events)
    );
}

/// Takes the only step of `chain()` to its end and returns the events of that.
fn run_to_the_end(state: &mut RunState) -> Vec<RunEvent> {
    let def = chain();
    let (fence, leased) = execution::lease_step(state, &sid("a")).expect("leased");
    let mut events = vec![leased];
    events.extend(
        execution::complete_step(&def, state, &sid("a"), fence, T0)
            .expect("completed")
            .events,
    );
    events
}

#[test]
fn concurrency_queued_holds_the_entry_and_the_run_waits() {
    let (state, progress) = execution::start_run_gated(
        &gated_chain(true, false),
        RunId::generate(),
        T0,
        vec![run_event(queued())],
    )
    .expect("admitted");
    assert_eq!(
        names(&progress.events),
        ["run.started", "concurrency.queued"]
    );
    assert_eq!(state.status(), RunStatus::Waiting);
    assert_eq!(
        state.step(&sid("a")).expect("entry").status(),
        StepStatus::Pending,
        "nothing is released while the run waits for its permit"
    );
    assert!(matches!(
        state.run_gates().concurrency().map(|slot| slot.phase()),
        Some(GatePhase::Queued {
            deadline_us: DEADLINE
        })
    ));
    replays(&state, &progress.events);
}

#[test]
fn concurrency_acquired_from_the_queue_releases_the_entry_once() {
    let def = gated_chain(true, false);
    let (mut state, admitted) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(queued())])
            .expect("admitted");
    let granted = execution::apply_gate(
        &def,
        &mut state,
        T0,
        run_event(GateChange::ConcurrencyAcquired),
    )
    .expect("granted");
    assert_eq!(
        names(&granted.events),
        ["concurrency.acquired", "step.released"]
    );
    assert_eq!(state.status(), RunStatus::Running);
    let again = execution::apply_gate(
        &def,
        &mut state,
        T0,
        run_event(GateChange::ConcurrencyAcquired),
    );
    assert!(again.is_err(), "a permit is granted once: {again:?}");
    let mut events = admitted.events;
    events.extend(granted.events);
    replays(&state, &events);
}

#[test]
fn concurrency_released_comes_before_the_run_finishes_and_never_while_it_works() {
    let def = gated_chain(true, false);
    let (mut state, admitted) = execution::start_run_gated(
        &def,
        RunId::generate(),
        T0,
        vec![run_event(GateChange::ConcurrencyAcquired)],
    )
    .expect("admitted with a free permit");
    assert_eq!(
        names(&admitted.events),
        ["run.started", "concurrency.acquired", "step.released"]
    );
    let early = state
        .clone()
        .apply(&RunEvent::Gate(run_event(GateChange::ConcurrencyReleased)));
    assert!(
        early.is_err(),
        "a permit is not given back while the run works: {early:?}"
    );
    let rest = run_to_the_end(&mut state);
    let tail: Vec<_> = names(&rest).into_iter().rev().take(2).collect();
    assert_eq!(
        tail,
        ["run.finished", "concurrency.released"],
        "the permit is given back right before the last event: {:?}",
        names(&rest)
    );
    assert_eq!(state.status(), RunStatus::Succeeded);
    assert!(state.run_gates().held_permit().is_none());
    let mut events = admitted.events;
    events.extend(rest);
    replays(&state, &events);
}

#[test]
fn concurrency_wait_timed_out_ends_the_run_failed_without_any_step_having_run() {
    let def = gated_chain(true, false);
    let (mut state, mut events) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(queued())])
            .map(|(state, progress)| (state, progress.events))
            .expect("admitted");
    let timed_out = execution::apply_gate(
        &def,
        &mut state,
        T0,
        run_event(GateChange::ConcurrencyWaitTimedOut),
    )
    .expect("timed out");
    assert_eq!(names(&timed_out.events), ["concurrency.wait_timed_out"]);
    assert_eq!(
        state.status(),
        RunStatus::Waiting,
        "the entry stays held until the run is stopped"
    );
    events.extend(timed_out.events);
    let stopped = execution::stop_run(
        &def,
        &mut state,
        StopReason::Failed,
        Some(FailureCode::concurrency_wait_timeout()),
        T0,
    )
    .expect("stopped");
    assert_eq!(
        names(&stopped.events),
        ["run.stop_requested", "step.cancelled", "run.finished"]
    );
    assert_eq!(state.status(), RunStatus::Failed);
    events.extend(stopped.events);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, RunEvent::StepLeased { .. }))
    );
    replays(&state, &events);
}

#[test]
fn concurrency_dequeued_by_a_cancellation_before_the_run_finishes() {
    let def = gated_chain(true, false);
    let (mut state, mut events) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(queued())])
            .map(|(state, progress)| (state, progress.events))
            .expect("admitted");
    let cancelled =
        execution::stop_run(&def, &mut state, StopReason::Cancelled, None, T0).expect("cancelled");
    assert_eq!(
        names(&cancelled.events),
        [
            "run.stop_requested",
            "concurrency.dequeued",
            "step.cancelled",
            "run.finished"
        ]
    );
    assert_eq!(state.status(), RunStatus::Cancelled);
    events.extend(cancelled.events);
    replays(&state, &events);
}

#[test]
fn rate_limit_queued_and_granted_hold_and_release_the_entry() {
    let def = gated_chain(false, true);
    let (mut state, mut events) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(rate_queued())])
            .map(|(state, progress)| (state, progress.events))
            .expect("admitted");
    assert_eq!(names(&events), ["run.started", "rate_limit.queued"]);
    assert_eq!(state.status(), RunStatus::Waiting);
    let granted = execution::apply_gate(&def, &mut state, T0, run_event(rate_granted(true)))
        .expect("granted");
    assert_eq!(
        names(&granted.events),
        ["rate_limit.granted", "step.released"]
    );
    events.extend(granted.events);
    let rest = run_to_the_end(&mut state);
    assert_eq!(
        names(&rest).last(),
        Some(&"run.finished"),
        "a slot is never given back: it stops counting by time"
    );
    events.extend(rest);
    replays(&state, &events);
}

#[test]
fn rate_limit_wait_timed_out_and_dequeued_of_a_run_waiting_for_both_gates() {
    let def = gated_chain(true, true);
    let (mut state, mut events) = execution::start_run_gated(
        &def,
        RunId::generate(),
        T0,
        vec![run_event(queued()), run_event(rate_queued())],
    )
    .map(|(state, progress)| (state, progress.events))
    .expect("admitted, waiting for both");
    // the timeout routine: the refusing gate's wait_timed_out (rate first), the other's dequeued, then the stop
    for change in [
        GateChange::RateWaitTimedOut,
        GateChange::ConcurrencyDequeued,
    ] {
        events.extend(
            execution::apply_gate(&def, &mut state, T0, run_event(change))
                .expect("applied")
                .events,
        );
    }
    events.extend(
        execution::stop_run(
            &def,
            &mut state,
            StopReason::Failed,
            Some(FailureCode::rate_limit_wait_timeout()),
            T0,
        )
        .expect("stopped")
        .events,
    );
    assert_eq!(state.status(), RunStatus::Failed);
    assert_eq!(
        names(&events),
        [
            "run.started",
            "concurrency.queued",
            "rate_limit.queued",
            "rate_limit.wait_timed_out",
            "concurrency.dequeued",
            "run.stop_requested",
            "step.cancelled",
            "run.finished"
        ]
    );
    replays(&state, &events);
}

#[test]
fn rate_limit_dequeued_by_a_cancellation() {
    let def = gated_chain(false, true);
    let (mut state, mut events) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(rate_queued())])
            .map(|(state, progress)| (state, progress.events))
            .expect("admitted");
    events.extend(
        execution::stop_run(&def, &mut state, StopReason::Cancelled, None, T0)
            .expect("cancelled")
            .events,
    );
    assert!(names(&events).contains(&"rate_limit.dequeued"));
    assert_eq!(state.status(), RunStatus::Cancelled);
    replays(&state, &events);
}

#[test]
fn a_run_level_slot_is_never_returned_and_a_run_cannot_finish_while_it_waits() {
    let def = gated_chain(false, true);
    let (state, _) =
        execution::start_run_gated(&def, RunId::generate(), T0, vec![run_event(rate_queued())])
            .expect("admitted");
    let returned = state
        .clone()
        .apply(&RunEvent::Gate(run_event(GateChange::RateReturned)));
    assert!(returned.is_err(), "{returned:?}");
    let mut stopping = state.clone();
    stopping
        .apply(&RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        })
        .expect("stop");
    stopping
        .apply(&RunEvent::StepCancelled { node: id("a") })
        .expect("entry cancelled");
    let finished = stopping.apply(&RunEvent::RunFinished {
        status: RunStatus::Cancelled,
    });
    assert!(
        matches!(finished, Err(ExecutionError::Invariant(_))),
        "a waiter leaves its queue before the run ends: {finished:?}"
    );
}

#[test]
fn a_run_gate_is_decided_at_admission_only_or_by_a_grant_of_its_queue() {
    let def = chain();
    let (mut state, _) = execution::start_run(&def, RunId::generate(), T0).expect("ungated");
    for change in [
        queued(),
        GateChange::ConcurrencyAcquired,
        rate_granted(false),
    ] {
        let late = state.clone().apply(&RunEvent::Gate(run_event(change)));
        assert!(
            late.is_err(),
            "after the entry is released, nothing decides a run gate: {late:?}"
        );
    }
    let dequeued = state.apply(&RunEvent::Gate(run_event(GateChange::ConcurrencyDequeued)));
    assert!(dequeued.is_err(), "nothing to dequeue: {dequeued:?}");
}

fn ready_step() -> (WorkflowDefinition, RunState, Vec<RunEvent>) {
    let def = chain();
    let (state, progress) = execution::start_run(&def, RunId::generate(), T0).expect("started");
    (def, state, progress.events)
}

#[test]
fn a_step_waits_for_its_gates_and_is_ready_once_the_last_is_granted() {
    let (_, mut state, mut events) = ready_step();
    for change in [queued(), rate_queued()] {
        let event = RunEvent::Gate(step_event("a", 1, change));
        state.apply(&event).expect("queued");
        events.push(event);
    }
    let step = state.step(&sid("a")).expect("step");
    assert_eq!(step.status(), StepStatus::Waiting);
    assert_eq!(step.waiting_on(), Some(&WaitingOn::Gate));
    assert_eq!(step.attempt(), 0, "waiting spends no attempt");
    assert!(
        execution::lease_step(&mut state.clone(), &sid("a")).is_err(),
        "a waiting step is not offered to a worker"
    );
    let event = RunEvent::Gate(step_event("a", 1, rate_granted(true)));
    state.apply(&event).expect("slot");
    events.push(event);
    assert_eq!(
        state.step(&sid("a")).expect("step").status(),
        StepStatus::Waiting,
        "still waiting for its permit"
    );
    let event = RunEvent::Gate(step_event("a", 1, GateChange::ConcurrencyAcquired));
    state.apply(&event).expect("permit");
    events.push(event);
    assert_eq!(
        state.step(&sid("a")).expect("step").status(),
        StepStatus::Ready
    );
    let (fence, leased) = execution::lease_step(&mut state, &sid("a")).expect("now it is leased");
    events.push(leased);
    let done = execution::complete_step(&chain(), &mut state, &sid("a"), fence, T0)
        .expect("the attempt ends");
    let names = names(&done.events);
    let released = names
        .iter()
        .position(|name| *name == "concurrency.released")
        .expect("the attempt's permit is given back");
    assert!(
        released
            < names
                .iter()
                .position(|name| *name == "run.finished")
                .expect("finished"),
        "{names:?}"
    );
    events.extend(done.events);
    replays(&state, &events);
}

#[test]
fn a_step_gate_is_for_the_attempt_it_would_start_next() {
    let (_, state, _) = ready_step();
    for attempt in [0, 2] {
        let wrong = state
            .clone()
            .apply(&RunEvent::Gate(step_event("a", attempt, queued())));
        assert!(wrong.is_err(), "attempt {attempt}: {wrong:?}");
    }
}

#[test]
fn rate_limit_returned_by_a_granted_step_that_was_never_leased() {
    let (_, mut state, mut events) = ready_step();
    for event in [
        RunEvent::Gate(step_event("a", 1, rate_granted(false))),
        // the granted step leaves READY without being leased (here, a cancellation)
        RunEvent::StepCancelled { node: id("a") },
        RunEvent::Gate(step_event("a", 1, GateChange::RateReturned)),
    ] {
        state.apply(&event).expect("applied");
        events.push(event);
    }
    assert!(matches!(
        state
            .step(&sid("a"))
            .and_then(|step| step.gates().rate())
            .map(|slot| slot.phase()),
        Some(GatePhase::Ended)
    ));
    replays(&state, &events);
    let (_, mut leased_state, _) = ready_step();
    leased_state
        .apply(&RunEvent::Gate(step_event("a", 1, rate_granted(false))))
        .expect("granted");
    execution::lease_step(&mut leased_state, &sid("a")).expect("leased");
    let late = leased_state.apply(&RunEvent::Gate(step_event(
        "a",
        1,
        GateChange::RateReturned,
    )));
    assert!(
        late.is_err(),
        "a leased attempt never returns its slot: {late:?}"
    );
}

#[test]
fn a_step_whose_wait_timed_out_fails_through_its_route_without_an_attempt() {
    let (_, mut state, mut events) = ready_step();
    for event in [
        RunEvent::Gate(step_event("a", 1, rate_queued())),
        RunEvent::Gate(step_event("a", 1, GateChange::RateWaitTimedOut)),
        RunEvent::StepFailed {
            node: id("a"),
            code: FailureCode::rate_limit_wait_timeout(),
            fence: None,
        },
    ] {
        state.apply(&event).expect("applied");
        events.push(event);
    }
    let step = state.step(&sid("a")).expect("step");
    assert_eq!(step.status(), StepStatus::Failed);
    assert_eq!(step.attempt(), 0);
    replays(&state, &events);
}

#[test]
fn a_step_dequeued_by_a_cancellation() {
    let (_, mut state, mut events) = ready_step();
    for event in [
        RunEvent::Gate(step_event("a", 1, queued())),
        RunEvent::Gate(step_event("a", 1, GateChange::ConcurrencyDequeued)),
        RunEvent::StepCancelled { node: id("a") },
    ] {
        state.apply(&event).expect("applied");
        events.push(event);
    }
    assert_eq!(
        state.step(&sid("a")).expect("step").status(),
        StepStatus::Cancelled
    );
    replays(&state, &events);
}

#[test]
fn the_journal_names_are_the_ten_of_the_contract_and_the_label_is_a_digest_prefix() {
    let all = [
        queued(),
        GateChange::ConcurrencyAcquired,
        GateChange::ConcurrencyReleased,
        GateChange::ConcurrencyWaitTimedOut,
        GateChange::ConcurrencyDequeued,
        rate_queued(),
        rate_granted(false),
        GateChange::RateWaitTimedOut,
        GateChange::RateDequeued,
        GateChange::RateReturned,
    ];
    let named: Vec<_> = all
        .iter()
        .map(|change| RunEvent::Gate(run_event(*change)).name())
        .collect();
    assert_eq!(
        named,
        [
            "concurrency.queued",
            "concurrency.acquired",
            "concurrency.released",
            "concurrency.wait_timed_out",
            "concurrency.dequeued",
            "rate_limit.queued",
            "rate_limit.granted",
            "rate_limit.wait_timed_out",
            "rate_limit.dequeued",
            "rate_limit.returned"
        ]
    );
    let label = KeyLabel::of_digest(&[0xab; 32]);
    assert_eq!(label.as_str(), "abababababab");
    assert!(KeyLabel::parse("ABABABABABAB").is_err());
    assert!(KeyLabel::parse("abababab").is_err());
    let step = RunEvent::Gate(step_event("a", 3, queued()));
    assert_eq!(step.node(), Some(&id("a")));
    assert_eq!(step.attempt(), Some(3));
    assert_eq!(RunEvent::Gate(run_event(queued())).node(), None);
}

// ------------------------------------------------------------------ the review of 2026-11-15 (correctness M1..M3)

/// Correctness review M1: a gate that timed out or left its queue decides the wait; the other gate cannot then grant.
#[test]
fn a_grant_after_the_other_gate_gave_up_is_refused() {
    let (_, mut state, _) = ready_step();
    for change in [queued(), rate_queued(), GateChange::RateWaitTimedOut] {
        state
            .apply(&RunEvent::Gate(step_event("a", 1, change)))
            .expect("applied");
    }
    let late = state.apply(&RunEvent::Gate(step_event(
        "a",
        1,
        GateChange::ConcurrencyAcquired,
    )));
    assert!(late.is_err(), "the step must not become ready: {late:?}");
    let def = gated_chain(true, true);
    let (mut run, _) = execution::start_run_gated(
        &def,
        RunId::generate(),
        T0,
        vec![run_event(queued()), run_event(rate_queued())],
    )
    .expect("admitted");
    run.apply(&RunEvent::Gate(run_event(GateChange::RateDequeued)))
        .expect("dequeued");
    let late = run.apply(&RunEvent::Gate(run_event(GateChange::ConcurrencyAcquired)));
    assert!(late.is_err(), "a doomed start takes no permit: {late:?}");
}

/// Correctness review M2: a permit is given back only once its attempt has ended, and an unleased slot only once the
/// step has left `READY`; a step whose gate was given back is not leased on it.
#[test]
fn a_permit_or_slot_is_given_back_only_when_its_attempt_no_longer_needs_it() {
    let (_, mut state, _) = ready_step();
    state
        .apply(&RunEvent::Gate(step_event(
            "a",
            1,
            GateChange::ConcurrencyAcquired,
        )))
        .expect("granted");
    execution::lease_step(&mut state, &sid("a")).expect("leased");
    let early = state.clone().apply(&RunEvent::Gate(step_event(
        "a",
        1,
        GateChange::ConcurrencyReleased,
    )));
    assert!(early.is_err(), "the attempt still runs: {early:?}");

    let (_, mut ready, _) = ready_step();
    ready
        .apply(&RunEvent::Gate(step_event("a", 1, rate_granted(false))))
        .expect("granted");
    let returned = ready.clone().apply(&RunEvent::Gate(step_event(
        "a",
        1,
        GateChange::RateReturned,
    )));
    assert!(
        returned.is_err(),
        "a READY step keeps its slot: {returned:?}"
    );
}

/// Correctness review M3: a new attempt does not overwrite a permit the last one still holds, and a run does not finish
/// while a step holds a permit; the planner gives an ended attempt's permit back first.
#[test]
fn a_held_permit_is_neither_overwritten_nor_left_behind_at_the_end() {
    let (_, mut state, _) = ready_step();
    state
        .apply(&RunEvent::Gate(step_event(
            "a",
            1,
            GateChange::ConcurrencyAcquired,
        )))
        .expect("granted");
    let (fence, _) = execution::lease_step(&mut state, &sid("a")).expect("leased");
    let mut retried = state.clone();
    for event in [
        RunEvent::StepRetryScheduled {
            node: id("a"),
            attempt: fence.attempt,
            generation: fence.generation,
            code: FailureCode::worker_error(),
            not_before: T0,
        },
        RunEvent::StepRetryDue { node: id("a") },
    ] {
        retried.apply(&event).expect("retry");
    }
    let overwrite = retried.apply(&RunEvent::Gate(step_event(
        "a",
        2,
        GateChange::ConcurrencyAcquired,
    )));
    assert!(
        overwrite.is_err(),
        "attempt 1 still holds the permit: {overwrite:?}"
    );
    let mut raw = state.clone();
    for event in [
        RunEvent::StepCompleted {
            node: id("a"),
            attempt: fence.attempt,
            generation: fence.generation,
        },
        RunEvent::StepRouted {
            node: id("a"),
            activated: Vec::new(),
        },
    ] {
        raw.apply(&event).expect("completed");
    }
    let finished = raw.apply(&RunEvent::RunFinished {
        status: RunStatus::Succeeded,
    });
    assert!(
        finished.is_err(),
        "the step's permit is still held: {finished:?}"
    );
}

/// An admission journals a decision for each gate its definition declares, and only those.
#[test]
fn an_admission_decides_exactly_the_gates_the_definition_declares() {
    let declared = build("a", vec![activity("a")]).with_run_flow_control(FlowControl::new(
        Some(concurrency(1)),
        Some(rate(10, RatePeriod::Second)),
    ));
    for decisions in [
        vec![run_event(queued())],
        vec![run_event(rate_queued()), run_event(rate_granted(false))],
    ] {
        assert!(
            execution::start_run_gated(&declared, RunId::generate(), T0, decisions.clone())
                .is_err(),
            "{decisions:?}"
        );
    }
    assert!(
        execution::start_run_gated(&chain(), RunId::generate(), T0, vec![run_event(queued())])
            .is_err(),
        "an ungated definition admits through no gate"
    );
    execution::start_run_gated(
        &declared,
        RunId::generate(),
        T0,
        vec![
            run_event(GateChange::ConcurrencyAcquired),
            run_event(rate_granted(false)),
        ],
    )
    .expect("both granted");
}
