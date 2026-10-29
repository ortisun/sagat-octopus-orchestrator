//! Exact dormant ChildRoot Acquire port; shared native transfer protocol.
use crate::capacity_transfer::{TransferOperation, acquire_error, transfer};
use sagat_application::capacity_acquire::{
    CapacityAcquireCommand, CapacityAcquireError as Error, CapacityAcquireResult,
    CurrentChildRootAcquireStore,
};
use sagat_telemetry::{CAPACITY_ACQUIRE_SPAN, CapacityAcquireOutcome};
use sqlx::PgPool;
use tracing::Instrument;

#[derive(Clone)]
pub struct PgCapacityAcquireStore {
    pool: PgPool,
}
impl PgCapacityAcquireStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CurrentChildRootAcquireStore for PgCapacityAcquireStore {
    async fn acquire_current_child_root(
        &self,
        command: &CapacityAcquireCommand,
    ) -> Result<CapacityAcquireResult, Error> {
        let i = command.identity;
        // Inherits a caller's trace. Untraced direct calls still emit metrics but no OTLP span;
        // a future production caller must establish trace context before entering this port.
        let span = tracing::info_span!(
            CAPACITY_ACQUIRE_SPAN,
            sagat.tenant.id = %i.scope.tenant_id(),
            sagat.namespace.id = %i.scope.namespace_id(),
            sagat.capacity.group_id = %i.group_id,
            sagat.capacity.member_id = %i.member_id,
            sagat.chain.id = %i.chain_id,
            sagat.run.id = %i.head_run_id,
            sagat.outcome = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        let entered = span.clone();
        async {
            let started = std::time::Instant::now();
            let result = transfer(
                &self.pool,
                TransferOperation::Acquire,
                command.operation_key,
                command.identity,
                command.expected,
                &command.scheduler,
                command.fingerprint(),
            )
            .await
            .map_err(acquire_error)
            .map(|receipt| CapacityAcquireResult {
                group_id: i.group_id,
                member_id: i.member_id,
                head_run_id: i.head_run_id,
                revision: receipt.revision,
                epoch: receipt.epoch,
                active_members: receipt.active_members,
                possession: receipt.possession,
                acquired_at: receipt.transitioned_at,
                audit_id: receipt.audit_id,
                outbox_id: receipt.outbox_id,
                replayed: receipt.replayed,
            });
            let outcome = match &result {
                Ok(result) if result.replayed => CapacityAcquireOutcome::Replayed,
                Ok(_) => CapacityAcquireOutcome::Applied,
                Err(Error::Busy) => CapacityAcquireOutcome::Busy,
                Err(Error::Storage(_)) => CapacityAcquireOutcome::StorageError,
                Err(Error::CommitOutcomeUnknown { .. }) => CapacityAcquireOutcome::Unknown,
                Err(Error::NotMovable) => CapacityAcquireOutcome::NotMovable,
                Err(Error::Occupied) => CapacityAcquireOutcome::Occupied,
                Err(
                    Error::StaleHead { .. }
                    | Error::StaleGroupRevision { .. }
                    | Error::StaleGroupEpoch { .. }
                    | Error::StaleChainRevision { .. }
                    | Error::StaleRunRevision { .. },
                ) => CapacityAcquireOutcome::Stale,
                Err(Error::SchedulerFenceLost | Error::Missing | Error::BindingMismatch) => {
                    CapacityAcquireOutcome::Denied
                }
                Err(
                    Error::ChainFinal
                    | Error::ContradictoryProjection
                    | Error::InactiveMember
                    | Error::FingerprintConflict
                    | Error::CounterExhausted,
                ) => CapacityAcquireOutcome::Refused,
            };
            // A disabled child leaves the caller current. Record only on this exact handle.
            span.record("sagat.outcome", outcome.as_str());
            if matches!(
                outcome,
                CapacityAcquireOutcome::StorageError | CapacityAcquireOutcome::Unknown
            ) {
                span.record("otel.status_code", "error");
            }
            outcome.record(started.elapsed());
            tracing::debug!(
                outcome = outcome.as_str(),
                "capacity Acquire attempt completed"
            );
            result
        }
        .instrument(entered)
        .await
    }
}
