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

use crate::pool::BeginSafely;
use sagat_application::authz::NamespaceDirectory;
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

/// Takes the lock of the secrets of one namespace for the rest of the transaction: creating a secret
/// and counting them are one decision, and so are two creators of the same name.
pub(crate) async fn lock_secrets(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::Secrets, tenant, Some(namespace)).await
}

/// Takes the lock of the webhook endpoints of one namespace for the rest of the transaction.
pub(crate) async fn lock_event_endpoints(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::EventEndpoints, tenant, Some(namespace)).await
}

/// Takes the lock on the names of the sources of events of one namespace (endpoints and connectors
/// share them) for the rest of the transaction.
pub(crate) async fn lock_event_sources(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("event-sources:{tenant}:{namespace}"))
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

/// A new endpoint fits in its namespace. The caller holds [`lock_event_endpoints`].
pub(crate) async fn ensure_room_for_endpoint(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_endpoints WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::EventEndpoints, count, limit)
}

/// Takes the lock of the outbound webhook subscriptions of one namespace for the rest of the
/// transaction.
pub(crate) async fn lock_subscriptions(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(
        conn,
        QuotaResource::WebhookSubscriptions,
        tenant,
        Some(namespace),
    )
    .await
}

/// A new subscription fits in its namespace. The caller holds [`lock_subscriptions`].
pub(crate) async fn ensure_room_for_subscription(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_subscriptions WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::WebhookSubscriptions, count, limit)
}

/// Takes the lock of the connectors of one namespace for the rest of the transaction.
pub(crate) async fn lock_connectors(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::Connectors, tenant, Some(namespace)).await
}

/// A new connector fits in its namespace. The caller holds [`lock_connectors`].
pub(crate) async fn ensure_room_for_connector(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_connectors WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::Connectors, count, limit)
}

/// Takes the lock of the scripts of one namespace for the rest of the transaction.
pub(crate) async fn lock_scripts(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::Scripts, tenant, Some(namespace)).await
}

/// A new script fits in its namespace. The caller holds [`lock_scripts`].
pub(crate) async fn ensure_room_for_script(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM scripts WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::Scripts, count, limit)
}

/// Takes the lock of the builds and test runs of one namespace for the rest of the transaction.
pub(crate) async fn lock_script_jobs(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    lock(conn, QuotaResource::ScriptJobs, tenant, Some(namespace)).await
}

/// The builds and test runs of a namespace that have not finished.
pub(crate) async fn script_jobs_in_progress(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<i64, StoreError> {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM script_build_jobs \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND status IN ('QUEUED', 'RUNNING')) \
              + (SELECT count(*) FROM script_test_runs \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND status IN ('QUEUED', 'RUNNING'))",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)
}

/// A new build or test run fits in its namespace. The caller holds [`lock_script_jobs`].
pub(crate) async fn ensure_room_for_script_job(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count = script_jobs_in_progress(conn, tenant, namespace).await?;
    judge(QuotaResource::ScriptJobs, count, limit)
}

/// A new secret fits in its namespace. The caller holds [`lock_secrets`].
pub(crate) async fn ensure_room_for_secret(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM secrets WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::Secrets, count, limit)
}

