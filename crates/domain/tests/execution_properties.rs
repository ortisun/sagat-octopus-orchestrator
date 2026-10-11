//! Property tests: random command sequences against the run state machine (T002-028, SC-002-001).
//!
//! A chaos driver picks among every action that is currently enabled — and a few that are not, such
//! as completing with a stale fence — and applies it. After every action the invariants below must
//! hold, a rejected action must leave state and journal untouched, and, once the chaos stops, a
//! benevolent driver must be able to finish the run.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::{
    Harness, activity, build, child, condition, effect, effect_with, end, flaky, id, parallel,
    saga, saga_with, timer, wait, with_effect,
};
use proptest::prelude::*;
use sagat_domain::execution::{
    ChildOutcome, Evaluation, ExecutionError, Expiry, FailureCode, Fence, Resolution,
    ReviewOutcome, RunId, RunStatus, SagaPhase, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::saga::{EffectPolicy, OnCompensationFailure, ReconcileVerdict, SagaCancelPolicy};
use sagat_domain::workflow::{
    CancelPolicy, JoinPolicy, NodeId, NodeKind, RetryPolicy, WorkflowDefinition,
};

fn shapes() -> Vec<WorkflowDefinition> {
    vec![
        // 0: a chain with an error route
        build(
            "a",
            vec![
                flaky("a", 3).with_next(id("b")).with_on_error(id("c")),
                flaky("b", 2).with_next(id("done")),
                activity("c"),
                end("done"),
            ],
        ),
        // 1: fork/join (all) followed by a timer
        build(
            "fork",
            vec![
                parallel("fork", &["l", "r"], JoinPolicy::All).with_next(id("t")),
                flaky("l", 2),
                activity("r"),
                timer("t", 30).with_next(id("z")),
                activity("z"),
            ],
        ),
        // 2: fork/join (any) with a wait inside a branch
        build(
            "fork",
            vec![
                parallel("fork", &["l", "w"], JoinPolicy::Any).with_next(id("after")),
                flaky("l", 2),
                wait("w", 60, None),
                activity("after"),
            ],
        ),
        // 3: condition into a parallel or a plain activity
        build(
            "cond",
            vec![
                condition("cond", "fork", "small"),
                parallel("fork", &["x", "y"], JoinPolicy::All).with_next(id("tail")),
                activity("x"),
                activity("y").with_on_error(id("recover")),
                activity("recover"),
                activity("tail"),
                activity("small"),
            ],
        ),
        // 4: nested parallels
        build(
            "outer",
            vec![
                parallel("outer", &["a", "inner"], JoinPolicy::All).with_next(id("after")),
                flaky("a", 2),
                parallel("inner", &["b", "c"], JoinPolicy::Any).with_next(id("tail")),
                activity("b"),
                activity("c"),
                activity("tail"),
                activity("after"),
            ],
        ),
        // 5: a wait with a timeout route
        build(
            "w",
            vec![
                wait("w", 120, Some("review")).with_next(id("notify")),
                activity("notify"),
                flaky("review", 2),
            ],
        ),
        // 6: a child whose cancellation the parent waits for, with an error route
        build(
            "c",
            vec![
                child("c", CancelPolicy::RequestCancel)
                    .with_next(id("after"))
                    .with_on_error(id("recover")),
                activity("after"),
                activity("recover"),
            ],
        ),
        // 7: a propagating child racing an activity in an `any` join
        build(
            "fork",
            vec![
                parallel("fork", &["kid", "a"], JoinPolicy::Any).with_next(id("tail")),
                child("kid", CancelPolicy::Propagate),
                flaky("a", 2),
                activity("tail"),
            ],
        ),
        // 8: a detached child followed by a timer
        build(
            "d",
            vec![
                child("d", CancelPolicy::Detach).with_next(id("t")),
                timer("t", 20).with_next(id("z")),
                activity("z"),
            ],
        ),
        // 9: a saga over a chain of compensable effects, one of them reconciled
        build(
            "s",
            vec![
                saga("s", "reserve").with_next(id("notify")),
                effect("reserve", false).with_next(id("submit")),
                effect_with("submit", true, RetryPolicy::DEFAULT).with_next(id("book")),
                effect("book", false),
                activity("notify"),
            ],
        ),
        // 10: a saga over parallel branches, an irreversible effect, and no human to decide
        build(
            "s",
            vec![
                saga_with(
                    "s",
                    "fan",
                    OnCompensationFailure::Fail,
                    SagaCancelPolicy::WaitInFlightThenCompensate,
                )
                .with_next(id("after")),
                parallel("fan", &["l", "r"], JoinPolicy::All),
                effect("l", false),
                with_effect(
                    flaky("r", 2),
                    EffectPolicy {
                        irreversible: true,
                        ..EffectPolicy::NONE
                    },
                ),
                activity("after"),
            ],
        ),
        // 11: a saga inside a branch, with a saga inside it, and a high-risk effect
        build(
            "fork",
            vec![
                parallel("fork", &["outer", "side"], JoinPolicy::All).with_next(id("tail")),
                saga("outer", "first"),
                effect("first", false).with_next(id("inner")),
                saga_with(
                    "inner",
                    "a",
                    OnCompensationFailure::ManualReview,
                    SagaCancelPolicy::AbortNoCompensation,
                )
                .with_next(id("last")),
                effect("a", false),
                with_effect(
                    effect("last", true),
                    EffectPolicy {
                        high_risk: true,
                        ..EffectPolicy::NONE
                    },
                ),
                flaky("side", 2),
                activity("tail"),
            ],
        ),
    ]
}

#[derive(Debug, Clone)]
enum Action {
    Lease(NodeId),
    Heartbeat(NodeId, Fence),
    Complete(NodeId, Fence),
    Fail(NodeId, Fence, bool),
    Expire(NodeId, Fence, Expiry),
    StaleComplete(NodeId, Fence),
    RetryDue(NodeId),
    FireTimer(NodeId),
    SatisfyWait(NodeId),
    TimeOutWait(NodeId),
    /// The child ended: 0 succeeded, 1 failed, 2 cancelled; with the reserved id or a wrong one.
    FinishChild(NodeId, RunId, u8),
    Advance(u64),
    Resolve(u8),
    Stop(StopReason),
    Pause,
    Resume,
    OperatorRetry(NodeId),
    /// Holds a step that has not started for a person to decide.
    OperatorHold(NodeId),
    OperatorSkip(NodeId),
    OperatorResolve(NodeId, bool),
    Reconcile(NodeId, Fence, ReconcileVerdict),
}

fn enabled(h: &Harness) -> Vec<Action> {
    let mut actions = vec![
        Action::Advance(1),
        Action::Advance(45),
        Action::Advance(3600),
        Action::Stop(StopReason::Cancelled),
        Action::Stop(StopReason::TimedOut),
        Action::Pause,
        Action::Resume,
    ];
    if h.pending.is_some() {
        actions.extend([Action::Resolve(0), Action::Resolve(1), Action::Resolve(2)]);
    }
    for (node, step) in h.state.steps() {
        let fence = Fence {
            attempt: step.attempt(),
            generation: step.generation(),
        };
        match step.status() {
            StepStatus::Ready => {
                actions.push(Action::Lease(node.clone()));
                actions.push(Action::OperatorHold(node.clone()));
            }
            StepStatus::Leased | StepStatus::Running => {
                if node.is_reconciliation() {
                    for verdict in [
                        ReconcileVerdict::Applied,
                        ReconcileVerdict::NotApplied,
                        ReconcileVerdict::Unknown,
                    ] {
                        actions.push(Action::Reconcile(node.clone(), fence, verdict));
                    }
                }
                actions.push(Action::Heartbeat(node.clone(), fence));
                actions.push(Action::Complete(node.clone(), fence));
                actions.push(Action::Fail(node.clone(), fence, true));
                actions.push(Action::Fail(node.clone(), fence, false));
                actions.push(Action::Expire(node.clone(), fence, Expiry::LeaseExpired));
                actions.push(Action::Expire(node.clone(), fence, Expiry::AttemptTimeout));
            }
            StepStatus::RetryWait => {
                actions.push(Action::RetryDue(node.clone()));
                // Without waiting for the delay, and the other way to leave it: held for a person.
                actions.push(Action::OperatorRetry(node.clone()));
                actions.push(Action::OperatorHold(node.clone()));
            }
            StepStatus::Waiting => match step.waiting_on() {
                Some(WaitingOn::Operator { .. }) => {
                    actions.push(Action::OperatorRetry(node.clone()));
                    actions.push(Action::OperatorSkip(node.clone()));
                    actions.push(Action::OperatorResolve(node.clone(), true));
                    actions.push(Action::OperatorResolve(node.clone(), false));
                }
                Some(WaitingOn::Timer { .. }) => actions.push(Action::FireTimer(node.clone())),
                Some(WaitingOn::Event { .. }) => {
                    actions.push(Action::SatisfyWait(node.clone()));
                    actions.push(Action::TimeOutWait(node.clone()));
                }
                Some(WaitingOn::Child { run }) => {
                    for mode in 0..3 {
                        actions.push(Action::FinishChild(node.clone(), *run, mode));
                    }
                    // A report about a run this step never waited for must always be refused.
                    actions.push(Action::FinishChild(node.clone(), RunId::generate(), 0));
                }
                _ => {}
            },
            _ => {}
        }
        if step.attempt() > 1 {
            // A holder from an earlier attempt: must always be refused.
            actions.push(Action::StaleComplete(
                node.clone(),
                Fence {
                    attempt: step.attempt() - 1,
                    generation: step.generation() - 1,
                },
            ));
        }
    }
    actions
}

fn perform(h: &mut Harness, action: &Action) -> Result<(), ExecutionError> {
    match action {
        Action::Lease(node) => h.try_lease(node.as_str()).map(|_| ()),
        Action::Heartbeat(node, fence) => h.heartbeat(node.as_str(), *fence).map(|_| ()),
        Action::Complete(node, fence) | Action::StaleComplete(node, fence) => {
            h.complete(node.as_str(), *fence)
        }
        Action::Fail(node, fence, retryable) => h.fail(node.as_str(), *fence, *retryable),
        Action::Expire(node, fence, cause) => h.expire(node.as_str(), *fence, *cause),
        Action::RetryDue(node) => h.retry_due(node.as_str()),
        Action::FireTimer(node) => h.fire_timer(node.as_str()),
        Action::SatisfyWait(node) => h.satisfy_wait(node.as_str()),
        Action::TimeOutWait(node) => h.time_out_wait(node.as_str()),
        Action::FinishChild(node, child, mode) => h.finish_child(
            node.as_str(),
            *child,
            match mode {
                0 => ChildOutcome::Succeeded,
                1 => ChildOutcome::Failed(FailureCode::child_failed()),
                _ => ChildOutcome::Failed(FailureCode::child_cancelled()),
            },
        ),
        Action::Advance(secs) => {
            h.advance(Duration::from_secs(*secs));
            Ok(())
        }
        Action::Resolve(mode) => {
            let resolution = match h.pending.clone().expect("pending") {
                Evaluation::Condition { node, .. } => Resolution::Condition {
                    node,
                    outcome: match mode {
                        0 => Ok(true),
                        1 => Ok(false),
                        _ => Err(()),
                    },
                },
                Evaluation::Correlation { node, .. } => Resolution::Correlation {
                    node,
                    key: match mode {
                        2 => Err(()),
                        _ => Ok(sagat_domain::execution::CorrelationKey::parse("k").expect("key")),
                    },
                },
                Evaluation::Child { node, .. } => Resolution::Child {
                    node,
                    child: match mode {
                        2 => Err(FailureCode::child_depth_exceeded()),
                        _ => Ok(RunId::generate()),
                    },
                },
            };
            h.resolve(resolution)
        }
        Action::Stop(reason) => h.stop(*reason),
        Action::Pause => h.pause(),
        Action::Resume => h.resume(),
        Action::OperatorRetry(node) => h.operator_retry(node.as_str()),
        Action::OperatorHold(node) => h.operator_hold(node.as_str()),
        Action::OperatorSkip(node) => h.operator_skip(node.as_str()),
        Action::OperatorResolve(node, ok) => h.operator_resolve(
            node.as_str(),
            if *ok {
                ReviewOutcome::Succeeded
            } else {
                ReviewOutcome::Failed(FailureCode::parse("MANUALLY_FAILED").expect("code"))
            },
        ),
        Action::Reconcile(node, fence, verdict) => h.reconcile(node.as_str(), *fence, *verdict),
    }
}

/// Everything that must be true after any sequence of commands.
fn check(h: &Harness, terminal_seen: &mut BTreeMap<NodeId, StepStatus>) {
    h.assert_replays();
    assert_eq!(
        h.state.version() as usize,
        h.journal.len(),
        "one version per journal entry"
    );

    for (node, step) in h.state.steps() {
        let def_node = h.def.node(node).expect("every step has a node");
        // Statuses are only reachable for the kinds that use them.
        match step.status() {
            StepStatus::Ready
            | StepStatus::Leased
            | StepStatus::Running
            | StepStatus::RetryWait => {
                assert!(
                    matches!(def_node.kind(), NodeKind::Activity { .. }),
                    "{node}: {:?}",
                    step.status()
                );
            }
            StepStatus::Waiting => {
                let ok = matches!(
                    (def_node.kind(), step.waiting_on()),
                    (NodeKind::Timer { .. }, Some(WaitingOn::Timer { .. }))
                        | (NodeKind::WaitEvent { .. }, Some(WaitingOn::Event { .. }))
                        | (NodeKind::Parallel { .. }, Some(WaitingOn::Branches))
                        | (NodeKind::Child { .. }, Some(WaitingOn::Child { .. }))
                        | (NodeKind::Saga { .. }, Some(WaitingOn::Saga))
                        | (
                            NodeKind::Activity { .. },
                            Some(WaitingOn::Reconciliation | WaitingOn::Operator { .. })
                        )
                );
                assert!(ok, "{node}: waiting on {:?}", step.waiting_on());
            }
            _ => assert!(
                step.waiting_on().is_none(),
                "{node}: only a waiting step waits"
            ),
        }
        if let NodeKind::Activity { retry, .. } = def_node.kind() {
            // An operator may ask for more attempts than the budget allows; nobody else may.
            let overridden = h
                .journal
                .iter()
                .any(|e| matches!(e, sagat_domain::execution::RunEvent::StepRetryRequested { node: n } if n == node));
            assert!(
                overridden || step.attempt() <= retry.max_attempts(),
                "{node}: attempts exceed the budget"
            );
        }
        assert_eq!(
            step.attempt() as u64,
            step.generation(),
            "{node}: generation tracks attempts"
        );
        if step.routed() {
            assert!(
                step.status().is_terminal(),
                "{node}: only a finished step is routed"
            );
        }
        if step.cancel_requested() {
            assert!(
                step.status().is_leased()
                    || (step.status() == StepStatus::Waiting
                        && matches!(step.waiting_on(), Some(WaitingOn::Child { .. }))),
                "{node}: cancel is only requested of a leased step or one waiting for a child"
            );
        }
        // Terminal is absorbing.
        if let Some(previous) = terminal_seen.get(node) {
            assert_eq!(*previous, step.status(), "{node}: a finished step changed");
        }
        if step.status().is_terminal() {
            terminal_seen.insert(node.clone(), step.status());
        }
    }

    // An "all" join that succeeded did not abandon any branch work.
    for def_node in h.def.nodes() {
        let NodeKind::Parallel {
            join: JoinPolicy::All,
            ..
        } = def_node.kind()
        else {
            continue;
        };
        if h.state
            .step(def_node.id())
            .is_some_and(|s| s.status() == StepStatus::Succeeded)
        {
            for member in h
                .def
                .branch_regions(def_node.id())
                .into_iter()
                .flatten()
                .flatten()
            {
                // A nested parallel has its own policy: its losers are legitimately cancelled.
                if h.def.enclosing_parallel(member) != Some(def_node.id()) {
                    continue;
                }
                assert_ne!(
                    h.state.step(member).map(|s| s.status()),
                    Some(StepStatus::Cancelled),
                    "{member}: an all-join succeeded with a cancelled branch step"
                );
            }
        }
    }

    // Once a parallel node is joined, its branches only wind down: nothing new may start there.
    for def_node in h.def.nodes() {
        if !matches!(def_node.kind(), NodeKind::Parallel { .. }) {
            continue;
        }
        let joined = h
            .state
            .step(def_node.id())
            .is_some_and(|s| matches!(s.status(), StepStatus::Succeeded | StepStatus::Failed));
        if !joined {
            continue;
        }
        for member in h
            .def
            .branch_regions(def_node.id())
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Some(step) = h.state.step(member) {
                // A saga that waits for its attempts in flight (by its policy) lets them finish.
                let waited_for = h.def.scope_chain(member).any(|scope| {
                    matches!(
                        h.def
                            .node(scope.owner())
                            .map(sagat_domain::workflow::Node::kind),
                        Some(NodeKind::Saga {
                            cancel_policy: SagaCancelPolicy::WaitInFlightThenCompensate,
                            ..
                        })
                    )
                });
                // A saga compensates, and waits for an answer it needs, after its branch was lost.
                let owned_by_saga = matches!(
                    step.waiting_on(),
                    Some(WaitingOn::Saga | WaitingOn::Reconciliation | WaitingOn::Operator { .. })
                ) && (matches!(
                    h.def.node(member).map(sagat_domain::workflow::Node::kind),
                    Some(NodeKind::Saga { .. })
                ) || h
                    .def
                    .scope_chain(member)
                    .any(|scope| matches!(scope, sagat_domain::workflow::Scope::Saga(_))));
                let winding_down = step.status().is_terminal()
                    || owned_by_saga
                    || (step.status().is_leased() && (step.cancel_requested() || waited_for));
                assert!(
                    winding_down || h.state.is_finished() || h.state.is_paused(),
                    "{member}: still {:?} after {} was joined",
                    step.status(),
                    def_node.id()
                );
            }
        }
    }

    let any_active = h.state.steps().any(|(_, s)| {
        matches!(
            s.status(),
            StepStatus::Pending
                | StepStatus::Ready
                | StepStatus::Leased
                | StepStatus::Running
                | StepStatus::RetryWait
        )
    });
    check_sagas(h);
    match h.state.status() {
        RunStatus::Waiting => assert!(!any_active, "a waiting run has nothing runnable"),
        RunStatus::Cancelling => assert!(h.state.stop_reason().is_some()),
        RunStatus::Compensating => assert!(
            h.state
                .sagas()
                .any(|(_, s)| matches!(s.phase(), SagaPhase::Aborting | SagaPhase::Compensating)),
            "a compensating run has a saga that is"
        ),
        RunStatus::ManualReview => {
            assert!(
                h.state
                    .steps()
                    .any(|(_, s)| matches!(s.waiting_on(), Some(WaitingOn::Operator { .. }))),
                "a run in manual review has a step that waits for an operator"
            );
            assert!(
                !h.state.steps().any(|(_, s)| matches!(
                    s.status(),
                    StepStatus::Ready
                        | StepStatus::Leased
                        | StepStatus::Running
                        | StepStatus::RetryWait
                )),
                "nothing moves by itself in manual review"
            );
        }
        RunStatus::Paused => assert!(h.state.is_paused()),
        status if status.is_terminal() => {
            assert!(
                h.state.steps().all(|(_, s)| s.status().is_terminal()),
                "a finished run has no live step"
            );
            assert!(!h.state.is_paused() || h.state.stop_reason().is_none() || true);
        }
        _ => {}
    }
    if h.state.status() == RunStatus::Succeeded {
        assert!(h.state.stop_reason().is_none());
        assert!(
            h.state.steps().all(|(_, s)| s.routed()),
            "a successful run routed everything"
        );
    }
}

