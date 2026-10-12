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
//! Resource creation scopes lock namespace before tenant. Activity admission has a separate policy
//! hierarchy (shared tenant, shared namespace, exact activity), with activities visited in name
//! order and administrative edits exclusively fencing their changed level.

use crate::pool::BeginSafely;
use sagat_application::authz::NamespaceDirectory;
use sagat_application::error::StoreError;
use sagat_application::quotas::{QuotaLookup, QuotaStore, StoredActivityLimit, StoredLimit};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::{self, QuotaResource};
use sagat_domain::workflow::ActivityName;
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

/// Activity policy hierarchy: tenant, namespace, then exact activity. Claims share the policy
/// fences; changing a default/ceiling exclusively fences only its affected level. These keys are
/// separate from other resources' admission locks and no policy editor takes a run-row lock.
async fn activity_policy_lock(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    exclusive: bool,
) -> Result<(), StoreError> {
    let key = match namespace {
        Some(namespace) => format!("activity-policy:{tenant}:{namespace}"),
        None => format!("activity-policy:{tenant}"),
    };
    let sql = if exclusive {
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))"
    } else {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))"
    };
    sqlx::query(sql)
        .bind(key)
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn activity_policy_change(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
) -> Result<(), StoreError> {
    if namespace.is_some() {
        activity_policy_lock(conn, tenant, None, false).await?;
    }
    activity_policy_lock(conn, tenant, namespace, true).await
}

/// Takes shared policy fences followed by the activity lock for the rest of the transaction.
/// Callers acquiring several activities must visit them in activity-name order.
async fn lock_activity(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    activity: &ActivityName,
) -> Result<(), StoreError> {
    activity_policy_lock(conn, tenant, None, false).await?;
    activity_policy_lock(conn, tenant, Some(namespace), false).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "activity-quota:{tenant}:{namespace}:{}",
            activity.as_str()
        ))
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

