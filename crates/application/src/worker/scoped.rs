//! Current exact-path worker metadata commands; no full-run restoration or provider I/O.
use super::*;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub(super) async fn open_worker_path(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<<S as PathExecutionStore>::Tx, AppError> {
        require_supported_task(task.instance, self.store.supports_scoped_worker_protocol())?;
        let Some(mut tx) = self
            .store
            .begin_current_path(
                principal.tenant_id(),
                task.run_id,
                task.instance.path().clone(),
                Lock::Wait,
            )
            .await?
        else {
            return Err(self.missing_task_error(principal, task).await);
        };
        let snapshot = tx.snapshot();
        if task.step_id.is_some_and(|claimed| {
            claimed
                != instance_step_id(
                    snapshot.info.version_id,
                    snapshot.info.run_id,
                    task.instance,
                )
        }) || snapshot
            .state
            .local_instance(task.instance.node())
            .is_none()
        {
            return Err(AppError::StaleAttempt);
        }
        // Current authority precedes receipt lookup as well as every new report.
        Self::ensure_path_bound(principal, &mut tx, task).await?;
        Ok(tx)
    }

    pub(super) async fn verify_path_lease(
        tx: &mut <S as PathExecutionStore>::Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        if !tx
            .lease_matches(task.instance, &Token::hash_of(task.lease_token))
            .await?
        {
            return Err(AppError::StaleAttempt);
        }
        Ok(())
    }

    pub(super) async fn live_path_fence(
        tx: &mut <S as PathExecutionStore>::Tx,
        task: &TaskRef<'_>,
        allow_cancellation: bool,
    ) -> Result<(), AppError> {
        Self::verify_path_lease(tx, task).await?;
        let snapshot = tx.snapshot();
        let valid = snapshot
            .state
            .local_instance(task.instance.node())
            .is_some_and(|step| {
                step.attempt() == task.fence.attempt
                    && step.generation() == task.fence.generation
                    && (allow_cancellation || !step.cancel_requested())
            })
            && !snapshot.state.global().status().is_terminal()
            && (allow_cancellation
                || task.instance.node().is_derived()
                || snapshot.state.global().stop_reason().is_none());
        if !valid
            || !tx
                .lease_unexpired(task.instance, &Token::hash_of(task.lease_token))
                .await?
        {
            return Err(AppError::StaleAttempt);
        }
        Ok(())
    }

    #[tracing::instrument(name = "worker.scoped.heartbeat", skip_all,
        fields(run_id = %task.run_id, tenant_id = %principal.tenant_id()))]
    pub(super) async fn heartbeat_path(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<HeartbeatAck, AppError> {
        require_supported_task(task.instance, self.store.supports_scoped_worker_protocol())?;
        let Some(mut tx) = self
            .store
            .begin_current_path(
                principal.tenant_id(),
                task.run_id,
                task.instance.path().clone(),
                Lock::Wait,
            )
            .await?
        else {
            return Err(self.missing_task_error(principal, task).await);
        };
        let info = &tx.snapshot().info;
        if task
            .step_id
            .is_some_and(|id| id != instance_step_id(info.version_id, info.run_id, task.instance))
        {
            return Err(AppError::StaleAttempt);
        }
        let token_hash = Token::hash_of(task.lease_token);
        if !tx.lease_matches(task.instance, &token_hash).await? {
            if tx.reported_by(task.instance, &token_hash).await? {
                Self::ensure_path_bound(principal, &mut tx, task).await?;
                tx.commit().await?;
                metrics::counter!("worker_heartbeats_after_report_total").increment(1);
                return Ok(HeartbeatAck {
                    cancel_requested: false,
                    lease_expires_at: None,
                });
            }
            return Err(AppError::StaleAttempt);
        }
        Self::ensure_path_bound(principal, &mut tx, task).await?;
        let beat =
            execution::heartbeat_path_instance(&mut tx.snapshot().state, task.instance, task.fence)
                .map_err(|error| engine_error(EngineError::Execution(error)))?;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.renewal = Some(LeaseRenewal {
            instance: task.instance.clone(),
            ttl: self.settings.lease_ttl,
        });
        let events: Vec<_> = beat.event.into_iter().collect();
        tx.persist_addressed(&events, &[], effects).await?;
        let lease_expires_at = tx
            .lease_expiry(task.instance)
            .await?
            .unwrap_or_else(|| tx.now());
        tx.commit().await?;
        Ok(HeartbeatAck {
            cancel_requested: beat.cancel_requested,
            lease_expires_at: Some(lease_expires_at),
        })
    }

    pub(super) async fn ensure_path_bound<Tx: PathExecutionTx>(
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        let activity = {
            let snapshot = tx.snapshot();
            let node = snapshot.definition.node_for_instance(task.instance);
            if node.is_some_and(|node| node.database_operation().is_some()) {
                return Err(AppError::NotAllowed(
                    "scoped managed database tasks are not enabled",
                ));
            }
            match node.map(sagat_domain::workflow::Node::kind) {
                Some(NodeKind::Activity { activity, .. }) => Some(activity.to_string()),
                _ => None,
            }
        };
        if let Some(activity) = activity {
            let bound = tx
                .bound_patterns(principal.subject().as_str())
                .await?
                .iter()
                .map(|text| ActivityPattern::parse(text))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AppError::Internal)?;
            if sagat_domain::binding::first_uncovered(&bound, &[activity]).is_some() {
                return Err(AppError::NotBound);
            }
        }
        if !tx
            .current_worker_authorized(principal.subject().as_str())
            .await?
        {
            return Err(AppError::Forbidden {
                permission: Permission::WorkerExecute,
            });
        }
        Ok(())
    }
}
