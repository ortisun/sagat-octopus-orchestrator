//! Tenant-scoped admission under the caller's publication/run transaction. No provider I/O.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    identity::{corrupt, store_error},
    quotas::lock_secrets,
};
use sagat_application::{error::StoreError, execution::RunInfo};
use sagat_domain::{
    database::{ConnectionId, OperationKind},
    execution::RunId,
    identity::{NamespaceId, Permission, Role, Subject, TenantId},
    workflow::{NodeId, WorkflowDefinition},
};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// Publisher plus initiating user. A triggered run uses its publisher; a child also retains the
/// root initiator (or root publisher for a triggered tree). `system:engine` grants no authority.
pub(crate) async fn run_actors(
    conn: &mut PgConnection,
    info: &RunInfo,
) -> Result<Vec<String>, StoreError> {
    let publisher: String = sqlx::query_scalar(
        "SELECT v.published_by FROM workflow_versions v JOIN workflows w ON w.tenant_id=v.tenant_id AND w.id=v.workflow_id WHERE v.tenant_id=$1 AND v.id=$2 AND w.namespace_id=$3")
        .bind(info.tenant_id.as_uuid()).bind(info.version_id.as_uuid()).bind(info.namespace_id.as_uuid())
        .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
    let mut actors = BTreeSet::from([publisher]);
    if info.started_by != Subject::engine().as_str() {
        actors.insert(info.started_by.clone());
    }
    if info.parent.is_some() {
        let (starter, publisher): (String, String) = sqlx::query_as(
            "SELECT r.started_by,v.published_by FROM runs r JOIN workflow_versions v ON v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3")
            .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(info.root.as_uuid())
            .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
        actors.insert(if starter == Subject::engine().as_str() {
            publisher
        } else {
            starter
        });
    }
    Ok(actors.into_iter().collect())
}

/// Resolved revisions only; connection settings and all credentials stay out of admission receipts.
#[tracing::instrument(skip_all, name = "database.admit")]
pub(crate) async fn admit_definition(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    actors: &[&str],
    definition: &WorkflowDefinition,
    pinned: Option<RunId>,
    run_info: Option<&RunInfo>,
) -> Result<Vec<(NodeId, ConnectionId, u32)>, StoreError> {
    if !definition.nodes().any(|n| n.database_operation().is_some()) {
        return Ok(Vec::new());
    }
    let started = std::time::Instant::now();
    let result = check_definition(
        conn, tenant, namespace, actors, definition, pinned, run_info, None,
    )
    .await;
    let outcome = match &result {
        Ok(_) => "authorized",
        Err(StoreError::DatabaseUnavailable | StoreError::DatabaseLimit) => "denied",
        Err(_) => "error",
    };
    metrics::counter!("database_admissions_total", "outcome" => outcome).increment(1);
    metrics::histogram!("database_admission_seconds", "outcome" => outcome)
        .record(started.elapsed().as_secs_f64());
    result
}

