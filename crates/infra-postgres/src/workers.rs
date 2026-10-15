//! Worker sessions and the index queries behind task claiming.

use crate::pool::BeginSafely;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::worker::{NewSession, SessionRecord, WorkerStore};
use sagat_domain::audit::AuditRecord;
use sagat_domain::binding::{ActivityPattern, first_uncovered};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};
use crate::quotas::ensure_room_for_session;

/// The runs with the oldest ready steps for some activity of the session.
///
/// Per activity it walks `steps_ready_idx` (tenant, namespace, activity, ready_at) in order and
/// stops after `$4` qualifying rows, so its cost is bounded by the answer and not by the backlog (an
/// aggregate over all ready steps cost 209 ms with 200 000 of them). Paused and stopping runs are
/// skipped by a primary-key probe each.
///
/// Compensations have an index of their own (`steps_ready_compensation_idx`), and the branch that
/// reads it is gated by the circuit breaker of the activity with a condition that does not depend on
/// the rows: when the breaker is open the planner does not enter that branch at all. An outage can
/// leave very many compensations READY behind an open breaker, and neither the claim of forward
/// work nor the claim of the compensations of a healthy provider walks them.
// The activity quota is only a candidate-selection hint: it prevents a capped oldest backlog
// from hiding other activities behind the global LIMIT. The transaction rechecks the exact quota
// under its admission locks. With no configured override/ceiling, leave the default to that gate.
// OFFSET/LIMIT stops after the configured number of active leases; it never aggregates READY rows.
pub const CLAIMABLE: &str = "SELECT s.run_id FROM unnest($3::text[]) AS a(activity) \
     CROSS JOIN LATERAL ( \
       SELECT least( \
         coalesce( \
           (SELECT max_inflight FROM activity_quota_limits WHERE tenant_id = $1 AND namespace_id = $2 AND activity = a.activity), \
           (SELECT max_value FROM quota_limits WHERE tenant_id = $1 AND namespace_id = $2 AND resource = 'activity_concurrency'), \
           (SELECT max_value FROM quota_limits WHERE tenant_id = $1 AND namespace_id IS NULL AND resource = 'activity_concurrency'), \
           (SELECT max_value FROM quota_ceilings WHERE tenant_id = $1 AND resource = 'activity_concurrency')), \
         (SELECT max_value FROM quota_ceilings WHERE tenant_id = $1 AND resource = 'activity_concurrency')) AS max_inflight \
     ) quota \
     CROSS JOIN LATERAL ( \
         (SELECT st.run_id, st.ready_at FROM steps st \
          JOIN runs r ON r.tenant_id = st.tenant_id AND r.id = st.run_id \
          WHERE st.tenant_id = $1 AND st.namespace_id = $2 AND st.activity = a.activity \
            AND st.status = 'READY' \
            AND ($5 >= 5 OR cardinality(st.instance_path_nodes) = 0) \
            AND st.kind <> 'COMPENSATION' AND NOT r.paused \
            AND (r.stop_reason IS NULL OR st.kind <> 'FORWARD') \
            AND ($5 >= 3 OR (r.input_blob_id IS NULL AND NOT EXISTS ( \
                SELECT 1 FROM blobs blob \
                WHERE blob.tenant_id = r.tenant_id AND blob.namespace_id = r.namespace_id \
                  AND blob.run_id = r.id AND blob.purpose = 'STEP_OUTPUT' \
                  AND blob.state = 'COMMITTED'))) \
          ORDER BY st.ready_at LIMIT $4) \
         UNION ALL \
         (SELECT st.run_id, st.ready_at FROM steps st \
          JOIN runs r ON r.tenant_id = st.tenant_id AND r.id = st.run_id \
          WHERE st.tenant_id = $1 AND st.namespace_id = $2 AND st.activity = a.activity \
            AND st.status = 'READY' \
            AND ($5 >= 5 OR cardinality(st.instance_path_nodes) = 0) \
            AND st.kind = 'COMPENSATION' \
            AND NOT r.paused \
            AND ($5 >= 3 OR (r.input_blob_id IS NULL AND NOT EXISTS ( \
                SELECT 1 FROM blobs blob \
                WHERE blob.tenant_id = r.tenant_id AND blob.namespace_id = r.namespace_id \
                  AND blob.run_id = r.id AND blob.purpose = 'STEP_OUTPUT' \
                  AND blob.state = 'COMMITTED'))) \
            AND NOT EXISTS (SELECT 1 FROM compensation_breakers b \
                WHERE b.tenant_id = $1 AND b.namespace_id = $2 AND b.activity = a.activity \
                  AND b.opened_until > clock_timestamp()) \
          ORDER BY st.ready_at LIMIT $4) \
     ) s \
     WHERE quota.max_inflight IS NULL OR (quota.max_inflight > 0 AND NOT EXISTS ( \
         SELECT 1 FROM steps active \
         WHERE active.tenant_id = $1 AND active.namespace_id = $2 AND active.activity = a.activity \
           AND active.status IN ('LEASED', 'RUNNING') AND active.lease_expires_at > clock_timestamp() \
         OFFSET greatest(quota.max_inflight - 1, 0) LIMIT 1 \
     )) \
     ORDER BY s.ready_at, s.run_id LIMIT $4";

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

