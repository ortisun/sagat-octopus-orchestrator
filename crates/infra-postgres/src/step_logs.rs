//! The step-log storage of design 041: the ingest of section 2 (task T041-6, [`PgStepLogStore`]), the purge of section
//! 4 and the quota-lease housekeeping of section 2b (task T041-10). The carry of section 2a (task T041-36) joins this
//! module.
//!
//! Every statement maps its error without the database's text (`store_error_without_text`): a message of PostgreSQL
//! can quote a value of the row, and the ingest's statements bind line text.
//!
//! The ingest takes no binding lock, no namespace secrets lock and no membership lock (design section 2, T7): its
//! authority is read without a lock, so it never delays a report, a lease, a heartbeat, a secret change or a managed
//! database admission. A test of `quotas.rs` holds that this file calls none of them.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::maintenance::{IdleLeaseReclaim, StepLogPurge, StepLogQuotaCounters};
use sagat_application::step_logs::{
    AttemptRef, CounterUpdate, InsertOutcome, InsertRows, InsertedRow, LeaseRead, QuotaRefusal,
    STEP_LOG_PROTOCOL_MINOR, SessionRef, StepLogPolicy, StepLogStore, StepLogTx, StreamRead,
    StreamRef,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Role, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::{
    CeilingReason, NotStoredReason, StepLogLevel, StreamCounters, TenantAllowance,
};
use sqlx::{PgPool, Postgres, Row as _, Transaction};
use uuid::Uuid;

use crate::identity::{PgIdentityStore, store_error_without_text};
use crate::pool::BeginSafely as _;

/// The streams one purge transaction locks at most: the removed streams of one tenant, so that the transaction holds
/// one tenant's usage row at its commit and a tenant with many small streams is purged in few transactions.
const PURGE_STREAMS: i64 = 100;

/// Today's day number since the Unix epoch, by the expression of `step_log_streams.expires_day` (UTC, no time zone).
macro_rules! today {
    () => {
        "(floor(extract(epoch FROM (now() - '1970-01-01 00:00:00+00'::timestamptz)) / 86400))::integer"
    };
}

fn bound(limit: u32) -> i64 {
    i64::from(limit)
}

fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// Step 1 of the purge: the expired streams, found by the expiry index on `expires_day` and the exact time rechecked,
/// locked `SKIP LOCKED` (a row an ingest holds is left to a later call), get `removed_at`. Under read committed a row
/// that changed since the statement's snapshot is read again before it is locked, so a stream whose last batch moved
/// its expiry is not marked.
pub(crate) async fn mark_expired_streams(pool: &PgPool, limit: u32) -> Result<u64, StoreError> {
    sqlx::query(concat!(
        "WITH due AS (SELECT tenant_id, stream_id FROM step_log_streams \
                        WHERE removed_at IS NULL AND expires_day <= ",
        today!(),
        " AND expires_at <= now() ORDER BY expires_day LIMIT $1 FOR UPDATE SKIP LOCKED) \
         UPDATE step_log_streams s SET removed_at = now() FROM due \
          WHERE s.tenant_id = due.tenant_id AND s.stream_id = due.stream_id"
    ))
    .bind(bound(limit))
    .execute(pool)
    .await
    .map(|done| done.rows_affected())
    .map_err(store_error_without_text)
}

/// Step 2 of the purge, one transaction: the removed and not yet purged streams of the first tenant after `after` and at
/// most `until` that has one, locked `SKIP LOCKED`; at most `limit` of their lines deleted in key order; those streams with no line left
/// marked `purged_at`. The deletes' release runs at the commit, under the tenant's usage row (ADR-0097 decision 5), and
/// a release that would take the usage below zero is refused: the transaction is rolled back and the refusal returned.
/// Once a tenant is chosen, every failure names it (decision D041-145).
pub(crate) async fn purge_lines(
    pool: &PgPool,
    after: Option<TenantId>,
    until: Option<TenantId>,
    limit: u32,
) -> Result<StepLogPurge, StoreError> {
    let mut tx = pool
        .begin_safely()
        .await
        .map_err(store_error_without_text)?;
    let streams: Vec<(Uuid, i64)> = sqlx::query_as(
        "WITH first AS (SELECT tenant_id FROM step_log_streams \
                          WHERE removed_at IS NOT NULL AND purged_at IS NULL \
                            AND ($2::uuid IS NULL OR tenant_id > $2) AND ($3::uuid IS NULL OR tenant_id <= $3) \
                          ORDER BY tenant_id, stream_id LIMIT 1 FOR UPDATE SKIP LOCKED) \
         SELECT s.tenant_id, s.stream_id FROM step_log_streams s JOIN first ON first.tenant_id = s.tenant_id \
          WHERE s.removed_at IS NOT NULL AND s.purged_at IS NULL \
          ORDER BY s.stream_id LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(PURGE_STREAMS)
    .bind(after.map(|tenant| tenant.as_uuid()))
    .bind(until.map(|tenant| tenant.as_uuid()))
    .fetch_all(&mut *tx)
    .await
    .map_err(store_error_without_text)?;
    let Some(&(tenant, _)) = streams.first() else {
        return Ok(StepLogPurge::Nothing);
    };
    let ids: Vec<i64> = streams.iter().map(|(_, stream)| *stream).collect();
    let served = TenantId::new(tenant);
    match purge_tenant(tx, tenant, &ids, limit).await {
        Ok((lines, streams)) => Ok(StepLogPurge::Purged {
            tenant: served,
            lines,
            streams,
        }),
        Err(sqlx::Error::Database(e)) if e.constraint() == Some("step_log_usage_underflow") => {
            Ok(StepLogPurge::Underflow { tenant: served })
        }
        Err(e) => Ok(StepLogPurge::Failed {
            tenant: served,
            error: store_error_without_text(e),
        }),
    }
}

/// The deletes and the purged marks of one tenant's locked streams, and the commit.
async fn purge_tenant(
    mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: Uuid,
    ids: &[i64],
    limit: u32,
) -> Result<(u64, u64), sqlx::Error> {
    let lines = sqlx::query(
        "DELETE FROM step_log_lines l \
          USING (SELECT tenant_id, stream_id, sequence FROM step_log_lines \
                  WHERE tenant_id = $1 AND stream_id = ANY ($2) ORDER BY stream_id, sequence LIMIT $3) d \
          WHERE l.tenant_id = d.tenant_id AND l.stream_id = d.stream_id AND l.sequence = d.sequence",
    )
    .bind(tenant)
    .bind(ids)
    .bind(bound(limit))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let purged = sqlx::query(
        "UPDATE step_log_streams s SET purged_at = now() \
          WHERE s.tenant_id = $1 AND s.stream_id = ANY ($2) \
            AND NOT EXISTS (SELECT 1 FROM step_log_lines l WHERE l.tenant_id = s.tenant_id AND l.stream_id = s.stream_id)",
    )
    .bind(tenant)
    .bind(ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok((lines, purged))
}

/// The streams expired or being removed and not yet purged, by the two partial indexes.
pub(crate) async fn purge_backlog(pool: &PgPool) -> Result<u64, StoreError> {
    let n: i64 = sqlx::query_scalar(concat!(
        "SELECT (SELECT count(*) FROM step_log_streams WHERE removed_at IS NOT NULL AND purged_at IS NULL) \
              + (SELECT count(*) FROM step_log_streams WHERE removed_at IS NULL AND expires_day <= ",
        today!(),
        " AND expires_at <= now())"
    ))
    .fetch_one(pool)
    .await
    .map_err(store_error_without_text)?;
    Ok(count(n))
}

/// The tenants with an idle or ended-backend quota lease, read once, then one tenant a transaction (each call of
/// `sagat_quota_step_log_reclaim_idle` is a statement in a transaction of its own). A tenant whose reclaim fails (its
/// usage row held past the statement timeout, say) is counted and passed over, so it never costs the others theirs.
pub(crate) async fn reclaim_idle_leases(
    pool: &PgPool,
    max_tenants: u32,
) -> Result<IdleLeaseReclaim, StoreError> {
    let tenants: Vec<Uuid> =
        sqlx::query_scalar("SELECT t FROM sagat_quota_step_log_idle_tenants($1) AS t")
            .bind(i32::try_from(max_tenants).unwrap_or(i32::MAX))
            .fetch_all(pool)
            .await
            .map_err(store_error_without_text)?;
    let mut done = IdleLeaseReclaim::default();
    for tenant in tenants {
        let answer: Result<(i32, bool), sqlx::Error> =
            sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
                .bind(tenant)
                .fetch_one(pool)
                .await;
        match answer {
            Ok((_, true)) => done.skipped += 1,
            Ok((reclaimed, false)) if reclaimed > 0 => {
                done.tenants += 1;
                done.leases += u64::try_from(reclaimed).unwrap_or(0);
            }
            Ok(_) => {}
            Err(error) => {
                done.failed += 1;
                done.last_error = Some(store_error_without_text(error));
            }
        }
    }
    Ok(done)
}

pub(crate) async fn quota_counters(pool: &PgPool) -> Result<StepLogQuotaCounters, StoreError> {
    let (refills, reclaims, idle, ended, lease_rows): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT refills, reclaims, idle, ended, lease_rows FROM sagat_quota_step_log_counters()",
    )
    .fetch_one(pool)
    .await
    .map_err(store_error_without_text)?;
    Ok(StepLogQuotaCounters {
        refills: count(refills),
        reclaims: count(reclaims),
        idle: count(idle),
        ended: count(ended),
        lease_rows: count(lease_rows),
    })
}

// ---------------------------------------------------------------------------------- the ingest (task T041-6)

/// The lock timeout of a batch's transaction (design section 2): a lock not granted within it rolls the batch back
/// and answers `RETRY_LATER`.
pub const STEP_LOG_LOCK_TIMEOUT: Duration = Duration::from_millis(200);

/// The three namespace resources of data-model.md §6, in the order of `QuotaResource`: the order T3 takes their
/// fences and the order of [`StepLogPolicy`]'s fields.
const POLICIES: [QuotaResource; 3] = [
    QuotaResource::StepLogAttemptRecords,
    QuotaResource::StepLogAttemptBytes,
    QuotaResource::StepLogRetentionDays,
];

/// The constraint names the quota-lease charge raises with (ADR-0097, D041-142 (8)).
const QUOTA_EXCEEDED: &str = "tenant_step_log_quota_exceeded";
const QUOTA_HELD: &str = "tenant_step_log_quota_held";

/// The text of the attempt's stream lock (L1b): an ingest takes it at T3, after the policy fences, waiting under its
/// lock timeout; the attempt's carry only tries it (task T041-36, whose SQL builds the same text and is tested equal
/// to this one). Every part is an identifier of the attempt, never text a worker chose.
#[must_use]
pub fn stream_lock_key(
    tenant: TenantId,
    run: RunId,
    step: Uuid,
    attempt: u32,
    generation: u64,
) -> String {
    format!("step-log-stream:{tenant}:{run}:{step}:{attempt}:{generation}")
}

/// The policy fences T3 takes, in its order: for each resource of [`POLICIES`] the tenant's fence and then the
/// namespace's (the keys `content_usage::policy_key` builds for every admission and change).
#[must_use]
pub fn step_log_policy_fence_keys(tenant: TenantId, namespace: NamespaceId) -> Vec<String> {
    crate::content_usage::admission_fence_keys(tenant, namespace, &POLICIES)
}

/// The step-log store of the ingest: a pool of the replica, the replica's wait slots *Q* (sent in every
/// transaction's T1, decision D041-137) and the lock timeout of a batch.
#[derive(Debug, Clone)]
pub struct PgStepLogStore {
    pool: PgPool,
    identity: PgIdentityStore,
    wait_slots: u32,
    lock_timeout: Duration,
}

impl PgStepLogStore {
    /// A store over `pool`, whose *Q* is computed once from the pool's maximum size, with the lock timeout of 200 ms.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            identity: PgIdentityStore::new(pool.clone()),
            wait_slots: crate::pool::step_log_wait_slots(&pool),
            pool,
            lock_timeout: STEP_LOG_LOCK_TIMEOUT,
        }
    }

    /// The same store with another lock timeout: tests that hold a lock the batch waits for longer than 200 ms.
    #[must_use]
    pub const fn with_lock_timeout(mut self, lock_timeout: Duration) -> Self {
        self.lock_timeout = lock_timeout;
        self
    }

    /// *Q*, the replica's wait slots this store sends.
    #[must_use]
    pub const fn wait_slots(&self) -> u32 {
        self.wait_slots
    }

    /// Waits until the denial audits this store accepted have reached PostgreSQL.
    pub async fn flush_denial_writes(&self) {
        self.identity.flush_denial_writes().await;
    }
}

