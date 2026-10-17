//! The planner: given a definition and the current state, what must happen next?
//!
//! [`plan`] is a pure function of `(definition, state, now)`. It returns at most one action; the
//! caller applies it and asks again until the planner is [`Plan::Idle`]. Anything that needs the
//! outside world (evaluating an expression over the run's data) is returned as an
//! [`Evaluation`] for an adapter to resolve, and the answer comes back as a journal event, so a
//! replay never has to evaluate anything again.

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::event::{
    AddressedRunEvent, FailureCode, JoinOutcome, RunEvent, StopReason, SwitchSelection,
};
use super::instance::InstancePath;
use super::sagas;
use super::state::{RunState, SagaPhase, StepState, StepStatus, WaitingOn};
use crate::saga::{NodeOrigin, SagaOutcome};
use crate::workflow::{
    CancelPolicy, Expression, JoinPolicy, NodeId, NodeKind, Scope, SwitchCase, WorkflowDefinition,
    WorkflowLimits,
};

/// What the engine needs to know before it can continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evaluation {
    /// Evaluate a boolean expression over the run's data.
    Condition {
        node: NodeId,
        expression: Expression,
    },
    /// Evaluate ordered predicates until one is true, or choose the default.
    Switch {
        node: NodeId,
        cases: Vec<SwitchCase>,
    },
    /// Evaluate the correlation expression of a `waitEvent` node to a string key.
    Correlation {
        node: NodeId,
        expression: Expression,
    },
    /// Prepare the child run of a `child` node: evaluate its input (when the node defines one),
    /// check the nesting depth and reserve the child's id.
    Child {
        node: NodeId,
        input: Option<Expression>,
    },
}

/// One planning decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Apply this event, then plan again.
    Emit(RunEvent),
    /// Resolve this evaluation, apply the resulting event, then plan again.
    Evaluate(Evaluation),
    /// Nothing more can happen without an external event (a worker, a timer, a signal).
    Idle,
}

/// One deterministic planning decision within an already admitted execution path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressedPlan {
    Emit(AddressedRunEvent),
    Idle,
}

/// Advances activity routing within one admitted structured path.
///
/// Structured control nodes keep their own transactional owners. This planner releases activities
/// and routes their recorded success or failure while leaving a pending structured node idle for
/// its dedicated transition.
pub fn plan_addressed(
    def: &WorkflowDefinition,
    state: &RunState,
    path: &InstancePath,
) -> Result<AddressedPlan, ExecutionError> {
    if path.is_root() {
        return Err(ExecutionError::Invariant(
            "addressed planning requires a scoped path",
        ));
    }
    for (instance, step) in state
        .instances()
        .filter(|(instance, _)| instance.path() == path)
    {
        if !matches!(
            step.status(),
            StepStatus::Succeeded | StepStatus::Failed | StepStatus::Skipped
        ) || step.routed()
        {
            continue;
        }
        let declared = def
            .node_for_instance(instance)
            .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
        let activated = if declared.origin() != &NodeOrigin::Declared {
            Vec::new()
        } else if step.status() == StepStatus::Failed {
            declared.on_error().cloned().into_iter().collect()
        } else {
            declared.next().cloned().into_iter().collect()
        };
        return addressed(
            path,
            RunEvent::StepRouted {
                node: instance.node().clone(),
                activated,
            },
        );
    }
    for (instance, step) in state
        .instances()
        .filter(|(instance, _)| instance.path() == path)
    {
        if step.status() == StepStatus::Pending {
            let declared = def
                .node_for_instance(instance)
                .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
            // A compensation is released by its scope, when everything it waits for has finished.
            if matches!(declared.origin(), NodeOrigin::Compensation(_)) {
                continue;
            }
            return match declared.kind() {
                NodeKind::Activity { .. } => addressed(
                    path,
                    RunEvent::StepReleased {
                        node: instance.node().clone(),
                    },
                ),
                NodeKind::ForEach { .. }
                | NodeKind::Approval { .. }
                | NodeKind::While { .. }
                | NodeKind::RepeatUntil { .. }
                | NodeKind::Return { .. } => Ok(AddressedPlan::Idle),
                _ => Err(ExecutionError::UnsupportedScopedEvent(
                    "scoped non-activity planning",
                )),
            };
        }
    }
    Ok(AddressedPlan::Idle)
}

