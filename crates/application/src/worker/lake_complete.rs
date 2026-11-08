//! The completion of a lake step (feature 035, T035-059, D035-142): the receipt the runner reports is read into typed fields,
//! checked against the step, and written again. What the run keeps as the output of the step is that receipt, never the bytes
//! the runner sent.
use super::*;
use crate::database::ExpectedReceipt;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// The canonical receipt of the Iceberg write `task` is the attempt of, or the refusal of what the runner reported. Runs in
    /// the transaction of the completion, so the lease it was checked against is the lease that completes the step.
    ///
    /// Refused without a word about why, as a managed step is: no codec to read the report with, and an attempt that never held
    /// the credentials of its write (a completion that no resolve preceded is not one). A receipt that is not the step's is
    /// refused by name and leaves the step as it was, for the runner to report the failure it has.
    pub(super) async fn lake_receipt<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        output: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        let codec = self.lake_receipts.as_ref().ok_or(AppError::Unavailable)?;
        let node = task.instance.node();
        if !tx.lake_attempt_resolved(node).await? {
            return Err(AppError::Unavailable);
        }
        let write = tx
            .parts()
            .0
            .node(node)
            .and_then(sagat_domain::workflow::Node::iceberg_write)
            .cloned()
            .ok_or(AppError::Internal)?;
        let template = Self::task_template(tx.parts().0, task.instance);
        let info = tx.info().clone();
        let data = tx.data().await?;
        let (_, effect_key, _) = self.task_identity(template, data, &info, task.instance)?;
        codec
            .rebuild(
                output,
                &ExpectedReceipt {
                    effect_key: &effect_key,
                    reference: write.reference(),
                    mode: write.intent().mode(),
                },
            )
            .map_err(|_| AppError::NotAllowed("invalid lake receipt"))
    }
}
