//! Fenced inline/blob completion, secret rejection and retained-content quota recovery.
use super::*;
use crate::blobs::{BlobCommit, descriptor_json};
use sagat_domain::blobs::BlobDescriptor;
use sha2::{Digest as _, Sha256};

/// A catalog descriptor and the verified bytes required for managed database completion.
/// The application independently checks size/digest; adapters cannot assert a scan with a boolean.
pub struct BlobOutput<'a> {
    pub descriptor: &'a BlobDescriptor,
    pub expected_digest: &'a str,
    pub content: Option<&'a [u8]>,
}

struct BlobCompletion<'a> {
    commit: BlobCommit,
    identity: Vec<u8>,
    content: Option<&'a [u8]>,
    content_type: &'a str,
}

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
    ) -> Result<StepAck, AppError>
    where
        S: PathExecutionStore,
    {
        self.complete_inner(principal, task, output, None, idempotency_key)
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
        report: BlobOutput<'_>,
        idempotency_key: &str,
    ) -> Result<StepAck, AppError>
    where
        S: PathExecutionStore,
    {
        let blob = report.descriptor;
        let scope = match self
            .prepare_blob_completion(
                principal,
                task,
                blob.id(),
                report.expected_digest,
                idempotency_key,
            )
            .await?
        {
            BlobCompletionTarget::Replayed(ack) => return Ok(ack),
            BlobCompletionTarget::Ready(scope) => scope,
        };
        if let Some(content) = report.content
            && (content.len()
                > sagat_domain::database::DatabaseLimits::MAX.max_result_bytes as usize
                || content.len() as u64 != blob.size()
                || Sha256::digest(content).as_slice() != blob.digest().as_bytes())
        {
            return Err(AppError::NotAllowed(
                "managed blob content does not match its descriptor",
            ));
        }
        if !task.instance.path().is_root() {
            let output = descriptor_json(blob);
            let identity = blob_report_identity(blob.id(), report.expected_digest);
            let idempotency = completion_idempotency(task, idempotency_key, &identity, true);
            return self
                .report_path(
                    principal,
                    task,
                    super::scoped_reports::PathReport::Completion {
                        output: &output,
                        blob: Some(BlobCommit::new(scope, blob)),
                    },
                    idempotency,
                )
                .await;
        }
        self.complete_inner(
            principal,
            task,
            &descriptor_json(blob),
            Some(BlobCompletion {
                commit: BlobCommit::new(scope, blob),
                identity: blob_report_identity(blob.id(), report.expected_digest),
                content: report.content,
                content_type: blob.content_type().as_str(),
            }),
            idempotency_key,
        )
        .await
    }

    async fn complete_inner(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        output: &[u8],
        blob: Option<BlobCompletion<'_>>,
        idempotency_key: &str,
    ) -> Result<StepAck, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        if output.len() > MAX_PAYLOAD_BYTES || !self.engine.evaluator().is_json(output) {
            return Err(AppError::NotAllowed(
                "a step output must be JSON of at most 256 KiB",
            ));
        }
        let idempotency = completion_idempotency(
            task,
            idempotency_key,
            blob.as_ref().map_or(output, |b| b.identity.as_slice()),
            blob.is_some(),
        );
        if !task.instance.path().is_root() && blob.is_none() {
            return self
                .report_path(
                    principal,
                    task,
                    super::scoped_reports::PathReport::Completion { output, blob: None },
                    idempotency,
                )
                .await;
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let mut database_secret_in_output = None;
        if Self::managed_database(&mut tx, task) {
            if blob
                .as_ref()
                .is_some_and(|blob| blob.content_type != "application/json")
            {
                return Err(AppError::NotAllowed(
                    "managed database results require application/json",
                ));
            }
            let content = match &blob {
                Some(blob) => blob.content.ok_or(AppError::NotAllowed(
                    "managed database blobs require content scanning",
                ))?,
                None => output,
            };
            let before = self
                .database_output_snapshot(principal, &mut tx, task)
                .await?;
            tx.commit().await?;
            let contains = self
                .scan_database_output(&before.0, before.1, content)
                .await?;
            tx = self.open(principal, task, Lock::Wait).await?;
            if let Some(ack) = self
                .replayed(principal, &mut tx, task, &idempotency)
                .await?
            {
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
            if !task.instance.path().is_root() {
                return Err(AppError::NotAllowed(
                    "blob output for a scoped task is not enabled",
                ));
            }
            if task.instance.node().is_reconciliation() || task.instance.node().is_compensation() {
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
                .lease_unexpired_instance(task.instance, &Token::hash_of(task.lease_token))
                .await?
            {
                return Err(AppError::StaleAttempt);
            }
        }
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        // A secret the step was given must not come back in its output, where it would be stored in
        // the journal and handed to every later step: the step fails for good instead.
        let leaked_secret = if database_secret_in_output.is_some() {
            database_secret_in_output
        } else {
            match self
                .checked_secret_report(principal, task, tx, &idempotency, output, false)
                .await?
            {
                SecretReport::Ready(current, leaked) => {
                    tx = current;
                    leaked
                }
                SecretReport::Replayed(ack) => return Ok(ack),
            }
        };
        if let Some(name) = leaked_secret {
            return self
                .reject_secret_output(principal, task, tx, name, idempotency)
                .await;
        }
        // A reconciliation does not produce the output of a step, it says whether the step happened.
        if task.instance.node().is_reconciliation() {
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
        if task.instance.node().is_compensation() {
            let completed = self
                .apply_scoped_or_root_completion(&mut tx, task, effects, None)
                .await;
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
        let retained = tx.logical_output_bytes().await?;
        let output_bytes = u64::try_from(output.len()).unwrap_or(u64::MAX);
        if retained.saturating_add(output_bytes) > MAX_RUN_OUTPUT_BYTES as u64 {
            let code = if Self::database_write(&mut tx, task) {
                FailureCode::outcome_unknown()
            } else {
                FailureCode::run_data_limit()
            };
            let completed = self
                .apply_scoped_or_root_failure(
                    &mut tx,
                    task,
                    effects,
                    execution::Failure {
                        fence: task.fence,
                        code,
                        retryable: false,
                        jitter_sample: 0.0,
                    },
                )
                .await;
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
        effects.blobs = blob.map(|b| b.commit).into_iter().collect();
        let completed = self
            .apply_scoped_or_root_completion(&mut tx, task, effects, Some(output.to_vec()))
            .await;
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

    pub(super) async fn reject_secret_output(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        mut tx: S::Tx,
        name: SecretName,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        // A detection, counted when found: should the commit below fail the retry finds it again.
        metrics::counter!("secret_output_detected_total").increment(1);
        effects.audit = Some(self.leak_audit(principal, tx.info(), task, &name, "output"));
        let fallback_audit = effects.audit.clone();
        let code = if Self::database_write(&mut tx, task) {
            FailureCode::outcome_unknown()
        } else {
            FailureCode::secret_in_output()
        };
        let completed = self
            .apply_scoped_or_root_failure(
                &mut tx,
                task,
                effects,
                execution::Failure {
                    fence: task.fence,
                    code,
                    retryable: false,
                    jitter_sample: 0.0,
                },
            )
            .await;
        let result = match completed {
            Ok(()) => self.finish(tx, task, idempotency.clone()).await,
            Err(error) => {
                drop(tx);
                Err(error)
            }
        };
        self.recover_success_storage_quota(principal, task, idempotency, fallback_audit, result)
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
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let node = task.instance.node().clone();
        let fence = task.fence;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        effects.audit = audit;
        let managed_write = Self::database_write(&mut tx, task);
        if !task.instance.path().is_root() {
            self.apply_scoped_or_root_failure(
                &mut tx,
                task,
                effects,
                execution::Failure {
                    fence,
                    code: FailureCode::storage_quota_exceeded(),
                    retryable: false,
                    jitter_sample: 0.0,
                },
            )
            .await?;
        } else {
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
        }
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
        let forward = task.instance.node().forward().ok_or(AppError::Internal)?;
        let rejected = match self.engine.evaluator().reconcile_answer(output) {
            Err(failure) => {
                tracing::warn!(error = %failure, node = %task.instance.node(), "a reconciliation answered nonsense");
                Some(FailureCode::reconciliation_answer_invalid())
            }
            Ok(answer) => {
                let substitute = (answer.verdict == sagat_domain::saga::ReconcileVerdict::Applied)
                    .then(|| answer.output.clone().unwrap_or_else(|| b"{}".to_vec()));
                let exceeds_limit = if let Some(bytes) = &substitute {
                    let retained = tx.logical_output_bytes().await?;
                    let additional = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    retained.saturating_add(additional) > MAX_RUN_OUTPUT_BYTES as u64
                } else {
                    false
                };
                if exceeds_limit {
                    Some(FailureCode::run_data_limit())
                } else {
                    let (node, fence, verdict) =
                        (task.instance.node().clone(), task.fence, answer.verdict);
                    let jitter = self.entropy.unit();
                    if task.instance.path().is_root() {
                        // The forward step succeeded after all: later steps read the output it
                        // would have had.
                        if let Some(bytes) = substitute {
                            effects.outputs = vec![(forward.clone(), bytes)];
                        }
                        self.engine
                            .run(&mut tx, effects, move |def, state, now| {
                                let progress = execution::complete_reconciliation(
                                    def, state, &node, fence, verdict, jitter, now,
                                )?;
                                Ok(((), progress))
                            })
                            .await
                            .map_err(engine_error)?;
                    } else {
                        let instance = task.instance.clone();
                        let forward_instance =
                            ExecutionInstanceId::in_path(instance.path().clone(), forward.clone());
                        let outputs = substitute
                            .map(|bytes| vec![(forward_instance, bytes)])
                            .unwrap_or_default();
                        self.engine
                            .run_addressed(&mut tx, outputs, effects, move |def, state, now| {
                                let progress = execution::complete_reconciliation_instance(
                                    def, state, &instance, fence, verdict, jitter, now,
                                )?;
                                Ok(((), progress))
                            })
                            .await
                            .map_err(engine_error)?;
                    }
                    return self.finish(tx, task, idempotency).await;
                }
            }
        };
        let code = rejected.ok_or(AppError::Internal)?;
        let failure = execution::Failure {
            fence: task.fence,
            code,
            retryable: false,
            jitter_sample: 0.0,
        };
        self.apply_scoped_or_root_failure(&mut tx, task, effects, failure)
            .await?;
        self.finish(tx, task, idempotency).await
    }
}
