//! Exact-path reports with current authority, out-of-transaction scans and durable receipts.
use super::*;
use crate::blobs::BlobCommit;
use crate::secrets::StoredSecret;

type PathTx<S> = <S as PathExecutionStore>::Tx;

enum CheckedPathReport<Tx> {
    Ready {
        tx: Tx,
        leaked: Option<SecretName>,
        scanned_versions: Option<Vec<StoredSecret>>,
    },
    Replayed(StepAck),
}

pub(super) enum RefusalFencePolicy {
    LeaseMatch,
    Live { allow_cancellation: bool },
}

pub(super) struct RefusalCandidate {
    pub(super) success: execution::ScopedOutputRefusalSuccess,
    pub(super) output: Vec<u8>,
    pub(super) fence: RefusalFencePolicy,
    pub(super) scanned_versions: Option<Vec<StoredSecret>>,
}

impl RefusalCandidate {
    pub(super) fn matches_definition(
        &self,
        definition: &sagat_domain::workflow::WorkflowDefinition,
        instance: &ExecutionInstanceId,
    ) -> bool {
        let [segment] = instance.path().segments() else {
            return false;
        };
        if !matches!(
            definition
                .node(segment.scope_node())
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::ForEach { .. })
        ) {
            return false;
        }
        let Some(node) = definition.node_for_instance(instance) else {
            return false;
        };
        if node.database_operation().is_some() {
            return false;
        }
        match self.success {
            execution::ScopedOutputRefusalSuccess::Forward => {
                *node.origin() == sagat_domain::saga::NodeOrigin::Declared
                    && matches!(node.kind(), NodeKind::Activity { .. })
            }
            execution::ScopedOutputRefusalSuccess::ReconciliationApplied => {
                let sagat_domain::saga::NodeOrigin::Reconciliation(forward) = node.origin() else {
                    return false;
                };
                let forward =
                    ExecutionInstanceId::in_path(instance.path().clone(), forward.clone());
                instance.node().is_reconciliation()
                    && matches!(node.kind(), NodeKind::Activity { .. })
                    && definition
                        .node_for_instance(&forward)
                        .is_some_and(|forward| forward.database_operation().is_none())
            }
        }
    }
}

pub(super) enum PathReportApply {
    Finished(StepAck),
    Refusal(RefusalCandidate),
}

pub(super) enum PathReport<'a> {
    Completion {
        output: &'a [u8],
        blob: Option<BlobCommit>,
    },
    Failure {
        raw_code: &'a str,
        code: FailureCode,
        retryable: bool,
    },
}

