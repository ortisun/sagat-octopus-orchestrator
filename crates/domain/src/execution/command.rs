//! Commands: the operations the application performs on a run.
//!
//! Each command validates its primary event against the current state (fencing, status, due
//! times), applies it, then lets the planner cascade (routing, joins, releasing the next step)
//! until nothing more can happen without the outside world. The events produced are returned in
//! order and are exactly what must be journaled, in one transaction, with the state they produced.
//!
//! Anything that needs data (a condition, a correlation key) comes back as
//! [`Progress::pending`]; the caller resolves it with [`resolve`] and continues.

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::event::{
    AddressedRunEvent, ChildOutcome, CorrelationKey, FailureCode, Fence, ReconcileResult,
    ReviewOutcome, RunEvent, StopReason, SwitchSelection,
};
use super::instance::ExecutionInstanceId;
use super::plan::{AddressedPlan, Evaluation, Plan, after, plan, plan_addressed};
use super::sagas::policies;
use super::state::{RunId, RunState, SagaPhase, StepState, StepStatus, WaitingOn};
use crate::saga::{NodeOrigin, OnCompensationFailure, ReconcileVerdict};
use crate::workflow::{NodeId, NodeKind, Scope, WorkflowDefinition};

/// The events a command produced and, if the planner got stuck on one, the evaluation it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub events: Vec<RunEvent>,
    pub pending: Option<Evaluation>,
}

/// Journal entries produced while advancing one admitted structured path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressedProgress {
    pub events: Vec<AddressedRunEvent>,
}

/// The outcome of an evaluation, handed back by the adapter that performed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// `Err(())` means the expression failed (a typing or runtime error); the step then fails with
    /// `EXPRESSION_ERROR` instead of retrying, because the same data would fail again.
    Condition {
        node: NodeId,
        outcome: Result<bool, ()>,
    },
    Switch {
        node: NodeId,
        selection: Result<SwitchSelection, ()>,
    },
    Correlation {
        node: NodeId,
        key: Result<CorrelationKey, ()>,
    },
    /// The child run reserved for a `child` node, or why there can be none: the code is
    /// `EXPRESSION_ERROR` for an input that does not evaluate and `CHILD_DEPTH_EXCEEDED` for a
    /// nesting that is too deep.
    Child {
        node: NodeId,
        child: Result<RunId, FailureCode>,
    },
}

/// What a heartbeat tells the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    /// The run asks the worker to stop; it should abandon the work and report back.
    pub cancel_requested: bool,
    /// The first heartbeat of an attempt is journaled as `step.started`.
    pub event: Option<RunEvent>,
}

/// The heartbeat result for one explicitly addressed logical instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceHeartbeat {
    pub cancel_requested: bool,
    pub event: Option<AddressedRunEvent>,
}

/// Why an attempt ended without the worker reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// The lease ran out: the worker stopped heartbeating.
    LeaseExpired,
    /// The attempt exceeded the node's `timeout`.
    AttemptTimeout,
}

fn settle(
    def: &WorkflowDefinition,
    state: &mut RunState,
    now: OffsetDateTime,
    mut events: Vec<RunEvent>,
) -> Result<Progress, ExecutionError> {
    // Every iteration applies one event and the run has a bounded number of steps, so this is a
    // safety net against a planner bug, not a limit that real workflows approach.
    let budget = 64 + 16 * def.total_len();
    for _ in 0..budget {
        match plan(def, state, now)? {
            Plan::Emit(event) => {
                state.apply(&event)?;
                events.push(event);
            }
            Plan::Evaluate(evaluation) => {
                return Ok(Progress {
                    events,
                    pending: Some(evaluation),
                });
            }
            Plan::Idle => {
                return Ok(Progress {
                    events,
                    pending: None,
                });
            }
        }
    }
    Err(ExecutionError::Invariant("the planner did not settle"))
}

fn apply_then_settle(
    def: &WorkflowDefinition,
    state: &mut RunState,
    now: OffsetDateTime,
    primary: RunEvent,
) -> Result<Progress, ExecutionError> {
    state.apply(&primary)?;
    settle(def, state, now, vec![primary])
}

fn settle_addressed(
    def: &WorkflowDefinition,
    state: &mut RunState,
    path: &super::instance::InstancePath,
    mut events: Vec<AddressedRunEvent>,
) -> Result<AddressedProgress, ExecutionError> {
    let budget = 8 + def.total_len();
    for _ in 0..budget {
        match plan_addressed(def, state, path)? {
            AddressedPlan::Emit(event) => {
                state.apply_addressed(&event)?;
                events.push(event);
            }
            AddressedPlan::Idle => return Ok(AddressedProgress { events }),
        }
    }
    Err(ExecutionError::Invariant(
        "the addressed planner did not settle",
    ))
}

/// Looks a step up. A finished run answers "finished" before anything about the step: it is the
/// more useful thing to tell a late caller.
fn require_step<'a>(state: &'a RunState, node: &NodeId) -> Result<&'a StepState, ExecutionError> {
    if state.is_finished() {
        return Err(ExecutionError::RunFinished(state.status()));
    }
    state
        .step(node)
        .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))
}

/// The step must be leased to exactly this attempt and generation.
fn require_lease<'a>(
    state: &'a RunState,
    node: &NodeId,
    fence: Fence,
) -> Result<&'a StepState, ExecutionError> {
    let step = require_step(state, node)?;
    if !step.status().is_leased() {
        return Err(ExecutionError::IllegalTransition {
            node: node.clone(),
            status: step.status(),
            event: "lease check",
        });
    }
    if step.attempt() != fence.attempt || step.generation() != fence.generation {
        return Err(ExecutionError::StaleAttempt { node: node.clone() });
    }
    Ok(step)
}

