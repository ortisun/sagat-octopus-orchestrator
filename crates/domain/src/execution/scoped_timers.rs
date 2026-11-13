//! Clocked qualified timer commands; no wall clock or worker capability enters the domain.
use super::{
    AddressedProgress, AddressedRunEvent, ExecutionError, ExecutionInstanceId, InstancePath,
    PathExecutionState, RunEvent, RunState, StepStatus, WaitingOn,
};
use crate::workflow::{NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

#[derive(Clone, Copy)]
enum Action {
    Start,
    Fire,
}

/// Starts a qualified timer from explicit time and settles only that member's path.
///
/// # Errors
/// Rejects a wrong kind/path, nonpristine Pending state, forward fence or clock overflow.
pub fn start_timer_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    apply_full(definition, state, instance, now, Action::Start)
}

/// Starts the same timer using only its bounded path and ancestor fences.
///
/// # Errors
/// As [`start_timer_instance`].
pub fn start_timer_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    apply_path(definition, state, instance, now, Action::Start)
}

/// Records a due qualified timer firing. Pause defers routing but does not change its deadline.
///
/// # Errors
/// Rejects a wrong kind/path, nonwaiting timer, early deadline, stopped Run or inactive ancestor.
pub fn fire_timer_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    apply_full(definition, state, instance, now, Action::Fire)
}

/// Fires the same due timer using only its bounded path and ancestor fences.
///
/// # Errors
/// As [`fire_timer_instance`].
pub fn fire_timer_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    apply_path(definition, state, instance, now, Action::Fire)
}

/// Settles recorded local outcomes after a resume, without evaluating data or reading a clock.
///
/// # Errors
/// Returns a planner/reducer invariant error for malformed state or unsupported local work.
pub fn release_pending_instance_path(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    path: &InstancePath,
) -> Result<AddressedProgress, ExecutionError> {
    super::command::settle_addressed(definition, state, path, Vec::new())
}

/// Bounded counterpart of [`release_pending_instance_path`].
///
/// # Errors
/// Returns a planner/reducer invariant error for malformed state or unsupported local work.
pub fn release_pending_path(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
) -> Result<AddressedProgress, ExecutionError> {
    super::command::settle_path(definition, state, Vec::new())
}

fn apply_full(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
    action: Action,
) -> Result<AddressedProgress, ExecutionError> {
    let local = PathExecutionState::from_run(state, instance.path().clone())?;
    let event = prepared(definition, &local, instance, now, action)?;
    state.apply_addressed(&event)?;
    super::command::settle_addressed(definition, state, instance.path(), vec![event])
}

fn apply_path(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
    action: Action,
) -> Result<AddressedProgress, ExecutionError> {
    let event = prepared(definition, state, instance, now, action)?;
    state.apply_addressed(&event)?;
    super::command::settle_path(definition, state, vec![event])
}

fn prepared(
    definition: &WorkflowDefinition,
    state: &PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
    action: Action,
) -> Result<AddressedRunEvent, ExecutionError> {
    if instance.path().is_root() || instance.path() != state.path() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let step = state
        .local_instance(instance.node())
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    let illegal = || ExecutionError::IllegalInstanceTransition {
        instance: instance.clone(),
        status: step.status(),
        event: match action {
            Action::Start => "timer.started",
            Action::Fire => "timer.fired",
        },
    };
    let declared = definition
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    let NodeKind::Timer { delay } = declared.kind() else {
        return Err(ExecutionError::Invariant(
            "timer command targeted another kind",
        ));
    };
    if !step.is_valid_scoped_timer() {
        return Err(ExecutionError::Invariant("invalid scoped timer state"));
    }
    if state.global().status().is_terminal()
        || state.global().stop_reason().is_some()
        || !state
            .ancestors()
            .iter()
            .all(super::AncestorExecutionFence::allows_forward_progress)
    {
        return Err(illegal());
    }
    let event = match action {
        Action::Start => {
            if !state.allows_forward_progress() || step.status() != StepStatus::Pending {
                return Err(illegal());
            }
            RunEvent::TimerStarted {
                node: instance.node().clone(),
                due_at: super::plan::after(now, *delay)?,
            }
        }
        Action::Fire => {
            let Some(WaitingOn::Timer { due_at }) = step.waiting_on() else {
                return Err(illegal());
            };
            if step.status() != StepStatus::Waiting {
                return Err(illegal());
            }
            if *due_at > now {
                return Err(ExecutionError::NotYetDue(instance.node().clone()));
            }
            RunEvent::TimerFired {
                node: instance.node().clone(),
            }
        }
    };
    AddressedRunEvent::in_path(instance.path().clone(), event)
        .map_err(|_| ExecutionError::Invariant("a timer command lost its instance path"))
}