impl StepLogStore for PgStepLogStore {
    type Tx = PgStepLogTx;

    async fn begin(&self) -> Result<PgStepLogTx, StoreError> {
        let tx = self
            .pool
            .begin_safely()
            .await
            .map_err(store_error_without_text)?;
        Ok(PgStepLogTx {
            tx,
            wait_slots: self.wait_slots,
            lock_timeout_ms: self.lock_timeout.as_millis().max(1),
        })
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM runs WHERE id = $1 AND tenant_id <> $2)")
            .bind(run.as_uuid())
            .bind(tenant.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(store_error_without_text)
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        use sagat_application::identity::IdentityStore as _;
        self.identity.record_denial(audit).await
    }
}

/// The transaction of one batch.
#[derive(Debug)]
pub struct PgStepLogTx {
    tx: Transaction<'static, Postgres>,
    wait_slots: u32,
    lock_timeout_ms: u128,
}

impl PgStepLogTx {
    /// The server backend of this transaction: the writer whose quota lease its inserts charge. For tests that look
    /// for it in `pg_stat_activity` and `pg_locks`.
    ///
    /// # Errors
    /// The statement failed.
    pub async fn backend_pid(&mut self) -> Result<i32, StoreError> {
        sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error_without_text)
    }
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn count32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn corrupt(what: &str) -> StoreError {
    StoreError::Internal {
        detail: format!("corrupt step-log row: {what}"),
    }
}

