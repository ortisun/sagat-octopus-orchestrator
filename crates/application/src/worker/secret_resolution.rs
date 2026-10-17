//! One secret handoff policy over the existing Root and exact-path transaction lifecycles.
use super::*;
use crate::execution::RunInfo;
use crate::secrets::SecretInputs;

pub(super) enum SecretResolutionTx<Root, Scoped> {
    Root(Root),
    Scoped(Scoped),
}

impl<Root: RunTx, Scoped: PathExecutionTx> SecretResolutionTx<Root, Scoped> {
    pub(super) fn info(&mut self) -> &RunInfo {
        match self {
            Self::Root(tx) => tx.info(),
            Self::Scoped(tx) => &tx.snapshot().info,
        }
    }

    pub(super) fn declared_secrets(&mut self, task: &TaskRef<'_>) -> Vec<SecretName> {
        let definition = match self {
            Self::Root(tx) => tx.parts().0,
            Self::Scoped(tx) => &tx.snapshot().definition,
        };
        definition
            .node_for_instance(task.instance)
            .map(|node| node.secrets().to_vec())
            .unwrap_or_default()
    }

    pub(super) async fn secret_inputs(
        &mut self,
        names: &[SecretName],
    ) -> Result<SecretInputs, StoreError> {
        match self {
            Self::Root(tx) => tx.secret_inputs(names).await,
            Self::Scoped(tx) => tx.secret_inputs(names).await,
        }
    }

    pub(super) async fn secret_pool_refusal(
        &mut self,
        names: &[SecretName],
        subject: &str,
        check: PoolCheck,
    ) -> Result<Option<SecretName>, StoreError> {
        match self {
            Self::Root(tx) => tx.secret_pool_refusal(names, subject, check).await,
            Self::Scoped(tx) => tx.secret_pool_refusal(names, subject, check).await,
        }
    }

    pub(super) async fn audit(&mut self, effects: Effects) -> Result<(), StoreError> {
        match self {
            Self::Root(tx) => tx.persist(&[], effects).await,
            Self::Scoped(tx) => tx.persist_addressed(&[], &[], effects).await,
        }
    }

    pub(super) async fn commit(self) -> Result<(), StoreError> {
        match self {
            Self::Root(tx) => tx.commit().await,
            Self::Scoped(tx) => tx.commit().await,
        }
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub(super) async fn open_secret_resolution(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<SecretResolutionTx<<S as RunStore>::Tx, <S as PathExecutionStore>::Tx>, AppError>
    {
        if task.instance.path().is_root() {
            let mut tx = self.open(principal, task, Lock::Wait).await?;
            self.live_capability_fence(principal, &mut tx, task).await?;
            Ok(SecretResolutionTx::Root(tx))
        } else {
            let mut tx = self.open_worker_path(principal, task).await?;
            Self::live_path_fence(&mut tx, task, false).await?;
            Ok(SecretResolutionTx::Scoped(tx))
        }
    }
}