fn addressed(path: &InstancePath, event: RunEvent) -> Result<AddressedPlan, ExecutionError> {
    AddressedRunEvent::in_path(path.clone(), event)
        .map(AddressedPlan::Emit)
        .map_err(|_| ExecutionError::Invariant("an addressed plan lost its instance path"))
}

pub(super) fn after(
    now: OffsetDateTime,
    delay: std::time::Duration,
) -> Result<OffsetDateTime, ExecutionError> {
    time::Duration::try_from(delay)
        .ok()
        .and_then(|d| now.checked_add(d))
        .ok_or(ExecutionError::Invariant("a duration overflowed the clock"))
}

/// Decides the next action for `state`.
///
/// # Errors
/// [`ExecutionError`] when `state` refers to a node the definition does not have (the state belongs
/// to another version) or a duration overflows.
pub fn plan(
    def: &WorkflowDefinition,
    state: &RunState,
    now: OffsetDateTime,
) -> Result<Plan, ExecutionError> {
    if state.is_finished() {
        return Ok(Plan::Idle);
    }
    if let Some(reason) = state.stop_reason() {
        return drain(def, state, reason);
    }
    if state.is_paused() {
        return Ok(Plan::Idle);
    }
    if let Some(action) = start_pending(def, state, now)? {
        return Ok(action);
    }
    if let Some(action) = settle_joins(def, state)? {
        return Ok(action);
    }
    if let Some(action) = sagas::advance(def, state, None)? {
        return Ok(action);
    }
    if let Some(event) = cancel_joined_leftovers(def, state) {
        return Ok(Plan::Emit(event));
    }
    if let Some(action) = route_finished(def, state)? {
        return Ok(action);
    }
    let all_done = state.has_started()
        && state
            .instances()
            .all(|(_, step)| step.status().is_terminal() && step.routed());
    Ok(if all_done {
        Plan::Emit(RunEvent::RunFinished {
            status: super::state::RunStatus::Succeeded,
        })
    } else {
        Plan::Idle
    })
}

fn node_of<'a>(
    def: &'a WorkflowDefinition,
    node: &NodeId,
) -> Result<&'a crate::workflow::Node, ExecutionError> {
    def.node(node)
        .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))
}

/// The run is being stopped: the sagas compensate what they are to, then what is not in flight is
/// cancelled, what is gets asked to stop, and the run finishes.
fn drain(
    def: &WorkflowDefinition,
    state: &RunState,
    reason: StopReason,
) -> Result<Plan, ExecutionError> {
    if let Some(action) = sagas::advance(def, state, Some(reason))? {
        return Ok(action);
    }
    for (node, step) in state.steps() {
        // A saga that is compensating winds down its own body, and its compensations must run to
        // the end whatever stopped the run.
        if sagas::owned(def, state, node, true) {
            continue;
        }
        if matches!(step.waiting_on(), Some(WaitingOn::Collection)) {
            let collection = state
                .collection(&super::ExecutionInstanceId::root(node.clone()))
                .ok_or(ExecutionError::Invariant(
                    "a collection wait has no durable scope",
                ))?;
            match collection.phase() {
                super::CollectionPhase::Open if reason == StopReason::Cancelled => {
                    continue;
                }
                super::CollectionPhase::Draining => continue,
                super::CollectionPhase::Open => {
                    return Err(ExecutionError::UnsupportedStructuredNode(node.clone()));
                }
                super::CollectionPhase::Succeeded
                | super::CollectionPhase::Failed
                | super::CollectionPhase::Cancelled => {}
            }
        }
        if matches!(step.waiting_on(), Some(WaitingOn::Loop)) {
            let loop_state = state
                .loop_state(&super::ExecutionInstanceId::root(node.clone()))
                .ok_or(ExecutionError::Invariant(
                    "a loop wait has no durable scope",
                ))?;
            match loop_state.phase() {
                // A cancelled loop is drained by the scheduler under its own fence, as a collection
                // is; the run's deadline over a running loop stays unsupported (O382).
                super::LoopPhase::Ready
                | super::LoopPhase::Running
                | super::LoopPhase::Delaying
                    if reason == StopReason::Cancelled =>
                {
                    continue;
                }
                super::LoopPhase::Ready
                | super::LoopPhase::Running
                | super::LoopPhase::Delaying => {
                    return Err(ExecutionError::UnsupportedStructuredNode(node.clone()));
                }
                // The scheduler drains the active iteration and settles the owner under its own
                // fence; cancelling the owner here would take the drain out from under it.
                super::LoopPhase::Draining => continue,
                super::LoopPhase::Succeeded
                | super::LoopPhase::Failed
                | super::LoopPhase::Cancelled => {}
            }
        }
        match step.status() {
            // A step parked on a child that the policy asks to be cancelled *and awaited* is asked
            // to stop like an attempt in flight, then left alone until the child has settled.
            StepStatus::Waiting if waits_for_cancelled_child(def, node, step) => {
                if !step.cancel_requested() {
                    return Ok(Plan::Emit(RunEvent::StepCancelRequested {
                        node: node.clone(),
                    }));
                }
            }
            StepStatus::Pending
            | StepStatus::Ready
            | StepStatus::RetryWait
            | StepStatus::Waiting => {
                return Ok(Plan::Emit(RunEvent::StepCancelled { node: node.clone() }));
            }
            StepStatus::Leased | StepStatus::Running if !step.cancel_requested() => {
                return Ok(Plan::Emit(RunEvent::StepCancelRequested {
                    node: node.clone(),
                }));
            }
            _ => {}
        }
    }
    Ok(
        if state.steps().all(|(_, step)| step.status().is_terminal()) {
            Plan::Emit(RunEvent::RunFinished {
                status: state.final_status_for(reason),
            })
        } else {
            Plan::Idle
        },
    )
}

