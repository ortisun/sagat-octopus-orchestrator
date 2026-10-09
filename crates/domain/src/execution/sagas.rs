//! What the planner does about sagas (feature 003).
//!
//! A saga moves through four phases, each one a journal event:
//!
//! 1. **Forward** — its body runs like any other part of the graph. If a step inside fails for good
//!    (or the run is stopped and the saga's policy compensates), `saga.aborting`.
//! 2. **Aborting** — the body is wound down: what has not started is cancelled, what is in flight is
//!    asked to stop (or awaited, by policy), and an attempt whose outcome is unknown is reconciled
//!    first. When nothing in the body is active, `compensation.planned` records which completed steps
//!    are to be undone and which of those each one has to wait for.
//! 3. **Compensating** — each compensation is released when everything it waits for has finished.
//!    Independent branches go in parallel.
//! 4. **Done** — `saga.compensated` with the outcome, derived from how the compensations ended.
//!
//! All of it is a pure function of the definition and the state; the events are the only record.

use std::collections::BTreeSet;

use super::error::ExecutionError;
use super::event::{FailureCode, PlannedEntry, RunEvent, StopReason};
use super::plan::{Plan, in_abandoned_region, waits_for_cancelled_child};
use super::state::{RunState, SagaPhase, StepState, StepStatus, WaitingOn};
use crate::saga::{OnCompensationFailure, SagaCancelPolicy, SagaOutcome, reverse_dependencies};
use crate::workflow::{NodeId, NodeKind, Scope, WorkflowDefinition};

/// The failure policy and the cancellation policy of a `saga` node.
pub(super) fn policies(
    def: &WorkflowDefinition,
    saga: &NodeId,
) -> (OnCompensationFailure, SagaCancelPolicy) {
    match def.node(saga).map(crate::workflow::Node::kind) {
        Some(NodeKind::Saga {
            on_compensation_failure,
            cancel_policy,
            ..
        }) => (*on_compensation_failure, *cancel_policy),
        _ => (
            OnCompensationFailure::default(),
            SagaCancelPolicy::default(),
        ),
    }
}

/// The sagas of the definition, inner ones before the ones that contain them.
fn inner_first(def: &WorkflowDefinition) -> Vec<&NodeId> {
    let mut sagas: Vec<(&NodeId, usize)> = def
        .sagas()
        .map(|saga| (saga, def.saga_region(saga).map_or(0, BTreeSet::len)))
        .collect();
    sagas.sort_by_key(|(saga, size)| (*size, (*saga).clone()));
    sagas.into_iter().map(|(saga, _)| saga).collect()
}

fn running(state: &RunState, saga: &NodeId) -> bool {
    state.step(saga).is_some_and(|step| {
        step.status() == StepStatus::Waiting && matches!(step.waiting_on(), Some(WaitingOn::Saga))
    })
}

/// Whether a saga handles the wind-down of `node` itself, so the generic one must leave it alone:
/// the node sits in the body of a saga that is past its forward phase (or is about to compensate
/// because the run is being stopped), or it is a compensation or reconciliation of such a node.
pub(super) fn owned(
    def: &WorkflowDefinition,
    state: &RunState,
    node: &NodeId,
    stopping: bool,
) -> bool {
    let forward = node.forward().unwrap_or_else(|| node.clone());
    let handles = |saga: &NodeId| {
        running(state, saga)
            && state.saga(saga).is_some_and(|s| {
                s.phase() != SagaPhase::Forward || (stopping && policies(def, saga).1.compensates())
            })
    };
    handles(&forward)
        || def
            .scope_chain(&forward)
            .any(|scope| matches!(scope, Scope::Saga(saga) if handles(saga)))
}

/// The code recorded as the cause of a compensation that the stop of the run started.
fn stop_cause(state: &RunState, reason: StopReason) -> FailureCode {
    match reason {
        StopReason::Cancelled => FailureCode::run_cancelled(),
        StopReason::TimedOut => FailureCode::run_timed_out(),
        _ => state
            .stop_code()
            .cloned()
            .unwrap_or_else(FailureCode::branch_failed),
    }
}

