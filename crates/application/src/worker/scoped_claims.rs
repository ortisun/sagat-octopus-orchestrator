//! Bounded worker discovery and one exact-path lease transaction at a time.
use super::*;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    #[allow(clippy::too_many_arguments)] // the existing poll's authority and finite receive budget
    pub(super) async fn claim_run(
        &self,
        principal: &Principal,
        session: &SessionRecord,
        session_token_hash: &[u8],
        run: RunId,
        path: sagat_domain::execution::InstancePath,
        remaining: usize,
        byte_budget: usize,
        trace_id: Option<&str>,
        bytes: &mut usize,
    ) -> Result<Vec<ClaimedTask>, AppError> {
        if path.is_root() {
            self.claim_root_run(
                principal,
                session,
                session_token_hash,
                run,
                remaining,
                byte_budget,
                trace_id,
                bytes,
            )
            .await
        } else if self.store.supports_scoped_worker_protocol()
            && session.accepted_protocol_minor >= SCOPED_PROTOCOL_MINOR
        {
            self.claim_path(
                principal,
                session,
                session_token_hash,
                run,
                path,
                remaining,
                byte_budget,
                trace_id,
                bytes,
            )
            .await
        } else {
            Ok(Vec::new())
        }
    }

    #[allow(clippy::too_many_arguments)] // one path plus the existing poll's authority/budget
    #[tracing::instrument(name = "worker.scoped.claim", skip_all,
        fields(run_id = %run, tenant_id = %session.tenant_id))]
    async fn claim_path(
        &self,
        principal: &Principal,
        session: &SessionRecord,
        session_token_hash: &[u8],
        run: RunId,
        path: sagat_domain::execution::InstancePath,
        remaining: usize,
        byte_budget: usize,
        trace_id: Option<&str>,
        bytes: &mut usize,
    ) -> Result<Vec<ClaimedTask>, AppError> {
        let mut tx = match self
            .store
            .begin_current_path(session.tenant_id, run, path, Lock::SkipLocked)
            .await
        {
            Ok(Some(tx)) => tx,
            Ok(None) => return Ok(Vec::new()),
            Err(StoreError::UnreadableDefinition) => {
                metrics::counter!("worker_claim_skipped_total", "reason" => "definition_unreadable")
                    .increment(1);
                tracing::warn!(%run, "a scoped run pinned to an unreadable definition was skipped by a poll");
                return Ok(Vec::new());
            }
            Err(error) => return Err(error.into()),
        };
        if tx.snapshot().info.namespace_id != session.namespace_id
            || tx.worker_drained(&session.subject).await?
        {
            return Ok(Vec::new());
        }
        // Binding replacement locks binding authority before closing sessions. Acquire that
        // same authority before the session row, then keep current membership through commit.
        let patterns = tx
            .bound_patterns(&session.subject)
            .await?
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::Internal)?;
        if sagat_domain::binding::first_uncovered(&patterns, &session.activities).is_some() {
            return Ok(Vec::new());
        }
        let room = tx
            .worker_session_room(session.id, session_token_hash, &session.subject)
            .await?;
        let remaining = remaining.min(room as usize);
        if remaining == 0
            || !tx.current_worker_authorized(&session.subject).await?
            || tx.required_worker_protocol_minor().await? > session.accepted_protocol_minor
        {
            return Ok(Vec::new());
        }
        let mut ready = {
            let snapshot = tx.snapshot();
            snapshot
                .state
                .local_instances()
                .filter(|(_, step)| step.status() == StepStatus::Ready)
                .filter_map(|(node, _)| {
                    let instance =
                        ExecutionInstanceId::in_path(snapshot.state.path().clone(), node.clone());
                    let node = snapshot.definition.node_for_instance(&instance)?;
                    if node.database_operation().is_some() {
                        return None;
                    }
                    match node.kind() {
                        NodeKind::Activity { activity, .. }
                            if session.activities.iter().any(|a| a == activity.as_str()) =>
                        {
                            Some((
                                instance,
                                activity.clone(),
                                node.timeout(),
                                node.secrets().to_vec(),
                            ))
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
        };
        ready.sort_by(|left, right| {
            left.1
                .as_str()
                .cmp(right.1.as_str())
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut claimed = Vec::new();
        let mut run_bytes = 0;
        for (instance, activity, timeout, declared) in ready {
            if claimed.len() >= remaining || *bytes + run_bytes >= byte_budget {
                break;
            }
            if !declared.is_empty()
                && let Some(withheld) = tx
                    .secret_pool_refusal(&declared, &session.subject, PoolCheck::Hint)
                    .await?
            {
                let snapshot = tx.snapshot();
                let attempt = snapshot
                    .state
                    .local_instance(instance.node())
                    .map_or(0, |s| s.attempt())
                    .saturating_add(1);
                let effects = self.pool_refusal_effects(
                    principal,
                    &snapshot.info,
                    &instance,
                    attempt,
                    &withheld,
                    "lease",
                    trace_id,
                );
                tx.persist_addressed(&[], &[], effects).await?;
                metrics::counter!("secret_delivery_refused_total", "stage" => "lease").increment(1);
                continue;
            }
            let input = self.engine.evaluator()
                .task_input_instance(&tx.snapshot().data, &instance)
                .map_err(|error| {
                    tracing::error!(%error, instance = ?instance, "failed to materialize scoped task input");
                    AppError::Internal
                })?;
            if input.len() > MAX_TASK_INPUT_BYTES {
                let now = tx.now();
                let snapshot = tx.snapshot();
                let (fence, leased) =
                    execution::lease_path_instance(&mut snapshot.state, &instance)
                        .map_err(|e| engine_error(EngineError::Execution(e)))?;
                let mut progress = execution::fail_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    &instance,
                    execution::Failure {
                        fence,
                        code: FailureCode::run_data_limit(),
                        retryable: false,
                        jitter_sample: 0.0,
                    },
                    now,
                )
                .map_err(|e| engine_error(EngineError::Execution(e)))?;
                progress.events.insert(0, leased);
                tx.persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(
                        Actor::Worker(session.id.to_string()),
                        trace_id.map(str::to_owned),
                    ),
                )
                .await?;
                metrics::counter!("worker_task_input_rejected_total", "reason" => "run_data_limit")
                    .increment(1);
                break;
            }
            if input.len() > byte_budget - *bytes - run_bytes {
                break;
            }
            if !tx.activity_admitted(&activity).await? {
                metrics::counter!("quota_exceeded_total", "limit" => "activity_concurrency")
                    .increment(1);
                continue;
            }
            if instance.node().is_compensation() && !tx.compensation_admitted(&activity).await? {
                continue;
            }
            if !tx.script_execution_admitted(&instance, &activity).await? {
                metrics::counter!("quota_exceeded_total", "limit" => QuotaResource::ScriptRetentionDays.limit_name()).increment(1);
                continue;
            }
            let token = Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)?;
            let snapshot = tx.snapshot();
            let (fence, event) = execution::lease_path_instance(&mut snapshot.state, &instance)
                .map_err(|e| engine_error(EngineError::Execution(e)))?;
            let mut effects = Effects::by(
                Actor::Worker(session.id.to_string()),
                trace_id.map(str::to_owned),
            );
            effects.lease = Some(LeaseGrant {
                instance: instance.clone(),
                token_hash: token.hash(),
                worker_id: session.id.to_string(),
                ttl: self.settings.lease_ttl,
                attempt_timeout: timeout,
            });
            tx.persist_addressed(&[event], &[], effects).await?;
            let expires_at = tx
                .lease_expiry(&instance)
                .await?
                .ok_or(AppError::Internal)?;
            let snapshot = tx.snapshot();
            let (kind, idempotency_key, target) = self.task_identity(
                Self::task_template(&snapshot.definition, &instance),
                &snapshot.data,
                &snapshot.info,
                &instance,
            )?;
            run_bytes += input.len();
            claimed.push(ClaimedTask {
                run_id: run,
                step_id: instance_step_id(snapshot.info.version_id, run, &instance),
                instance,
                fence,
                lease_token: token,
                expires_at,
                activity,
                input,
                input_blob: snapshot.data.input_blob,
                idempotency_key,
                trace_id: snapshot.info.trace_id.clone(),
                kind,
                target,
            });
        }
        tx.commit().await?;
        *bytes += run_bytes;
        Ok(claimed)
    }
}