/// Whether `step` is parked on a child run whose node says the parent must wait for the child to
/// settle after asking it to stop.
pub(super) fn waits_for_cancelled_child(
    def: &WorkflowDefinition,
    node: &NodeId,
    step: &StepState,
) -> bool {
    matches!(step.waiting_on(), Some(WaitingOn::Child { .. }))
        && matches!(
            def.node(node).map(crate::workflow::Node::kind),
            Some(NodeKind::Child {
                cancel_policy: CancelPolicy::RequestCancel,
                ..
            })
        )
}

/// The run's instance ceiling when its author lowered it; `None` while it is the platform's, which
/// no acyclic root graph can reach and which the store alone guards.
fn lowered_instance_limit(def: &WorkflowDefinition) -> Option<usize> {
    let limit = usize::try_from(def.limits().max_instances()).ok()?;
    (limit < usize::try_from(WorkflowLimits::default().max_instances()).ok()?).then_some(limit)
}

/// Gives a freshly routed step whatever it needs to make progress.
fn start_pending(
    def: &WorkflowDefinition,
    state: &RunState,
    now: OffsetDateTime,
) -> Result<Option<Plan>, ExecutionError> {
    for (node, step) in state.steps() {
        if step.status() != StepStatus::Pending {
            continue;
        }
        let declared = node_of(def, node)?;
        // A compensation is released by its saga, when everything it waits for has finished.
        if matches!(declared.origin(), NodeOrigin::Compensation(_)) {
            continue;
        }
        // Activating this step would be one instance beyond the author's lowered ceiling: the run
        // stops with a durable, typed failure instead of the store refusing the step at commit.
        if lowered_instance_limit(def).is_some_and(|limit| state.charged_instances() >= limit) {
            return Ok(Some(Plan::Emit(RunEvent::RunStopRequested {
                reason: StopReason::Failed,
                code: Some(FailureCode::instance_limit()),
            })));
        }
        let action = match declared.kind() {
            NodeKind::Activity { .. } => Plan::Emit(RunEvent::StepReleased { node: node.clone() }),
            NodeKind::Saga { begin, .. } => Plan::Emit(RunEvent::SagaEntered {
                node: node.clone(),
                begin: begin.clone(),
            }),
            NodeKind::Timer { delay } => Plan::Emit(RunEvent::TimerStarted {
                node: node.clone(),
                due_at: after(now, *delay)?,
            }),
            NodeKind::WaitEvent { correlation, .. } => Plan::Evaluate(Evaluation::Correlation {
                node: node.clone(),
                expression: correlation.clone(),
            }),
            NodeKind::Condition { expression, .. } => Plan::Evaluate(Evaluation::Condition {
                node: node.clone(),
                expression: expression.clone(),
            }),
            NodeKind::Switch { cases, .. } => Plan::Evaluate(Evaluation::Switch {
                node: node.clone(),
                cases: cases.clone(),
            }),
            NodeKind::Parallel { branches, .. } => Plan::Emit(RunEvent::ParallelForked {
                node: node.clone(),
                branches: branches.clone(),
            }),
            NodeKind::Child { input, .. } => Plan::Evaluate(Evaluation::Child {
                node: node.clone(),
                input: input.clone(),
            }),
            NodeKind::End { .. } => Plan::Emit(RunEvent::EndReached { node: node.clone() }),
            NodeKind::Approval { .. }
            | NodeKind::ContinueAsNew { .. }
            | NodeKind::ForEach { .. }
            | NodeKind::While { .. }
            | NodeKind::RepeatUntil { .. } => Plan::Idle,
            NodeKind::Return { .. } => {
                return Err(ExecutionError::UnsupportedStructuredNode(node.clone()));
            }
        };
        return Ok(Some(action));
    }
    Ok(None)
}