/// A failed step of the body that nothing inside handles: the saga must compensate.
fn unhandled_failure(
    def: &WorkflowDefinition,
    state: &RunState,
    saga: &NodeId,
) -> Option<FailureCode> {
    def.saga_region(saga)?.iter().find_map(|member| {
        let step = state.step(member)?;
        let unhandled = step.status() == StepStatus::Failed
            && def.node(member).is_some_and(|n| n.on_error().is_none())
            && def.enclosing_scope(member) == Some(&Scope::Saga(saga.clone()));
        unhandled.then(|| {
            step.failure()
                .cloned()
                .unwrap_or_else(FailureCode::branch_failed)
        })
    })
}

/// Whether the body of the saga has run to its end: every step it created is finished and routed.
fn body_done(def: &WorkflowDefinition, state: &RunState, saga: &NodeId) -> bool {
    let Some(region) = def.saga_region(saga) else {
        return false;
    };
    let mut any = false;
    for member in region {
        if let Some(step) = state.step(member) {
            any = true;
            if !(step.status().is_terminal() && step.routed()) {
                return false;
            }
        }
    }
    any
}

/// What winding down one step of an aborting body asks for. A step that waits for an answer the
/// saga needs (a reconciliation, a nested saga) is left alone; one that waits for an operator is
/// given up on when the run is being stopped.
fn wind_down(
    def: &WorkflowDefinition,
    node: &NodeId,
    step: &StepState,
    policy: SagaCancelPolicy,
    stopping: bool,
) -> Option<RunEvent> {
    match step.status() {
        StepStatus::Waiting if waits_for_cancelled_child(def, node, step) => {
            (!step.cancel_requested()).then(|| RunEvent::StepCancelRequested { node: node.clone() })
        }
        StepStatus::Pending | StepStatus::Ready | StepStatus::RetryWait => {
            Some(RunEvent::StepCancelled { node: node.clone() })
        }
        StepStatus::Waiting => match step.waiting_on() {
            Some(WaitingOn::Saga | WaitingOn::Reconciliation) => None,
            Some(WaitingOn::Operator { .. }) if !stopping => None,
            _ => Some(RunEvent::StepCancelled { node: node.clone() }),
        },
        StepStatus::Leased | StepStatus::Running
            if policy.cancels_in_flight() && !step.cancel_requested() =>
        {
            Some(RunEvent::StepCancelRequested { node: node.clone() })
        }
        _ => None,
    }
}

/// The compensations to run for `saga` and the steps whose effect cannot be undone.
///
/// A step is undone if it completed, and also if its last attempt ended with an unknown outcome:
/// the compensation is asked to undo an effect that may never have happened, so it must tolerate
/// that (it is idempotent by contract). An unknown outcome with nothing to compensate it with is
/// flagged when the step is irreversible or high risk, so the saga cannot claim `COMPENSATED`.
pub(super) fn plan_entries(
    def: &WorkflowDefinition,
    state: &RunState,
    saga: &NodeId,
) -> (Vec<PlannedEntry>, Vec<NodeId>) {
    let Some(region) = def.saga_region(saga) else {
        return (Vec::new(), Vec::new());
    };
    let mut entries: BTreeSet<NodeId> = BTreeSet::new();
    let mut irreversible: Vec<NodeId> = Vec::new();
    for member in region {
        let Some(step) = state.step(member) else {
            continue;
        };
        let Some(node) = def.node(member) else {
            continue;
        };
        let completed = step.status() == StepStatus::Succeeded;
        let unknown = matches!(step.status(), StepStatus::Cancelled | StepStatus::Failed)
            && step
                .failure()
                .is_some_and(FailureCode::leaves_outcome_unknown);
        if !(completed || unknown) {
            continue;
        }
        let compensable = def.node(&member.compensation()).is_some()
            && state.step(&member.compensation()).is_none();
        if compensable {
            entries.insert(member.clone());
        }
        let cannot_undo =
            node.effect().irreversible || (unknown && !compensable && node.effect().high_risk);
        if cannot_undo {
            irreversible.push(member.clone());
        }
    }
    let after = reverse_dependencies(def, &entries);
    let planned = entries
        .into_iter()
        .map(|node| PlannedEntry {
            after: after.get(&node).cloned().unwrap_or_default(),
            node,
        })
        .collect();
    (planned, irreversible)
}