async fn effective(
    pool: &PgPool,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<u64, StoreError> {
    // The settings and the ceiling in one round trip: the ceiling bounds whatever the settings say.
    let rows: Vec<(Option<uuid::Uuid>, i64, bool)> = sqlx::query_as(
        "SELECT namespace_id, max_value, false FROM quota_limits \
         WHERE tenant_id = $1 AND resource = $2 AND (namespace_id IS NULL OR namespace_id = $3) \
         UNION ALL \
         SELECT NULL, max_value, true FROM quota_ceilings WHERE tenant_id = $1 AND resource = $2",
    )
    .bind(tenant.as_uuid())
    .bind(resource.as_str())
    .bind(namespace.map(NamespaceId::as_uuid))
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    let value = |scoped: bool| {
        rows.iter()
            .find(|(ns, _, ceiling)| !ceiling && ns.is_some() == scoped)
            .map(|(_, v, _)| u64::try_from(*v).unwrap_or(0))
    };
    let granted = rows
        .iter()
        .find(|(_, _, ceiling)| *ceiling)
        .map(|(_, v, _)| u64::try_from(*v).unwrap_or(0));
    let ceiling = quota::ceiling(resource, granted);
    Ok(quota::resolve(resource, value(true), value(false), ceiling).0)
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

impl NamespaceDirectory for PgRunStore {
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
}

impl QuotaStore for PgRunStore {
    async fn ceilings(&self, tenant: TenantId) -> Result<Vec<StoredLimit>, StoreError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT resource, max_value FROM quota_ceilings WHERE tenant_id = $1 ORDER BY resource",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        stored_limits(rows)
    }

    async fn put_scheduling_weight(
        &self,
        tenant: TenantId,
        weight: Option<u16>,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        match weight {
            Some(weight) => {
                sqlx::query(
                    "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) \
                     VALUES ($1, $2, clock_timestamp()) \
                     ON CONFLICT (tenant_id) DO UPDATE SET \
                         weight = EXCLUDED.weight, updated_at = clock_timestamp()",
                )
                .bind(tenant.as_uuid())
                .bind(i16::try_from(weight).map_err(|_| StoreError::Internal {
                    detail: "a scheduling weight does not fit".into(),
                })?)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
            None => {
                sqlx::query("DELETE FROM tenant_scheduling WHERE tenant_id = $1")
                    .bind(tenant.as_uuid())
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
            }
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn put_ceiling(
        &self,
        tenant: TenantId,
        resource: QuotaResource,
        value: Option<u64>,
        granted_by: &str,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        match value {
            Some(value) => {
                let value = i64::try_from(value).map_err(|_| StoreError::Internal {
                    detail: "a quota ceiling does not fit".into(),
                })?;
                sqlx::query(
                    "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (tenant_id, resource) DO UPDATE SET \
                         max_value = EXCLUDED.max_value, granted_by = EXCLUDED.granted_by, \
                         granted_at = clock_timestamp()",
                )
                .bind(tenant.as_uuid())
                .bind(resource.as_str())
                .bind(value)
                .bind(granted_by)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
            None => {
                sqlx::query("DELETE FROM quota_ceilings WHERE tenant_id = $1 AND resource = $2")
                    .bind(tenant.as_uuid())
                    .bind(resource.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
            }
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
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
        stored_limits(rows)
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
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
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
            QuotaResource::Secrets => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM secrets WHERE tenant_id = $1 AND namespace_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::EventEndpoints => sqlx::query_scalar(
                "SELECT count(*) FROM event_endpoints WHERE tenant_id = $1 AND namespace_id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(namespace)
            .fetch_one(&self.pool)
            .await,
            QuotaResource::WebhookSubscriptions => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM webhook_subscriptions \
                 WHERE tenant_id = $1 AND namespace_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::Connectors => sqlx::query_scalar(
                "SELECT count(*) FROM event_connectors WHERE tenant_id = $1 AND namespace_id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(namespace)
            .fetch_one(&self.pool)
            .await,
            QuotaResource::Scripts => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM scripts WHERE tenant_id = $1 AND namespace_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::ScriptJobs => {
                sqlx::query_scalar(
                    "SELECT (SELECT count(*) FROM script_build_jobs \
                              WHERE tenant_id = $1 AND namespace_id = $2 \
                                AND status IN ('QUEUED', 'RUNNING')) \
                          + (SELECT count(*) FROM script_test_runs \
                              WHERE tenant_id = $1 AND namespace_id = $2 \
                                AND status IN ('QUEUED', 'RUNNING'))",
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

fn stored_limits(rows: Vec<(String, i64)>) -> Result<Vec<StoredLimit>, StoreError> {
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
