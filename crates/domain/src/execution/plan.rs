//! The planner: given a definition and the current state, what must happen next?
//!
//! [`plan`] is a pure function of `(definition, state, now)`. It returns at most one action; the
//! caller applies it and asks again until the planner is [`Plan::Idle`]. Anything that needs the
//! outside world (evaluating an expression over the run's data) is returned as an
//! [`Evaluation`] for an adapter to resolve, and the answer comes back as a journal event, so a
//! replay never has to evaluate anything again.

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::event::{JoinOutcome, RunEvent, StopReason};
use super::state::{RunState, StepState, StepStatus, WaitingOn};
use crate::workflow::{CancelPolicy, Expression, JoinPolicy, NodeId, NodeKind, WorkflowDefinition};

/// What the engine needs to know before it can continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evaluation {
    /// Evaluate a boolean expression over the run's data.
    Condition {
        node: NodeId,
        expression: Expression,
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
        return Ok(drain(def, state, reason));
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
    if let Some(event) = cancel_joined_leftovers(def, state) {
        return Ok(Plan::Emit(event));
    }
    if let Some(action) = route_finished(def, state)? {
        return Ok(action);
    }
    let all_done = state.has_started()
        && state
            .steps()
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

/// The run is being stopped: cancel what is not in flight, ask what is to stop, then finish.
fn drain(def: &WorkflowDefinition, state: &RunState, reason: StopReason) -> Plan {
    for (node, step) in state.steps() {
        match step.status() {
            // A step parked on a child that the policy asks to be cancelled *and awaited* is asked
            // to stop like an attempt in flight, then left alone until the child has settled.
            StepStatus::Waiting if waits_for_cancelled_child(def, node, step) => {
                if !step.cancel_requested() {
                    return Plan::Emit(RunEvent::StepCancelRequested { node: node.clone() });
                }
            }
            StepStatus::Pending
            | StepStatus::Ready
            | StepStatus::RetryWait
            | StepStatus::Waiting => {
                return Plan::Emit(RunEvent::StepCancelled { node: node.clone() });
            }
            StepStatus::Leased | StepStatus::Running if !step.cancel_requested() => {
                return Plan::Emit(RunEvent::StepCancelRequested { node: node.clone() });
            }
            _ => {}
        }
    }
    if state.steps().all(|(_, step)| step.status().is_terminal()) {
        Plan::Emit(RunEvent::RunFinished {
            status: reason.final_status(),
        })
    } else {
        Plan::Idle
    }
}

/// Whether `step` is parked on a child run whose node says the parent must wait for the child to
/// settle after asking it to stop.
fn waits_for_cancelled_child(def: &WorkflowDefinition, node: &NodeId, step: &StepState) -> bool {
    matches!(step.waiting_on(), Some(WaitingOn::Child { .. }))
        && matches!(
            def.node(node).map(crate::workflow::Node::kind),
            Some(NodeKind::Child {
                cancel_policy: CancelPolicy::RequestCancel,
                ..
            })
        )
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
        let action = match node_of(def, node)?.kind() {
            NodeKind::Activity { .. } => Plan::Emit(RunEvent::StepReleased { node: node.clone() }),
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
            NodeKind::Parallel { branches, .. } => Plan::Emit(RunEvent::ParallelForked {
                node: node.clone(),
                branches: branches.clone(),
            }),
            NodeKind::Child { input, .. } => Plan::Evaluate(Evaluation::Child {
                node: node.clone(),
                input: input.clone(),
            }),
            NodeKind::End { .. } => Plan::Emit(RunEvent::EndReached { node: node.clone() }),
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
    node: &NodeId,
    step: &StepState,
) -> Result<Vec<NodeId>, ExecutionError> {
    let def_node = node_of(def, node)?;
    if step.status() == StepStatus::Failed {
        return Ok(def_node.on_error().cloned().into_iter().collect());
    }
    Ok(match def_node.kind() {
        NodeKind::Condition {
            if_true, if_false, ..
        } => vec![if step.condition() == Some(true) {
            if_true.clone()
        } else {
            if_false.clone()
        }],
        NodeKind::WaitEvent { on_timeout, .. } if step.timed_out() => {
            on_timeout.iter().cloned().collect()
        }
        NodeKind::End { .. } => Vec::new(),
        _ => def_node.next().cloned().into_iter().collect(),
    })
}

/// Whether `node` sits in a branch of a parallel node that has already been joined (it won, it
/// failed, or a sibling decided it). Whatever such a step reports late is recorded, but it must not
/// start anything new.
fn in_abandoned_region(def: &WorkflowDefinition, state: &RunState, node: &NodeId) -> bool {
    let mut current = def.enclosing_parallel(node);
    while let Some(parallel) = current {
        let joined = state
            .step(parallel)
            .is_some_and(|s| matches!(s.status(), StepStatus::Succeeded | StepStatus::Failed));
        if joined {
            return true;
        }
        current = def.enclosing_parallel(parallel);
    }
    false
}

fn route_finished(
    def: &WorkflowDefinition,
    state: &RunState,
) -> Result<Option<Plan>, ExecutionError> {
    for (node, step) in state.steps() {
        if !matches!(step.status(), StepStatus::Succeeded | StepStatus::Failed) || step.routed() {
            continue;
        }
        if in_abandoned_region(def, state, node) {
            return Ok(Some(Plan::Emit(RunEvent::StepRouted {
                node: node.clone(),
                activated: Vec::new(),
            })));
        }
        let activated = successors_after(def, node, step)?;
        if step.status() == StepStatus::Failed && activated.is_empty() {
            return Ok(Some(if def.enclosing_parallel(node).is_some() {
                // The enclosing join decides what a failed branch means.
                Plan::Emit(RunEvent::StepRouted {
                    node: node.clone(),
                    activated,
                })
            } else {
                Plan::Emit(RunEvent::RunStopRequested {
                    reason: StopReason::Failed,
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