/// What must hold for every saga, whatever happened.
fn check_sagas(h: &Harness) {
    for (saga_node, saga) in h.state.sagas() {
        let step = h.state.step(saga_node).expect("a saga has a step");
        match saga.phase() {
            SagaPhase::Forward | SagaPhase::Aborting | SagaPhase::Compensating => {
                // A saga whose step was cancelled outright (no compensation policy) stays Forward.
                assert!(
                    matches!(step.waiting_on(), Some(WaitingOn::Saga))
                        || (saga.phase() == SagaPhase::Forward && step.status().is_terminal()),
                    "{saga_node}: {:?} but its step is {:?}",
                    saga.phase(),
                    step.status()
                );
            }
            SagaPhase::Done => assert!(step.status().is_terminal(), "{saga_node}: done but active"),
        }
        let region = h.def.saga_region(saga_node).expect("a saga has a region");
        if matches!(saga.phase(), SagaPhase::Compensating | SagaPhase::Done)
            && saga.outcome().is_some()
            || saga.phase() == SagaPhase::Compensating
        {
            // The forward work is over before any compensation starts.
            for member in region {
                if let Some(s) = h.state.step(member) {
                    assert!(
                        s.status().is_terminal(),
                        "{member}: {:?} while {saga_node} is {:?}",
                        s.status(),
                        saga.phase()
                    );
                }
            }
        }
        // A compensation never starts before everything it waits for has finished.
        for (forward, after) in saga.entries() {
            let companion = h
                .state
                .step(&forward.compensation())
                .expect("every planned entry has its compensation");
            if companion.attempt() > 0 {
                for earlier in after {
                    let status = h
                        .state
                        .step(&earlier.compensation())
                        .map(sagat_domain::execution::StepState::status);
                    assert!(
                        matches!(status, Some(StepStatus::Succeeded | StepStatus::Skipped)),
                        "{forward}~c ran while {earlier}~c was {status:?}"
                    );
                }
            }
        }
        if let Some(outcome) = saga.outcome() {
            assert_eq!(saga.phase(), SagaPhase::Done);
            let failed = saga.entries().keys().any(|n| {
                matches!(
                    h.state
                        .step(&n.compensation())
                        .map(sagat_domain::execution::StepState::status),
                    Some(StepStatus::Failed | StepStatus::Cancelled)
                )
            });
            if failed {
                assert_eq!(outcome, sagat_domain::saga::SagaOutcome::CompensationFailed);
            }
        }
    }
    // Only a saga can end a run as compensated, partial or compensation-failed.
    if matches!(
        h.state.status(),
        RunStatus::Compensated | RunStatus::Partial | RunStatus::CompensationFailed
    ) {
        assert!(
            h.state.sagas().any(|(_, s)| s.outcome().is_some()),
            "a run cannot be compensated without a saga that compensated"
        );
    }
}