impl StepLogTx for PgStepLogTx {
    /// T1, in one select list evaluated left to right: the lock timeout of the transaction (`set_config` with
    /// `is_local`, exactly `SET LOCAL lock_timeout`, decision D041-146 (12)), the transaction's id, and *Q*.
    async fn assign_transaction_id(&mut self) -> Result<(), StoreError> {
        sqlx::query(
            "SELECT set_config('lock_timeout', $2, true), pg_current_xact_id(), \
                    set_config('sagat.step_log_wait_slots', $1, true)",
        )
        .bind(self.wait_slots.to_string())
        .bind(format!("{}ms", self.lock_timeout_ms))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        Ok(())
    }

    async fn lease(&mut self, attempt: &AttemptRef<'_>) -> Result<Option<LeaseRead>, StoreError> {
        let (nodes, indices) = crate::execution::stored_path_parts(attempt.instance.path());
        let row = sqlx::query(
            "SELECT s.namespace_id, s.activity, s.cancel_requested, \
                    r.finished_at IS NOT NULL AS finished, r.stop_reason IS NOT NULL AS stopping \
               FROM steps s JOIN runs r ON r.tenant_id = s.tenant_id AND r.id = s.run_id \
              WHERE s.tenant_id = $1 AND s.step_id = $2 AND s.run_id = $3 AND s.node = $4 \
                AND s.instance_path_nodes = $5 AND s.instance_path_indices = $6 \
                AND s.status IN ('LEASED', 'RUNNING') AND s.attempt = $7 AND s.attempt >= 1 \
                AND s.generation = $8 AND s.lease_token_hash = $9 \
                AND s.lease_expires_at > clock_timestamp()",
        )
        .bind(attempt.tenant.as_uuid())
        .bind(attempt.step_id)
        .bind(attempt.run_id.as_uuid())
        .bind(attempt.instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(i32::try_from(attempt.fence.attempt).unwrap_or(i32::MAX))
        .bind(signed(attempt.fence.generation))
        .bind(attempt.lease_token_hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let read = |e: sqlx::Error| store_error_without_text(e);
        Ok(Some(LeaseRead {
            namespace: NamespaceId::new(row.try_get("namespace_id").map_err(read)?),
            activity: row.try_get("activity").map_err(read)?,
            cancel_requested: row.try_get("cancel_requested").map_err(read)?,
            run_finished: row.try_get("finished").map_err(read)?,
            run_stopping: row.try_get("stopping").map_err(read)?,
        }))
    }

    async fn session(&mut self, session: &SessionRef<'_>) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM worker_sessions \
                             WHERE tenant_id = $1 AND id = $2 AND namespace_id = $3 AND subject = $4 \
                               AND token_hash = $5 AND expires_at > clock_timestamp() \
                               AND accepted_protocol_minor >= $6)",
        )
        .bind(session.tenant.as_uuid())
        .bind(session.worker_id)
        .bind(session.namespace.as_uuid())
        .bind(session.subject)
        .bind(session.token_hash)
        .bind(i32::try_from(STEP_LOG_PROTOCOL_MINOR).unwrap_or(i32::MAX))
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error_without_text)
    }