/// Whether the run is being ended by somebody's decision or by the clock, and not by one of its own
/// failures. Only then is a person who was asked to decide something given up on: when a run is
/// cancelled nobody is going to decide a compensation, but when it ends because *another branch*
/// failed, the people the sagas ask are still the only ones who can say what to do.
const fn ended_from_outside(stop: Option<StopReason>) -> bool {
    matches!(stop, Some(StopReason::Cancelled | StopReason::TimedOut))
}

/// Whether the operators who were asked to decide a *compensation* are given up on: the run is
/// cancelled, or passes its deadline, *after* the saga started to compensate. A compensation that
/// fails because of the stop itself waits for an operator like any other.
fn give_up(stop: Option<StopReason>, saga: &super::state::SagaState) -> bool {
    ended_from_outside(stop) && !saga.during_stop()
}

/// The next thing the sagas of the run need, if there is one. `stop` is why the run is being
/// stopped, when it is.
pub(super) fn advance(
    def: &WorkflowDefinition,
    state: &RunState,
    stop: Option<StopReason>,
) -> Result<Option<Plan>, ExecutionError> {
    for saga in inner_first(def) {
        if !running(state, saga) {
            continue;
        }
        let Some(current) = state.saga(saga) else {
            continue;
        };
        let (_, policy) = policies(def, saga);
        match current.phase() {
            SagaPhase::Forward => {
                if let Some(reason) = stop {
                    if policy.compensates() {
                        return Ok(Some(Plan::Emit(RunEvent::SagaAborting {
                            node: saga.clone(),
                            cause: stop_cause(state, reason),
                        })));
                    }
                    continue;
                }
                // The branch this saga was in lost its join: what the saga did is compensated, not
                // dropped.
                if policy.compensates() && in_abandoned_region(def, state, saga) {
                    return Ok(Some(Plan::Emit(RunEvent::SagaAborting {
                        node: saga.clone(),
                        cause: FailureCode::branch_failed(),
                    })));
                }
                if let Some(cause) = unhandled_failure(def, state, saga) {
                    return Ok(Some(Plan::Emit(RunEvent::SagaAborting {
                        node: saga.clone(),
                        cause,
                    })));
                }
                if body_done(def, state, saga) {
                    return Ok(Some(Plan::Emit(RunEvent::SagaCompleted {
                        node: saga.clone(),
                    })));
                }
            }
            SagaPhase::Aborting => {
                // Winding the body down gives up on an operator when the run is cancelled or times
                // out: even when that is what started the abort, nobody is left to decide a forward
                // step (it is compensated defensively if its outcome was unknown). A run that ends
                // because of one of its own failures leaves the people the sagas ask to decide.
                if let Some(plan) = abort(def, state, saga, policy, ended_from_outside(stop)) {
                    return Ok(Some(plan));
                }
            }
            SagaPhase::Compensating => {
                if let Some(plan) = compensate(state, saga, give_up(stop, current)) {
                    return Ok(Some(plan));
                }
            }
            SagaPhase::Done => {}
        }
    }
    Ok(None)
}

