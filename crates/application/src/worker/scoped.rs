//! Current exact-path worker metadata commands; no full-run restoration or provider I/O.
use super::*;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
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

    async fn ensure_path_bound<Tx: PathExecutionTx>(
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
