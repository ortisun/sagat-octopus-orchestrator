//! Current operator/namespace ceilings, fenced through the caller's admission transaction.
use crate::identity::{corrupt, store_error};
use sagat_application::error::StoreError;
use sagat_domain::{
    database::DatabaseLimits,
    identity::{NamespaceId, TenantId},
    quota::{self, QuotaResource},
};
use sqlx::PgConnection;

pub(crate) struct DatabasePolicy {
    pub limits: DatabaseLimits,
    pub connections: u32,
    pub namespace_concurrency: u32,
    pub tenant_concurrency: u32,
    pub provider_operations: u32,
}

async fn lock(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    exclusive: bool,
) -> Result<(), StoreError> {
    let key = match namespace {
        Some(namespace) => format!("database-policy:{tenant}:{namespace}"),
        None => format!("database-policy:{tenant}"),
    };
    sqlx::query(if exclusive {
        "SELECT pg_advisory_xact_lock(hashtextextended($1,0))"
    } else {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended($1,0))"
    })
    .bind(key)
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

/// Policy writers never acquire run, catalog, secret or concurrency locks.
pub(crate) async fn change(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
) -> Result<(), StoreError> {
    if namespace.is_some() {
        lock(conn, tenant, None, false).await?;
    }
    lock(conn, tenant, namespace, true).await
}

/// Caller holds the namespace secret/catalog lock. Locks survive through commit, including when
/// no explicit settings exist yet. New leases additionally hold the tenant concurrency lock first.
pub(crate) async fn read(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<DatabasePolicy, StoreError> {
    lock(conn, tenant, None, false).await?;
    lock(conn, tenant, Some(namespace), false).await?;
    let resources: Vec<_> = QuotaResource::ALL
        .into_iter()
        .filter(|r| r.is_database())
        .map(QuotaResource::as_str)
        .collect();
    let rows: Vec<(String, Option<uuid::Uuid>, i64, bool)> = sqlx::query_as(
        "SELECT resource,namespace_id,max_value,false FROM quota_limits
         WHERE tenant_id=$1 AND resource=ANY($3) AND (namespace_id IS NULL OR namespace_id=$2)
         UNION ALL SELECT resource,NULL,max_value,true FROM quota_ceilings
         WHERE tenant_id=$1 AND resource=ANY($3)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(resources)
    .fetch_all(conn)
    .await
    .map_err(store_error)?;
    let effective = |resource: QuotaResource| -> Result<u32, StoreError> {
        let value = |scoped: bool, ceiling: bool| {
            rows.iter()
                .find(|(r, ns, _, c)| {
                    r == resource.as_str() && ns.is_some() == scoped && *c == ceiling
                })
                .map(|(_, _, v, _)| u64::try_from(*v).unwrap_or(0))
        };
        let ceiling = quota::ceiling(resource, value(false, true));
        let result = quota::resolve(resource, value(true, false), value(false, false), ceiling).0;
        quota::validate(resource, false, result, ceiling)
            .map_err(|_| corrupt("database policy"))?;
        u32::try_from(result).map_err(|_| corrupt("database policy"))
    };
    use QuotaResource as R;
    Ok(DatabasePolicy {
        limits: DatabaseLimits {
            max_statement_bytes: effective(R::DatabaseStatementBytes)?,
            max_parameters: effective(R::DatabaseParameters)?,
            max_parameter_bytes: effective(R::DatabaseParameterBytes)?,
            max_rows: effective(R::DatabaseRows)?,
            max_result_bytes: effective(R::DatabaseResultBytes)?,
            timeout_ms: effective(R::DatabaseTimeoutMs)?,
            connect_timeout_ms: effective(R::DatabaseConnectTimeoutMs)?,
            pool_size: effective(R::DatabasePoolSize)?,
            idle_timeout_ms: effective(R::DatabaseIdleTimeoutMs)?,
        },
        connections: effective(R::DatabaseConnections)?,
        namespace_concurrency: effective(R::DatabaseConcurrency)?,
        tenant_concurrency: effective(R::TenantDatabaseConcurrency)?,
        provider_operations: effective(R::SecretProviderOperations)?,
    })
}
