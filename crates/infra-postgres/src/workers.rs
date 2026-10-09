//! Worker sessions and the index queries behind task claiming.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::worker::{NewSession, SessionRecord, WorkerStore};
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
pub const CLAIMABLE: &str = "SELECT s.run_id FROM unnest($3::text[]) AS a(activity) \
     CROSS JOIN LATERAL ( \
         SELECT st.run_id, st.ready_at FROM steps st \
         JOIN runs r ON r.tenant_id = st.tenant_id AND r.id = st.run_id \
         WHERE st.tenant_id = $1 AND st.namespace_id = $2 AND st.activity = a.activity \
           AND st.status = 'READY' AND NOT r.paused AND r.stop_reason IS NULL \
         ORDER BY st.ready_at LIMIT $4) s \
     ORDER BY s.ready_at, s.run_id LIMIT $4";

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

impl WorkerStore for PgRunStore {
    async fn create_session(&self, session: &NewSession) -> Result<Uuid, StoreError> {
        // One session per (identity, worker name): registering again replaces the token (and keeps
        // the id). A new name is refused once the identity holds the maximum of live sessions; the
        // count is not serialised, so racing registrations can overshoot it by their own number.
        let mut tx = self.pool().begin().await.map_err(store_error)?;
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
              pools, max_inflight, expires_at) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                    clock_timestamp() + $11 * interval '1 microsecond' \
             WHERE EXISTS (SELECT 1 FROM worker_sessions \
                           WHERE tenant_id = $2 AND subject = $4 AND worker_name = $5) \
                OR (SELECT count(*) FROM worker_sessions \
                    WHERE tenant_id = $2 AND subject = $4 AND expires_at > clock_timestamp()) < $12 \
             ON CONFLICT (tenant_id, subject, worker_name) DO UPDATE SET \
                    namespace_id = EXCLUDED.namespace_id, token_hash = EXCLUDED.token_hash, \
                    activities = EXCLUDED.activities, languages = EXCLUDED.languages, \
                    pools = EXCLUDED.pools, max_inflight = EXCLUDED.max_inflight, \
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
        .bind(micros(session.ttl))
        .bind(i64::from(sagat_application::worker::MAX_SESSIONS_PER_IDENTITY))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let id = stored.ok_or(StoreError::QuotaExceeded {
            what: "worker sessions of one identity",
        })?;
        tx.commit().await.map_err(store_error)?;
        Ok(id)
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
             RETURNING id, tenant_id, namespace_id, subject, worker_name, activities, max_inflight, expires_at",
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
            Ok(SessionRecord {
                id: col(&row, "id")?,
                tenant_id: TenantId::new(col::<Uuid>(&row, "tenant_id")?),
                namespace_id: NamespaceId::new(col::<Uuid>(&row, "namespace_id")?),
                subject: col(&row, "subject")?,
                worker_name: col(&row, "worker_name")?,
                activities: col(&row, "activities")?,
                max_inflight: u32::try_from(inflight).map_err(|_| corrupt("max inflight"))?,
                expires_at: col(&row, "expires_at")?,
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
    ) -> Result<Vec<RunId>, StoreError> {
        let rows: Vec<Uuid> = sqlx::query_scalar(CLAIMABLE)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(activities)
            .bind(i64::from(limit))
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

    async fn inflight(&self, tenant: TenantId, id: Uuid) -> Result<u32, StoreError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE tenant_id = $1 AND status IN ('LEASED', 'RUNNING') \
             AND leased_by = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.to_string())
        .fetch_one(self.pool())
        .await
        .map_err(store_error)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}
