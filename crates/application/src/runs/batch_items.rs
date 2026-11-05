//! The single-run use cases a batch job applies to each run it selected (feature 018, `T018-047`).
//! One action, one use case, under the key `batch:{job}:{run}` so that a repeat of an item changes
//! nothing more. What a use case refuses because the run is not in a state the action applies to is a
//! skip with its reason, never a failure.

use super::*;
use crate::batch_executor::{ItemActions, ItemEffect, ItemRequest};
use crate::declared_operations::DeclaredOperationStoreError;
use sagat_domain::batch::{Action, SkipReason};
use sagat_domain::execution::{OperationError, UpdateReceiptStatus, UpdateRejection};

impl<S, C, E> ItemActions for RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        let id = RunId::new(request.run);
        // the audit of the run says which job did it, as well as why
        let reason: String = format!("batch {}: {}", request.job.id, request.job.reason)
            .chars()
            .take(512)
            .collect();
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
                    .and_then(|step| NodeId::parse_stored(step).ok())
                    .ok_or(AppError::Internal)?;
                let action = if request.job.action == Action::RetryStep {
                    ReviewAction::Retry
                } else {
                    ReviewAction::Hold
                };
                // A run the creator cannot see is hidden from them. Past this read, a `NotFound`
                // from the review means the run has no such step, which is not for this action.
                self.get(request.principal, id, request.ctx).await?;
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
                    // the run has no such step, or it does not wait for an operator, or has moved on
                    Err(AppError::StateConflict(_) | AppError::NotFound) => {
                        skip(SkipReason::NotWaiting)
                    }
                    Err(other) => Err(other),
                }
            }
            Action::Rerun => {
                let business = if params.drop_business_id {
                    RerunBusinessId::Drop
                } else {
                    RerunBusinessId::Keep
                };
                match self
                    .rerun_with(
                        request.principal,
                        id,
                        &reason,
                        request.key,
                        business,
                        request.ctx,
                    )
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
            Action::Update => {
                let (Some(name), Some(payload)) = (params.name.as_deref(), request.payload) else {
                    return Err(AppError::Internal);
                };
                // the key of the item is the key of the Update: a repeat of the item answers from the
                // receipt it left, and the Update is applied once
                let (Ok(operation), Ok(update_key), Ok(arguments)) = (
                    ActivityName::parse(name),
                    UpdateKey::parse(request.key),
                    DeclaredOperationArguments::parse(payload),
                ) else {
                    return Err(AppError::Internal);
                };
                // A run the creator cannot see is hidden from them. Past this read, what the Update
                // refuses is not for this action.
                let namespace = self
                    .get(request.principal, id, request.ctx)
                    .await?
                    .info
                    .namespace_id;
                let update = UpdateAtStart {
                    operation: &operation,
                    update_key: &update_key,
                    arguments: &arguments,
                    expected_state_revision: None,
                    batch_job: Some(request.job.id),
                };
                match self
                    .update_in_own_transaction(
                        request.principal,
                        id,
                        namespace,
                        update,
                        HolderReceipt::None,
                        request.ctx,
                    )
                    .await
                    .map(|(applied, _)| applied)
                {
                    Ok(applied) => match applied.resolution.receipt().status() {
                        UpdateReceiptStatus::Applied => Ok(done()),
                        // a business rejection fails the item as `updateRun` fails the call
                        UpdateReceiptStatus::Rejected => {
                            match applied.resolution.receipt().rejection() {
                                Some(UpdateRejection::RevisionConflict { .. }) => {
                                    Err(AppError::StateConflict("the declared update was rejected"))
                                }
                                _ => Err(AppError::NotAllowed("the declared update was rejected")),
                            }
                        }
                    },
                    Err(AppError::DeclaredUpdate(error)) => update_refusal(error),
                    Err(other) => Err(other),
                }
            }
            // the feature 019 repin does not exist without that feature
            Action::Repin => Err(AppError::NotAllowed(
                "the action is not available to batch jobs yet",
            )),
        }
    }
}

