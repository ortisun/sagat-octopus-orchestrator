//! Clocked timer start and deferred local routing use one bounded member transaction.
use super::*;

pub(super) fn local_progress_candidate(
    definition: &WorkflowDefinition,
    instance: &ExecutionInstanceId,
    step: &execution::StepState,
) -> bool {
    match definition
        .node_for_instance(instance)
        .map(sagat_domain::workflow::Node::kind)
    {
        Some(NodeKind::Timer { .. }) if step.status() == execution::StepStatus::Pending => true,
        Some(
            NodeKind::Activity { .. }
            | NodeKind::Condition { .. }
            | NodeKind::Switch { .. }
            | NodeKind::Timer { .. },
        ) => step.status() == execution::StepStatus::Succeeded && !step.routed(),
        _ => false,
    }
}

impl<S, E, R> Scheduler<S, E, R>
where
    S: RunStore
        + MaintenanceStore
        + DefinitionStore
        + CollectionStore
        + LoopProgressStore
        + ScopeFrontierStore
        + ApprovalSchedulerStore,
    E: ExpressionEvaluator,
    R: Entropy,
{
    #[tracing::instrument(name="scheduler.action",skip_all,
        fields(kind="collection_local",run_id=%transition.run_id(),tenant_id=%transition.tenant_id(),
            trace_id=transition.trace_id(),parent_span_id=%sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance=?instance,expected_version=transition.expected_run_version()))]
    pub(super) async fn advance_collection_local(
        &self,
        view: &View,
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        instance: &ExecutionInstanceId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError>
    where
        S: PathExecutionStore,
    {
        let Some(snapshot) = self
            .store
            .load_path(
                transition.tenant_id(),
                transition.run_id(),
                instance.path().clone(),
            )
            .await?
        else {
            return Ok(CollectionAdvance::Waiting);
        };
        if snapshot.state.global().version() != transition.expected_run_version()
            || !snapshot.state.allows_forward_progress()
            || !snapshot
                .state
                .local_instance(instance.node())
                .is_some_and(|step| local_progress_candidate(&snapshot.definition, instance, step))
        {
            return Ok(CollectionAdvance::Waiting);
        }
        let mut tx = match self.store.begin_path(snapshot, Lock::Wait).await {
            Ok(Some(tx)) => tx,
            Ok(None) | Err(StoreError::RevisionConflict { .. }) => {
                return Ok(CollectionAdvance::Waiting);
            }
            Err(error) => return Err(error.into()),
        };
        if !view
            .holds_path(&mut tx, &view.hold_for(transition.run_id()))
            .await?
        {
            return Err(CollectionAdvanceError::FenceUnavailable);
        }
        let now = tx.now();
        let progress = {
            let snapshot = tx.snapshot();
            if snapshot
                .state
                .local_instance(instance.node())
                .is_some_and(|step| step.status() == execution::StepStatus::Pending)
            {
                execution::start_timer_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    instance,
                    now,
                )?
            } else {
                execution::release_pending_path(&snapshot.definition, &mut snapshot.state)?
            }
        };
        if progress.events.is_empty() {
            return Ok(CollectionAdvance::Waiting);
        }
        let effects = Effects::by(
            Actor::System("scheduler"),
            transition.trace_id().map(str::to_owned),
        );
        match tx.persist_addressed(&progress.events, &[], effects).await {
            Ok(()) => {}
            Err(error) if content_store_quota(&error) => {
                drop(tx);
                return self
                    .drain_collection_for_storage_quota(transition, scope_id)
                    .await;
            }
            Err(error) => return Err(error.into()),
        }
        tx.commit().await?;
        Ok(CollectionAdvance::BodyProgress)
    }

    #[tracing::instrument(name="scheduler.action",skip_all,
        fields(kind="collection_local",run_id=%transition.run_id(),tenant_id=%transition.tenant_id(),
            trace_id=transition.trace_id(),parent_span_id=%sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance=?instance,expected_version=transition.expected_run_version()))]
    pub(super) async fn advance_collection_local_from_run<Tx: RunTx>(
        &self,
        view: &View,
        transition: CollectionTransition,
        mut tx: Tx,
        instance: &ExecutionInstanceId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        if !view
            .holds(&mut tx, &view.hold_for(transition.run_id()))
            .await?
        {
            return Err(CollectionAdvanceError::FenceUnavailable);
        }
        let effects = Effects::by(
            Actor::System("scheduler"),
            transition.trace_id().map(str::to_owned),
        );
        let result = self
            .engine
            .run_addressed(&mut tx, Vec::new(), effects, |definition, state, now| {
                let progress = if state
                    .instance(instance)
                    .is_some_and(|step| step.status() == execution::StepStatus::Pending)
                {
                    execution::start_timer_instance(definition, state, instance, now)?
                } else {
                    execution::release_pending_instance_path(definition, state, instance.path())?
                };
                Ok((!progress.events.is_empty(), progress))
            })
            .await;
        let progressed = match result {
            Ok(progressed) => progressed,
            Err(error) if content_quota(&error) => {
                drop(tx);
                let scope = self
                    .store
                    .collection_scope(
                        transition.tenant_id(),
                        transition.run_id(),
                        transition.owner(),
                    )
                    .await?
                    .ok_or(CollectionAdvanceError::InvalidProgress)?;
                return self
                    .drain_collection_for_storage_quota(transition, scope.scope_id)
                    .await;
            }
            Err(error) => return Err(error.into()),
        };
        if !progressed {
            return Ok(CollectionAdvance::Waiting);
        }
        tx.commit().await?;
        Ok(CollectionAdvance::BodyProgress)
    }
}
