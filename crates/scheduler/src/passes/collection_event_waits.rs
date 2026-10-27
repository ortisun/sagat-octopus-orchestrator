//! One clocked event subscription or deferred local route per collection transition.
use super::*;
use sagat_application::execution::RunData;
use sagat_domain::execution::{CorrelationKey, StepState, StepStatus};

pub(super) fn local_progress_candidate(
    definition: &WorkflowDefinition,
    instance: &ExecutionInstanceId,
    step: &StepState,
) -> bool {
    let Some(node) = definition.node_for_instance(instance) else {
        return false;
    };
    !instance.path().is_root()
        && step.is_valid_scoped_event_wait(node)
        && (step.status() == StepStatus::Pending
            || (matches!(step.status(), StepStatus::Succeeded | StepStatus::Failed)
                && !step.routed()))
}

/// The recorded failure has a declared local handler whose route is still deferred by pause.
/// It is not a handled failure yet; normal failure classification resumes after routing.
pub(super) fn deferred_error_route(
    definition: &WorkflowDefinition,
    instance: &ExecutionInstanceId,
    step: &StepState,
) -> bool {
    step.status() == StepStatus::Failed
        && !step.routed()
        && local_progress_candidate(definition, instance, step)
        && definition
            .node_for_instance(instance)
            .is_some_and(|node| node.on_error().is_some())
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
    fn event_wait_key(
        &self,
        definition: &WorkflowDefinition,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<Result<CorrelationKey, ()>, CollectionAdvanceError> {
        let Some(NodeKind::WaitEvent { correlation, .. }) = definition
            .node_for_instance(instance)
            .map(sagat_domain::workflow::Node::kind)
        else {
            return Err(CollectionAdvanceError::InvalidProgress);
        };
        Ok(self
            .engine
            .evaluator()
            .correlation_key_instance(correlation, data, instance)
            .map_err(|_| ()))
    }

    #[tracing::instrument(name="scheduler.action",skip_all,
        fields(kind="collection_wait_event",run_id=%transition.run_id(),tenant_id=%transition.tenant_id(),
            trace_id=transition.trace_id(),parent_span_id=%sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance=?instance,expected_version=transition.expected_run_version()))]
    pub(super) async fn advance_collection_event_wait(
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
        let key = if snapshot
            .state
            .local_instance(instance.node())
            .is_some_and(|step| step.status() == StepStatus::Pending)
        {
            self.event_wait_key(&snapshot.definition, &snapshot.data, instance)?
        } else {
            Err(())
        };
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
                .is_some_and(|step| step.status() == StepStatus::Pending)
            {
                execution::start_event_wait_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    instance,
                    key,
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
        match tx.commit().await {
            Ok(()) => Ok(CollectionAdvance::BodyProgress),
            Err(error) if content_store_quota(&error) => {
                self.drain_collection_for_storage_quota(transition, scope_id)
                    .await
            }
            Err(error) => Err(error.into()),
        }
    }

    #[tracing::instrument(name="scheduler.action",skip_all,
        fields(kind="collection_wait_event",run_id=%transition.run_id(),tenant_id=%transition.tenant_id(),
            trace_id=transition.trace_id(),parent_span_id=%sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance=?instance,expected_version=transition.expected_run_version()))]
    pub(super) async fn advance_collection_event_wait_from_run<Tx: RunTx>(
        &self,
        view: &View,
        transition: CollectionTransition,
        mut tx: Tx,
        instance: &ExecutionInstanceId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        let pending = {
            let (definition, state) = tx.parts();
            let local = execution::PathExecutionState::from_run(state, instance.path().clone())
                .map_err(|_| CollectionAdvanceError::InvalidProgress)?;
            if !local.allows_forward_progress()
                || !state
                    .instance(instance)
                    .is_some_and(|step| local_progress_candidate(definition, instance, step))
            {
                return Ok(CollectionAdvance::Waiting);
            }
            state
                .instance(instance)
                .is_some_and(|step| step.status() == StepStatus::Pending)
        };
        let key = if pending {
            let data = tx.data().await?.clone();
            let (definition, _) = tx.parts();
            self.event_wait_key(definition, &data, instance)?
        } else {
            Err(())
        };
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
                let progress = if pending {
                    execution::start_event_wait_instance(definition, state, instance, key, now)?
                } else {
                    execution::release_pending_instance_path(definition, state, instance.path())?
                };
                Ok((!progress.events.is_empty(), progress))
            })
            .await;
        match result {
            Ok(true) => {}
            Ok(false) => return Ok(CollectionAdvance::Waiting),
            Err(error) if content_quota(&error) => {
                drop(tx);
                return self.drain_event_wait_for_storage_quota(transition).await;
            }
            Err(error) => return Err(error.into()),
        }
        match tx.commit().await {
            Ok(()) => Ok(CollectionAdvance::BodyProgress),
            Err(error) if content_store_quota(&error) => {
                self.drain_event_wait_for_storage_quota(transition).await
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn drain_event_wait_for_storage_quota(
        &self,
        transition: CollectionTransition,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        let scope = self
            .store
            .collection_scope(
                transition.tenant_id(),
                transition.run_id(),
                transition.owner(),
            )
            .await?
            .ok_or(CollectionAdvanceError::InvalidProgress)?;
        self.drain_collection_for_storage_quota(transition, scope.scope_id)
            .await
    }
}