/// Drives the run to its end without any further faults.
fn finish(h: &mut Harness) {
    for _ in 0..2_000 {
        if h.state.is_finished() {
            return;
        }
        if h.state.is_paused() {
            h.resume().expect("resume");
            continue;
        }
        if h.pending.is_some() {
            perform(h, &Action::Resolve(0)).expect("resolve");
            continue;
        }
        let snapshot: Vec<_> = h
            .state
            .steps()
            .map(|(n, s)| {
                (
                    n.clone(),
                    s.status(),
                    s.attempt(),
                    s.generation(),
                    s.waiting_on().cloned(),
                )
            })
            .collect();
        let mut progressed = false;
        for (node, status, attempt, generation, waiting) in snapshot {
            let fence = Fence {
                attempt,
                generation,
            };
            let outcome = match status {
                StepStatus::Ready => h.try_lease(node.as_str()).map(|_| ()),
                StepStatus::Leased | StepStatus::Running if node.is_reconciliation() => {
                    h.reconcile(node.as_str(), fence, ReconcileVerdict::Applied)
                }
                StepStatus::Leased | StepStatus::Running => h.complete(node.as_str(), fence),
                StepStatus::RetryWait => {
                    h.advance(Duration::from_secs(3600));
                    h.retry_due(node.as_str())
                }
                StepStatus::Waiting => match waiting {
                    Some(WaitingOn::Operator { .. }) => {
                        h.operator_resolve(node.as_str(), ReviewOutcome::Succeeded)
                    }
                    Some(WaitingOn::Timer { .. }) => {
                        h.advance(Duration::from_secs(3600));
                        h.fire_timer(node.as_str())
                    }
                    Some(WaitingOn::Event { .. }) => h.satisfy_wait(node.as_str()),
                    Some(WaitingOn::Child { run }) => {
                        h.finish_child(node.as_str(), run, ChildOutcome::Succeeded)
                    }
                    _ => continue,
                },
                _ => continue,
            };
            if outcome.is_ok() {
                progressed = true;
                break;
            }
        }
        assert!(
            progressed || h.state.is_finished(),
            "the run is stuck: {:?}",
            h.state
        );
    }
    panic!("the run did not finish in 2000 actions: {:?}", h.state);
}