/// Whether `step` is a failure that nobody on its own level handles.
fn unhandled_failure_in(
    def: &WorkflowDefinition,
    parallel: &NodeId,
    node: &NodeId,
    step: &StepState,
) -> bool {
    step.status() == StepStatus::Failed
        && def.node(node).is_some_and(|n| n.on_error().is_none())
        && def.enclosing_parallel(node) == Some(parallel)
}

fn join_outcome(
    def: &WorkflowDefinition,
    state: &RunState,
    parallel: &NodeId,
) -> Result<Option<JoinOutcome>, ExecutionError> {
    let NodeKind::Parallel { join, .. } = node_of(def, parallel)?.kind() else {
        return Err(ExecutionError::Invariant(
            "a join was requested for a non-parallel node",
        ));
    };
    let regions = def
        .branch_regions(parallel)
        .ok_or(ExecutionError::Invariant("a parallel node has no regions"))?;
    let (mut any_ok, mut any_failed, mut all_ok, mut all_failed) = (false, false, true, true);
    for region in regions {
        let mut failed = false;
        let mut done = true;
        for member in region {
            let Some(step) = state.step(member) else {
                continue;
            };
            failed |= unhandled_failure_in(def, parallel, member, step);
            done &= step.status().is_terminal() && step.routed();
        }
        let ok = done && !failed;
        any_ok |= ok;
        any_failed |= failed;
        all_ok &= ok;
        all_failed &= failed;
    }
    Ok(match join {
        JoinPolicy::All if any_failed => Some(JoinOutcome::Failed),
        JoinPolicy::All if all_ok => Some(JoinOutcome::Succeeded),
        JoinPolicy::Any if any_ok => Some(JoinOutcome::Succeeded),
        JoinPolicy::Any if all_failed => Some(JoinOutcome::Failed),
        _ => None,
    })
}

fn settle_joins(
    def: &WorkflowDefinition,
    state: &RunState,
) -> Result<Option<Plan>, ExecutionError> {
    for (node, step) in state.steps() {
        if step.status() == StepStatus::Waiting
            && matches!(step.waiting_on(), Some(WaitingOn::Branches))
            && let Some(outcome) = join_outcome(def, state, node)?
        {
            return Ok(Some(Plan::Emit(RunEvent::ParallelJoined {
                node: node.clone(),
                outcome,
            })));
        }
    }
    Ok(None)
}

