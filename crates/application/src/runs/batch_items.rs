//! The single-run use cases a batch job applies to each run it selected (feature 018, `T018-047`).
//! One action, one use case, under the key `batch:{job}:{run}` so that a repeat of an item changes
//! nothing more. What a use case refuses because the run is not in a state the action applies to is a
//! skip with its reason, never a failure.

use super::*;
use crate::batch_executor::{ItemActions, ItemEffect, ItemRequest};
use sagat_domain::batch::{Action, SkipReason};

impl<S, C, E> ItemActions for RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        let id = RunId::new(request.run);
        let reason: String = request.job.reason.chars().take(512).collect();
        let done = || ItemEffect::Applied { new_run: None };
        let params = crate::batch::JobParams::from_json(&request.job.params)
            .map_err(|_| AppError::Internal)?;
        let skip = |reason: SkipReason| Ok(ItemEffect::Skip(reason));
        match request.job.action {
            Action::Cancel | Action::Pause | Action::Resume => {
                let operation = match request.job.action {
                    Action::Cancel => RunOperation::Cancel,
                    Action::Pause => RunOperation::Pause,
                    _ => RunOperation::Resume,
                };
                match self
                    .operate(
                        request.principal,
                        id,
                        operation,
                        &reason,
                        None,
                        request.key,
                        request.ctx,
                    )
                    .await
                {
                    Ok(_) => Ok(done()),
                    // the run is over, or is not in the state the operation needs
                    Err(AppError::StateConflict(_)) => skip(if operation == RunOperation::Cancel {
                        SkipReason::AlreadyTerminal
                    } else {
                        SkipReason::NotApplicable
                    }),
                    Err(other) => Err(other),
                }
            }
            Action::Signal => {
                let (Some(name), Some(payload)) = (params.name.as_deref(), request.payload) else {
                    return Err(AppError::Internal);
                };
                match self
                    .signal(
                        request.principal,
                        id,
                        name,
                        payload,
                        request.key,
                        request.ctx,
                    )
                    .await
                {
                    Ok(_) => Ok(done()),
                    // no step waits for that signal: it is never buffered or dropped silently
                    Err(AppError::StateConflict(_)) => skip(SkipReason::NotWaiting),
                    Err(other) => Err(other),
                }
            }
            Action::RetryStep | Action::MarkManualReview => {
                let node = params
                    .step_id
                    .as_deref()
                    .and_then(|step| NodeId::parse(step).ok())
                    .ok_or(AppError::Internal)?;
                let action = if request.job.action == Action::RetryStep {
                    ReviewAction::Retry
                } else {
                    ReviewAction::Hold
                };
                match self
                    .review(
                        request.principal,
                        id,
                        &node,
                        &action,
                        &reason,
                        None,
                        request.key,
                        request.ctx,
                    )
                    .await
                {
                    Ok(_) => Ok(done()),
                    // the step does not wait for an operator, or has moved on
                    Err(AppError::StateConflict(_)) => skip(SkipReason::NotWaiting),
                    Err(other) => Err(other),
                }
            }
            Action::Rerun => {
                match self
                    .rerun(request.principal, id, &reason, request.key, request.ctx)
                    .await
                {
                    Ok(started) => Ok(ItemEffect::Applied {
                        new_run: Some(started.info.run_id.as_uuid()),
                    }),
                    // only a run that has ended is started again
                    Err(AppError::StateConflict(_)) => skip(SkipReason::RunContinued),
                    Err(other) => Err(other),
                }
            }
            // an Update goes through the declared operations of the run, which the batch does not
            // call yet; the feature 019 repin does not exist without that feature
            Action::Update | Action::Repin => Err(AppError::NotAllowed(
                "the action is not available to batch jobs yet",
            )),
        }
    }
}