fn require_instance<'a>(
    state: &'a RunState,
    instance: &ExecutionInstanceId,
) -> Result<&'a StepState, ExecutionError> {
    if state.is_finished() {
        return Err(ExecutionError::RunFinished(state.status()));
    }
    state
        .instance(instance)
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))
}

fn require_instance_lease<'a>(
    state: &'a RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
) -> Result<&'a StepState, ExecutionError> {
    let step = require_instance(state, instance)?;
    if !step.status().is_leased() {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "lease check",
        });
    }
    if step.attempt() != fence.attempt || step.generation() != fence.generation {
        return Err(ExecutionError::StaleInstanceAttempt {
            instance: instance.clone(),
        });
    }
    Ok(step)
}

/// Creates the state of a new run and schedules its entrypoint.
///
/// # Errors
/// [`ExecutionError`] only on an internal inconsistency.
pub fn start_run(
    def: &WorkflowDefinition,
    run_id: RunId,
    now: OffsetDateTime,
) -> Result<(RunState, Progress), ExecutionError> {
    let mut state = RunState::new(run_id);
    let started = RunEvent::RunStarted {
        entrypoint: def.entrypoint().clone(),
    };
    state.apply(&started)?;
    let progress = settle(def, &mut state, now, vec![started])?;
    Ok((state, progress))
}

/// Grants the lease on a ready step. The caller must already hold the run's row lock.
///
/// # Errors
/// - [`ExecutionError::RunNotRunnable`] when the run is paused, stopping or finished.
/// - [`ExecutionError::IllegalTransition`] when the step is not ready.
pub fn lease_step(
    state: &mut RunState,
    node: &NodeId,
) -> Result<(Fence, RunEvent), ExecutionError> {
    // A run that is being stopped starts nothing new, except the compensations and reconciliations
    // that its sagas are running to the end.
    if state.is_finished()
        || state.is_paused()
        || (state.stop_reason().is_some() && !node.is_derived())
    {
        return Err(ExecutionError::RunNotRunnable(state.status()));
    }
    let step = require_step(state, node)?;
    let fence = Fence {
        attempt: step.attempt() + 1,
        generation: step.generation() + 1,
    };
    let event = RunEvent::StepLeased {
        node: node.clone(),
        attempt: fence.attempt,
        generation: fence.generation,
    };
    state.apply(&event)?;
    Ok((fence, event))
}

/// Grants a lease to one scoped logical instance without invoking structured planning.
///
/// # Errors
/// Returns a typed instance error when the addressed instance is missing or not ready.
pub fn lease_instance(
    state: &mut RunState,
    instance: &ExecutionInstanceId,
) -> Result<(Fence, AddressedRunEvent), ExecutionError> {
    if instance.path().is_root() {
        let (fence, event) = lease_step(state, instance.node())?;
        return Ok((fence, AddressedRunEvent::root(event)));
    }
    if state.is_finished()
        || state.is_paused()
        || (state.stop_reason().is_some() && !instance.node().is_derived())
    {
        return Err(ExecutionError::RunNotRunnable(state.status()));
    }
    let step = require_instance(state, instance)?;
    if step.status() != StepStatus::Ready {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "step.leased",
        });
    }
    let fence = Fence {
        attempt: step.attempt() + 1,
        generation: step.generation() + 1,
    };
    let event = AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::StepLeased {
            node: instance.node().clone(),
            attempt: fence.attempt,
            generation: fence.generation,
        },
    )
    .map_err(|_| ExecutionError::Invariant("a step event lost its instance address"))?;
    state.apply_addressed(&event)?;
    Ok((fence, event))
}

/// Records a heartbeat. The first one of an attempt marks the step as started.
///
/// # Errors
/// [`ExecutionError::StaleAttempt`] or [`ExecutionError::IllegalTransition`] for a worker that no
/// longer holds the lease.
pub fn heartbeat(
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
) -> Result<Heartbeat, ExecutionError> {
    let step = require_lease(state, node, fence)?;
    let cancel_requested = step.cancel_requested();
    let event = if step.status() == StepStatus::Leased {
        let started = RunEvent::StepStarted {
            node: node.clone(),
            attempt: fence.attempt,
            generation: fence.generation,
        };
        state.apply(&started)?;
        Some(started)
    } else {
        None
    };
    Ok(Heartbeat {
        cancel_requested,
        event,
    })
}

/// Records a heartbeat for one scoped logical instance.
///
/// # Errors
/// Returns a typed instance error for a stale fence or illegal state.
pub fn heartbeat_instance(
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
) -> Result<InstanceHeartbeat, ExecutionError> {
    if instance.path().is_root() {
        let heartbeat = heartbeat(state, instance.node(), fence)?;
        return Ok(InstanceHeartbeat {
            cancel_requested: heartbeat.cancel_requested,
            event: heartbeat.event.map(AddressedRunEvent::root),
        });
    }
    let step = require_instance_lease(state, instance, fence)?;
    let cancel_requested = step.cancel_requested();
    let event = if step.status() == StepStatus::Leased {
        let started = AddressedRunEvent::in_path(
            instance.path().clone(),
            RunEvent::StepStarted {
                node: instance.node().clone(),
                attempt: fence.attempt,
                generation: fence.generation,
            },
        )
        .map_err(|_| ExecutionError::Invariant("a step event lost its instance address"))?;
        state.apply_addressed(&started)?;
        Some(started)
    } else {
        None
    };
    Ok(InstanceHeartbeat {
        cancel_requested,
        event,
    })
}

