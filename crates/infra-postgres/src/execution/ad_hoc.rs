//! Retain or reuse one immutable definition inside the run admission transaction.
use super::*;
use sagat_application::runs::{ConnectionProbe, RetainedDefinition};

pub(super) async fn retain(
    conn: &mut PgConnection,
    info: &mut RunInfo,
    retained: &RetainedDefinition,
) -> Result<(), StoreError> {
    if info.origin != Some(retained.origin()) || info.parent.is_some() {
        return Err(StoreError::MissingReference);
    }
    let tenant = info.tenant_id;
    let namespace = info.namespace_id;
    // Match normal workflow creation's quota lock. Lookup before count allows reuse at capacity.
    crate::quotas::lock_workflows(conn, tenant, namespace).await?;
    let existing: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT workflow_id,version_id FROM private_execution_definitions WHERE tenant_id=$1 AND namespace_id=$2 AND actor=$3 AND digest=$4")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(&info.started_by).bind(retained.digest().to_hex())
        .fetch_optional(&mut *conn).await.map_err(store_error)?;
    if let Some((workflow, version)) = existing {
        info.workflow_id = WorkflowId::new(workflow);
        info.version_id = WorkflowVersionId::new(version);
        return Ok(());
    }
    crate::quotas::ensure_room_for_workflow(conn, tenant, namespace, retained.workflow_limit())
        .await?;
    // Generated identity is storage-only; it is never part of canonical content or request identity.
    let name = format!("private-{}", info.workflow_id.as_uuid().simple());
    sqlx::query("INSERT INTO workflows(id,tenant_id,namespace_id,name,draft,created_at,updated_at,draft_edited_by,internal) VALUES($1,$2,$3,$4,'{}'::jsonb,$5,$5,$6,true)")
        .bind(info.workflow_id.as_uuid()).bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name)
        .bind(info.created_at).bind(&info.started_by).execute(&mut *conn).await.map_err(store_error)?;
    sqlx::query("INSERT INTO workflow_versions(id,tenant_id,workflow_id,version_number,digest,document,published_by,published_at,classification_rules) VALUES($1,$2,$3,1,$4,$5,$6,$7,1)")
        .bind(info.version_id.as_uuid()).bind(tenant.as_uuid()).bind(info.workflow_id.as_uuid())
        .bind(retained.digest().to_hex()).bind(retained.document()).bind(&info.started_by).bind(info.created_at)
        .execute(&mut *conn).await.map_err(store_error)?;
    sqlx::query("INSERT INTO private_execution_definitions(tenant_id,namespace_id,actor,digest,workflow_id,version_id) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(&info.started_by).bind(retained.digest().to_hex())
        .bind(info.workflow_id.as_uuid()).bind(info.version_id.as_uuid())
        .execute(&mut *conn).await.map_err(store_error)?;
    Ok(())
}

pub(super) async fn private_definition(
    store: &PgRunStore,
    tenant: TenantId,
    actor: &str,
    run: RunId,
) -> Result<Option<Vec<u8>>, StoreError> {
    sqlx::query_scalar("SELECT v.document FROM runs r JOIN private_execution_definitions d ON d.tenant_id=r.tenant_id AND d.namespace_id=r.namespace_id AND d.workflow_id=r.workflow_id AND d.version_id=r.workflow_version_id AND d.actor=r.started_by JOIN workflow_versions v ON v.tenant_id=d.tenant_id AND v.id=d.version_id WHERE r.tenant_id=$1 AND r.id=$2 AND r.started_by=$3 AND r.origin IS NOT NULL")
        .bind(tenant.as_uuid()).bind(run.as_uuid()).bind(actor).fetch_optional(&store.pool).await.map_err(store_error)
}

pub(super) async fn latest_probe(
    store: &PgRunStore,
    tenant: TenantId,
    namespace: NamespaceId,
    name: &Slug,
) -> Result<Option<ConnectionProbe>, StoreError> {
    let existing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM database_connections WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND status <> 'DELETED')")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).fetch_one(&store.pool).await.map_err(store_error)?;
    if !existing {
        return Err(StoreError::MissingReference);
    }
    let row = sqlx::query("SELECT r.*,p.revision,s.failure_code FROM runs r JOIN run_database_connections p ON p.tenant_id=r.tenant_id AND p.namespace_id=r.namespace_id AND p.run_id=r.id JOIN database_connections c ON c.tenant_id=p.tenant_id AND c.namespace_id=p.namespace_id AND c.id=p.connection_id LEFT JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node=p.node_id AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND c.name=$3 AND c.status <> 'DELETED' AND r.origin='CONNECTION_PROBE' ORDER BY r.id DESC LIMIT 1")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).fetch_optional(&store.pool).await.map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let run = run_record(&row)?;
    let revision =
        u32::try_from(col::<i32>(&row, "revision")?).map_err(|_| corrupt("probe revision"))?;
    let failure: Option<String> = col(&row, "failure_code")?;
    // This is a metadata surface: never forward an arbitrary worker-provided failure string.
    let failure_code = failure.and_then(|value| {
        sagat_domain::database::DatabaseFailure::parse(&value).map(|code| code.code().to_owned())
    });
    Ok(Some(ConnectionProbe {
        run,
        revision,
        failure_code,
    }))
}
