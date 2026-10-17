//! Candidate queries for the scheduler and the transactional outbox.
//!
//! Every comparison uses the database's clock (`now()`), so schedulers on machines with different
//! clocks agree on what is due. It is `now()` and not `clock_timestamp()` because the planner can
//! use a stable function as the bound of an index range and cannot use a volatile one: with
//! `clock_timestamp()` the filter on a deadline is applied to every entry of the index, in order,
//! until the limit is reached, so a pass with nothing due reads every waiting step. With `now()`
//! (the start of the statement, which is when these candidate queries are asked) it reads the
//! entries that are due and stops. The queries only *find* candidates; the authority is the
//! run's row lock, taken by the caller, under which the domain re-checks the condition.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::maintenance::{
    EndedChild, LeaseLoss, LostLease, MaintenanceStore, OutboxDue, OutboxMessage, OutboxStore,
    PendingChild, RunRef, StepRef, Window,
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

fn run_ref(row: &PgRow) -> Result<RunRef, StoreError> {
    Ok(RunRef {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "id")?),
    })
}

fn step_ref(row: &PgRow) -> Result<StepRef, StoreError> {
    let node: String = col(row, "node")?;
    Ok(StepRef {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        node: NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
    })
}

/// The messages of the rows a claim returned, in the order they were written.
fn outbox_messages(rows: &[sqlx::postgres::PgRow]) -> Result<Vec<OutboxMessage>, StoreError> {
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
                routing_key: col(row, "routing_key")?,
                payload: serde_json::to_vec(&payload).map_err(|_| corrupt("outbox payload"))?,
                attempt_count: u32::try_from(attempts).map_err(|_| corrupt("attempts"))?,
                created_at: col(row, "created_at")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    out.sort_by_key(|m| m.created_at);
    Ok(out)
}

/// The window as the three parameters every candidate query takes: how many, the shards to look at
/// and the tenants to leave out.
fn window_params(window: &Window) -> (i64, Vec<i16>, Vec<Uuid>) {
    (
        i64::from(window.limit),
        window.shards.indexes(),
        window.exclude.iter().map(|t| t.as_uuid()).collect(),
    )
}

impl MaintenanceStore for PgRunStore {
    // Each query is the same shape: for every shard the replica owns, the oldest `limit` due items of
    // that shard (the first entries of an index whose leading column is the shard, so a shard with
    // nothing due costs one probe), then the oldest `limit` of all of them. The oldest `limit` of the
    // owned shards are among the oldest `limit` of each, so the answer is exactly what one query
    // over all of them would give.
    async fn due_timers(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, due_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'TIMER' \
                   AND due_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY due_at LIMIT $1) t \
             ORDER BY t.due_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn due_wait_deadlines(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, due_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'EVENT' \
                   AND due_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY due_at LIMIT $1) t \
             ORDER BY t.due_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn due_retries(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, next_attempt_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'RETRY_WAIT' \
                   AND next_attempt_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY next_attempt_at LIMIT $1) t \
             ORDER BY t.next_attempt_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn lost_leases(&self, window: &Window) -> Result<Vec<LostLease>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // Two index scans (one per partial index) rather than one query with OR.
        let rows = sqlx::query(
            "(SELECT t.tenant_id, t.run_id, t.node, t.attempt, t.generation, t.cause \
                FROM unnest($2::smallint[]) AS sh(s) \
                CROSS JOIN LATERAL ( \
                    SELECT tenant_id, run_id, node, attempt, generation, 'TIMED_OUT' AS cause, \
                           attempt_deadline_at AS at FROM steps \
                    WHERE run_shard(run_id) = sh.s AND status IN ('LEASED', 'RUNNING') \
                      AND attempt_deadline_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                    ORDER BY attempt_deadline_at LIMIT $1) t \
              ORDER BY t.at LIMIT $1) \
             UNION ALL \
             (SELECT t.tenant_id, t.run_id, t.node, t.attempt, t.generation, t.cause \
                FROM unnest($2::smallint[]) AS sh(s) \
                CROSS JOIN LATERAL ( \
                    SELECT tenant_id, run_id, node, attempt, generation, 'EXPIRED' AS cause, \
                           lease_expires_at AS at FROM steps \
                    WHERE run_shard(run_id) = sh.s AND status IN ('LEASED', 'RUNNING') \
                      AND lease_expires_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                    ORDER BY lease_expires_at LIMIT $1) t \
              ORDER BY t.at LIMIT $1)",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
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
        out.truncate(window.limit as usize);
        Ok(out)
    }