/// A worker reports success.
///
/// # Errors
/// [`ExecutionError::StaleAttempt`] / [`ExecutionError::IllegalTransition`] for a stale caller;
/// [`ExecutionError::RunFinished`] when the run ended meanwhile.
pub fn complete_step(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    if node.is_reconciliation() {
        return Err(ExecutionError::NeedsVerdict(node.clone()));
    }
    require_lease(state, node, fence)?;
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepCompleted {
            node: node.clone(),
            attempt: fence.attempt,
            generation: fence.generation,
        },
    )
}

/// Records success for one leased activity instance and advances its admitted path.
///
/// # Errors
/// Returns a typed stale-fence or scoped-planning error. Root work keeps using
/// [`complete_step`].
pub fn complete_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
) -> Result<AddressedProgress, ExecutionError> {
    if instance.path().is_root() {
        return Err(ExecutionError::Invariant(
            "root completion used the scoped command",
        ));
    }
    if instance.node().is_reconciliation() {
        return Err(ExecutionError::NeedsVerdict(instance.node().clone()));
    }
    require_instance_lease(state, instance, fence)?;
    let primary = AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::StepCompleted {
            node: instance.node().clone(),
            attempt: fence.attempt,
            generation: fence.generation,
        },
    )
    .map_err(|_| ExecutionError::Invariant("a completion lost its instance path"))?;
    state.apply_addressed(&primary)?;
    settle_addressed(def, state, instance.path(), vec![primary])
}

/// A failed attempt as reported by a worker or inferred by the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// The attempt and generation that held the lease.
    pub fence: Fence,
    pub code: FailureCode,
    /// Whether another attempt may help (the worker's judgement; the budget still applies).
    pub retryable: bool,
    /// A uniform sample in `[0, 1)` for the backoff jitter, drawn by the caller.
    pub jitter_sample: f64,
}

/// Whether the outcome of an attempt that ended with this code is unknown: the effect may or may
/// not have happened.
fn is_ambiguous(code: &FailureCode) -> bool {
    *code == FailureCode::lease_expired()
        || *code == FailureCode::attempt_timeout()
        || *code == FailureCode::outcome_unknown()
}

/// What a compensation does when its retries are spent, by the policy of the saga it belongs to.
fn compensation_failure_policy(
    def: &WorkflowDefinition,
    forward: &NodeId,
) -> OnCompensationFailure {
    def.scope_chain(forward)
        .find_map(|scope| match scope {
            Scope::Saga(saga) => Some(policies(def, saga).0),
            Scope::Parallel(_) => None,
        })
        .unwrap_or_default()
}

fn end_attempt(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    failure: Failure,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let Failure {
        fence,
        code,
        retryable,
        jitter_sample,
    } = failure;
    let step = require_lease(state, node, fence)?;
    let def_node = def
        .node(node)
        .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))?;
    // An attempt whose outcome is unknown must not be retried or compensated blindly: it is
    // reconciled when the activity says how, and otherwise waits for an operator if it is high risk.
    if *def_node.origin() == NodeOrigin::Declared
        && matches!(def_node.kind(), NodeKind::Activity { .. })
        && is_ambiguous(&code)
    {
        if def.node(&node.reconciliation()).is_some() {
            return apply_then_settle(
                def,
                state,
                now,
                RunEvent::ReconciliationRequested {
                    node: node.clone(),
                    fence,
                    cause: code,
                },
            );
        }
        if def_node.effect().high_risk {
            return apply_then_settle(
                def,
                state,
                now,
                RunEvent::StepNeedsReview {
                    node: node.clone(),
                    code,
                    fence: Some(fence),
                },
            );
        }
    }
    if step.cancel_requested() {
        // The run asked this attempt to stop; the retry budget is irrelevant.
        let cancelled = RunEvent::StepCancelled { node: node.clone() };
        // If the attempt ended without anyone knowing whether the effect happened, the step keeps
        // saying so: it is recorded as the attempt that was lost, which is what it was, and then
        // cancelled. A saga that compensates later still undoes an effect that may have happened,
        // whichever came first, the cancellation or the lost lease.
        if *def_node.origin() == NodeOrigin::Declared
            && matches!(def_node.kind(), NodeKind::Activity { .. })
            && (is_ambiguous(&code) || code == FailureCode::storage_quota_exceeded())
        {
            let lost = RunEvent::StepRetryScheduled {
                node: node.clone(),
                attempt: fence.attempt,
                generation: fence.generation,
                code,
                not_before: now,
            };
            state.apply(&lost)?;
            state.apply(&cancelled)?;
            return settle(def, state, now, vec![lost, cancelled]);
        }
        return apply_then_settle(def, state, now, cancelled);
    }
    let policy = def_node.retry_policy();
    if retryable && policy.has_attempts_left(step.attempt()) {
        let event = RunEvent::StepRetryScheduled {
            node: node.clone(),
            attempt: fence.attempt,
            generation: fence.generation,
            code,
            not_before: after(now, policy.delay_after(step.attempt(), jitter_sample))?,
        };
        return apply_then_settle(def, state, now, event);
    }
    // The retries are spent (or the error is final).
    let failed = RunEvent::StepFailed {
        node: node.clone(),
        code: code.clone(),
        fence: Some(fence),
    };
    match def_node.origin() {
        NodeOrigin::Compensation(forward)
            if compensation_failure_policy(def, forward) == OnCompensationFailure::ManualReview =>
        {
            apply_then_settle(
                def,
                state,
                now,
                RunEvent::StepNeedsReview {
                    node: node.clone(),
                    code,
                    fence: Some(fence),
                },
            )
        }
        NodeOrigin::Reconciliation(forward) => {
            // Nobody could find out what happened to the step the reconciliation was about.
            state.apply(&failed)?;
            let mut events = vec![failed];
            let waiting = state.step(forward).is_some_and(|s| {
                s.status() == StepStatus::Waiting
                    && matches!(s.waiting_on(), Some(WaitingOn::Reconciliation))
            });
            if waiting {
                let review = RunEvent::StepNeedsReview {
                    node: forward.clone(),
                    code: FailureCode::reconciliation_failed(),
                    fence: None,
                };
                state.apply(&review)?;
                events.push(review);
            }
            settle(def, state, now, events)
        }
        _ => apply_then_settle(def, state, now, failed),
    }
}

