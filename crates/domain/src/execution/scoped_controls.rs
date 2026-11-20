//! Qualified condition/switch answers, sharing one validation rule across both state models.

use super::{
    AddressedProgress, AddressedRunEvent, ExecutionError, ExecutionInstanceId, FailureCode,
    PathExecutionState, Resolution, RunEvent, RunState, StepStatus, SwitchSelection,
};
use crate::workflow::{NodeKind, WorkflowDefinition};

/// Records an evaluated control answer in the full replay model and routes its local successor.
/// The caller must fence the snapshot revision when committing a prepared answer.
///
/// # Errors
/// The instance is not a pending condition/switch in an active path, the answer addresses another
/// node/kind, its case is undeclared, or the run/ancestor no longer permits forward progress.
pub fn resolve_control_instance(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    answer: Resolution,
) -> Result<AddressedProgress, ExecutionError> {
    let local = PathExecutionState::from_run(state, instance.path().clone())?;
    let event = prepared_event(definition, &local, instance, answer)?;
    state.apply_addressed(&event)?;
    super::command::settle_addressed(definition, state, instance.path(), vec![event])
}

/// Records the same control answer using only the exact path and its global/ancestor fences.
///
/// # Errors
/// As [`resolve_control_instance`]; another path is never visible to this command.
pub fn resolve_control_path_instance(
    definition: &WorkflowDefinition,
    state: &mut PathExecutionState,
    instance: &ExecutionInstanceId,
    answer: Resolution,
) -> Result<AddressedProgress, ExecutionError> {
    let event = prepared_event(definition, state, instance, answer)?;
    state.apply_addressed(&event)?;
    super::command::settle_path(definition, state, vec![event])
}

fn prepared_event(
    definition: &WorkflowDefinition,
    state: &PathExecutionState,
    instance: &ExecutionInstanceId,
    answer: Resolution,
) -> Result<AddressedRunEvent, ExecutionError> {
    if instance.path() != state.path() {
        return Err(ExecutionError::UnknownInstance(instance.clone()));
    }
    let step = state
        .local_instance(instance.node())
        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
    if !state.allows_forward_progress() || step.status() != StepStatus::Pending {
        return Err(ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status: step.status(),
            event: "scoped_control.resolve",
        });
    }
    let declared = definition
        .node_for_instance(instance)
        .ok_or_else(|| ExecutionError::UnknownStep(instance.node().clone()))?;
    if !step.is_valid_scoped_control(declared) {
        return Err(ExecutionError::Invariant("invalid scoped control state"));
    }
    let failed = || RunEvent::StepFailed {
        node: instance.node().clone(),
        code: FailureCode::expression_error(),
        fence: None,
    };
    let event = match (declared.kind(), answer) {
        (NodeKind::Condition { .. }, Resolution::Condition { node, outcome })
            if &node == instance.node() =>
        {
            match outcome {
                Ok(outcome) => RunEvent::ConditionEvaluated { node, outcome },
                Err(()) => failed(),
            }
        }
        (NodeKind::Switch { cases, .. }, Resolution::Switch { node, selection })
            if &node == instance.node() =>
        {
            match selection {
                Ok(selection) => {
                    if let SwitchSelection::Case(id) = &selection
                        && !cases.iter().any(|case| case.id() == id)
                    {
                        return Err(ExecutionError::Invariant("switch selected an unknown case"));
                    }
                    RunEvent::SwitchEvaluated { node, selection }
                }
                Err(()) => failed(),
            }
        }
        _ => {
            return Err(ExecutionError::Invariant(
                "control answer targeted another node or kind",
            ));
        }
    };
    AddressedRunEvent::in_path(instance.path().clone(), event)
        .map_err(|_| ExecutionError::Invariant("a control answer lost its instance path"))
}