/// Reserves one concurrency place for an activity until `conn` commits or rolls back.
///
/// The lock key includes every tenant-owned dimension. Counting only unexpired held leases keeps a
/// crashed worker from consuming quota beyond its lease, while the run lock still fences reclamation.
pub(crate) async fn activity_admitted(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    activity: &ActivityName,
) -> Result<bool, StoreError> {
    lock_activity(conn, tenant, namespace, activity).await?;
    let limit: i64 = sqlx::query_scalar(
        "SELECT least( \
             coalesce( \
                 (SELECT max_inflight FROM activity_quota_limits \
                   WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3), \
                 (SELECT max_value FROM quota_limits \
                   WHERE tenant_id = $1 AND namespace_id = $2 AND resource = 'activity_concurrency'), \
                 (SELECT max_value FROM quota_limits \
                   WHERE tenant_id = $1 AND namespace_id IS NULL AND resource = 'activity_concurrency'), \
                 (SELECT max_value FROM quota_ceilings \
                   WHERE tenant_id = $1 AND resource = 'activity_concurrency'), $4), \
             coalesce( \
                 (SELECT max_value FROM quota_ceilings \
                   WHERE tenant_id = $1 AND resource = 'activity_concurrency'), $4))::bigint",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(activity.as_str())
    .bind(i64::try_from(QuotaResource::ActivityConcurrency.default_limit()).unwrap_or(i64::MAX))
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if limit <= 0 {
        return Ok(false);
    }
    let used: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3 \
           AND status IN ('LEASED', 'RUNNING') AND lease_expires_at > clock_timestamp()",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(activity.as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(used < limit)
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
    // Read from the counters the database keeps with the runs (`run_active_counts`): at most sixteen
    // rows, whatever the number of runs. Counting the runs cost a start one index entry per active
    // run, which is what stopped a million waits from being made.
    let in_namespace: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts \
         WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(QuotaResource::ActiveRuns, in_namespace, namespace_limit)?;
    lock(conn, QuotaResource::TenantActiveRuns, tenant, None).await?;
    let in_tenant: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts WHERE tenant_id = $1",
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
    resource: QuotaResource,
) -> Result<(), StoreError> {
    lock(conn, resource, tenant, Some(namespace)).await
}

/// A new subscription fits in its namespace. The caller holds [`lock_subscriptions`].
pub(crate) async fn ensure_room_for_subscription(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    resource: QuotaResource,
    limit: u64,
) -> Result<(), StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_subscriptions WHERE tenant_id = $1 AND namespace_id = $2 \
           AND (($3 = 'webhook_subscriptions' AND destination_kind = 'WEBHOOK') \
             OR ($3 = 'broker_subscriptions' AND destination_kind <> 'WEBHOOK'))",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(resource.as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    judge(resource, count, limit)
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
    async fn activity_settings(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<StoredActivityLimit>, StoreError> {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT q.activity, q.max_inflight, \
                    (SELECT count(*) FROM steps s \
                      WHERE s.tenant_id = q.tenant_id AND s.namespace_id = q.namespace_id \
                        AND s.activity = q.activity AND s.status IN ('LEASED', 'RUNNING') \
                        AND s.lease_expires_at > clock_timestamp())::bigint AS used \
             FROM activity_quota_limits q \
             WHERE q.tenant_id = $1 AND q.namespace_id = $2 ORDER BY q.activity",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.into_iter()
            .map(|(activity, value, used)| {
                Ok(StoredActivityLimit {
                    activity: ActivityName::parse(&activity).map_err(|_| StoreError::Internal {
                        detail: "a stored activity quota has an invalid name".into(),
                    })?,
                    value: u64::try_from(value).map_err(|_| StoreError::Internal {
                        detail: "a stored activity quota is negative".into(),
                    })?,
                    used: u64::try_from(used).map_err(|_| StoreError::Internal {
                        detail: "stored activity quota usage is negative".into(),
                    })?,
                })
            })
            .collect()
    }

    async fn put_activity_setting(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activity: &ActivityName,
        value: Option<u64>,
        changed_by: &str,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // A completed change governs every claim that begins its quota decision afterwards. A
        // claim already holding this lock may finish before a lower limit takes effect.
        lock_activity(&mut tx, tenant, namespace, activity).await?;
        match value {
            Some(value) => {
                sqlx::query(
                    "INSERT INTO activity_quota_limits \
                     (tenant_id, namespace_id, activity, max_inflight, updated_by) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (tenant_id, namespace_id, activity) DO UPDATE SET \
                       max_inflight = excluded.max_inflight, updated_by = excluded.updated_by, \
                       updated_at = clock_timestamp()",
                )
                .bind(tenant.as_uuid())
                .bind(namespace.as_uuid())
                .bind(activity.as_str())
                .bind(i64::try_from(value).unwrap_or(i64::MAX))
                .bind(changed_by)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM activity_quota_limits \
                     WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3",
                )
                .bind(tenant.as_uuid())
                .bind(namespace.as_uuid())
                .bind(activity.as_str())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn activity_usage(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activity: &ActivityName,
    ) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE tenant_id = $1 AND namespace_id = $2 \
             AND activity = $3 AND status IN ('LEASED', 'RUNNING') \
             AND lease_expires_at > clock_timestamp()",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(activity.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(u64::try_from(count).unwrap_or(u64::MAX))
    }

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
        if resource == QuotaResource::ActivityConcurrency {
            activity_policy_change(&mut tx, tenant, None).await?;
        }
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
        if resource == QuotaResource::ActivityConcurrency {
            activity_policy_change(&mut tx, tenant, namespace).await?;
        }
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
                    "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts \
                     WHERE tenant_id = $1 AND namespace_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::TenantActiveRuns => {
                sqlx::query_scalar(
                    "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts \
                     WHERE tenant_id = $1",
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
            QuotaResource::ActivityConcurrency => {
                // The generic limit applies separately to every activity; its scalar usage is the
                // busiest activity, while activity-specific views expose their own exact count.
                sqlx::query_scalar(
                    "SELECT coalesce(max(held), 0)::bigint FROM ( \
                         SELECT count(*) AS held FROM steps \
                          WHERE tenant_id = $1 AND namespace_id = $2 \
                            AND status IN ('LEASED', 'RUNNING') \
                            AND lease_expires_at > clock_timestamp() GROUP BY activity \
                     ) activity_usage",
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
                 WHERE tenant_id = $1 AND namespace_id = $2 AND destination_kind = 'WEBHOOK'",
                )
                .bind(tenant.as_uuid())
                .bind(namespace)
                .fetch_one(&self.pool)
                .await
            }
            QuotaResource::BrokerSubscriptions => {
                sqlx::query_scalar(
                    "SELECT count(*) FROM webhook_subscriptions \
                     WHERE tenant_id = $1 AND namespace_id = $2 AND destination_kind <> 'WEBHOOK'",
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