/// A worker reports a failed attempt.
///
/// # Errors
/// As [`complete_step`].
pub fn fail_step(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    failure: Failure,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    end_attempt(def, state, node, failure, now)
}

/// Whether the scope around `path` still goes forward: the collection admits iterations, the loop
/// runs or waits to run its next one. A step of a scope that is draining or compensating is not
/// retried by a lookup or by a person: the iteration drain would cancel it the moment it was granted.
fn scope_goes_forward(state: &RunState, path: &super::instance::InstancePath) -> bool {
    let Some(owner) = path.scope_owner() else {
        return true;
    };
    if let Some(collection) = state.collection(&owner) {
        return collection.phase() == super::state::CollectionPhase::Open;
    }
    if let Some(scope) = state.loop_state(&owner) {
        return matches!(
            scope.phase(),
            super::state::LoopPhase::Ready
                | super::state::LoopPhase::Running
                | super::state::LoopPhase::Delaying
        );
    }
    true
}

fn emit(
    state: &mut RunState,
    events: &mut Vec<AddressedRunEvent>,
    path: &super::instance::InstancePath,
    event: RunEvent,
) -> Result<(), ExecutionError> {
    let addressed = AddressedRunEvent::in_path(path.clone(), event)
        .map_err(|_| ExecutionError::Invariant("a step event lost its instance address"))?;
    state.apply_addressed(&addressed)?;
    events.push(addressed);
    Ok(())
}

/// Records a failed attempt for one admitted activity instance, with the policy of the root
/// (`end_attempt`) at the instance's path: an attempt with an unknown outcome is looked up (or waits
/// for a person) before anything is repeated, a compensation that is out of attempts waits for a
/// person, and a lookup that cannot be completed sends the step it was about to a person.
///
/// A worker that observes an addressed cancellation may acknowledge it with a failed report;
/// ordinary retries and terminal `onError` routing remain isolated to the addressed path.
pub fn fail_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    failure: Failure,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    if instance.path().is_root() {
        return Err(ExecutionError::Invariant(
            "root failure used the scoped command",
        ));
    }
    let Failure {
        fence,
        code,
        retryable,
        jitter_sample,
    } = failure;
    let step = require_instance_lease(state, instance, fence)?;
    let (attempt, cancel_requested) = (step.attempt(), step.cancel_requested());
    let declared = def
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    let path = instance.path();
    let node = instance.node();
    let reconciliation = ExecutionInstanceId::in_path(path.clone(), node.reconciliation());
    let is_forward_activity = *declared.origin() == NodeOrigin::Declared
        && matches!(declared.kind(), NodeKind::Activity { .. });
    let mut events = Vec::new();

    // An attempt whose outcome is unknown must not be retried or compensated blindly: it is
    // reconciled when the activity says how, and otherwise waits for an operator if it is high risk.
    if is_forward_activity && is_ambiguous(&code) {
        if def.node_for_instance(&reconciliation).is_some() {
            emit(
                state,
                &mut events,
                path,
                RunEvent::ReconciliationRequested {
                    node: node.clone(),
                    fence,
                    cause: code,
                },
            )?;
            return settle_addressed(def, state, path, events);
        }
        if declared.effect().high_risk {
            emit(
                state,
                &mut events,
                path,
                RunEvent::StepNeedsReview {
                    node: node.clone(),
                    code,
                    fence: Some(fence),
                },
            )?;
            return settle_addressed(def, state, path, events);
        }
    }
    if cancel_requested {
        // The run asked this attempt to stop; the retry budget is irrelevant. If the attempt ended
        // without anyone knowing whether the effect happened, the step keeps saying so: it is
        // recorded as the attempt that was lost, which is what it was, and then cancelled.
        if is_forward_activity
            && (is_ambiguous(&code) || code == FailureCode::storage_quota_exceeded())
        {
            emit(
                state,
                &mut events,
                path,
                RunEvent::StepRetryScheduled {
                    node: node.clone(),
                    attempt: fence.attempt,
                    generation: fence.generation,
                    code,
                    not_before: now,
                },
            )?;
        }
        emit(
            state,
            &mut events,
            path,
            RunEvent::StepCancelled { node: node.clone() },
        )?;
        return settle_addressed(def, state, path, events);
    }
    let policy = declared.retry_policy();
    if retryable && policy.has_attempts_left(attempt) {
        emit(
            state,
            &mut events,
            path,
            RunEvent::StepRetryScheduled {
                node: node.clone(),
                attempt: fence.attempt,
                generation: fence.generation,
                code,
                not_before: after(now, policy.delay_after(attempt, jitter_sample))?,
            },
        )?;
        return Ok(AddressedProgress { events });
    }
    // The retries are spent (or the error is final).
    match declared.origin() {
        // A scope compensates with the saga default: a compensation that fails for good waits for a
        // person, and nothing is assumed done (D386).
        NodeOrigin::Compensation(_) => emit(
            state,
            &mut events,
            path,
            RunEvent::StepNeedsReview {
                node: node.clone(),
                code,
                fence: Some(fence),
            },
        )?,
        NodeOrigin::Reconciliation(forward) => {
            emit(
                state,
                &mut events,
                path,
                RunEvent::StepFailed {
                    node: node.clone(),
                    code,
                    fence: Some(fence),
                },
            )?;
            // Nobody could find out what happened to the step the reconciliation was about.
            let forward_instance = ExecutionInstanceId::in_path(path.clone(), forward.clone());
            let waiting = state.instance(&forward_instance).is_some_and(|s| {
                s.status() == StepStatus::Waiting
                    && matches!(s.waiting_on(), Some(WaitingOn::Reconciliation))
            });
            if waiting {
                emit(
                    state,
                    &mut events,
                    path,
                    RunEvent::StepNeedsReview {
                        node: forward.clone(),
                        code: FailureCode::reconciliation_failed(),
                        fence: None,
                    },
                )?;
            }
        }
        NodeOrigin::Declared => emit(
            state,
            &mut events,
            path,
            RunEvent::StepFailed {
                node: node.clone(),
                code,
                fence: Some(fence),
            },
        )?,
    }
    settle_addressed(def, state, path, events)
}