/// Branches that lost a join (or were abandoned by a failed one) must not keep running.
fn cancel_joined_leftovers(def: &WorkflowDefinition, state: &RunState) -> Option<RunEvent> {
    for node in def.nodes() {
        if !matches!(node.kind(), NodeKind::Parallel { .. }) {
            continue;
        }
        let joined = state
            .step(node.id())
            .is_some_and(|s| matches!(s.status(), StepStatus::Succeeded | StepStatus::Failed));
        if !joined {
            continue;
        }
        for member in def
            .branch_regions(node.id())
            .into_iter()
            .flatten()
            .flatten()
        {
            let Some(step) = state.step(member) else {
                continue;
            };
            // A saga in a branch that lost compensates what it did; it winds its own body down.
            if sagas::owned(def, state, member, false) {
                continue;
            }
            match step.status() {
                StepStatus::Pending
                | StepStatus::Ready
                | StepStatus::RetryWait
                | StepStatus::Waiting => {
                    return Some(RunEvent::StepCancelled {
                        node: member.clone(),
                    });
                }
                StepStatus::Leased | StepStatus::Running if !step.cancel_requested() => {
                    return Some(RunEvent::StepCancelRequested {
                        node: member.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    None
}

fn successors_after(
    def: &WorkflowDefinition,
    state: &RunState,
    node: &NodeId,
    step: &StepState,
) -> Result<Vec<NodeId>, ExecutionError> {
    let def_node = node_of(def, node)?;
    if step.status() == StepStatus::Failed {
        // The error route of a saga is for a failure that was undone. If something was left in
        // place, or a compensation failed for good, the run says so instead of carrying on as if
        // nothing had happened.
        let undone = state
            .saga(node)
            .and_then(super::state::SagaState::outcome)
            .is_none_or(|outcome| outcome == SagaOutcome::Compensated);
        return Ok(if undone {
            def_node.on_error().cloned().into_iter().collect()
        } else {
            Vec::new()
        });
    }
    Ok(match def_node.kind() {
        NodeKind::Condition {
            if_true, if_false, ..
        } => vec![if step.condition() == Some(true) {
            if_true.clone()
        } else {
            if_false.clone()
        }],
        NodeKind::Switch { cases, default } => match step.switch_selection() {
            Some(SwitchSelection::Case(id)) => {
                let target = cases
                    .iter()
                    .find(|case| case.id() == id)
                    .ok_or(ExecutionError::Invariant("switch selected an unknown case"))?
                    .target()
                    .clone();
                vec![target]
            }
            Some(SwitchSelection::Default) => vec![default.clone()],
            None => {
                return Err(ExecutionError::Invariant(
                    "switch has no recorded selection",
                ));
            }
        },
        NodeKind::WaitEvent { on_timeout, .. } if step.timed_out() => {
            on_timeout.iter().cloned().collect()
        }
        NodeKind::End { .. } => Vec::new(),
        _ => def_node.next().cloned().into_iter().collect(),
    })
}

/// Whether `node` sits in a region that is over: a branch of a parallel node that has already been
/// joined (it won, it failed, or a sibling decided it), or the body of a saga that has stopped its
/// forward work. Whatever such a step reports late is recorded, but it must not start anything new.
pub(super) fn in_abandoned_region(
    def: &WorkflowDefinition,
    state: &RunState,
    node: &NodeId,
) -> bool {
    def.scope_chain(node).any(|scope| match scope {
        Scope::Parallel(parallel) => state
            .step(parallel)
            .is_some_and(|s| matches!(s.status(), StepStatus::Succeeded | StepStatus::Failed)),
        Scope::Saga(saga) => state
            .saga(saga)
            .is_some_and(|s| s.phase() != SagaPhase::Forward),
    })
}

fn route_finished(
    def: &WorkflowDefinition,
    state: &RunState,
) -> Result<Option<Plan>, ExecutionError> {
    for (node, step) in state.steps() {
        if !matches!(
            step.status(),
            StepStatus::Succeeded | StepStatus::Failed | StepStatus::Skipped
        ) || step.routed()
        {
            continue;
        }
        // Nothing follows a compensation or a reconciliation: a saga or the step it belongs to
        // decides what its end means.
        if node_of(def, node)?.origin() != &NodeOrigin::Declared {
            return Ok(Some(Plan::Emit(RunEvent::StepRouted {
                node: node.clone(),
                activated: Vec::new(),
            })));
        }
        if in_abandoned_region(def, state, node) {
            return Ok(Some(Plan::Emit(RunEvent::StepRouted {
                node: node.clone(),
                activated: Vec::new(),
            })));
        }
        let activated = successors_after(def, state, node, step)?;
        if step.status() == StepStatus::Failed && activated.is_empty() {
            return Ok(Some(if def.enclosing_scope(node).is_some() {
                // The enclosing join or saga decides what a failed step means.
                Plan::Emit(RunEvent::StepRouted {
                    node: node.clone(),
                    activated,
                })
            } else {
                // A failure that a saga compensated ends the run as what the saga made of it.
                let reason = sagas::origin_outcome(def, state, node)
                    .map_or(StopReason::Failed, StopReason::of_outcome);
                Plan::Emit(RunEvent::RunStopRequested {
                    reason,
                    code: step.failure().cloned(),
                })
            }));
        }
        return Ok(Some(Plan::Emit(RunEvent::StepRouted {
            node: node.clone(),
            activated,
        })));
    }
    Ok(None)
}
