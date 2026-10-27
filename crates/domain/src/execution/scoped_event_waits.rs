//! Pure preparation of qualified event waits; no subscription or external delivery authority.
use super::{
    AddressedProgress, AddressedRunEvent, CorrelationKey, ExecutionError, ExecutionInstanceId,
    FailureCode, PathExecutionState, RunEvent, RunState, StepStatus,
};
use crate::workflow::{NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

/// Records an evaluated correlation key and explicit-clock deadline for one pending member.
/// The adapter must fence the evaluated snapshot and obtain native scheduling authority.
///
/// # Errors
/// Rejects an invalid kind/path/source state, stopped ancestor or overflowing deadline.
pub fn start_event_wait_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    key: Result<CorrelationKey, ()>,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let local = PathExecutionState::from_run(state, instance.path().clone())?;
    let event = prepared_start(definition, &local, instance, key, now)?;
    state.apply_addressed(&event)?;
    super::command::settle_addressed(definition, state, instance.path(), vec![event])
}

/// Bounded path counterpart of [`start_event_wait_instance`].
///
/// # Errors
/// As [`start_event_wait_instance`].
pub fn start_event_wait_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    key: Result<CorrelationKey, ()>,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let event = prepared_start(definition, state, instance, key, now)?;
    state.apply_addressed(&event)?;
    super::command::settle_path(definition, state, vec![event])
}

fn prepared_start(
    definition: &WorkflowDefinition,
    state: &PathExecutionState,
    instance: &ExecutionInstanceId,
    key: Result<CorrelationKey, ()>,
    now: OffsetDateTime,
) -> Result<AddressedRunEvent, ExecutionError> {
    if instance.path().is_root() || instance.path() != state.path() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let step = state
        .local_instance(instance.node())
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    if !state.allows_forward_progress() || step.status() != StepStatus::Pending {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "wait.started",
        });
    }
    let declared = definition
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    let NodeKind::WaitEvent { timeout, .. } = declared.kind() else {
        return Err(ExecutionError::Invariant(
            "event start targeted another kind",
        ));
    };
    if step.attempt() != 0
        || step.generation() != 0
        || step.cancel_requested()
        || step.routed()
        || step.timed_out()
        || step.next_attempt_at().is_some()
        || step.waiting_on().is_some()
        || step.failure().is_some()
        || step.condition().is_some()
        || step.switch_selection().is_some()
    {
        return Err(ExecutionError::Invariant(
            "invalid pending scoped event state",
        ));
    }
    let event = match key {
        Ok(correlation_key) => RunEvent::WaitStarted {
            node: instance.node().clone(),
            correlation_key,
            deadline: super::plan::after(now, *timeout)?,
        },
        Err(()) => RunEvent::StepFailed {
            node: instance.node().clone(),
            code: FailureCode::expression_error(),
            fence: None,
        },
    };
    AddressedRunEvent::in_path(instance.path().clone(), event)
        .map_err(|_| ExecutionError::Invariant("an event start lost its instance path"))
}