/// The reconciliation of a scoped step answered. `instance` is the lookup (`<id>~r`) and holds the
/// lease that `fence` names; the step it is about is concluded in the same commit.
///
/// # Errors
/// As [`complete_instance`]; [`ExecutionError::Invariant`] when `instance` is not a lookup.
pub fn complete_reconciliation_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
    verdict: ReconcileVerdict,
    jitter_sample: f64,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let Some(forward) = instance
        .node()
        .forward()
        .filter(|_| instance.node().is_reconciliation())
    else {
        return Err(ExecutionError::Invariant(
            "a verdict was given for a node that is not a reconciliation",
        ));
    };
    if instance.path().is_root() {
        return Err(ExecutionError::Invariant(
            "root reconciliation used the scoped command",
        ));
    }
    require_instance_lease(state, instance, fence)?;
    let path = instance.path();
    let forward = ExecutionInstanceId::in_path(path.clone(), forward);
    let mut events = Vec::new();
    emit(
        state,
        &mut events,
        path,
        RunEvent::StepCompleted {
            node: instance.node().clone(),
            attempt: fence.attempt,
            generation: fence.generation,
        },
    )?;
    let waiting = state.instance(&forward).cloned().filter(|s| {
        s.status() == StepStatus::Waiting
            && matches!(s.waiting_on(), Some(WaitingOn::Reconciliation))
    });
    if let Some(step) = waiting {
        let policy = def
            .node_for_instance(&forward)
            .ok_or_else(|| ExecutionError::UnknownInstance(forward.clone()))?
            .retry_policy();
        let result = match verdict {
            ReconcileVerdict::Applied => ReconcileResult::Succeeded,
            ReconcileVerdict::NotApplied
                if scope_goes_forward(state, path) && policy.has_attempts_left(step.attempt()) =>
            {
                ReconcileResult::RetryAt(after(
                    now,
                    policy.delay_after(step.attempt(), jitter_sample),
                )?)
            }
            ReconcileVerdict::NotApplied => {
                ReconcileResult::Failed(FailureCode::reconciled_not_applied())
            }
            ReconcileVerdict::Unknown => {
                ReconcileResult::Review(FailureCode::reconciliation_unknown())
            }
        };
        emit(
            state,
            &mut events,
            path,
            RunEvent::ReconciliationConcluded {
                node: forward.node().clone(),
                verdict,
                result,
            },
        )?;
    }
    settle_addressed(def, state, path, events)
}

fn review_event(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    event: RunEvent,
) -> Result<AddressedProgress, ExecutionError> {
    if instance.path().is_root() {
        return Err(ExecutionError::Invariant(
            "a root decision used the scoped command",
        ));
    }
    require_instance(state, instance)?;
    let mut events = Vec::new();
    emit(state, &mut events, instance.path(), event)?;
    settle_addressed(def, state, instance.path(), events)
}

/// An operator asks for another attempt of a scoped step that waits for a decision.
///
/// # Errors
/// [`ExecutionError::IllegalInstanceTransition`] when the step does not wait for an operator (a
/// double command: the first one won) or belongs to a scope that stopped going forward.
pub fn retry_review_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    _now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let step = require_instance(state, instance)?;
    // The scope that stopped going forward will not run a forward step again: it would be cancelled
    // the moment it was granted, and the decision that matters there is whether the step happened.
    let forward = def
        .node_for_instance(instance)
        .is_some_and(|n| *n.origin() == NodeOrigin::Declared);
    if forward && !scope_goes_forward(state, instance.path()) {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "step.retry_requested",
        });
    }
    review_event(
        def,
        state,
        instance,
        RunEvent::StepRetryRequested {
            node: instance.node().clone(),
        },
    )
}

/// An operator holds a scoped step that has not started and puts it among the ones that wait for a
/// decision.
///
/// # Errors
/// [`ExecutionError::IllegalInstanceTransition`] when the step is not ready or waiting to retry.
pub fn mark_for_review_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    _now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let step = require_instance(state, instance)?;
    if instance.node().is_reconciliation()
        || !matches!(step.status(), StepStatus::Ready | StepStatus::RetryWait)
    {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "step.needs_review",
        });
    }
    review_event(
        def,
        state,
        instance,
        RunEvent::StepNeedsReview {
            node: instance.node().clone(),
            code: FailureCode::marked_for_review(),
            fence: None,
        },
    )
}

