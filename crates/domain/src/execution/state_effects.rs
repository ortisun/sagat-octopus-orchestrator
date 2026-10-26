//! Shared effect/review reducer for full replay and bounded path projections.
//!
//! The storage shape supplies addressed step access; transition validation and mutations are one
//! implementation. This module is a child of state so step fields remain private to the reducer.

use super::{
    ExecutionError, ExecutionInstanceId, FailureCode, ReconcileResult, ReconcileVerdict,
    ReviewOutcome, RunEvent, RunState, StepState, StepStatus, WaitingOn, illegal_transition,
    stale_attempt,
};

pub(in crate::execution) trait InstanceSteps {
    fn get_effect_step(&self, instance: &ExecutionInstanceId) -> Option<&StepState>;
    fn get_effect_step_mut(&mut self, instance: &ExecutionInstanceId) -> Option<&mut StepState>;
    fn reconciliation_step_mut(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<&mut StepState, ExecutionError>;

    fn locate_effect_step(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &'static str,
        allowed: &[StepStatus],
    ) -> Result<&mut StepState, ExecutionError> {
        let step = self.get_effect_step_mut(instance).ok_or_else(|| {
            if instance.path().is_root() {
                ExecutionError::UnknownStep(instance.node().clone())
            } else {
                ExecutionError::UnknownInstance(instance.clone())
            }
        })?;
        if allowed.contains(&step.status) {
            Ok(step)
        } else {
            Err(illegal_transition(instance, step.status, event))
        }
    }

    fn locate_fenced_effect_step(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &'static str,
        allowed: &[StepStatus],
        attempt: u32,
        generation: u64,
    ) -> Result<&mut StepState, ExecutionError> {
        let step = self.locate_effect_step(instance, event, allowed)?;
        if step.fenced_by(attempt, generation) {
            Ok(step)
        } else {
            Err(stale_attempt(instance))
        }
    }
}

impl InstanceSteps for RunState {
    fn get_effect_step(&self, instance: &ExecutionInstanceId) -> Option<&StepState> {
        self.steps.get(instance)
    }
    fn get_effect_step_mut(&mut self, instance: &ExecutionInstanceId) -> Option<&mut StepState> {
        self.steps.get_mut(instance)
    }
    fn reconciliation_step_mut(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<&mut StepState, ExecutionError> {
        Ok(self
            .steps
            .entry(instance.clone())
            .or_insert_with(StepState::pending))
    }
}

pub(in crate::execution) fn apply_effect_event<S: InstanceSteps>(
    state: &mut S,
    instance: &ExecutionInstanceId,
    event: &RunEvent,
) -> Result<(), ExecutionError> {
    use StepStatus::{
        Failed, Leased, Pending, Ready, RetryWait, Running, Skipped, Succeeded, Waiting,
    };
    let name = event.name();
    let node = instance.node();
    match event {
        RunEvent::ReconciliationRequested { fence, cause, .. } => {
            let reconciliation =
                ExecutionInstanceId::in_path(instance.path().clone(), node.reconciliation());
            let reusable = match state.get_effect_step(&reconciliation) {
                None => true,
                Some(step) => matches!(step.status, Succeeded | Failed),
            };
            if !reusable {
                return Err(ExecutionError::AlreadyScheduled(
                    reconciliation.node().clone(),
                ));
            }
            let step = state.locate_fenced_effect_step(
                instance,
                name,
                &[Leased, Running],
                fence.attempt,
                fence.generation,
            )?;
            step.status = Waiting;
            step.waiting = Some(WaitingOn::Reconciliation);
            step.failure = Some(cause.clone());
            step.cancel_requested = false;
            // A step that is reconciled again (after an attempt that was retried, or after an
            // operator asked for another one) reopens its reconciliation: the counters go on, so
            // the retry budget of the reconciliation is shared by every round.
            let companion = state.reconciliation_step_mut(&reconciliation)?;
            companion.status = Pending;
            companion.routed = false;
            companion.waiting = None;
            companion.failure = None;
            companion.cancel_requested = false;
            companion.next_attempt_at = None;
        }
        RunEvent::ReconciliationConcluded {
            verdict, result, ..
        } => {
            let step = state.locate_effect_step(instance, name, &[Waiting])?;
            if !matches!(step.waiting, Some(WaitingOn::Reconciliation)) {
                return Err(illegal_transition(instance, step.status, name));
            }
            let paired = matches!(
                (verdict, result),
                (ReconcileVerdict::Applied, ReconcileResult::Succeeded)
                    | (
                        ReconcileVerdict::NotApplied,
                        ReconcileResult::RetryAt(_) | ReconcileResult::Failed(_)
                    )
                    | (ReconcileVerdict::Unknown, ReconcileResult::Review(_))
            );
            if !paired {
                return Err(ExecutionError::Invariant(
                    "a reconciliation result does not follow from what it found",
                ));
            }
            step.waiting = None;
            match result {
                ReconcileResult::Succeeded => {
                    step.status = Succeeded;
                    step.failure = None;
                }
                ReconcileResult::RetryAt(at) => {
                    step.status = RetryWait;
                    step.next_attempt_at = Some(*at);
                    // The lookup said it did not happen: the step is no longer one whose outcome
                    // is unknown, so a saga that stops while it backs off has nothing to undo.
                    step.failure = Some(FailureCode::reconciled_not_applied());
                }
                ReconcileResult::Failed(code) => {
                    step.status = Failed;
                    step.failure = Some(code.clone());
                }
                ReconcileResult::Review(code) => {
                    step.status = Waiting;
                    step.waiting = Some(WaitingOn::Operator { code: code.clone() });
                    step.failure = Some(code.clone());
                }
            }
        }
        RunEvent::StepNeedsReview { code, fence, .. } => {
            let step = state.locate_effect_step(
                instance,
                name,
                &[Leased, Running, Waiting, Ready, RetryWait],
            )?;
            match (step.status.is_leased(), fence, &step.waiting) {
                (true, Some(f), _) if step.fenced_by(f.attempt, f.generation) => {}
                (true, _, _) => return Err(stale_attempt(instance)),
                (false, None, Some(WaitingOn::Reconciliation)) => {}
                // An operator holds a step that has not started: it waits for a decision like
                // the ones that ran out of attempts.
                (false, None, None) if matches!(step.status, Ready | RetryWait) => {}
                _ => return Err(illegal_transition(instance, step.status, name)),
            }
            // An operator's hold of a step that has not started says why it waits now (the
            // code of the waiting); it does not say what happened to its last attempt. If that
            // attempt ended without anyone knowing whether the effect happened, the step still
            // says so: a saga that is stopped later must undo it.
            let held = fence.is_none() && matches!(step.status, Ready | RetryWait);
            step.status = Waiting;
            step.waiting = Some(WaitingOn::Operator { code: code.clone() });
            if !(held && step.failure.is_some()) {
                step.failure = Some(code.clone());
            }
            step.cancel_requested = false;
            step.next_attempt_at = None;
        }
        RunEvent::StepRetryRequested { .. } => {
            // A step that waits for a decision, or one whose retry is scheduled and an operator
            // does not want to wait for: the scheduler's own `retry_due` finds it no longer in
            // `RETRY_WAIT` and does nothing, so the two cannot both start an attempt.
            let step = state.locate_effect_step(instance, name, &[Waiting, RetryWait])?;
            if step.status == Waiting && !matches!(step.waiting, Some(WaitingOn::Operator { .. })) {
                return Err(illegal_transition(instance, step.status, name));
            }
            step.status = Ready;
            step.waiting = None;
            step.next_attempt_at = None;
        }
        RunEvent::StepSkipped { .. } => {
            if !node.is_compensation() {
                return Err(ExecutionError::Invariant(
                    "only a compensation can be skipped",
                ));
            }
            let step = state.locate_effect_step(instance, name, &[Waiting])?;
            if !matches!(step.waiting, Some(WaitingOn::Operator { .. })) {
                return Err(illegal_transition(instance, step.status, name));
            }
            step.status = Skipped;
            step.waiting = None;
        }
        RunEvent::StepResolved { outcome, .. } => {
            let step = state.locate_effect_step(instance, name, &[Waiting])?;
            if !matches!(step.waiting, Some(WaitingOn::Operator { .. })) {
                return Err(illegal_transition(instance, step.status, name));
            }
            step.waiting = None;
            match outcome {
                ReviewOutcome::Succeeded => {
                    step.status = Succeeded;
                    step.failure = None;
                }
                ReviewOutcome::Failed(code) => {
                    step.status = Failed;
                    step.failure = Some(code.clone());
                }
            }
        }
        _ => {
            return Err(ExecutionError::Invariant(
                "a non-effect event reached the effect reducer",
            ));
        }
    }
    Ok(())
}
