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
use super::event::{ChildOutcome, CorrelationKey, FailureCode, Fence, RunEvent, StopReason};
use super::plan::{Evaluation, Plan, after, plan};
use super::state::{RunId, RunState, StepState, StepStatus, WaitingOn};
use crate::workflow::{NodeId, NodeKind, WorkflowDefinition};

/// The events a command produced and, if the planner got stuck on one, the evaluation it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub events: Vec<RunEvent>,
    pub pending: Option<Evaluation>,
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
    let budget = 64 + 16 * def.len();
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
    if state.is_finished() || state.is_paused() || state.stop_reason().is_some() {
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
    let event = if step.cancel_requested() {
        // The run asked this attempt to stop; the retry budget is irrelevant.
        RunEvent::StepCancelled { node: node.clone() }
    } else {
        let policy = def
            .node(node)
            .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))?
            .retry_policy();
        if retryable && policy.has_attempts_left(step.attempt()) {
            RunEvent::StepRetryScheduled {
                node: node.clone(),
                attempt: fence.attempt,
                generation: fence.generation,
                code,
                not_before: after(now, policy.delay_after(step.attempt(), jitter_sample))?,
            }
        } else {
            RunEvent::StepFailed {
                node: node.clone(),
                code,
                fence: Some(fence),
            }
        }
    };
    apply_then_settle(def, state, now, event)
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
