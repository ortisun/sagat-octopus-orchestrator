//! Exact dormant ChildRoot Vacate port; shared native transfer protocol.
use crate::capacity_transfer::{TransferOperation, transfer, vacate_error};
use sagat_application::capacity_vacate::{
    CapacityVacateCommand, CapacityVacateError as Error, CapacityVacateResult,
    CurrentChildRootVacateStore,
};
use sagat_telemetry::{CAPACITY_VACATE_SPAN, CapacityVacateOutcome};
use sqlx::PgPool;
use tracing::Instrument;

#[derive(Clone)]
pub struct PgCapacityVacateStore {
    pool: PgPool,
}
impl PgCapacityVacateStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CurrentChildRootVacateStore for PgCapacityVacateStore {
    async fn vacate_current_child_root(
        &self,
        command: &CapacityVacateCommand,
    ) -> Result<CapacityVacateResult, Error> {
        let i = command.identity;
        // Inherits a caller's trace. Untraced direct calls still emit metrics but no OTLP span;
        // a future production caller must establish trace context before entering this port.
        let span = tracing::info_span!(
            CAPACITY_VACATE_SPAN,
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
                TransferOperation::Vacate,
                command.operation_key,
                command.identity,
                command.expected,
                &command.scheduler,
                command.fingerprint(),
            )
            .await
            .map_err(vacate_error)
            .map(|receipt| CapacityVacateResult {
                group_id: i.group_id,
                member_id: i.member_id,
                head_run_id: i.head_run_id,
                revision: receipt.revision,
                epoch: receipt.epoch,
                active_members: receipt.active_members,
                possession: receipt.possession,
                vacated_at: receipt.transitioned_at,
                audit_id: receipt.audit_id,
                outbox_id: receipt.outbox_id,
                replayed: receipt.replayed,
            });
            let outcome = match &result {
                Ok(result) if result.replayed => CapacityVacateOutcome::Replayed,
                Ok(_) => CapacityVacateOutcome::Applied,
                Err(Error::Busy) => CapacityVacateOutcome::Busy,
                Err(Error::Storage(_)) => CapacityVacateOutcome::StorageError,
                Err(Error::CommitOutcomeUnknown { .. }) => CapacityVacateOutcome::Unknown,
                Err(Error::NotQuiescent) => CapacityVacateOutcome::NotQuiescent,
                Err(Error::NotHolder) => CapacityVacateOutcome::NotHolder,
                Err(
                    Error::StaleHead { .. }
                    | Error::StaleGroupRevision { .. }
                    | Error::StaleGroupEpoch { .. }
                    | Error::StaleChainRevision { .. }
                    | Error::StaleRunRevision { .. },
                ) => CapacityVacateOutcome::Stale,
                Err(Error::SchedulerFenceLost | Error::Missing | Error::BindingMismatch) => {
                    CapacityVacateOutcome::Denied
                }
                Err(
                    Error::ChainFinal
                    | Error::ContradictoryProjection
                    | Error::InactiveMember
                    | Error::FingerprintConflict
                    | Error::CounterExhausted,
                ) => CapacityVacateOutcome::Refused,
            };
            // A disabled child leaves the caller current. Record only on this exact handle.
            span.record("sagat.outcome", outcome.as_str());
            if matches!(
                outcome,
                CapacityVacateOutcome::StorageError | CapacityVacateOutcome::Unknown
            ) {
                span.record("otel.status_code", "error");
            }
            outcome.record(started.elapsed());
            tracing::debug!(
                outcome = outcome.as_str(),
                "capacity Vacate attempt completed"
            );
            result
        }
        .instrument(entered)
        .await
    }
}