impl WorkerStore for PgRunStore {
    fn supports_scoped_worker_protocol(&self) -> bool {
        self.scoped_worker_protocol
    }

    async fn bound_patterns(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)
    }

    async fn record_unbound(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        sagat_application::runs::RunStore::record_denial(self, audit).await
    }

    async fn create_session(&self, session: &NewSession) -> Result<Option<Uuid>, StoreError> {
        // One session per (identity, worker name): registering again replaces the token (and keeps
        // the id). A new name is refused once the identity holds the maximum of live sessions; the
        // count is not serialised, so racing registrations can overshoot it by their own number.
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        // The lock that replacing a binding takes, then the binding itself: either this session is
        // stored before the replacement (which then finds it and closes it if it is no longer
        // covered) or it is judged against the new binding. Taken before the quota lock below, like
        // everywhere else (binding, then quota).
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "binding:{}:{}:{}",
                session.tenant_id, session.namespace_id, session.subject
            ))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let bound: Vec<String> = sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(session.tenant_id.as_uuid())
        .bind(session.namespace_id.as_uuid())
        .bind(&session.subject)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let patterns = bound
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| StoreError::Internal {
                detail: "a stored binding pattern does not parse".into(),
            })?;
        if first_uncovered(&patterns, &session.activities).is_some() {
            return Ok(None);
        }
        // A worker that reconnects replaces its own session and is never "one more"; a new name has
        // to fit in the tenant's quota, counted under a lock (a security limit is not an estimate).
        let replacing: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM worker_sessions \
             WHERE tenant_id = $1 AND subject = $2 AND worker_name = $3)",
        )
        .bind(session.tenant_id.as_uuid())
        .bind(&session.subject)
        .bind(&session.worker_name)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !replacing {
            ensure_room_for_session(&mut tx, session.tenant_id, session.tenant_limit).await?;
        }
        let stored: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO worker_sessions \
             (id, tenant_id, namespace_id, subject, worker_name, token_hash, activities, languages, \
              pools, max_inflight, accepted_protocol_minor, expires_at) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                    $11, clock_timestamp() + $12 * interval '1 microsecond' \
             WHERE EXISTS (SELECT 1 FROM worker_sessions \
                           WHERE tenant_id = $2 AND subject = $4 AND worker_name = $5) \
                OR (SELECT count(*) FROM worker_sessions \
                    WHERE tenant_id = $2 AND subject = $4 AND expires_at > clock_timestamp()) < $13 \
             ON CONFLICT (tenant_id, subject, worker_name) DO UPDATE SET \
                    namespace_id = EXCLUDED.namespace_id, token_hash = EXCLUDED.token_hash, \
                    activities = EXCLUDED.activities, languages = EXCLUDED.languages, \
                    pools = EXCLUDED.pools, max_inflight = EXCLUDED.max_inflight, \
                    accepted_protocol_minor = EXCLUDED.accepted_protocol_minor, \
                    expires_at = EXCLUDED.expires_at \
             RETURNING id",
        )
        .bind(session.id)
        .bind(session.tenant_id.as_uuid())
        .bind(session.namespace_id.as_uuid())
        .bind(&session.subject)
        .bind(&session.worker_name)
        .bind(&session.token_hash)
        .bind(&session.activities)
        .bind(&session.languages)
        .bind(&session.pools)
        .bind(i32::try_from(session.max_inflight).map_err(|_| corrupt("max inflight"))?)
        .bind(
            i32::try_from(session.accepted_protocol_minor)
                .map_err(|_| corrupt("accepted protocol minor"))?,
        )
        .bind(micros(session.ttl))
        .bind(i64::from(sagat_application::worker::MAX_SESSIONS_PER_IDENTITY))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let id = stored.ok_or(StoreError::QuotaExceeded {
            what: "worker sessions of one identity",
        })?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(id))
    }

    async fn use_session(
        &self,
        tenant: TenantId,
        id: Uuid,
        token_hash: &[u8],
        ttl: Duration,
    ) -> Result<Option<SessionRecord>, StoreError> {
        let row = sqlx::query(
            "UPDATE worker_sessions SET expires_at = clock_timestamp() + $4 * interval '1 microsecond' \
             WHERE tenant_id = $1 AND id = $2 AND token_hash = $3 AND expires_at > clock_timestamp() \
             RETURNING id, tenant_id, namespace_id, subject, worker_name, activities, max_inflight, \
                       accepted_protocol_minor, expires_at, \
                       EXISTS (SELECT 1 FROM worker_drains d \
                               WHERE d.tenant_id = worker_sessions.tenant_id \
                                 AND d.namespace_id = worker_sessions.namespace_id \
                                 AND d.subject = worker_sessions.subject) AS draining",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(token_hash)
        .bind(micros(ttl))
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        row.map(|row| {
            let inflight: i32 = col(&row, "max_inflight")?;
            let protocol_minor: i32 = col(&row, "accepted_protocol_minor")?;
            Ok(SessionRecord {
                id: col(&row, "id")?,
                tenant_id: TenantId::new(col::<Uuid>(&row, "tenant_id")?),
                namespace_id: NamespaceId::new(col::<Uuid>(&row, "namespace_id")?),
                subject: col(&row, "subject")?,
                worker_name: col(&row, "worker_name")?,
                activities: col(&row, "activities")?,
                max_inflight: u32::try_from(inflight).map_err(|_| corrupt("max inflight"))?,
                accepted_protocol_minor: u32::try_from(protocol_minor)
                    .map_err(|_| corrupt("accepted protocol minor"))?,
                expires_at: col(&row, "expires_at")?,
                draining: col(&row, "draining")?,
            })
        })
        .transpose()
    }

    async fn claimable_runs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activities: &[String],
        limit: u32,
        accepted_protocol_minor: u32,
    ) -> Result<Vec<RunId>, StoreError> {
        let rows: Vec<Uuid> = sqlx::query_scalar(CLAIMABLE)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(activities)
            .bind(i64::from(limit))
            .bind(
                i32::try_from(accepted_protocol_minor)
                    .map_err(|_| corrupt("accepted protocol minor"))?,
            )
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
        // A run may have several ready steps: keep each run once, oldest first.
        let mut seen = std::collections::BTreeSet::new();
        Ok(rows
            .into_iter()
            .filter(|run| seen.insert(*run))
            .map(RunId::new)
            .collect())
    }
}