impl PathReport<'_> {
    fn scan_bytes(&self) -> &[u8] {
        match self {
            Self::Completion { output, .. } => output,
            Self::Failure { raw_code, .. } => raw_code.as_bytes(),
        }
    }

    fn acknowledges_cancellation(&self) -> bool {
        matches!(
            self,
            Self::Failure {
                raw_code: "CANCELLED",
                retryable: false,
                ..
            }
        )
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    #[tracing::instrument(name = "worker.scoped.report", skip_all,
        fields(run_id = %task.run_id, tenant_id = %principal.tenant_id()))]
    pub(super) async fn report_path(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        report: PathReport<'_>,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let leak_what = if matches!(&report, PathReport::Completion { .. }) {
            "output"
        } else {
            "failure code"
        };
        let blob_report = matches!(&report, PathReport::Completion { blob: Some(_), .. });
        loop {
            let (tx, leaked, scanned_versions) = match self
                .checked_path_report(principal, task, &report, &idempotency)
                .await?
            {
                CheckedPathReport::Replayed(ack) => return Ok(ack),
                CheckedPathReport::Ready {
                    tx,
                    leaked,
                    scanned_versions,
                } => (tx, leaked, scanned_versions),
            };
            let fallback_leak = leaked.clone();
            let result = self
                .apply_path_report(
                    principal,
                    task,
                    &report,
                    tx,
                    leaked,
                    scanned_versions,
                    idempotency.clone(),
                )
                .await;
            let result = match result {
                Ok(PathReportApply::Finished(ack)) => Ok(ack),
                Ok(PathReportApply::Refusal(candidate)) => {
                    match self
                        .refuse_scoped_output(principal, task, candidate, idempotency.clone())
                        .await?
                    {
                        super::scoped_output_refusal::RefusalDisposition::Finished(ack) => {
                            return Ok(ack);
                        }
                        super::scoped_output_refusal::RefusalDisposition::RetryOrdinary => {
                            continue;
                        }
                    }
                }
                Err(error) => Err(error),
            };
            return match result {
                Err(AppError::QuotaExceeded(what))
                    if what == QuotaResource::TenantContentBytes.limit_name() =>
                {
                    metrics::counter!("quota_exceeded_total", "limit" => what).increment(1);
                    let mut tx = self.open_worker_path(principal, task).await?;
                    if let Some(stored) = tx.replay(&idempotency).await? {
                        return Ok(parse_ack(&stored, true, task.run_id));
                    }
                    if blob_report {
                        Self::live_path_fence(&mut tx, task, true).await?;
                    } else {
                        Self::verify_path_lease(&mut tx, task).await?;
                    }
                    let now = tx.now();
                    let snapshot = tx.snapshot();
                    let progress = execution::fail_path_instance(
                        &snapshot.definition,
                        &mut snapshot.state,
                        task.instance,
                        execution::Failure {
                            fence: task.fence,
                            code: FailureCode::storage_quota_exceeded(),
                            retryable: false,
                            jitter_sample: 0.0,
                        },
                        now,
                    )
                    .map_err(|e| engine_error(EngineError::Execution(e)))?;
                    let mut effects =
                        Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
                    effects.reported = Some(reported_lease(task));
                    if let Some(name) = &fallback_leak {
                        effects.audit = Some(self.leak_audit(
                            principal,
                            &tx.snapshot().info,
                            task,
                            name,
                            leak_what,
                        ));
                    }
                    tx.persist_addressed(&progress.events, &[], effects).await?;
                    self.finish_path_report(tx, task, idempotency).await
                }
                other => other,
            };
        }
    }

    #[tracing::instrument(name = "secret.output.scan", skip_all)]
    async fn checked_path_report(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        report: &PathReport<'_>,
        idempotency: &Idempotency,
    ) -> Result<CheckedPathReport<PathTx<S>>, AppError> {
        let mut tx = self.open_worker_path(principal, task).await?;
        if let Some(stored) = tx.replay(idempotency).await? {
            return Ok(CheckedPathReport::Replayed(parse_ack(
                &stored,
                true,
                task.run_id,
            )));
        }
        Self::verify_path_lease(&mut tx, task).await?;
        let names = tx
            .snapshot()
            .definition
            .node_for_instance(task.instance)
            .map(|node| node.secrets().to_vec())
            .unwrap_or_default();
        if matches!(report, PathReport::Completion { blob: Some(_), .. }) {
            if task.instance.node().is_derived() {
                return Err(AppError::NotAllowed(
                    "blob output requires a forward activity",
                ));
            }
            if !names.is_empty() {
                return Err(AppError::NotAllowed(
                    "blob outputs with declared secrets require content scanning",
                ));
            }
            // As for Root blob reports, retain the live lease but allow a known result to be
            // recorded during cancellation so scope drain can account for the effect.
            Self::live_path_fence(&mut tx, task, true).await?;
        }
        if self.vault.is_none() || names.is_empty() {
            return Ok(CheckedPathReport::Ready {
                tx,
                leaked: None,
                scanned_versions: None,
            });
        }
        let allow_cancellation = report.acknowledges_cancellation();
        Self::live_path_fence(&mut tx, task, allow_cancellation).await?;
        let before = tx.secret_versions(&names).await?;
        tx.commit().await?;
        let scanned = self.secret_in_output(&before, report.scan_bytes()).await;
        let mut tx = self.open_worker_path(principal, task).await?;
        if let Some(stored) = tx.replay(idempotency).await? {
            return Ok(CheckedPathReport::Replayed(parse_ack(
                &stored,
                true,
                task.run_id,
            )));
        }
        Self::live_path_fence(&mut tx, task, allow_cancellation).await?;
        if before != tx.secret_versions(&names).await? {
            return Err(AppError::Unavailable);
        }
        Ok(CheckedPathReport::Ready {
            tx,
            leaked: scanned?,
            scanned_versions: Some(before),
        })
    }

    #[allow(clippy::too_many_arguments)] // Retain the scanned secret versions with the checked report.
    async fn apply_path_report(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        report: &PathReport<'_>,
        mut tx: PathTx<S>,
        leaked: Option<SecretName>,
        scanned_versions: Option<Vec<StoredSecret>>,
        idempotency: Idempotency,
    ) -> Result<PathReportApply, AppError> {
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        let mut failure = None;
        let mut outputs = Vec::new();
        let mut verdict = None;
        let mut output_blob = None;
        if let Some(name) = leaked {
            metrics::counter!("secret_output_detected_total").increment(1);
            effects.audit = Some(self.leak_audit(
                principal,
                &tx.snapshot().info,
                task,
                &name,
                if matches!(report, PathReport::Completion { .. }) {
                    "output"
                } else {
                    "failure code"
                },
            ));
            failure = Some((FailureCode::secret_in_output(), false));
        } else {
            match report {
                PathReport::Failure {
                    code, retryable, ..
                } => failure = Some((code.clone(), *retryable)),
                PathReport::Completion { output, .. }
                    if task.instance.node().is_reconciliation() =>
                {
                    match self.engine.evaluator().reconcile_answer(output) {
                        Err(error) => {
                            tracing::warn!(%error, "a scoped reconciliation answered nonsense");
                            failure = Some((FailureCode::reconciliation_answer_invalid(), false));
                        }
                        Ok(answer) => {
                            verdict = Some(answer.verdict);
                            if answer.verdict == sagat_domain::saga::ReconcileVerdict::Applied {
                                let forward =
                                    task.instance.node().forward().ok_or(AppError::Internal)?;
                                outputs.push((
                                    ExecutionInstanceId::in_path(
                                        task.instance.path().clone(),
                                        forward.clone(),
                                    ),
                                    answer.output.unwrap_or_else(|| b"{}".to_vec()),
                                ));
                            }
                        }
                    }
                }
                PathReport::Completion { output, blob }
                    if !task.instance.node().is_compensation() =>
                {
                    outputs.push((task.instance.clone(), output.to_vec()));
                    output_blob = blob.clone();
                }
                PathReport::Completion { .. } => {}
            }
        }
        if !outputs.is_empty() {
            let additional = outputs
                .iter()
                .map(|(_, bytes)| bytes.len() as u64)
                .sum::<u64>();
            let exceeds = tx
                .logical_output_bytes()
                .await?
                .checked_add(additional)
                .is_none_or(|total| total > MAX_RUN_OUTPUT_BYTES as u64);
            if exceeds {
                let success = if task.instance.node().is_reconciliation()
                    && verdict == Some(sagat_domain::saga::ReconcileVerdict::Applied)
                {
                    Some(execution::ScopedOutputRefusalSuccess::ReconciliationApplied)
                } else if !task.instance.node().is_derived() {
                    Some(execution::ScopedOutputRefusalSuccess::Forward)
                } else {
                    None
                };
                if let Some(success) = success {
                    let fence = if matches!(report, PathReport::Completion { blob: Some(_), .. }) {
                        RefusalFencePolicy::Live {
                            allow_cancellation: true,
                        }
                    } else if scanned_versions.is_some() {
                        RefusalFencePolicy::Live {
                            allow_cancellation: false,
                        }
                    } else {
                        RefusalFencePolicy::LeaseMatch
                    };
                    let candidate = RefusalCandidate {
                        success,
                        output: outputs[0].1.clone(),
                        fence,
                        scanned_versions,
                    };
                    if candidate.matches_definition(&tx.snapshot().definition, task.instance) {
                        return Ok(PathReportApply::Refusal(candidate));
                    }
                }
                outputs.clear();
                failure = Some((FailureCode::run_data_limit(), false));
            }
        }
        let now = tx.now();
        if failure.is_none() {
            effects.blobs.extend(output_blob);
        }
        let snapshot = tx.snapshot();
        let progress = if let Some((code, retryable)) = failure {
            execution::fail_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                task.instance,
                execution::Failure {
                    fence: task.fence,
                    code,
                    retryable,
                    jitter_sample: self.entropy.unit(),
                },
                now,
            )
        } else if let Some(verdict) = verdict {
            execution::complete_reconciliation_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                task.instance,
                task.fence,
                verdict,
                self.entropy.unit(),
                now,
            )
        } else {
            execution::complete_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                task.instance,
                task.fence,
            )
        }
        .map_err(|e| engine_error(EngineError::Execution(e)))?;
        tx.persist_addressed(&progress.events, &outputs, effects)
            .await?;
        self.finish_path_report(tx, task, idempotency)
            .await
            .map(PathReportApply::Finished)
    }

    async fn finish_path_report(
        &self,
        mut tx: PathTx<S>,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let snapshot = tx.snapshot();
        let step_status = snapshot
            .state
            .local_instance(task.instance.node())
            .map_or(StepStatus::Cancelled, |s| s.status());
        let run_status = snapshot.state.global().status();
        let version = snapshot.state.global().version();
        let mut effects = Effects::by(Actor::Worker("ack".into()), None);
        effects.idempotency = Some((
            idempotency,
            format!(
                "{}|{}|{}",
                step_status.as_str(),
                run_status.as_str(),
                version
            ),
        ));
        tx.persist_addressed(&[], &[], effects).await?;
        tx.commit().await?;
        Ok(StepAck {
            command_id: format!("{}:{version}", task.run_id),
            already_applied: false,
            step_status,
            run_status,
        })
    }
}
