//! The signal a Signal-with-Start holds with its queued run (feature 022, T022-109; FR-022-018, FR-018-024; decision
//! D022-087): written by the run's creation transaction, taken by the grant of its start (which delivers it, or drops it
//! with an audit record when no step waits for it), and dropped with an audit record by the commit that ends the queued
//! run instead. Every statement runs under the run's own lock, in the run's transaction.

use sagat_application::execution::{HeldSignal, HeldSignalDrop};
use sagat_telemetry::flow_control::HeldSignalOutcome;

use super::*;

impl PgRunTx {
    /// See [`sagat_application::execution::RunTx::hold_start_signal`].
    pub(super) async fn hold_start_signal_inner(
        &mut self,
        signal: &HeldSignal,
        receipt: &Idempotency,
        marker: &Idempotency,
    ) -> Result<(), StoreError> {
        let tenant = self.info.tenant_id;
        let digest = Sha256::digest(&signal.payload);
        sqlx::query(
            "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
             ON CONFLICT (tenant_id, digest) DO NOTHING",
        )
        .bind(tenant.as_uuid())
        .bind(digest.as_slice())
        .bind(&signal.payload)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO flow_control_held_signals (tenant_id, namespace_id, run_id, name, payload_digest, actor, \
                                                   trace_id, held_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp())",
        )
        .bind(tenant.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&signal.name)
        .bind(digest.as_slice())
        .bind(&signal.actor)
        .bind(signal.trace_id.as_deref())
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        // the receipts a retry of the request is answered from: nothing delivered, and held
        for (record, result) in [(receipt, ""), (marker, "held")] {
            remember(
                &mut self.tx,
                tenant,
                record,
                &hash(&record.fingerprint),
                result,
            )
            .await?;
        }
        self.held_signal = Some(HeldSignalOutcome::Held);
        Ok(())
    }

    /// See [`sagat_application::execution::RunTx::take_held_signal`]: one probe of the primary key.
    pub(super) async fn take_held_signal_inner(
        &mut self,
    ) -> Result<Option<HeldSignal>, StoreError> {
        let row: Option<(String, Vec<u8>, String, Option<String>)> = sqlx::query_as(
            "WITH taken AS (DELETE FROM flow_control_held_signals WHERE tenant_id = $1 AND run_id = $2 \
                            RETURNING tenant_id, name, payload_digest, actor, trace_id) \
             SELECT t.name, p.content, t.actor, t.trace_id \
               FROM taken t JOIN payloads p ON p.tenant_id = t.tenant_id AND p.digest = t.payload_digest",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        // delivered, unless its drop is audited next in this transaction
        if row.is_some() {
            self.held_signal = Some(HeldSignalOutcome::Delivered);
        }
        Ok(row.map(|(name, payload, actor, trace_id)| HeldSignal {
            name,
            payload,
            actor,
            trace_id,
        }))
    }

    /// See [`sagat_application::execution::RunTx::audit_held_signal_dropped`].
    pub(super) async fn audit_held_signal_dropped_inner(
        &mut self,
        signal: &HeldSignal,
        reason: HeldSignalDrop,
    ) -> Result<(), StoreError> {
        let now = self.now;
        insert_audit(
            &mut self.tx,
            &dropped(self.info.tenant_id, self.info.run_id, signal, reason, now),
        )
        .await?;
        // every reason its own outcome: a drop is never counted as the delivery an earlier take set (decision D022-132)
        self.held_signal = Some(match reason {
            HeldSignalDrop::NotWaitingAtGrant => HeldSignalOutcome::DroppedNotWaiting,
            HeldSignalDrop::OutputBound => HeldSignalOutcome::DroppedOutputBound,
            HeldSignalDrop::RunEnded => HeldSignalOutcome::DroppedRunEnded,
            HeldSignalDrop::DeliveryFailed => HeldSignalOutcome::DroppedDeliveryFailed,
        });
        Ok(())
    }

    /// In a commit that ends a run whose start declares a gate: drops the signal held with it, audited (decision D022-087
    /// item 4). One probe of the primary key; nothing for a run without one.
    pub(super) async fn drop_held_signal_of_ended_run(&mut self) -> Result<(), StoreError> {
        if let Some(signal) = self.take_held_signal_inner().await? {
            self.audit_held_signal_dropped_inner(&signal, HeldSignalDrop::RunEnded)
                .await?;
        }
        Ok(())
    }
}

/// The audit record of a held signal dropped for `reason`: the engine's, naming the signal, why, and the caller who sent
/// it (the held row's actor; batch 5 security minor 2), never its payload.
fn dropped(
    tenant: TenantId,
    run: sagat_domain::execution::RunId,
    signal: &HeldSignal,
    reason: HeldSignalDrop,
    now: OffsetDateTime,
) -> AuditRecord {
    AuditRecord::engine(
        tenant,
        AuditAction::RunSignalDropped,
        ResourceType::Run,
        run.to_string(),
        format!(
            "{}: signal '{}' held with the queued start, sent by '{}'",
            reason.as_str(),
            signal.name.chars().take(64).collect::<String>(),
            // the whole sender, which the held row's `CHECK` bounds at 255 characters (decision D022-129)
            signal.actor.chars().take(255).collect::<String>()
        ),
        signal.trace_id.clone(),
        now,
    )
}