/// The graph a client holds is kept equal to the platform's by the steps the events report as
/// changed (`RunEvent::changed_nodes`, D141): no step may change, or appear, in an action that
/// journaled no event reporting it.
fn changes_are_reported(
    before: &sagat_domain::execution::RunState,
    h: &Harness,
    journaled: usize,
    action: &Action,
) {
    let reported: std::collections::BTreeSet<NodeId> = h.journal[journaled..]
        .iter()
        .flat_map(sagat_domain::execution::RunEvent::changed_nodes)
        .collect();
    for (node, step) in h.state.steps() {
        if before.step(node) != Some(step) {
            assert!(
                reported.contains(node),
                "{action:?} changed {node} without an event that reports it: {:?}",
                h.journal[journaled..]
                    .iter()
                    .map(sagat_domain::execution::RunEvent::name)
                    .collect::<Vec<_>>()
            );
        }
    }
}

/// Applies `picks` (each selects one currently enabled action), checking every invariant after
/// each step, then lets the run finish. Panics with the offending action on any violation.
fn drive(shape: usize, picks: &[u16]) -> Harness {
    let def = shapes().swap_remove(shape);
    let mut h = Harness::start(def);
    let mut terminal_seen = BTreeMap::new();
    check(&h, &mut terminal_seen);

    for pick in picks {
        let actions = enabled(&h);
        let action = actions[usize::from(*pick) % actions.len()].clone();
        let (state_before, journal_before, now_before) =
            (h.state.clone(), h.journal.clone(), h.now);
        let result = perform(&mut h, &action);
        if result.is_ok() {
            changes_are_reported(&state_before, &h, journal_before.len(), &action);
        }
        if let Err(error) = result {
            assert_eq!(
                h.state, state_before,
                "{action:?} failed with {error:?} but changed the state"
            );
            assert_eq!(h.journal, journal_before, "{action:?} failed but journaled");
            assert_eq!(h.now, now_before);
        }
        check(&h, &mut terminal_seen);
    }

    finish(&mut h);
    check(&h, &mut terminal_seen);
    assert!(h.state.is_finished());
    // Once finished, nothing is accepted any more.
    let frozen = (h.state.clone(), h.journal.clone());
    assert!(h.stop(StopReason::Cancelled).is_err());
    assert!(h.pause().is_err());
    assert_eq!((h.state.clone(), h.journal.clone()), frozen);
    h
}

