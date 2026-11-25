//! The step-log storage of design 041: the purge of section 4 and the quota-lease housekeeping of section 2b (task
//! T041-10). The ingest of section 2 (task T041-6) and the carry of section 2a (task T041-36) join this module.
//!
//! Every statement that reads or deletes line text maps its error without the database's text
//! (`store_error_without_text`): a message of PostgreSQL can quote a value of the row.

use sagat_application::error::StoreError;
use sagat_application::maintenance::{IdleLeaseReclaim, StepLogPurge, StepLogQuotaCounters};
use sqlx::PgPool;
use uuid::Uuid;

use crate::identity::store_error_without_text;
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

/// Step 2 of the purge, one transaction: the removed and not yet purged streams of the first tenant that has one,
/// locked `SKIP LOCKED`; at most `limit` of their lines deleted in key order; those streams with no line left marked
/// `purged_at`. The deletes' release runs at the commit, under the tenant's usage row (ADR-0097 decision 5), and a
/// release that would take the usage below zero is refused: the transaction is rolled back and the refusal returned.
pub(crate) async fn purge_lines(pool: &PgPool, limit: u32) -> Result<StepLogPurge, StoreError> {
    let mut tx = pool
        .begin_safely()
        .await
        .map_err(store_error_without_text)?;
    let streams: Vec<(Uuid, i64)> = sqlx::query_as(
        "WITH first AS (SELECT tenant_id FROM step_log_streams \
                          WHERE removed_at IS NOT NULL AND purged_at IS NULL \
                          ORDER BY tenant_id, stream_id LIMIT 1 FOR UPDATE SKIP LOCKED) \
         SELECT s.tenant_id, s.stream_id FROM step_log_streams s JOIN first ON first.tenant_id = s.tenant_id \
          WHERE s.removed_at IS NOT NULL AND s.purged_at IS NULL \
          ORDER BY s.stream_id LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(PURGE_STREAMS)
    .fetch_all(&mut *tx)
    .await
    .map_err(store_error_without_text)?;
    let Some(&(tenant, _)) = streams.first() else {
        return Ok(StepLogPurge::Purged {
            lines: 0,
            streams: 0,
        });
    };
    let ids: Vec<i64> = streams.iter().map(|(_, stream)| *stream).collect();
    let lines = sqlx::query(
        "DELETE FROM step_log_lines l \
          USING (SELECT tenant_id, stream_id, sequence FROM step_log_lines \
                  WHERE tenant_id = $1 AND stream_id = ANY ($2) ORDER BY stream_id, sequence LIMIT $3) d \
          WHERE l.tenant_id = d.tenant_id AND l.stream_id = d.stream_id AND l.sequence = d.sequence",
    )
    .bind(tenant)
    .bind(&ids)
    .bind(bound(limit))
    .execute(&mut *tx)
    .await
    .map_err(store_error_without_text)?
    .rows_affected();
    let purged = sqlx::query(
        "UPDATE step_log_streams s SET purged_at = now() \
          WHERE s.tenant_id = $1 AND s.stream_id = ANY ($2) \
            AND NOT EXISTS (SELECT 1 FROM step_log_lines l WHERE l.tenant_id = s.tenant_id AND l.stream_id = s.stream_id)",
    )
    .bind(tenant)
    .bind(&ids)
    .execute(&mut *tx)
    .await
    .map_err(store_error_without_text)?
    .rows_affected();
    match tx.commit().await {
        Ok(()) => Ok(StepLogPurge::Purged {
            lines,
            streams: purged,
        }),
        Err(sqlx::Error::Database(e)) if e.constraint() == Some("step_log_usage_underflow") => {
            Ok(StepLogPurge::Underflow)
        }
        Err(e) => Err(store_error_without_text(e)),
    }
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
/// `sagat_quota_step_log_reclaim_idle` is a statement in a transaction of its own).
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
        let (reclaimed, skipped): (i32, bool) =
            sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
                .bind(tenant)
                .fetch_one(pool)
                .await
                .map_err(store_error_without_text)?;
        if skipped {
            done.skipped += 1;
        } else if reclaimed > 0 {
            done.tenants += 1;
            done.leases += u64::try_from(reclaimed).unwrap_or(0);
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