fn abort(
    def: &WorkflowDefinition,
    state: &RunState,
    saga: &NodeId,
    policy: SagaCancelPolicy,
    stopping: bool,
) -> Option<Plan> {
    let region = def.saga_region(saga)?;
    let cause = state.saga(saga)?.cause()?.clone();
    // A saga inside this one goes down with it.
    for inner in def.sagas() {
        if region.contains(inner)
            && running(state, inner)
            && state
                .saga(inner)
                .is_some_and(|s| s.phase() == SagaPhase::Forward)
        {
            return Some(Plan::Emit(RunEvent::SagaAborting {
                node: inner.clone(),
                cause,
            }));
        }
    }
    for member in region {
        let Some(step) = state.step(member) else {
            continue;
        };
        if let Some(event) = wind_down(def, member, step, policy, stopping) {
            return Some(Plan::Emit(event));
        }
        // The answer a reconciliation is looking for decides whether the step is compensated.
        let reconciliation = member.reconciliation();
        if state
            .step(&reconciliation)
            .is_some_and(|s| s.status() == StepStatus::Pending)
        {
            return Some(Plan::Emit(RunEvent::StepReleased {
                node: reconciliation,
            }));
        }
    }
    let settled = region
        .iter()
        .filter_map(|member| state.step(member))
        .all(|step| step.status().is_terminal());
    if !settled {
        return None;
    }
    let (entries, irreversible) = plan_entries(def, state, saga);
    Some(Plan::Emit(RunEvent::CompensationPlanned {
        node: saga.clone(),
        entries,
        irreversible,
    }))
}

fn compensate(state: &RunState, saga: &NodeId, stopping: bool) -> Option<Plan> {
    let current = state.saga(saga)?;
    let status_of = |forward: &NodeId| state.step(&forward.compensation()).map(StepState::status);
    let broken = |forward: &NodeId| {
        matches!(
            status_of(forward),
            Some(StepStatus::Failed | StepStatus::Cancelled)
        )
    };
    // The run is being stopped: nobody is going to decide what a compensation that waits for an
    // operator should do, so it is given up on and the saga ends as failed.
    if stopping {
        for forward in current.entries().keys() {
            let step = state.step(&forward.compensation());
            if step.is_some_and(|s| {
                s.status() == StepStatus::Waiting
                    && matches!(s.waiting_on(), Some(WaitingOn::Operator { .. }))
            }) {
                return Some(Plan::Emit(RunEvent::StepCancelled {
                    node: forward.compensation(),
                }));
            }
        }
    }
    for (forward, after) in current.entries() {
        if status_of(forward) != Some(StepStatus::Pending) {
            continue;
        }
        // Whatever waits for a compensation that will not happen cannot be released either, so it
        // is abandoned and the saga ends as failed. A compensation is only ever failed or cancelled
        // for good: because the saga does not wait for a human, because an operator declared it
        // failed, or because the run was cancelled while it waited for one.
        if after.iter().any(&broken) {
            return Some(Plan::Emit(RunEvent::StepCancelled {
                node: forward.compensation(),
            }));
        }
        let ready = after.iter().all(|dep| {
            matches!(
                status_of(dep),
                Some(StepStatus::Succeeded | StepStatus::Skipped)
            )
        });
        if ready {
            return Some(Plan::Emit(RunEvent::StepReleased {
                node: forward.compensation(),
            }));
        }
    }
    let finished = current
        .entries()
        .keys()
        .all(|forward| status_of(forward).is_some_and(StepStatus::is_terminal));
    finished.then(|| {
        Plan::Emit(RunEvent::SagaCompensated {
            node: saga.clone(),
            outcome: state.expected_outcome(current),
        })
    })
}

/// How the failure at `node` ended, if a saga is behind it: the worst outcome of the saga that
/// failed and of the sagas nested in it. A `parallel` node that failed because a branch failed
/// takes the worst of its failed branches.
pub(super) fn origin_outcome(
    def: &WorkflowDefinition,
    state: &RunState,
    node: &NodeId,
) -> Option<SagaOutcome> {
    match def.node(node)?.kind() {
        NodeKind::Saga { .. } => {
            let mut worst = state.saga(node)?.outcome();
            for inner in def.sagas() {
                if def.saga_region(node).is_some_and(|r| r.contains(inner)) {
                    worst = worst.max(state.saga(inner).and_then(|s| s.outcome()));
                }
            }
            worst
        }
        NodeKind::Parallel { .. } => def
            .branch_regions(node)?
            .iter()
            .flatten()
            .filter(|member| {
                state
                    .step(member)
                    .is_some_and(|s| s.status() == StepStatus::Failed)
                    && def.enclosing_scope(member) == Some(&Scope::Parallel(node.clone()))
            })
            .filter_map(|member| origin_outcome(def, state, member))
            .max(),
        _ => None,
    }
}