/// An operator gives up on a scoped compensation.
///
/// # Errors
/// As [`retry_review_instance`]; [`ExecutionError::Invariant`] for a node that is not a
/// compensation.
pub fn skip_review_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    _now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    review_event(
        def,
        state,
        instance,
        RunEvent::StepSkipped {
            node: instance.node().clone(),
        },
    )
}

/// An operator settles a scoped step that waits for a decision: it did happen, or it did not.
///
/// # Errors
/// As [`retry_review_instance`].
pub fn resolve_review_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    outcome: ReviewOutcome,
    _now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    review_event(
        def,
        state,
        instance,
        RunEvent::StepResolved {
            node: instance.node().clone(),
            outcome,
        },
    )
}

/// Fails a successful worker report whose atomic persistence exceeded the retained-content quota.
///
/// For a forward report, a new stop request is applied before the failed attempt is settled. This
/// prevents an `onError` edge from releasing more forward work while retaining the ordinary stop
/// drain: already owed saga compensation and reconciliation still run to completion. A failed
/// reconciliation instead retains the forward step for review. If the run is already stopping, its
/// original reason and code remain authoritative.
///
/// # Errors
/// As [`fail_step`], plus the stop transition errors of [`stop_run`].
pub fn fail_step_for_storage_quota(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    fail_step_after_storage_quota(
        def,
        state,
        node,
        fence,
        FailureCode::storage_quota_exceeded(),
        now,
    )
}

/// Preserves a worker-reported failure when routing that failure exceeded retained storage.
///
/// The run is stopped before the original, known failure is settled, so an `onError` route cannot
/// allocate the content that made the first transaction fail. The original failure code remains on
/// the step and does not imply that its external effect happened.
///
/// # Errors
/// As [`fail_step_for_storage_quota`].
pub fn fail_report_for_storage_quota(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    code: FailureCode,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    fail_step_after_storage_quota(def, state, node, fence, code, now)
}

fn fail_step_after_storage_quota(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    code: FailureCode,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let stop = (state.stop_reason().is_none() && !node.is_reconciliation()).then(|| {
        RunEvent::RunStopRequested {
            reason: StopReason::Failed,
            code: Some(FailureCode::storage_quota_exceeded()),
        }
    });
    if let Some(event) = stop.as_ref() {
        state.apply(event)?;
    }
    let mut progress = end_attempt(
        def,
        state,
        node,
        Failure {
            fence,
            code,
            retryable: false,
            jitter_sample: 0.0,
        },
        now,
    )?;
    if let Some(event) = stop {
        progress.events.insert(0, event);
    }
    Ok(progress)
}

/// The attempt ended without a report (lease expiry or attempt timeout). Both consume an attempt
/// and are retryable: the side effect may have happened, so the retry uses the same idempotency key.
///
/// # Errors
/// As [`complete_step`].
pub fn expire_attempt(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    cause: Expiry,
    jitter_sample: f64,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let code = match cause {
        Expiry::LeaseExpired => FailureCode::lease_expired(),
        Expiry::AttemptTimeout => FailureCode::attempt_timeout(),
    };
    let failure = Failure {
        fence,
        code,
        retryable: true,
        jitter_sample,
    };
    end_attempt(def, state, node, failure, now)
}

/// Ends an expired scoped attempt under its exact fence.
///
/// # Errors
/// Returns the same retry-policy and stale-fence errors as [`fail_instance`].
pub fn expire_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
    cause: Expiry,
    jitter_sample: f64,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let code = match cause {
        Expiry::LeaseExpired => FailureCode::lease_expired(),
        Expiry::AttemptTimeout => FailureCode::attempt_timeout(),
    };
    fail_instance(
        def,
        state,
        instance,
        Failure {
            fence,
            code,
            retryable: true,
            jitter_sample,
        },
        now,
    )
}

/// A step in `RETRY_WAIT` whose backoff has elapsed becomes ready again.
///
/// # Errors
/// [`ExecutionError::NotYetDue`] before `next_attempt_at`.
pub fn retry_due(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let step = require_step(state, node)?;
    if step.status() == StepStatus::RetryWait && step.next_attempt_at().is_some_and(|at| at > now) {
        return Err(ExecutionError::NotYetDue(node.clone()));
    }
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepRetryDue { node: node.clone() },
    )
}

/// Releases one scoped retry whose durable backoff has elapsed.
///
/// # Errors
/// Returns [`ExecutionError::NotYetDue`] before `next_attempt_at`, or a typed scoped transition
/// error when the candidate is stale.
pub fn retry_due_instance(
    def: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    if instance.path().is_root() {
        return Err(ExecutionError::Invariant(
            "root retry used the scoped command",
        ));
    }
    let step = state
        .instance(instance)
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    if step.status() == StepStatus::RetryWait && step.next_attempt_at().is_some_and(|at| at > now) {
        return Err(ExecutionError::NotYetDue(instance.node().clone()));
    }
    let event = AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::StepRetryDue {
            node: instance.node().clone(),
        },
    )
    .map_err(|_| ExecutionError::Invariant("a retry lost its instance path"))?;
    state.apply_addressed(&event)?;
    settle_addressed(def, state, instance.path(), vec![event])
}

/// A timer reached its due time.
///
/// # Errors
/// [`ExecutionError::NotYetDue`] early; [`ExecutionError::IllegalTransition`] when something else
/// (a cancellation) already resolved the step — the first commit wins.
pub fn fire_timer(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let step = require_step(state, node)?;
    if let Some(WaitingOn::Timer { due_at }) = step.waiting_on()
        && step.status() == StepStatus::Waiting
        && *due_at > now
    {
        return Err(ExecutionError::NotYetDue(node.clone()));
    }
    apply_then_settle(def, state, now, RunEvent::TimerFired { node: node.clone() })
}