pub(crate) async fn admit_node(
    conn: &mut PgConnection,
    info: &RunInfo,
    definition: &WorkflowDefinition,
    node: &NodeId,
) -> Result<(), StoreError> {
    let actors = run_actors(conn, info).await?;
    check_definition(
        conn,
        info.tenant_id,
        info.namespace_id,
        &actors.iter().map(String::as_str).collect::<Vec<_>>(),
        definition,
        Some(info.run_id),
        Some(info),
        Some(node),
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)] // one admission scope, with optional existing run/node
async fn check_definition(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    actors: &[&str],
    definition: &WorkflowDefinition,
    pinned: Option<RunId>,
    run_info: Option<&RunInfo>,
    selected: Option<&NodeId>,
) -> Result<Vec<(NodeId, ConnectionId, u32)>, StoreError> {
    let operations: Vec<_> = definition
        .nodes()
        .filter(|n| selected.is_none_or(|id| n.id() == id))
        .filter_map(|n| n.database_operation().map(|op| (n.id(), op)))
        .collect();
    if operations.is_empty() {
        return Ok(Vec::new());
    }
    // Same order as catalog/secret writes. Membership and existing ACL rows remain locked through
    // commit, so a revocation that has returned stops every subsequent admission.
    lock_secrets(conn, tenant, namespace).await?;
    let roles: Vec<(String, String)> =
        sqlx::query_as("SELECT subject,role FROM public.sagat_lock_memberships($1,$2)")
            .bind(tenant.as_uuid())
            .bind(actors)
            .fetch_all(&mut *conn)
            .await
            .map_err(store_error)?;
    let roles: BTreeMap<_, _> = roles
        .into_iter()
        .map(|(subject, role)| {
            Ok((
                subject,
                role.parse::<Role>()
                    .map_err(|_| corrupt("database membership role"))?,
            ))
        })
        .collect::<Result<_, StoreError>>()?;
    let writes = operations
        .iter()
        .any(|(_, op)| op.kind == OperationKind::Execute);
    if actors.is_empty()
        || actors.iter().any(|a| {
            roles.get(*a).is_none_or(|r| {
                !r.grants(Permission::DatabaseUse)
                    || (run_info.is_some_and(|info| info.origin.is_some())
                        && (!r.grants(Permission::DatabaseAdHoc)
                            || !r.grants(Permission::RunStart)))
                    || (run_info
                        .is_some_and(|info| info.origin.is_some() && info.rerun_of.is_some())
                        && !r.grants(Permission::RunOperate))
                    || (writes && !r.grants(Permission::DatabaseWrite))
            })
        })
    {
        return Err(StoreError::DatabaseUnavailable);
    }
    let environment: String =
        sqlx::query_scalar("SELECT environment FROM public.sagat_lock_namespace($1,$2)")
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::DatabaseUnavailable)?;
    let pins: BTreeMap<String, (Uuid, i32)> = if let Some(run) = pinned {
        let rows: Vec<(String,Uuid,i32)> = sqlx::query_as(
            "SELECT node_id,connection_id,revision FROM run_database_connections WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(run.as_uuid()).fetch_all(&mut *conn).await.map_err(store_error)?;
        rows.into_iter()
            .map(|(node, id, revision)| (node, (id, revision)))
            .collect()
    } else {
        BTreeMap::new()
    };
    let mut resolved = BTreeMap::new();
    let mut result = Vec::with_capacity(operations.len());
    for (node, operation) in operations {
        let pin = if pinned.is_some() {
            Some(
                *pins
                    .get(node.as_str())
                    .ok_or(StoreError::DatabaseUnavailable)?,
            )
        } else {
            None
        };
        let revision = pin
            .map(|(_, r)| i64::from(r))
            .or(operation.connection.revision.map(i64::from));
        let key = (operation.connection.name.as_str(), revision);
        let (id, revision, config) = if let Some(found) = resolved.get(&key) {
            found
        } else {
            let row = sqlx::query(
                "SELECT c.id,r.revision,r.config FROM database_connections c JOIN database_connection_revisions r ON r.tenant_id=c.tenant_id AND r.namespace_id=c.namespace_id AND r.connection_id=c.id WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.name=$3 AND c.status='ACTIVE' AND r.revision=coalesce($4,c.current_revision) FOR SHARE OF c")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(operation.connection.name.as_str()).bind(revision)
                .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
            let id: Uuid = row.try_get("id").map_err(store_error)?;
            let revision: i32 = row.try_get("revision").map_err(store_error)?;
            let config = super::config_from(&row.try_get("config").map_err(store_error)?)?;
            if matches!(
                &config,
                sagat_domain::database::ConnectionConfig::Postgres {
                    tls: sagat_domain::database::TlsPolicy::DevelopmentPlaintext,
                    ..
                }
            ) && environment != "dev"
            {
                return Err(StoreError::DatabaseUnavailable);
            }
            let secret: Uuid = sqlx::query_scalar(
                "SELECT id FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND authority='openbao' FOR SHARE")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(config.secret().as_str())
                .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
            let granted: Vec<String> =
                sqlx::query_scalar("SELECT subject FROM public.sagat_lock_secret_access($1,$2,$3)")
                    .bind(tenant.as_uuid())
                    .bind(secret)
                    .bind(actors)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(store_error)?;
            if roles.iter().any(|(subject, role)| {
                !sagat_domain::secrets::may_use(*role, granted.contains(subject))
            }) {
                return Err(StoreError::DatabaseUnavailable);
            }
            resolved.entry(key).or_insert((id, revision, config))
        };
        if pin.is_some_and(|p| p != (*id, *revision))
            || operation
                .connection
                .revision
                .is_some_and(|r| i64::from(r) != i64::from(*revision))
        {
            return Err(StoreError::DatabaseUnavailable);
        }
        let mut effective = operation.clone();
        effective.limits = effective.limits.intersect(config.limits());
        effective
            .validate()
            .map_err(|_| StoreError::DatabaseLimit)?;
        result.push((
            node.clone(),
            ConnectionId::new(*id),
            u32::try_from(*revision).map_err(|_| corrupt("connection revision"))?,
        ));
    }
    Ok(result)
}
