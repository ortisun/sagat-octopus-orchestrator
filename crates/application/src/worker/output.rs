//! Fenced inline/blob completion, secret rejection and retained-content quota recovery.
use super::*;
use crate::blobs::{BlobCommit, descriptor_json};
use sagat_domain::blobs::BlobDescriptor;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// Records a successful attempt. A repeat with the same `idempotency_key` returns the original
    /// outcome with `already_applied` set, even if the run has moved on.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`] when the caller no longer holds the lease;
    /// [`AppError::NotAllowed`] for an invalid output;
    /// [`AppError::IdempotencyKeyReused`] when the key was used for a different report.
    pub async fn complete(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        output: &[u8],
        idempotency_key: &str,
    ) -> Result<StepAck, AppError> {
        self.complete_inner(principal, task, output, None, None, idempotency_key)
            .await
    }

    /// Records a successful attempt whose output is too large to travel: the worker uploaded it as a
    /// blob, and what the run keeps as the step's output is the small descriptor of that blob
    /// (FR-002-008). The blob is committed in the same transaction as the step, so it is used exactly
    /// once and cannot be swept from under the record that refers to it.
    ///
    /// # Errors
    /// As [`Self::complete`]; [`AppError::NotFound`] when the blob is not an uploaded, unused blob of
    /// this tenant with exactly the size and digest of `blob`.
    pub async fn complete_with_blob(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        blob: &BlobDescriptor,
        expected_digest: &str,
        idempotency_key: &str,
    ) -> Result<StepAck, AppError> {
        self.complete_inner(
            principal,
            task,
            &descriptor_json(blob),
            Some(BlobCommit::new(
                self.blob_report_scope(principal, task).await?,
                blob,
            )),
            Some(blob_report_identity(blob.id(), expected_digest)),
            idempotency_key,
        )
        .await
    }

    async fn complete_inner(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        output: &[u8],
        blob: Option<BlobCommit>,
        blob_identity: Option<Vec<u8>>,
        idempotency_key: &str,
    ) -> Result<StepAck, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        if output.len() > MAX_PAYLOAD_BYTES || !self.engine.evaluator().is_json(output) {
            return Err(AppError::NotAllowed(
                "a step output must be JSON of at most 256 KiB",
            ));
        }
        let idempotency = completion_idempotency(
            task,
            idempotency_key,
            blob_identity.as_deref().unwrap_or(output),
            blob.is_some(),
        );
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self.replayed(&mut tx, task, &idempotency).await? {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let mut database_secret_in_output = None;
        if Self::managed_database(&mut tx, task) {
            // Blob uploads/commit remain closed until the bounded pre-storage path is wired.
            if blob.is_some() {
                return Err(AppError::NotAllowed(
                    "managed database blobs require content scanning",
                ));
            }
            let before = self
                .database_output_snapshot(principal, &mut tx, task)
                .await?;
            tx.commit().await?;
            let contains = self
                .scan_database_output(&before.0, before.1, output)
                .await?;
            tx = self.open(principal, task, Lock::Wait).await?;
            if let Some(ack) = self.replayed(&mut tx, task, &idempotency).await? {
                return Ok(ack);
            }
            let after = self
                .database_output_snapshot(principal, &mut tx, task)
                .await?;
            if before != after {
                return Err(AppError::StaleAttempt);
            }
            database_secret_in_output = contains.then_some(before.0.meta.name);
        }
        if blob.is_some() {
            if task.node.is_reconciliation() || task.node.is_compensation() {
                return Err(AppError::NotAllowed(
                    "blob output requires a forward activity",
                ));
            }
            if !Self::declared_secrets(&mut tx, task).is_empty() {
                return Err(AppError::NotAllowed(
                    "blob outputs with declared secrets require content scanning",
                ));
            }
            if !tx
                .lease_unexpired(task.node, &Token::hash_of(task.lease_token))
                .await?
            {
                return Err(AppError::StaleAttempt);
            }
        }
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        // A secret the step was given must not come back in its output, where it would be stored in
        // the journal and handed to every later step: the step fails for good instead.
        let leaked_secret = if database_secret_in_output.is_some() {
            database_secret_in_output
        } else {
            self.secret_in_output(&mut tx, task, output).await?
        };
        if let Some(name) = leaked_secret {
            // A detection, counted when found: should the commit below fail the retry finds it again.
            metrics::counter!("secret_output_detected_total").increment(1);
            effects.audit = Some(self.leak_audit(principal, &tx, task, &name, "output"));
            let node = task.node.clone();
            let fence = task.fence;
            let fallback_audit = effects.audit.clone();
            let code = if Self::database_write(&mut tx, task) {
                FailureCode::outcome_unknown()
            } else {
                FailureCode::secret_in_output()
            };
            let completed = self
                .engine
                .run(&mut tx, effects, move |def, state, now| {
                    let failure = execution::Failure {
                        fence,
                        code,
                        retryable: false,
                        jitter_sample: 0.0,
                    };
                    Ok(((), execution::fail_step(def, state, &node, failure, now)?))
                })
                .await
                .map_err(engine_error);
            let result = match completed {
                Ok(()) => self.finish(tx, task, idempotency.clone()).await,
                Err(error) => {
                    drop(tx);
                    Err(error)
                }
            };
            return self
                .recover_success_storage_quota(principal, task, idempotency, fallback_audit, result)
                .await;
        }
        // A reconciliation does not produce the output of a step, it says whether the step happened.
        if task.node.is_reconciliation() {
            let result = self
                .complete_reconciliation(tx, task, output, effects, idempotency.clone())
                .await;
            return self
                .recover_success_storage_quota(principal, task, idempotency, None, result)
                .await;
        }
        // What a compensation answers is not kept: nothing reads it (a later step reads the output
        // of the step it follows, not of the one that undid it), so it must not count against the
        // run's budget either, or an undo that worked would be recorded as a failure because the
        // forward steps had filled it.
        if task.node.is_compensation() {
            let node = task.node.clone();
            let fence = task.fence;
            let completed = self
                .engine
                .run(&mut tx, effects, move |def, state, now| {
                    let progress = execution::complete_step(def, state, &node, fence, now)?;
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error);
            let result = match completed {
                Ok(()) => self.finish(tx, task, idempotency.clone()).await,
                Err(error) => {
                    drop(tx);
                    Err(error)
                }
            };
            return self
                .recover_success_storage_quota(principal, task, idempotency, None, result)
                .await;
        }
        // Every lease receives the outputs of the steps before it, so what a run retains is bounded:
        // a completion that would exceed it fails the step (a retry would only exceed it again).
        let retained: usize = tx.data().await?.outputs.values().map(Vec::len).sum();
        if retained + output.len() > MAX_RUN_OUTPUT_BYTES {
            let node = task.node.clone();
            let fence = task.fence;
            let code = if Self::database_write(&mut tx, task) {
                FailureCode::outcome_unknown()
            } else {
                FailureCode::run_data_limit()
            };
            let completed = self
                .engine
                .run(&mut tx, effects, move |def, state, now| {
                    let failure = execution::Failure {
                        fence,
                        code,
                        retryable: false,
                        jitter_sample: 0.0,
                    };
                    Ok(((), execution::fail_step(def, state, &node, failure, now)?))
                })
                .await
                .map_err(engine_error);
            let result = match completed {
                Ok(()) => self.finish(tx, task, idempotency.clone()).await,
                Err(error) => {
                    drop(tx);
                    Err(error)
                }
            };
            return self
                .recover_success_storage_quota(principal, task, idempotency, None, result)
                .await;
        }
        effects.outputs = vec![(task.node.clone(), output.to_vec())];
        effects.blobs = blob.into_iter().collect();
        let node = task.node.clone();
        let fence = task.fence;
        let completed = self
            .engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = execution::complete_step(def, state, &node, fence, now)?;
                Ok(((), progress))
            })
            .await
            .map_err(engine_error);
        let result = match completed {
            Ok(()) => self.finish(tx, task, idempotency.clone()).await,
            Err(error) => {
                drop(tx);
                Err(error)
            }
        };
        self.recover_success_storage_quota(principal, task, idempotency, None, result)
            .await
    }

    async fn recover_success_storage_quota(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
        audit: Option<AuditRecord>,
        result: Result<StepAck, AppError>,
    ) -> Result<StepAck, AppError> {
        match result {
            Err(AppError::QuotaExceeded(what))
                if what == QuotaResource::TenantContentBytes.limit_name() =>
            {
                metrics::counter!("quota_exceeded_total", "limit" => what).increment(1);
                self.fail_success_for_storage_quota(principal, task, idempotency, audit)
                    .await
            }
            other => other,
        }
    }

    /// The success transaction was rolled back in full because its retained data did not fit.
    /// Reopen the same fenced attempt, replay the original completion identity first, and commit a
    /// terminal storage failure under that identity. Stopping before settling the step prevents an
    /// `onError` route from allocating more retained data while preserving saga drain semantics.
    async fn fail_success_for_storage_quota(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
        audit: Option<AuditRecord>,
    ) -> Result<StepAck, AppError> {
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self.replayed(&mut tx, task, &idempotency).await? {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let node = task.node.clone();
        let fence = task.fence;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.audit = audit;
        let managed_write = Self::database_write(&mut tx, task);
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = if managed_write {
                    // A stop request would discard an operator wait during settlement. Preserve
                    // the ambiguous write for review; it has no automatic retry/onError route.
                    execution::fail_step(
                        def,
                        state,
                        &node,
                        execution::Failure {
                            fence,
                            code: FailureCode::outcome_unknown(),
                            retryable: false,
                            jitter_sample: 0.0,
                        },
                        now,
                    )?
                } else {
                    execution::fail_step_for_storage_quota(def, state, &node, fence, now)?
                };
                Ok(((), progress))
            })
            .await
            .map_err(engine_error)?;
        self.finish(tx, task, idempotency).await
    }

    /// The answer of a reconciliation (`x~r`) to the question whether step `x` happened. An answer
    /// that is not one, or that would make the run retain too much, fails the reconciliation for
    /// good: asking the same lookup again would get the same answer.
    async fn complete_reconciliation(
        &self,
        mut tx: S::Tx,
        task: &TaskRef<'_>,
        output: &[u8],
        mut effects: Effects,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let forward = task.node.forward().ok_or(AppError::Internal)?;
        let rejected = match self.engine.evaluator().reconcile_answer(output) {
            Err(failure) => {
                tracing::warn!(error = %failure, node = %task.node, "a reconciliation answered nonsense");
                Some(FailureCode::reconciliation_answer_invalid())
            }
            Ok(answer) => {
                let substitute = (answer.verdict == sagat_domain::saga::ReconcileVerdict::Applied)
                    .then(|| answer.output.clone().unwrap_or_else(|| b"{}".to_vec()));
                let retained: usize = tx.data().await?.outputs.values().map(Vec::len).sum();
                if substitute
                    .as_ref()
                    .is_some_and(|bytes| retained + bytes.len() > MAX_RUN_OUTPUT_BYTES)
                {
                    Some(FailureCode::run_data_limit())
                } else {
                    // The forward step succeeded after all: later steps read the output it would
                    // have had.
                    if let Some(bytes) = substitute {
                        effects.outputs = vec![(forward.clone(), bytes)];
                    }
                    let (node, fence, verdict) = (task.node.clone(), task.fence, answer.verdict);
                    let jitter = self.entropy.unit();
                    self.engine
                        .run(&mut tx, effects, move |def, state, now| {
                            let progress = execution::complete_reconciliation(
                                def, state, &node, fence, verdict, jitter, now,
                            )?;
                            Ok(((), progress))
                        })
                        .await
                        .map_err(engine_error)?;
                    return self.finish(tx, task, idempotency).await;
                }
            }
        };
        let code = rejected.ok_or(AppError::Internal)?;
        let (node, fence) = (task.node.clone(), task.fence);
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let failure = execution::Failure {
                    fence,
                    code,
                    retryable: false,
                    jitter_sample: 0.0,
                };
                Ok(((), execution::fail_step(def, state, &node, failure, now)?))
            })
            .await
            .map_err(engine_error)?;
        self.finish(tx, task, idempotency).await
    }
}