/// Continues a run whose state changed outside the planner: a durable transaction (an approval's
/// outcome) activated a step and left it `PENDING`. Nothing is applied that the planner would not
/// have chosen itself, so a run with nothing to release is returned unchanged.
///
/// # Errors
/// [`ExecutionError`] when the planner cannot settle the state.
pub fn release_pending(
    def: &WorkflowDefinition,
    state: &mut RunState,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    settle(def, state, now, Vec::new())
}

/// The awaited event arrived. Allowed until the timeout is committed, even past the deadline.
///
/// # Errors
/// [`ExecutionError::IllegalTransition`] when the wait already ended.
pub fn satisfy_wait(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::WaitSatisfied { node: node.clone() },
    )
}

/// The wait's deadline passed.
///
/// # Errors
/// [`ExecutionError::NotYetDue`] early; [`ExecutionError::IllegalTransition`] when the event won.
pub fn time_out_wait(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let step = require_step(state, node)?;
    if let Some(WaitingOn::Event { deadline, .. }) = step.waiting_on()
        && step.status() == StepStatus::Waiting
        && *deadline > now
    {
        return Err(ExecutionError::NotYetDue(node.clone()));
    }
    let handled = matches!(
        def.node(node).map(crate::workflow::Node::kind),
        Some(NodeKind::WaitEvent {
            on_timeout: Some(_),
            ..
        })
    );
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::WaitTimedOut {
            node: node.clone(),
            handled,
        },
    )
}

/// Starts winding the run down: cancellation, deadline, or an unrecoverable failure.
///
/// # Errors
/// [`ExecutionError::AlreadyStopping`] / [`ExecutionError::RunFinished`].
pub fn stop_run(
    def: &WorkflowDefinition,
    state: &mut RunState,
    reason: StopReason,
    code: Option<FailureCode>,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    apply_then_settle(def, state, now, RunEvent::RunStopRequested { reason, code })
}

/// Stops a run whose automatic transition could not fit within retained storage.
///
/// A prior stop remains authoritative. In that case this only advances its existing drain; it
/// never replaces cancellation, timeout, or a prior failure with the storage failure.
///
/// # Errors
/// As [`stop_run`].
pub fn stop_for_storage_quota(
    def: &WorkflowDefinition,
    state: &mut RunState,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    if state.stop_reason().is_some() {
        settle(def, state, now, Vec::new())
    } else {
        stop_run(
            def,
            state,
            StopReason::Failed,
            Some(FailureCode::storage_quota_exceeded()),
            now,
        )
    }
}

/// Pauses the run: nothing new is released or leased until it is resumed.
///
/// # Errors
/// [`ExecutionError::AlreadyPaused`] / [`ExecutionError::AlreadyStopping`] / finished.
pub fn pause_run(state: &mut RunState) -> Result<RunEvent, ExecutionError> {
    state.apply(&RunEvent::RunPaused)?;
    Ok(RunEvent::RunPaused)
}

/// Resumes a paused run and lets the planner catch up on what was held back.
///
/// # Errors
/// [`ExecutionError::NotPaused`] / finished.
pub fn resume_run(
    def: &WorkflowDefinition,
    state: &mut RunState,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    apply_then_settle(def, state, now, RunEvent::RunResumed)
}

/// Feeds the answer to an [`Evaluation`] back in and continues.
///
/// # Errors
/// [`ExecutionError`] when the node is no longer waiting for that answer.
pub fn resolve(
    def: &WorkflowDefinition,
    state: &mut RunState,
    resolution: Resolution,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let fail = |node: NodeId| RunEvent::StepFailed {
        node,
        code: FailureCode::expression_error(),
        fence: None,
    };
    let event = match resolution {
        Resolution::Condition { node, outcome } => match outcome {
            Ok(outcome) => RunEvent::ConditionEvaluated { node, outcome },
            Err(()) => fail(node),
        },
        Resolution::Switch { node, selection } => match selection {
            Ok(selection) => {
                let Some(declared) = def.node(&node) else {
                    return Err(ExecutionError::UnknownStep(node));
                };
                let NodeKind::Switch { cases, .. } = declared.kind() else {
                    return Err(ExecutionError::Invariant(
                        "a switch result targeted a different node kind",
                    ));
                };
                if let SwitchSelection::Case(id) = &selection
                    && !cases.iter().any(|case| case.id() == id)
                {
                    return Err(ExecutionError::Invariant("switch selected an unknown case"));
                }
                RunEvent::SwitchEvaluated { node, selection }
            }
            Err(()) => fail(node),
        },
        Resolution::Correlation { node, key } => match key {
            Ok(correlation_key) => {
                let NodeKind::WaitEvent { timeout, .. } = def
                    .node(&node)
                    .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))?
                    .kind()
                else {
                    return Err(ExecutionError::Invariant(
                        "a correlation was resolved for a node that does not wait for an event",
                    ));
                };
                RunEvent::WaitStarted {
                    node,
                    correlation_key,
                    deadline: after(now, *timeout)?,
                }
            }
            Err(()) => fail(node),
        },
        Resolution::Child { node, child } => match child {
            Ok(child) => RunEvent::ChildStarted { node, child },
            Err(code) => RunEvent::StepFailed {
                node,
                code,
                fence: None,
            },
        },
    };
    apply_then_settle(def, state, now, event)
}

/// The child run of a waiting `child` step ended (or could not be started).
///
/// # Errors
/// [`ExecutionError::IllegalTransition`] when the step is not parked on that child any more (the
/// parent's own cancellation won: the first commit wins), [`ExecutionError::RunFinished`] when the
/// parent ended meanwhile.
pub fn finish_child(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    child: RunId,
    outcome: ChildOutcome,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    require_step(state, node)?;
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::ChildFinished {
            node: node.clone(),
            child,
            outcome,
        },
    )
}