    async fn policies(&mut self, stream: &StreamRef<'_>) -> Result<StepLogPolicy, StoreError> {
        let key = stream_lock_key(
            stream.tenant,
            stream.run_id,
            stream.step_id,
            stream.fence.attempt,
            stream.fence.generation,
        );
        let values = crate::content_usage::admission_policies(
            &mut self.tx,
            stream.tenant,
            stream.namespace,
            &POLICIES,
            Some(&key),
        )
        .await
        .map_err(store_error_without_text)?;
        match values.as_slice() {
            [attempt_records, attempt_bytes, retention_days] => Ok(StepLogPolicy {
                attempt_records: *attempt_records,
                attempt_bytes: *attempt_bytes,
                retention_days: *retention_days,
            }),
            _ => Err(corrupt("policies")),
        }
    }

    async fn stream(&mut self, stream: &StreamRef<'_>) -> Result<StreamRead, StoreError> {
        let (nodes, indices) = crate::execution::stored_path_parts(stream.instance.path());
        let row = sqlx::query(
            "INSERT INTO step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
                                           instance_path_nodes, instance_path_indices, attempt, generation, \
                                           retention_days, last_at, expires_at) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, n.t, n.t + make_interval(days => $11::integer) \
               FROM (SELECT clock_timestamp() AS t) n \
             ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) \
                DO UPDATE SET tenant_id = step_log_streams.tenant_id \
             RETURNING stream_id, records, bytes, last_sequence, ceiling_sequence, ceiling_reason, \
                       not_stored_reason, dropped_by_worker, dropped_by_server, next_marker_sequence, \
                       truncated_records, removed_at IS NOT NULL AS removed, \
                       expires_at <= clock_timestamp() AS expired, final_at IS NOT NULL AS final",
        )
        .bind(stream.tenant.as_uuid())
        .bind(Uuid::now_v7())
        .bind(stream.namespace.as_uuid())
        .bind(stream.run_id.as_uuid())
        .bind(stream.step_id)
        .bind(stream.instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(i32::try_from(stream.fence.attempt).unwrap_or(i32::MAX))
        .bind(signed(stream.fence.generation))
        .bind(i16::try_from(stream.retention_days).unwrap_or(i16::MAX))
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        stream_read(&row)
    }

    async fn tenant_allowance(&mut self, tenant: TenantId) -> Result<TenantAllowance, StoreError> {
        let (used, limit, enforced): (i64, i64, bool) = sqlx::query_as(
            "SELECT used, max_value, enforced FROM sagat_quota_step_log_allowance($1)",
        )
        .bind(tenant.as_uuid())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        Ok(TenantAllowance {
            remaining: enforced.then(|| unsigned(limit).saturating_sub(unsigned(used))),
        })
    }

    async fn binding_patterns(
        &mut self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<Vec<String>, StoreError> {
        // The patterns as `bound_patterns` reads them, without its binding lock (OD-2026-11-16-7).
        sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error_without_text)
    }

    async fn membership_role(
        &mut self,
        tenant: TenantId,
        subject: &str,
    ) -> Result<Option<Role>, StoreError> {
        // A plain read: no `FOR SHARE`, no namespace secrets lock (OD-2026-11-16-7).
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2",
        )
        .bind(tenant.as_uuid())
        .bind(subject)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        role.map(|text| text.parse::<Role>().map_err(|_| corrupt("membership role")))
            .transpose()
    }

    async fn insert(&mut self, rows: &InsertRows<'_>) -> Result<InsertOutcome, StoreError> {
        let count = rows.lines.len() + usize::from(rows.marker.is_some());
        let mut sequences = Vec::with_capacity(count);
        let mut worker_times = Vec::with_capacity(count);
        let mut levels = Vec::with_capacity(count);
        let mut kinds = Vec::with_capacity(count);
        let mut truncated = Vec::with_capacity(count);
        let mut messages: Vec<Option<&str>> = Vec::with_capacity(count);
        let mut exceptions: Vec<Option<&str>> = Vec::with_capacity(count);
        let mut fields: Vec<Option<&str>> = Vec::with_capacity(count);
        let mut reasons: Vec<Option<&str>> = Vec::with_capacity(count);
        for line in &rows.lines {
            sequences.push(signed(line.sequence));
            worker_times.push(line.worker_time);
            levels.push(line.level.as_str());
            kinds.push("line");
            truncated.push(line.truncated);
            messages.push(Some(line.message.as_str()));
            exceptions.push(line.exception.as_deref());
            fields.push(line.fields.as_deref());
            reasons.push(None);
        }
        if let Some((sequence, reason)) = rows.marker {
            sequences.push(signed(sequence));
            worker_times.push(None);
            levels.push(StepLogLevel::Info.as_str());
            kinds.push("ceiling_marker");
            truncated.push(false);
            messages.push(None);
            exceptions.push(None);
            fields.push(None);
            reasons.push(Some(reason.as_str()));
        }
        let inserted = sqlx::query(
            "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, \
                                         truncated, message, exception, fields, marker_reason) \
             SELECT $1, $2, r.sequence, $3, r.worker_time, r.level, r.kind, r.truncated, r.message, \
                    r.exception, r.fields, r.marker_reason \
               FROM unnest($4::bigint[], $5::timestamptz[], $6::text[], $7::text[], $8::boolean[], \
                           $9::text[], $10::text[], $11::text[], $12::text[]) \
                    AS r(sequence, worker_time, level, kind, truncated, message, exception, fields, marker_reason) \
             ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING \
             RETURNING sequence, (octet_length(coalesce(message, '')) + octet_length(coalesce(exception, '')) \
                                  + octet_length(coalesce(fields, '')))::bigint AS text_bytes, \
                       truncated, level, kind <> 'line' AS marker",
        )
        .bind(rows.tenant.as_uuid())
        .bind(rows.stream_id)
        .bind(rows.run_id.as_uuid())
        .bind(&sequences)
        .bind(&worker_times)
        .bind(&levels)
        .bind(&kinds)
        .bind(&truncated)
        .bind(&messages)
        .bind(&exceptions)
        .bind(&fields)
        .bind(&reasons)
        .fetch_all(&mut *self.tx)
        .await;
        let inserted = match inserted {
            Ok(inserted) => inserted,
            Err(sqlx::Error::Database(e)) if e.constraint() == Some(QUOTA_EXCEEDED) => {
                return Ok(InsertOutcome::Refused(QuotaRefusal::Limit));
            }
            Err(sqlx::Error::Database(e)) if e.constraint() == Some(QUOTA_HELD) => {
                return Ok(InsertOutcome::Refused(QuotaRefusal::Held));
            }
            Err(error) => return Err(store_error_without_text(error)),
        };
        inserted
            .iter()
            .map(|row| {
                let level: String = row.try_get("level")?;
                Ok(InsertedRow {
                    sequence: unsigned(row.try_get("sequence")?),
                    text_bytes: unsigned(row.try_get("text_bytes")?),
                    truncated: row.try_get("truncated")?,
                    level: StepLogLevel::parse(&level).unwrap_or(StepLogLevel::Info),
                    marker: row.try_get("marker")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map(InsertOutcome::Inserted)
            .map_err(store_error_without_text)
    }

    async fn record_counters(&mut self, update: &CounterUpdate) -> Result<(), StoreError> {
        let c = &update.counters;
        sqlx::query(
            "UPDATE step_log_streams s \
                SET records = $3, bytes = $4, last_sequence = GREATEST(s.last_sequence, $5), \
                    ceiling_sequence = $6, ceiling_reason = $7, not_stored_reason = $8, \
                    dropped_by_worker = $9, dropped_by_server = $10, next_marker_sequence = $11, \
                    truncated_records = $12, first_at = coalesce(s.first_at, n.t), last_at = n.t, \
                    expires_at = n.t + make_interval(days => s.retention_days::integer), \
                    last_stored_xid = CASE WHEN $13 \
                        THEN GREATEST(coalesce(s.last_stored_xid, '0'::xid8), pg_current_xact_id()) \
                        ELSE s.last_stored_xid END \
               FROM (SELECT clock_timestamp() AS t) n \
              WHERE s.tenant_id = $1 AND s.stream_id = $2",
        )
        .bind(update.tenant.as_uuid())
        .bind(update.stream_id)
        .bind(count32(c.records))
        .bind(signed(c.bytes))
        .bind(signed(c.last_sequence))
        .bind(c.ceiling_sequence.map(signed))
        .bind(c.ceiling.map(CeilingReason::as_str))
        .bind(c.not_stored_reason.map(NotStoredReason::as_str))
        .bind(signed(c.dropped_by_worker))
        .bind(signed(c.dropped_by_server))
        .bind(signed(c.next_marker_sequence))
        .bind(count32(c.truncated_records))
        .bind(update.stored)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error_without_text)?;
        Ok(())
    }

    async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await.map_err(store_error_without_text)
    }

    async fn rollback(self) -> Result<(), StoreError> {
        self.tx.rollback().await.map_err(store_error_without_text)
    }
}

/// The stream row T4 returned.
fn stream_read(row: &sqlx::postgres::PgRow) -> Result<StreamRead, StoreError> {
    let get = |e: sqlx::Error| store_error_without_text(e);
    let ceiling: Option<String> = row.try_get("ceiling_reason").map_err(get)?;
    let not_stored: Option<String> = row.try_get("not_stored_reason").map_err(get)?;
    let ceiling = ceiling
        .map(|text| CeilingReason::parse(&text).ok_or_else(|| corrupt("ceiling reason")))
        .transpose()?;
    let not_stored_reason = not_stored
        .map(|text| NotStoredReason::parse(&text).ok_or_else(|| corrupt("not-stored reason")))
        .transpose()?;
    let records: i32 = row.try_get("records").map_err(get)?;
    let truncated: i32 = row.try_get("truncated_records").map_err(get)?;
    let ceiling_sequence: Option<i64> = row.try_get("ceiling_sequence").map_err(get)?;
    let int = |name: &str| row.try_get::<i64, _>(name).map(unsigned).map_err(get);
    Ok(StreamRead {
        stream_id: row.try_get("stream_id").map_err(get)?,
        counters: StreamCounters {
            records: u64::try_from(records).unwrap_or(0),
            bytes: int("bytes")?,
            last_sequence: int("last_sequence")?,
            ceiling_sequence: ceiling_sequence.map(unsigned),
            ceiling,
            not_stored_reason,
            dropped_by_worker: int("dropped_by_worker")?,
            dropped_by_server: int("dropped_by_server")?,
            next_marker_sequence: int("next_marker_sequence")?,
            truncated_records: u64::try_from(truncated).unwrap_or(0),
        },
        removed: row.try_get("removed").map_err(get)?,
        expired: row.try_get("expired").map_err(get)?,
        final_recorded: row.try_get("final").map_err(get)?,
    })
}

#[cfg(test)]
mod tests {
    use sagat_domain::execution::RunId;
    use sagat_domain::identity::{NamespaceId, TenantId};
    use uuid::Uuid;

    use super::{step_log_policy_fence_keys, stream_lock_key};

    /// The keys of T3, in its order: the six policy fences, each resource's tenant fence before its namespace's, in
    /// the order of `QuotaResource`, by the builder every admission and change uses; and the stream lock's key names
    /// the attempt and nothing else.
    #[test]
    fn the_fences_of_t3_are_the_policy_keys_in_resource_order_and_the_stream_key_names_the_attempt()
    {
        let tenant = TenantId::new(Uuid::from_u128(1));
        let namespace = NamespaceId::new(Uuid::from_u128(2));
        let t = tenant.to_string();
        let n = namespace.to_string();
        assert_eq!(
            step_log_policy_fence_keys(tenant, namespace),
            [
                format!("quota-policy:step_log_attempt_records:{t}"),
                format!("quota-policy:step_log_attempt_records:{t}:{n}"),
                format!("quota-policy:step_log_attempt_bytes:{t}"),
                format!("quota-policy:step_log_attempt_bytes:{t}:{n}"),
                format!("quota-policy:step_log_retention_days:{t}"),
                format!("quota-policy:step_log_retention_days:{t}:{n}"),
            ]
        );
        let run = RunId::new(Uuid::from_u128(3));
        let step = Uuid::from_u128(4);
        assert_eq!(
            stream_lock_key(tenant, run, step, 2, 7),
            format!("step-log-stream:{t}:{run}:{step}:2:7")
        );
    }
}
