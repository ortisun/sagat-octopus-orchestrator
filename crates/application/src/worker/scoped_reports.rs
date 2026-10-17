//! Exact-path inline reports with current authority, out-of-transaction scans and durable receipts.
use super::*;

type PathTx<S> = <S as PathExecutionStore>::Tx;

pub(super) enum PathReport<'a> {
    Completion(&'a [u8]),
    Failure {
        raw_code: &'a str,
        code: FailureCode,
        retryable: bool,
    },
}

impl PathReport<'_> {
    fn scan_bytes(&self) -> &[u8] {
        match self {
            Self::Completion(bytes) => bytes,
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
        let (tx, leaked) = match self
            .checked_path_report(principal, task, &report, &idempotency)
            .await?
        {
            SecretReport::Replayed(ack) => return Ok(ack),
            SecretReport::Ready(tx, leaked) => (tx, leaked),
        };
        let fallback_leak = leaked.clone();
        let leak_what = if matches!(&report, PathReport::Completion(_)) {
            "output"
        } else {
            "failure code"
        };
        let result = self
            .apply_path_report(principal, task, report, tx, leaked, idempotency.clone())
            .await;
        match result {
            Err(AppError::QuotaExceeded(what))
                if what == QuotaResource::TenantContentBytes.limit_name() =>
            {
                metrics::counter!("quota_exceeded_total", "limit" => what).increment(1);
                let mut tx = self.open_worker_path(principal, task).await?;
                if let Some(stored) = tx.replay(&idempotency).await? {
                    return Ok(parse_ack(&stored, true, task.run_id));
                }
                Self::verify_path_lease(&mut tx, task).await?;
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
        }
    }

    #[tracing::instrument(name = "secret.output.scan", skip_all)]
    async fn checked_path_report(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        report: &PathReport<'_>,
        idempotency: &Idempotency,
    ) -> Result<SecretReport<PathTx<S>>, AppError> {
        let mut tx = self.open_worker_path(principal, task).await?;
        if let Some(stored) = tx.replay(idempotency).await? {
            return Ok(SecretReport::Replayed(parse_ack(
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
        if self.vault.is_none() || names.is_empty() {
            return Ok(SecretReport::Ready(tx, None));
        }
        let allow_cancellation = report.acknowledges_cancellation();
        Self::live_path_fence(&mut tx, task, allow_cancellation).await?;
        let before = tx.secret_versions(&names).await?;
        tx.commit().await?;
        let scanned = self.secret_in_output(&before, report.scan_bytes()).await;
        let mut tx = self.open_worker_path(principal, task).await?;
        if let Some(stored) = tx.replay(idempotency).await? {
            return Ok(SecretReport::Replayed(parse_ack(
                &stored,
                true,
                task.run_id,
            )));
        }
        Self::live_path_fence(&mut tx, task, allow_cancellation).await?;
        if before != tx.secret_versions(&names).await? {
            return Err(AppError::Unavailable);
        }
        Ok(SecretReport::Ready(tx, scanned?))
    }

    async fn apply_path_report(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        report: PathReport<'_>,
        mut tx: PathTx<S>,
        leaked: Option<SecretName>,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        let mut failure = None;
        let mut outputs = Vec::new();
        let mut verdict = None;
        if let Some(name) = leaked {
            metrics::counter!("secret_output_detected_total").increment(1);
            effects.audit = Some(self.leak_audit(
                principal,
                &tx.snapshot().info,
                task,
                &name,
                if matches!(report, PathReport::Completion(_)) {
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
                } => failure = Some((code, retryable)),
                PathReport::Completion(output) if task.instance.node().is_reconciliation() => {
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
                PathReport::Completion(output) if !task.instance.node().is_compensation() => {
                    outputs.push((task.instance.clone(), output.to_vec()));
                }
                PathReport::Completion(_) => {}
            }
        }
        if !outputs.is_empty() {
            let additional = outputs
                .iter()
                .map(|(_, bytes)| bytes.len() as u64)
                .sum::<u64>();
            if tx.logical_output_bytes().await?.saturating_add(additional)
                > MAX_RUN_OUTPUT_BYTES as u64
            {
                outputs.clear();
                failure = Some((FailureCode::run_data_limit(), false));
            }
        }
        let now = tx.now();
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
        self.finish_path_report(tx, task, idempotency).await
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