/// Whether the step may be tried again after a reconciliation found that its effect did not happen:
/// not when the run is being stopped, and not when a saga around it has stopped its forward work.
fn may_retry_forward(def: &WorkflowDefinition, state: &RunState, forward: &NodeId) -> bool {
    state.stop_reason().is_none()
        && def.scope_chain(forward).all(|scope| match scope {
            Scope::Saga(saga) => state
                .saga(saga)
                .is_none_or(|s| s.phase() == SagaPhase::Forward),
            Scope::Parallel(_) => true,
        })
}

/// The reconciliation of a step answered. `node` is the reconciliation (`<id>~r`) and holds the lease
/// that `fence` names; the step it is about is concluded in the same commit.
///
/// # Errors
/// As [`complete_step`]; [`ExecutionError::Invariant`] when `node` is not a reconciliation.
pub fn complete_reconciliation(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    fence: Fence,
    verdict: ReconcileVerdict,
    jitter_sample: f64,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let Some(forward) = node.forward().filter(|_| node.is_reconciliation()) else {
        return Err(ExecutionError::Invariant(
            "a verdict was given for a node that is not a reconciliation",
        ));
    };
    require_lease(state, node, fence)?;
    let completed = RunEvent::StepCompleted {
        node: node.clone(),
        attempt: fence.attempt,
        generation: fence.generation,
    };
    state.apply(&completed)?;
    let mut events = vec![completed];
    let waiting = state.step(&forward).cloned().filter(|s| {
        s.status() == StepStatus::Waiting
            && matches!(s.waiting_on(), Some(WaitingOn::Reconciliation))
    });
    if let Some(step) = waiting {
        let policy = def
            .node(&forward)
            .ok_or_else(|| ExecutionError::UnknownStep(forward.clone()))?
            .retry_policy();
        let result = match verdict {
            ReconcileVerdict::Applied => ReconcileResult::Succeeded,
            ReconcileVerdict::NotApplied
                if may_retry_forward(def, state, &forward)
                    && policy.has_attempts_left(step.attempt()) =>
            {
                ReconcileResult::RetryAt(after(
                    now,
                    policy.delay_after(step.attempt(), jitter_sample),
                )?)
            }
            ReconcileVerdict::NotApplied => {
                ReconcileResult::Failed(FailureCode::reconciled_not_applied())
            }
            ReconcileVerdict::Unknown => {
                ReconcileResult::Review(FailureCode::reconciliation_unknown())
            }
        };
        let concluded = RunEvent::ReconciliationConcluded {
            node: forward,
            verdict,
            result,
        };
        state.apply(&concluded)?;
        events.push(concluded);
    }
    settle(def, state, now, events)
}

/// An operator asks for another attempt of a step that waits for a decision.
///
/// # Errors
/// [`ExecutionError::IllegalTransition`] when the step does not wait for an operator (a double
/// command: the first one won).
pub fn retry_review(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let step = require_step(state, node)?;
    // The body of a saga that stopped going forward will not run it again: it would be cancelled the
    // moment it was granted, and the decision that matters there is whether the step happened.
    if def
        .node(node)
        .is_some_and(|n| *n.origin() == NodeOrigin::Declared)
        && super::plan::in_abandoned_region(def, state, node)
    {
        return Err(ExecutionError::IllegalTransition {
            node: node.clone(),
            status: step.status(),
            event: "step.retry_requested",
        });
    }
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepRetryRequested { node: node.clone() },
    )
}

/// An operator holds a step that has not started (it is ready for a worker, or waits out the delay
/// of a retry) and puts it among the ones that wait for a decision: nothing more happens to it until
/// a person retries it, settles it or cancels the run.
///
/// # Errors
/// [`ExecutionError::UnknownNode`] for a step the run does not have;
/// [`ExecutionError::IllegalTransition`] when the step is not ready or waiting to retry (an attempt
/// in flight cannot be held, and a step that already waits for a person is not held twice).
pub fn mark_for_review(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    let step = require_step(state, node)?;
    // Said before the state is asked: an attempt in flight is not "stale", it is a step that cannot
    // be held, and the operator is told which.
    // A lookup is the platform's way of finding out whether the step it is for happened: a person
    // holds that step, and the lookup stays what it is, or the step would wait for an answer that a
    // person has declared will never come.
    if node.is_reconciliation()
        || !matches!(step.status(), StepStatus::Ready | StepStatus::RetryWait)
    {
        return Err(ExecutionError::IllegalTransition {
            node: node.clone(),
            status: step.status(),
            event: "step.needs_review",
        });
    }
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepNeedsReview {
            node: node.clone(),
            code: FailureCode::marked_for_review(),
            fence: None,
        },
    )
}

/// An operator gives up on a compensation.
///
/// # Errors
/// As [`retry_review`]; [`ExecutionError::Invariant`] for a node that is not a compensation.
pub fn skip_review(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    require_step(state, node)?;
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepSkipped { node: node.clone() },
    )
}

/// An operator settles a step that waits for a decision: it did happen, or it did not.
///
/// # Errors
/// As [`retry_review`].
pub fn resolve_review(
    def: &WorkflowDefinition,
    state: &mut RunState,
    node: &NodeId,
    outcome: ReviewOutcome,
    now: OffsetDateTime,
) -> Result<Progress, ExecutionError> {
    require_step(state, node)?;
    apply_then_settle(
        def,
        state,
        now,
        RunEvent::StepResolved {
            node: node.clone(),
            outcome,
        },
    )
}
