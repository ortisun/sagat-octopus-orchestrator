//! One local control decision per collection transition; bounded snapshots use revision CAS.

use super::*;
use sagat_application::execution::RunData;
use sagat_domain::execution::{Resolution, StepStatus, SwitchSelection};

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
    fn control_answer(
        &self,
        definition: &WorkflowDefinition,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<Resolution, CollectionAdvanceError> {
        let kind = definition
            .node_for_instance(instance)
            .ok_or(CollectionAdvanceError::InvalidProgress)?
            .kind();
        let evaluate = |expression| {
            self.engine
                .evaluator()
                .condition_instance(expression, data, instance)
                .map_err(|_| ())
        };
        match kind {
            NodeKind::Condition { expression, .. } => Ok(Resolution::Condition {
                node: instance.node().clone(),
                outcome: evaluate(expression),
            }),
            NodeKind::Switch { cases, .. } => {
                let mut selection = Ok(SwitchSelection::Default);
                for case in cases {
                    match evaluate(case.when()) {
                        Ok(true) => {
                            selection = Ok(SwitchSelection::Case(case.id().clone()));
                            break;
                        }
                        Ok(false) => {}
                        Err(()) => {
                            selection = Err(());
                            break;
                        }
                    }
                }
                Ok(Resolution::Switch {
                    node: instance.node().clone(),
                    selection,
                })
            }
            _ => Err(CollectionAdvanceError::InvalidProgress),
        }
    }

    #[tracing::instrument(
        name = "scheduler.action", skip_all,
        fields(kind = "collection_control", run_id = %transition.run_id(),
            tenant_id = %transition.tenant_id(), trace_id = transition.trace_id(),
            parent_span_id = %sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance = ?instance, expected_version = transition.expected_run_version())
    )]
    pub(super) async fn advance_collection_control(
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
                .is_some_and(|step| step.status() == StepStatus::Pending)
        {
            return Ok(CollectionAdvance::Waiting);
        }
        let answer = self.control_answer(&snapshot.definition, &snapshot.data, instance)?;
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
        let events = {
            let snapshot = tx.snapshot();
            execution::resolve_control_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                instance,
                answer,
            )?
            .events
        };
        let effects = Effects::by(
            Actor::System("scheduler"),
            transition.trace_id().map(str::to_owned),
        );
        match tx.persist_addressed(&events, &[], effects).await {
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

    #[tracing::instrument(
        name = "scheduler.action", skip_all,
        fields(kind = "collection_control", run_id = %transition.run_id(),
            tenant_id = %transition.tenant_id(), trace_id = transition.trace_id(),
            parent_span_id = %sagat_telemetry::run_span_id(transition.run_id().as_uuid()),
            instance = ?instance, expected_version = transition.expected_run_version())
    )]
    pub(super) async fn advance_collection_control_from_run<Tx: RunTx>(
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
        let data = tx.data().await?.clone();
        let answer = {
            let (definition, _) = tx.parts();
            self.control_answer(definition, &data, instance)?
        };
        let effects = Effects::by(
            Actor::System("scheduler"),
            transition.trace_id().map(str::to_owned),
        );
        let result = self
            .engine
            .run_addressed(&mut tx, Vec::new(), effects, |definition, state, _| {
                Ok((
                    (),
                    execution::resolve_control_instance(definition, state, instance, answer)?,
                ))
            })
            .await;
        match result {
            Ok(()) => {}
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
        }
        tx.commit().await?;
        Ok(CollectionAdvance::BodyProgress)
    }
}
