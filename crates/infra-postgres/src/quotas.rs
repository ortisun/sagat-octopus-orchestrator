//! Quotas in PostgreSQL: the limits in force, their administration, and the atomic counts that
//! enforce them (FR-009-006).
//!
//! A creating transaction calls one of the `ensure_room_*` functions *before* it inserts. Each takes
//! a transaction-scoped advisory lock on the counted scope, counts exactly, and refuses with
//! [`StoreError::QuotaExceeded`] when the new row would not fit. The lock makes the count and the
//! insert one decision: two replicas racing for the last place serialise on it and the second sees
//! the first's row once that commits. The lock is released with the transaction, so a crash cannot
//! leave a scope locked.
//!
//! The scopes lock in a fixed order (namespace before tenant), and nothing that holds one waits for
//! anything but the next in that order, so the locks cannot deadlock with each other.

use sagat_application::error::StoreError;
use sagat_application::quotas::{QuotaLookup, QuotaStore, StoredLimit};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::{self, QuotaResource};
use sqlx::{PgConnection, PgPool};

use crate::execution::PgRunStore;
use crate::identity::{insert_audit, store_error};

/// Takes the lock of one counted scope for the rest of the transaction.
async fn lock(
    conn: &mut PgConnection,
    resource: QuotaResource,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
) -> Result<(), StoreError> {
    let key = match namespace {
        Some(ns) => format!("quota:{resource}:{tenant}:{ns}"),
        None => format!("quota:{resource}:{tenant}"),
    };
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

/// Refuses when `count` has reached `limit`.
fn judge(resource: QuotaResource, count: i64, limit: u64) -> Result<(), StoreError> {
    if u64::try_from(count).unwrap_or(u64::MAX) >= limit {
        Err(StoreError::QuotaExceeded {
            what: resource.limit_name(),
        })
    } else {
        Ok(())
    }
}

/// A new run fits in its namespace and in its tenant.
pub(crate) async fn ensure_room_for_run(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    namespace_limit: u64,
    tenant_limit: u64,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::ActiveRuns, tenant, Some(namespace)).await?;
    let in_namespace: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs \
         WHERE tenant_id = $1 AND namespace_id = $2 AND finished_at IS NULL",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::ActiveRuns, in_namespace, namespace_limit)?;
    lock(conn, QuotaResource::TenantActiveRuns, tenant, None).await?;
    let in_tenant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE tenant_id = $1 AND finished_at IS NULL",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::TenantActiveRuns, in_tenant, tenant_limit)
}

/// A new workflow fits in its namespace.
pub(crate) async fn ensure_room_for_workflow(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::Workflows, tenant, Some(namespace)).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflows WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::Workflows, count, limit)
}

/// A new worker session fits in the tenant. Expired sessions do not count.
pub(crate) async fn ensure_room_for_session(
    conn: &mut PgConnection,
    tenant: TenantId,
    limit: u64,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::WorkerSessions, tenant, None).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM worker_sessions \
         WHERE tenant_id = $1 AND expires_at > clock_timestamp()",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::WorkerSessions, count, limit)
}

async fn effective(
    pool: &PgPool,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<u64, StoreError> {
    let rows: Vec<(Option<uuid::Uuid>, i64)> = sqlx::query_as(
        "SELECT namespace_id, max_value FROM quota_limits \
         WHERE tenant_id = $1 AND resource = $2 AND (namespace_id IS NULL OR namespace_id = $3)",
    )
    .bind(tenant.as_uuid())
    .bind(resource.as_str())
    .bind(namespace.map(NamespaceId::as_uuid))
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    let value = |scoped: bool| {
        rows.iter()
            .find(|(ns, _)| ns.is_some() == scoped)
            .map(|(_, v)| u64::try_from(*v).unwrap_or(0))
    };
    Ok(quota::resolve(resource, value(true), value(false)).0)
}

impl QuotaLookup for PgRunStore {
    async fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        effective(&self.pool, tenant, namespace, resource).await
    }
}

impl QuotaLookup for crate::definitions::PgDefinitionStore {
    async fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        effective(self.pool(), tenant, namespace, resource).await
    }
}

impl QuotaStore for PgRunStore {
    async fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE tenant_id = $1 AND id = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE id = $2 AND tenant_id <> $1)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        sagat_application::runs::RunStore::record_denial(self, audit).await
    }

    async fn settings(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
    ) -> Result<Vec<StoredLimit>, StoreError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT resource, max_value FROM quota_limits \
             WHERE tenant_id = $1 AND namespace_id IS NOT DISTINCT FROM $2 ORDER BY resource",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.map(NamespaceId::as_uuid))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.into_iter()
            .map(|(resource, value)| {
                Ok(StoredLimit {
                    resource: resource.parse().map_err(|_| StoreError::Internal {
                        detail: "a stored quota names an unknown resource".into(),
                    })?,
                    value: u64::try_from(value).map_err(|_| StoreError::Internal {
                        detail: "a stored quota is negative".into(),
                    })?,
                })
            })
            .collect()
    }

    async fn put_setting(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
        value: Option<u64>,
        changed_by: &str,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        match value {
            Some(value) => {
                let value = i64::try_from(value).map_err(|_| StoreError::Internal {
                    detail: "a quota value does not fit".into(),
                })?;
                // The unique index is on an expression, so the conflict is resolved by hand.
                let updated = sqlx::query(
                    "UPDATE quota_limits SET max_value = $4, updated_by = $5, \
                            updated_at = clock_timestamp() \
                     WHERE tenant_id = $1 AND namespace_id IS NOT DISTINCT FROM $2 AND resource = $3",
                )
                .bind(tenant.as_uuid())
                .bind(namespace.map(NamespaceId::as_uuid))
                .bind(resource.as_str())
                .bind(value)
                .bind(changed_by)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?
                .rows_affected();
                if updated == 0 {
                    sqlx::query(
                        "INSERT INTO quota_limits \
                         (tenant_id, namespace_id, resource, max_value, updated_by) \
                         VALUES ($1, $2, $3, $4, $5)",
                    )
                    .bind(tenant.as_uuid())
                    .bind(namespace.map(NamespaceId::as_uuid))
                    .bind(resource.as_str())
                    .bind(value)
                    .bind(changed_by)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                }
            }
            None => {
                sqlx::query(
                    "DELETE FROM quota_limits \
                     WHERE tenant_id = $1 AND namespace_id IS NOT DISTINCT FROM $2 AND resource = $3",
                )
                .bind(tenant.as_uuid())
                .bind(namespace.map(NamespaceId::as_uuid))
                .bind(resource.as_str())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn usage(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        let namespace = namespace.map(NamespaceId::as_uuid);
        let count: i64 = match resource {
            QuotaResource::ActiveRuns => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM runs WHERE tenant_id = $1 AND namespace_id = $2 \
                       AND finished_at IS NULL",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::TenantActiveRuns => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM runs WHERE tenant_id = $1 AND finished_at IS NULL",
                )
                .bind(tenant.as_uuid())
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::Workflows => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM workflows WHERE tenant_id = $1 AND namespace_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::WorkerSessions => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM worker_sessions \
                     WHERE tenant_id = $1 AND expires_at > clock_timestamp()",
                )
                .bind(tenant.as_uuid())
                .fetch_one(&self.pool)
                .await
            }
        }
        .map_err(store_error)?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
