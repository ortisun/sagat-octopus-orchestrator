//! Pure preparation of qualified event waits; no subscription or external delivery authority.
use super::{
    AddressedProgress, AddressedRunEvent, CorrelationKey, ExecutionError, ExecutionInstanceId,
    FailureCode, PathExecutionState, RunEvent, RunState, StepStatus, WaitingOn,
};
use crate::workflow::{EventType, Node, NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

/// Records the exact matching event outcome; the adapter must separately prove current ingress,
/// retained value/receipt authority and the owned evaluation/binding snapshot.
///
/// # Errors
/// Rejects wrong kind/path/binding/window, stopped ancestry or malformed/nonwaiting sources.
pub fn satisfy_event_wait_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    event_type: &EventType,
    key: &CorrelationKey,
    received_at: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    if instance.path().is_root() || state.instance(instance).is_none() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let local = PathExecutionState::from_run(state, instance.path().clone())?;
    let event = prepared_satisfaction(definition, &local, instance, event_type, key, received_at)?;
    let mut candidate = state.clone();
    candidate.apply_addressed(&event)?;
    let progress =
        super::command::settle_addressed(definition, &mut candidate, instance.path(), vec![event])?;
    *state = candidate;
    Ok(progress)
}

/// Exact-path counterpart of [`satisfy_event_wait_instance`].
///
/// # Errors
/// As [`satisfy_event_wait_instance`].
pub fn satisfy_event_wait_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    event_type: &EventType,
    key: &CorrelationKey,
    received_at: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let event = prepared_satisfaction(definition, state, instance, event_type, key, received_at)?;
    let mut candidate = state.clone();
    candidate.apply_addressed(&event)?;
    let progress = super::command::settle_path(definition, &mut candidate, vec![event])?;
    *state = candidate;
    Ok(progress)
}

/// Canonical lower bound for an immutable, single-start scoped event binding. Root legacy
/// projection/backfill semantics remain separate.
///
/// # Errors
/// Rejects a non-event declaration or an unrepresentable window.
pub fn scoped_event_wait_since(
    node: &Node,
    deadline: OffsetDateTime,
) -> Result<OffsetDateTime, ExecutionError> {
    let NodeKind::WaitEvent {
        timeout, lookback, ..
    } = node.kind()
    else {
        return Err(ExecutionError::Invariant(
            "event window requires its pinned declaration",
        ));
    };
    let timeout = time::Duration::try_from(*timeout)
        .map_err(|_| ExecutionError::Invariant("scoped event window overflow"))?;
    let lookback = time::Duration::try_from(*lookback)
        .map_err(|_| ExecutionError::Invariant("scoped event window overflow"))?;
    deadline
        .checked_sub(timeout)
        .and_then(|at| at.checked_sub(lookback))
        .ok_or(ExecutionError::Invariant("scoped event window overflow"))
}

fn prepared_satisfaction(
    definition: &WorkflowDefinition,
    state: &PathExecutionState,
    instance: &ExecutionInstanceId,
    event_type: &EventType,
    key: &CorrelationKey,
    received_at: OffsetDateTime,
) -> Result<AddressedRunEvent, ExecutionError> {
    if instance.path().is_root() || instance.path() != state.path() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let step = state
        .local_instance(instance.node())
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    if step.status() != StepStatus::Waiting
        || state.global().status().is_terminal()
        || state.global().stop_reason().is_some()
        || !state
            .ancestors()
            .iter()
            .all(super::AncestorExecutionFence::allows_forward_progress)
    {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "wait.satisfied",
        });
    }
    let declared = definition
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    let NodeKind::WaitEvent {
        event_type: declared_type,
        ..
    } = declared.kind()
    else {
        return Err(ExecutionError::Invariant(
            "event satisfaction targeted another kind",
        ));
    };
    let Some(WaitingOn::Event {
        deadline,
        correlation_key,
    }) = step.waiting_on()
    else {
        return Err(ExecutionError::Invariant(
            "event satisfaction requires an event binding",
        ));
    };
    if !step.is_valid_scoped_event_wait(declared) {
        return Err(ExecutionError::Invariant(
            "invalid scoped event waiting state",
        ));
    }
    if declared_type != event_type || correlation_key != key {
        return Err(ExecutionError::EventNotMatching {
            instance: instance.clone(),
        });
    }
    if received_at < scoped_event_wait_since(declared, *deadline)? {
        return Err(ExecutionError::EventNotMatching {
            instance: instance.clone(),
        });
    }
    AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::WaitSatisfied {
            node: instance.node().clone(),
        },
    )
    .map_err(|_| ExecutionError::Invariant("event satisfaction lost its instance path"))
}

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
    if !step.is_valid_scoped_event_wait(declared) {
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

/// Records a due timeout for the exact waiting member, deferring local routing while paused.
///
/// # Errors
/// Rejects early, stale, stopped, wrong-kind/path or malformed waiting sources.
pub fn time_out_event_wait_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let local = PathExecutionState::from_run(state, instance.path().clone())?;
    let event = prepared_timeout(definition, &local, instance, now)?;
    state.apply_addressed(&event)?;
    super::command::settle_addressed(definition, state, instance.path(), vec![event])
}

/// Exact-path counterpart of [`time_out_event_wait_instance`].
///
/// # Errors
/// As [`time_out_event_wait_instance`].
pub fn time_out_event_wait_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedProgress, ExecutionError> {
    let event = prepared_timeout(definition, state, instance, now)?;
    state.apply_addressed(&event)?;
    super::command::settle_path(definition, state, vec![event])
}

fn prepared_timeout(
    definition: &WorkflowDefinition,
    state: &PathExecutionState,
    instance: &ExecutionInstanceId,
    now: OffsetDateTime,
) -> Result<AddressedRunEvent, ExecutionError> {
    if instance.path().is_root() || instance.path() != state.path() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let step = state
        .local_instance(instance.node())
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    if step.status() != StepStatus::Waiting
        || state.global().status().is_terminal()
        || state.global().stop_reason().is_some()
        || !state
            .ancestors()
            .iter()
            .all(super::AncestorExecutionFence::allows_forward_progress)
    {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "wait.timed_out",
        });
    }
    let declared = definition
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    let NodeKind::WaitEvent { on_timeout, .. } = declared.kind() else {
        return Err(ExecutionError::Invariant(
            "event timeout targeted another kind",
        ));
    };
    let Some(WaitingOn::Event { deadline, .. }) = step.waiting_on() else {
        return Err(ExecutionError::Invariant(
            "event timeout requires an event binding",
        ));
    };
    if !step.is_valid_scoped_event_wait(declared) {
        return Err(ExecutionError::Invariant(
            "invalid scoped event waiting state",
        ));
    }
    if *deadline > now {
        return Err(ExecutionError::NotYetDue(instance.node().clone()));
    }
    AddressedRunEvent::in_path(
        instance.path().clone(),
        RunEvent::WaitTimedOut {
            node: instance.node().clone(),
            handled: on_timeout.is_some(),
        },
    )
    .map_err(|_| ExecutionError::Invariant("an event timeout lost its instance path"))
}