    async fn overdue_runs(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id, deadline_at FROM runs \
                 WHERE run_shard(id) = sh.s AND finished_at IS NULL AND stop_reason IS NULL \
                   AND deadline_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY deadline_at LIMIT $1) t \
             ORDER BY t.deadline_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn stale_hints(
        &self,
        older_than: Duration,
        window: &Window,
    ) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, last_hinted_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'READY' AND hint_count < 5 \
                   AND last_hinted_at <= now() - $4 * interval '1 microsecond' \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY last_hinted_at LIMIT $1) t \
             ORDER BY t.last_hinted_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .bind(micros(older_than))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn pending_children(&self, window: &Window) -> Result<Vec<PendingChild>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.run_id, t.node, t.child_run_id, t.child_input, \
                    clock_timestamp() AS db_now \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, child_run_id, child_input, updated_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'CHILD' \
                   AND child_started = false AND child_input IS NOT NULL \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY updated_at LIMIT $1) t \
             ORDER BY t.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(PendingChild {
                step: step_ref(row)?,
                child: RunId::new(col::<Uuid>(row, "child_run_id")?),
                input: col(row, "child_input")?,
                as_of: col(row, "db_now")?,
            })
        })
        .collect()
    }

    async fn run_result(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar(
            "SELECT p.content FROM runs r \
             JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.result_digest \
             WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn ended_children(&self, window: &Window) -> Result<Vec<EndedChild>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        // The child that ended is told to its parent: the shard is the parent's.
        sqlx::query(
            "SELECT t.tenant_id, t.id, t.parent_run_id, t.parent_node, t.status \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id, parent_run_id, parent_node, status, finished_at FROM runs \
                 WHERE run_shard(parent_run_id) = sh.s AND parent_run_id IS NOT NULL \
                   AND parent_settled = false AND finished_at IS NOT NULL \
                   AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY finished_at LIMIT $1) t \
             ORDER BY t.finished_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let node: String = col(row, "parent_node")?;
            let status: String = col(row, "status")?;
            Ok(EndedChild {
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
                child: RunId::new(col::<Uuid>(row, "id")?),
                parent: RunId::new(col::<Uuid>(row, "parent_run_id")?),
                node: NodeId::parse(&node).map_err(|_| corrupt("parent node"))?,
                status: status.parse().map_err(|_| corrupt("run status"))?,
            })
        })
        .collect()
    }

    async fn children_to_cancel(&self, window: &Window) -> Result<Vec<RunRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.tenant_id, t.id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, id FROM runs \
                 WHERE run_shard(id) = sh.s AND parent_cancel AND finished_at IS NULL \
                   AND stop_reason IS NULL AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY id LIMIT $1) t \
             ORDER BY t.id LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(run_ref)
        .collect()
    }

    async fn purge_idempotency(&self, older_than: Duration) -> Result<u64, StoreError> {
        sqlx::query(
            "DELETE FROM command_dedupe WHERE created_at < now() - $1 * interval '1 microsecond'",
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
                   AND delivered_at < now() - $1 * interval '1 microsecond' \
                 ORDER BY delivered_at LIMIT $2)",
        )
        .bind(micros(older_than))
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(store_error)
    }

    async fn purge_script_records(&self, limit: u32) -> Result<u64, StoreError> {
        self.purge_script_records_impl(limit).await
    }

    async fn purge_sessions(&self) -> Result<u64, StoreError> {
        // Kept for an hour after expiry so a late report still finds its session's identity.
        sqlx::query("DELETE FROM worker_sessions WHERE expires_at < now() - interval '1 hour'")
            .execute(self.pool())
            .await
            .map(|r| r.rows_affected())
            .map_err(store_error)
    }
}

impl PgRunStore {
    /// Bounded by `limit` per table, so a backlog never holds a long statement.
    pub(crate) async fn purge_script_records_impl(&self, limit: u32) -> Result<u64, StoreError> {
        let executions = sqlx::query(
            "DELETE FROM script_executions WHERE id IN ( \
                 SELECT id FROM script_executions WHERE expires_at < now() \
                 ORDER BY expires_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?
        .rows_affected();
        let test_runs = sqlx::query(
            "DELETE FROM script_test_runs WHERE id IN ( \
                 SELECT id FROM script_test_runs \
                 WHERE finished_at IS NOT NULL \
                   AND finished_at + make_interval(days => retention_days) < now() \
                 ORDER BY finished_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?
        .rows_affected();
        Ok(executions + test_runs)
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
                 WHERE delivered_at IS NULL AND next_attempt_at <= now() \
                 ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
             UPDATE event_outbox o \
                SET next_attempt_at = clock_timestamp() + $2 * interval '1 microsecond', \
                    attempt_count = o.attempt_count + 1 \
               FROM due WHERE o.id = due.id \
             RETURNING o.id, o.tenant_id, o.aggregate_id, o.event_type, o.routing_key, o.payload, \
                       o.attempt_count, o.created_at",
        )
        .bind(i64::from(limit))
        .bind(micros(visibility))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        outbox_messages(&rows)
    }

    async fn due_outbox(&self, window: &Window) -> Result<Vec<OutboxDue>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT t.id, t.tenant_id FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT id, tenant_id, next_attempt_at FROM event_outbox \
                 WHERE run_shard(aggregate_id) = sh.s AND delivered_at IS NULL \
                   AND next_attempt_at <= now() AND tenant_id <> ALL($3::uuid[]) \
                 ORDER BY next_attempt_at LIMIT $1) t \
             ORDER BY t.next_attempt_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(OutboxDue {
                id: col(row, "id")?,
                tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            })
        })
        .collect()
    }

    async fn claim_outbox_ids(
        &self,
        ids: &[Uuid],
        visibility: Duration,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        // The conditions are asked again here: the message may have been delivered, or claimed by
        // another dispatcher, since it was looked at. A message locked by one is skipped.
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT id FROM event_outbox \
                 WHERE id = ANY($1) AND delivered_at IS NULL AND next_attempt_at <= now() \
                 ORDER BY next_attempt_at FOR UPDATE SKIP LOCKED) \
             UPDATE event_outbox o \
                SET next_attempt_at = clock_timestamp() + $2 * interval '1 microsecond', \
                    attempt_count = o.attempt_count + 1 \
               FROM due WHERE o.id = due.id \
             RETURNING o.id, o.tenant_id, o.aggregate_id, o.event_type, o.routing_key, o.payload, \
                       o.attempt_count, o.created_at",
        )
        .bind(ids)
        .bind(micros(visibility))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        outbox_messages(&rows)
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