/// What a batch item does with what its declared Update refused.
fn update_refusal(error: DeclaredOperationStoreError) -> Result<ItemEffect, AppError> {
    let skip = |reason: SkipReason| Ok(ItemEffect::Skip(reason));
    match error {
        // the run's version declares no such Update
        DeclaredOperationStoreError::DefinitionUnavailable => {
            skip(SkipReason::OperationNotDeclared)
        }
        // the run vanished between the read before and the lock
        DeclaredOperationStoreError::NotFound => Err(AppError::NotFound),
        // the run is over, or is winding down, compensating or waiting for a person: only a run that
        // has ended is over for good
        DeclaredOperationStoreError::Operation(OperationError::RunNotMutable(status)) => {
            skip(if status.is_terminal() {
                SkipReason::AlreadyTerminal
            } else {
                SkipReason::NotApplicable
            })
        }
        DeclaredOperationStoreError::Operation(OperationError::RunContinued) => {
            skip(SkipReason::RunContinued)
        }
        // the same failures the other actions have: the platform's, to be tried again, and a right the
        // creator no longer has, which ends the job
        DeclaredOperationStoreError::Storage(failure) => Err(AppError::from(failure)),
        DeclaredOperationStoreError::Forbidden => Err(AppError::Forbidden {
            permission: Permission::RunUpdate,
        }),
        other => Err(AppError::DeclaredUpdate(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StoreError;
    use sagat_domain::execution::RunStatus;

    fn effect(error: DeclaredOperationStoreError) -> Result<ItemEffect, AppError> {
        update_refusal(error)
    }

    fn not_mutable(status: RunStatus) -> DeclaredOperationStoreError {
        DeclaredOperationStoreError::Operation(OperationError::RunNotMutable(status))
    }

    #[test]
    fn only_a_run_that_has_ended_is_over_for_good() {
        assert_eq!(
            effect(not_mutable(RunStatus::Succeeded)),
            Ok(ItemEffect::Skip(SkipReason::AlreadyTerminal))
        );
        assert_eq!(
            effect(not_mutable(RunStatus::Cancelled)),
            Ok(ItemEffect::Skip(SkipReason::AlreadyTerminal))
        );
        // a run that is winding down, compensating or waiting for a person is not
        for status in [
            RunStatus::Cancelling,
            RunStatus::Compensating,
            RunStatus::ManualReview,
        ] {
            assert_eq!(
                effect(not_mutable(status)),
                Ok(ItemEffect::Skip(SkipReason::NotApplicable)),
                "{status:?}"
            );
        }
    }

    #[test]
    fn what_is_undeclared_or_handed_off_is_skipped_with_its_own_reason() {
        assert_eq!(
            effect(DeclaredOperationStoreError::DefinitionUnavailable),
            Ok(ItemEffect::Skip(SkipReason::OperationNotDeclared))
        );
        assert_eq!(
            effect(DeclaredOperationStoreError::Operation(
                OperationError::RunContinued
            )),
            Ok(ItemEffect::Skip(SkipReason::RunContinued))
        );
        // a run that vanished is not an operation that was never declared
        assert_eq!(
            effect(DeclaredOperationStoreError::NotFound),
            Err(AppError::NotFound)
        );
    }

    #[test]
    fn a_failure_of_the_platform_is_tried_again_and_a_lost_right_ends_the_job() {
        assert_eq!(
            effect(DeclaredOperationStoreError::Storage(
                StoreError::Unavailable {
                    detail: "pool timed out".to_owned()
                }
            )),
            Err(AppError::Unavailable)
        );
        assert_eq!(
            effect(DeclaredOperationStoreError::Forbidden),
            Err(AppError::Forbidden {
                permission: Permission::RunUpdate
            })
        );
        assert!(matches!(
            effect(DeclaredOperationStoreError::Operation(
                OperationError::InvalidArguments
            )),
            Err(AppError::DeclaredUpdate(_))
        ));
    }
}
