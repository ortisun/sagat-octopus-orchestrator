//! Worker sessions and the index queries behind task claiming.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::worker::{NewSession, SessionRecord, WorkerStore};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

impl WorkerStore for PgRunStore {
    async fn create_session(&self, session: &NewSession) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO worker_sessions \
             (id, tenant_id, namespace_id, subject, worker_name, token_hash, activities, languages, \
              pools, max_inflight, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                     clock_timestamp() + $11 * interval '1 microsecond')",
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
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(())
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
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT s.run_id FROM steps s \
             JOIN runs r ON r.tenant_id = s.tenant_id AND r.id = s.run_id \
             WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.status = 'READY' \
               AND s.activity = ANY($3) AND NOT r.paused AND r.stop_reason IS NULL \
             GROUP BY s.run_id ORDER BY min(s.ready_at), s.run_id LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(activities)
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        Ok(rows.into_iter().map(RunId::new).collect())
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
