//! Candidate queries for the scheduler and the transactional outbox.
//!
//! Every comparison uses `clock_timestamp()` on the database, so schedulers on machines with
//! different clocks agree on what is due. The queries only *find* candidates; the authority is the
//! run's row lock, taken by the caller, under which the domain re-checks the condition.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::maintenance::{
    LeaseLoss, LostLease, MaintenanceStore, OutboxMessage, OutboxStore, RunRef, StepRef,
};
use sagat_domain::execution::{Fence, RunId};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::NodeId;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

fn step_ref(row: &PgRow) -> Result<StepRef, StoreError> {
    let node: String = col(row, "node")?;
    Ok(StepRef {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        node: NodeId::parse(&node).map_err(|_| corrupt("step node"))?,
    })
}

impl MaintenanceStore for PgRunStore {
    async fn due_timers(&self, limit: u32) -> Result<Vec<StepRef>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, run_id, node FROM steps \
             WHERE status = 'WAITING' AND waiting_kind = 'TIMER' AND due_at <= clock_timestamp() \
             ORDER BY due_at LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn due_wait_deadlines(&self, limit: u32) -> Result<Vec<StepRef>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, run_id, node FROM steps \
             WHERE status = 'WAITING' AND waiting_kind = 'EVENT' AND due_at <= clock_timestamp() \
             ORDER BY due_at LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn due_retries(&self, limit: u32) -> Result<Vec<StepRef>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, run_id, node FROM steps \
             WHERE status = 'RETRY_WAIT' AND next_attempt_at <= clock_timestamp() \
             ORDER BY next_attempt_at LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn lost_leases(&self, limit: u32) -> Result<Vec<LostLease>, StoreError> {
        // Two index scans (one per partial index) rather than one query with OR.
        let rows = sqlx::query(
            "(SELECT tenant_id, run_id, node, attempt, generation, 'TIMED_OUT' AS cause FROM steps \
              WHERE status IN ('LEASED', 'RUNNING') AND attempt_deadline_at <= clock_timestamp() \
              ORDER BY attempt_deadline_at LIMIT $1) \
             UNION ALL \
             (SELECT tenant_id, run_id, node, attempt, generation, 'EXPIRED' AS cause FROM steps \
              WHERE status IN ('LEASED', 'RUNNING') AND lease_expires_at <= clock_timestamp() \
              ORDER BY lease_expires_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        let mut out: Vec<LostLease> = Vec::with_capacity(rows.len());
        for row in &rows {
            let step = step_ref(row)?;
            let attempt: i32 = col(row, "attempt")?;
            let generation: i64 = col(row, "generation")?;
            let fence = Fence {
                attempt: u32::try_from(attempt).map_err(|_| corrupt("attempt"))?,
                generation: u64::try_from(generation).map_err(|_| corrupt("generation"))?,
            };
            let cause: String = col(row, "cause")?;
            // The timeout row comes first, so a step that is both reports the timeout.
            if out.iter().any(|l| l.step == step && l.fence == fence) {
                continue;
            }
            out.push(LostLease {
                step,
                fence,
                cause: if cause == "TIMED_OUT" {
                    LeaseLoss::TimedOut
                } else {
                    LeaseLoss::Expired
                },
            });
        }
        out.truncate(limit as usize);
        Ok(out)
    }

    async fn overdue_runs(&self, limit: u32) -> Result<Vec<RunRef>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, id FROM runs \
             WHERE finished_at IS NULL AND stop_reason IS NULL AND deadline_at <= clock_timestamp() \
             ORDER BY deadline_at LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(RunRef {
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                run_id: RunId::new(col::<Uuid>(row, "id")?),
            })
        })
        .collect()
    }

    async fn stale_hints(
        &self,
        older_than: Duration,
        limit: u32,
    ) -> Result<Vec<StepRef>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, run_id, node FROM steps \
             WHERE status = 'READY' \
               AND last_hinted_at <= clock_timestamp() - $1 * interval '1 microsecond' \
             ORDER BY last_hinted_at LIMIT $2",
        )
        .bind(micros(older_than))
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn purge_idempotency(&self, older_than: Duration) -> Result<u64, StoreError> {
        sqlx::query(
            "DELETE FROM command_dedupe WHERE created_at < clock_timestamp() - $1 * interval '1 microsecond'",
        )
        .bind(micros(older_than))
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }

    async fn purge_outbox(&self, older_than: Duration, limit: u32) -> Result<u64, StoreError> {
        sqlx::query(
            "DELETE FROM event_outbox WHERE id IN ( \
                 SELECT id FROM event_outbox \
                 WHERE delivered_at IS NOT NULL \
                   AND delivered_at < clock_timestamp() - $1 * interval '1 microsecond' \
                 ORDER BY delivered_at LIMIT $2)",
        )
        .bind(micros(older_than))
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }

    async fn purge_sessions(&self) -> Result<u64, StoreError> {
        // Kept for an hour after expiry so a late report still finds its session's identity.
        sqlx::query(
            "DELETE FROM worker_sessions WHERE expires_at < clock_timestamp() - interval '1 hour'",
        )
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }
}

impl OutboxStore for PgRunStore {
    async fn claim_outbox(
        &self,
        limit: u32,
        visibility: Duration,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT id FROM event_outbox \
                 WHERE delivered_at IS NULL AND next_attempt_at <= clock_timestamp() \
                 ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
             UPDATE event_outbox o \
                SET next_attempt_at = clock_timestamp() + $2 * interval '1 microsecond', \
                    attempt_count = o.attempt_count + 1 \
               FROM due WHERE o.id = due.id \
             RETURNING o.id, o.tenant_id, o.aggregate_id, o.event_type, o.payload, \
                       o.attempt_count, o.created_at",
        )
        .bind(i64::from(limit))
        .bind(micros(visibility))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        let mut out = rows
            .iter()
            .map(|row| {
                let payload: serde_json::Value = col(row, "payload")?;
                let attempts: i32 = col(row, "attempt_count")?;
                Ok(OutboxMessage {
                    id: col(row, "id")?,
                    tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                    aggregate_id: col(row, "aggregate_id")?,
                    event_type: col(row, "event_type")?,
                    payload: serde_json::to_vec(&payload).map_err(|_| corrupt("outbox payload"))?,
                    attempt_count: u32::try_from(attempts).map_err(|_| corrupt("attempts"))?,
                    created_at: col(row, "created_at")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        out.sort_by_key(|m| m.created_at);
        Ok(out)
    }

    async fn mark_delivered(&self, ids: &[Uuid]) -> Result<(), StoreError> {
        sqlx::query("UPDATE event_outbox SET delivered_at = clock_timestamp() WHERE id = ANY($1) AND delivered_at IS NULL")
            .bind(ids)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: Uuid,
        code: &str,
        retry_in: Duration,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE event_outbox \
                SET next_attempt_at = clock_timestamp() + $3 * interval '1 microsecond', last_error_code = $2 \
              WHERE id = $1 AND delivered_at IS NULL",
        )
        .bind(id)
        .bind(code)
        .bind(micros(retry_in))
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn outbox_backlog(&self) -> Result<u64, StoreError> {
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL")
                .fetch_one(self.pool())
                .await
                .map_err(store_error)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