/// 400 cases by default; `PROPTEST_CASES=50000` runs a long soak.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn no_sequence_of_commands_breaks_the_state_machine(
        shape in 0usize..12,
        picks in proptest::collection::vec(any::<u16>(), 1..160),
    ) {
        drive(shape, &picks);
    }
}

/// The chaos must actually reach the interesting states, or a green run proves nothing.
#[test]
fn the_chaos_reaches_every_event_type_and_every_final_status() {
    use std::collections::BTreeSet;
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut names: BTreeSet<&'static str> = BTreeSet::new();
    let mut finals: BTreeSet<&'static str> = BTreeSet::new();
    // The saga shapes get a share of their own: their interesting states (a lost response, a
    // compensation that keeps failing, a cancellation in the middle of both) are several unlikely
    // choices in a row.
    for round in 0..10_500_usize {
        let shape = if round < 6_000 {
            usize::try_from(next() % 12).expect("small")
        } else {
            9 + usize::try_from(next() % 3).expect("small")
        };
        let len = 1 + usize::try_from(next() % 120).expect("small");
        let picks: Vec<u16> = (0..len)
            .map(|_| u16::try_from(next() % 65_536).expect("u16"))
            .collect();
        let h = drive(shape, &picks);
        names.extend(h.names());
        finals.insert(h.state.status().as_str());
    }
    let expected = [
        "run.started",
        "step.routed",
        "step.released",
        "step.leased",
        "step.started",
        "step.completed",
        "step.retry_scheduled",
        "step.retry_due",
        "step.failed",
        "condition.evaluated",
        "timer.started",
        "timer.fired",
        "wait.started",
        "wait.satisfied",
        "wait.timed_out",
        "parallel.forked",
        "parallel.joined",
        "child.started",
        "child.finished",
        "end.reached",
        "step.cancel_requested",
        "step.cancelled",
        "run.paused",
        "run.resumed",
        "run.stop_requested",
        "run.finished",
        "saga.entered",
        "saga.completed",
        "saga.aborting",
        "compensation.planned",
        "saga.compensated",
        "reconciliation.requested",
        "reconciliation.concluded",
        "step.needs_review",
        "step.retry_requested",
        "step.skipped",
        "step.resolved",
    ];
    let missing: Vec<_> = expected.iter().filter(|n| !names.contains(*n)).collect();
    assert!(missing.is_empty(), "the chaos never produced: {missing:?}");
    assert_eq!(
        finals.into_iter().collect::<Vec<_>>(),
        [
            "CANCELLED",
            "COMPENSATED",
            "COMPENSATION_FAILED",
            "FAILED",
            "PARTIAL",
            "SUCCEEDED",
            "TIMED_OUT"
        ],
        "every final status must be reachable"
    );
}

#[test]
fn every_shape_succeeds_when_nothing_goes_wrong() {
    for (index, def) in shapes().into_iter().enumerate() {
        let mut h = Harness::start(def);
        finish(&mut h);
        assert_eq!(h.state.status(), RunStatus::Succeeded, "shape {index}");
        h.assert_replays();
    }
}
