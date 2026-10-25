//! Current exact-path blob authority. Object bytes still pass through the existing blob service.
use super::*;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    #[tracing::instrument(name = "worker.scoped.blob.authorize", skip_all,
        fields(run_id = %task.run_id, tenant_id = %principal.tenant_id()))]
    pub(super) async fn check_path_blob_access(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        writing: bool,
        audit_read: Option<(BlobId, Option<&str>)>,
    ) -> Result<BlobScope, AppError> {
        let mut tx = self.open_worker_path(principal, task).await?;
        let refuses_secret_upload = writing
            && tx
                .snapshot()
                .definition
                .node_for_instance(task.instance)
                .is_some_and(|node| !node.secrets().is_empty());
        // A cancelled but still current attempt retains the existing upload-policy refusal.
        // This exception only returns a denial; it never grants a capability over cancellation.
        Self::live_path_fence(&mut tx, task, refuses_secret_upload).await?;
        if refuses_secret_upload {
            return Err(AppError::NotAllowed(
                "blob outputs with declared secrets require content scanning",
            ));
        }
        let scope = Self::scope_from_info(&tx.snapshot().info, task);
        if let Some((blob, trace_id)) = audit_read {
            let effects = self.blob_read_effects(principal, task, scope, blob, trace_id);
            tx.persist_addressed(&[], &[], effects).await?;
        }
        tx.commit().await?;
        Ok(scope)
    }
}
