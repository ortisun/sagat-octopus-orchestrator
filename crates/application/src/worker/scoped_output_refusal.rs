//! Full-run refusal path for a known-success scoped report whose output no longer fits.
//!
//! The matching native drain and scheduler consume the durable stop. This path is reached only
//! through the closed candidate selected by `scoped_reports`.

use super::scoped_reports::{RefusalCandidate, RefusalFencePolicy};
use super::*;
use sagat_domain::audit::RunDataLimitAuditReason;
use sagat_domain::execution::{AddressedProgress, RunState, ScopedOutputRefusalSuccess};
use sagat_domain::workflow::WorkflowDefinition;

pub(super) enum RefusalDisposition {
    Finished(StepAck),
    RetryOrdinary,
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub(super) async fn refuse_scoped_output(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        candidate: RefusalCandidate,
        idempotency: Idempotency,
    ) -> Result<RefusalDisposition, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if tx.info().tenant_id != principal.tenant_id()
            || tx.info().run_id != task.run_id
            || !candidate.matches_definition(tx.parts().0, task.instance)
        {
            return Err(AppError::StaleAttempt);
        }

        // Current binding and membership authority precede receipt disclosure. A receipt still
        // wins before any new lease mutation or report effect.
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(RefusalDisposition::Finished(ack));
        }
        self.verify(&mut tx, task).await?;
        Self::strict_refusal_fence(&mut tx, task, &candidate.fence).await?;

        if let Some(scanned) = &candidate.scanned_versions {
            let names = Self::declared_secrets_of(&mut tx, task.instance);
            if *scanned != tx.secret_versions(&names).await? {
                return Err(AppError::Unavailable);
            }
        }

        let candidate_bytes =
            u64::try_from(candidate.output.len()).map_err(|_| AppError::Internal)?;
        let retained = tx.logical_output_bytes().await?;
        if retained
            .checked_add(candidate_bytes)
            .is_some_and(|total| total <= MAX_RUN_OUTPUT_BYTES as u64)
        {
            drop(tx);
            return Ok(RefusalDisposition::RetryOrdinary);
        }

        let success = candidate.success;
        let instance = task.instance.clone();
        let fence = task.fence;
        if tx.parts().1.stop_reason().is_some() {
            let (definition, state) = tx.parts();
            let mut projected = state.clone();
            let progress = execution::prepare_scoped_output_refusal(
                definition,
                &mut projected,
                &instance,
                fence,
                success,
            )
            .map_err(|error| engine_error(EngineError::Execution(error)))?;
            if progress.pending.is_some() || !progress.events.is_empty() {
                return Err(AppError::Internal);
            }
        } else {
            let effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
            self.engine
                .run(&mut tx, effects, move |definition, state, _| {
                    let progress = execution::prepare_scoped_output_refusal(
                        definition, state, &instance, fence, success,
                    )?;
                    if progress.pending.is_some() {
                        return Err(sagat_domain::execution::ExecutionError::Invariant(
                            "scoped output refusal unexpectedly requested evaluation",
                        ));
                    }
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        }

        let jitter = self.entropy.unit();
        let (step_status, run_status, version) = {
            let now = tx.now();
            let (definition, state) = tx.parts();
            let mut projected = state.clone();
            let progress = refusal_completion(
                definition,
                &mut projected,
                task.instance,
                task.fence,
                candidate.success,
                jitter,
                now,
            )
            .map_err(|error| engine_error(EngineError::Execution(error)))?;
            ensure_closed_refusal(&progress)
                .map_err(|error| engine_error(EngineError::Execution(error)))?;
            let step_status = projected.instance(task.instance).map_or(
                StepStatus::Cancelled,
                sagat_domain::execution::StepState::status,
            );
            (step_status, projected.status(), projected.version())
        };

        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        effects.audit = Some(AuditRecord::engine(
            tx.info().tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            task.run_id.to_string(),
            RunDataLimitAuditReason::SuccessfulScopedOutputRefused.as_str(),
            tx.info().trace_id.clone(),
            self.clock.now(),
        ));
        effects.idempotency = Some((
            idempotency,
            format!(
                "{}|{}|{}",
                step_status.as_str(),
                run_status.as_str(),
                version
            ),
        ));

        let instance = task.instance.clone();
        let fence = task.fence;
        self.engine
            .run_addressed(
                &mut tx,
                Vec::new(),
                effects,
                move |definition, state, now| {
                    let progress = refusal_completion(
                        definition, state, &instance, fence, success, jitter, now,
                    )?;
                    ensure_closed_refusal(&progress)?;
                    Ok(((), progress))
                },
            )
            .await
            .map_err(engine_error)?;
        tx.commit().await?;
        metrics::counter!("worker_scoped_output_refused_total", "reason" => "run_data_limit")
            .increment(1);
        Ok(RefusalDisposition::Finished(StepAck {
            command_id: format!("{}:{version}", task.run_id),
            already_applied: false,
            step_status,
            run_status,
        }))
    }

    async fn strict_refusal_fence<Tx: RunTx>(
        tx: &mut Tx,
        task: &TaskRef<'_>,
        policy: &RefusalFencePolicy,
    ) -> Result<(), AppError> {
        let valid = {
            let (_, state) = tx.parts();
            state.instance(task.instance).is_some_and(|step| {
                step.attempt() == task.fence.attempt
                    && step.generation() == task.fence.generation
                    && match policy {
                        RefusalFencePolicy::LeaseMatch => true,
                        RefusalFencePolicy::Live { allow_cancellation } => {
                            (*allow_cancellation || !step.cancel_requested())
                                && !state.is_finished()
                                && (*allow_cancellation
                                    || task.instance.node().is_derived()
                                    || state.stop_reason().is_none())
                        }
                    }
            })
        };
        if !valid {
            return Err(AppError::StaleAttempt);
        }
        if matches!(policy, RefusalFencePolicy::Live { .. })
            && !tx
                .lease_unexpired_instance(task.instance, &Token::hash_of(task.lease_token))
                .await?
        {
            return Err(AppError::StaleAttempt);
        }
        Ok(())
    }
}

fn refusal_completion(
    definition: &WorkflowDefinition,
    state: &mut RunState,
    instance: &ExecutionInstanceId,
    fence: Fence,
    success: ScopedOutputRefusalSuccess,
    jitter: f64,
    now: OffsetDateTime,
) -> Result<AddressedProgress, sagat_domain::execution::ExecutionError> {
    match success {
        ScopedOutputRefusalSuccess::Forward => {
            execution::complete_instance(definition, state, instance, fence)
        }
        ScopedOutputRefusalSuccess::ReconciliationApplied => {
            execution::complete_reconciliation_instance(
                definition,
                state,
                instance,
                fence,
                sagat_domain::saga::ReconcileVerdict::Applied,
                jitter,
                now,
            )
        }
    }
}

fn ensure_closed_refusal(
    progress: &AddressedProgress,
) -> Result<(), sagat_domain::execution::ExecutionError> {
    let released = progress
        .events
        .iter()
        .any(|addressed| match addressed.event() {
            RunEvent::StepReleased { .. } => true,
            RunEvent::StepRouted { activated, .. } => !activated.is_empty(),
            _ => false,
        });
    if released {
        Err(sagat_domain::execution::ExecutionError::Invariant(
            "scoped output refusal released forward work",
        ))
    } else {
        Ok(())
    }
}
