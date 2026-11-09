//! Retain or reuse one immutable definition inside the run admission transaction.
use super::*;
use sagat_application::runs::{ConnectionProbe, RetainedDefinition};
use sagat_domain::database::ConnectionId;

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

/// Whether the active connection `name` of the namespace, at `revision` (the current one when none), is an Apache Iceberg connection. A
/// plain read: nothing is locked, and every answer but "it is one" is `false`, so that a missing, paused or deleted connection, a revision
/// that is not there, another tenant's and a SQL one are the same answer.
pub(super) async fn is_lake_connection(
    store: &PgRunStore,
    tenant: TenantId,
    namespace: NamespaceId,
    name: &Slug,
    revision: Option<u32>,
) -> Result<bool, StoreError> {
    let Ok(revision) = revision.map(i32::try_from).transpose() else {
        return Ok(false);
    };
    let config: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT r.config FROM database_connections c JOIN database_connection_revisions r ON r.tenant_id=c.tenant_id AND r.namespace_id=c.namespace_id AND r.connection_id=c.id WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.name=$3 AND c.status='ACTIVE' AND r.revision=coalesce($4,c.current_revision)")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).bind(revision)
        .fetch_optional(&store.pool).await.map_err(store_error)?;
    let Some(config) = config else {
        return Ok(false);
    };
    Ok(crate::database::config_from(&config)?
        .as_iceberg()
        .is_some())
}

/// A probe of an Apache Iceberg connection is the only one in flight for its connection and revision (D035-150): each costs a container,
/// two secret reads and a call to the customer's catalog. Called inside the transaction that creates the run, **after admission**, which
/// takes the namespace's lock of secrets for the rest of the transaction (every start of a managed database step in it does), **and after
/// the active-run quota of the namespace was locked** (every creation in it takes that lock), so two starts of the same probe cannot both
/// see none. Either lock alone would do, which is why a hand mutation that moved the check above the quota lock was not caught: it is
/// an equivalent one. The check runs before the run's own pin is written, so only another run can be found. A probe of a SQL connection is
/// not held to this.
///
/// # Errors
/// `StateConflict` when another probe of the same connection and revision has not ended.
pub(super) async fn ensure_no_probe_in_flight(
    conn: &mut PgConnection,
    info: &RunInfo,
    pins: &[(NodeId, ConnectionId, u32)],
) -> Result<(), StoreError> {
    for (_, connection, revision) in pins {
        let in_flight: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs r JOIN run_database_connections p ON p.tenant_id=r.tenant_id AND p.namespace_id=r.namespace_id AND p.run_id=r.id WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.origin='CONNECTION_PROBE' AND r.finished_at IS NULL AND p.connection_id=$3 AND p.revision=$4)")
            .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(connection.as_uuid()).bind(i64::from(*revision))
            .fetch_one(&mut *conn).await.map_err(store_error)?;
        if in_flight {
            return Err(StoreError::StateConflict(
                "a probe of this connection and revision is in flight",
            ));
        }
    }
    Ok(())
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
    let row = sqlx::query("SELECT r.*,p.revision,s.failure_code,s.activity AS probe_activity FROM runs r JOIN run_database_connections p ON p.tenant_id=r.tenant_id AND p.namespace_id=r.namespace_id AND p.run_id=r.id JOIN database_connections c ON c.tenant_id=p.tenant_id AND c.namespace_id=p.namespace_id AND c.id=p.connection_id LEFT JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node=p.node_id AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND c.name=$3 AND c.status <> 'DELETED' AND r.origin='CONNECTION_PROBE' ORDER BY r.id DESC LIMIT 1")
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
    // The probe of an Iceberg connection is on its own activity, and the only one whose output is read: it is the server's own writing of a
    // result of closed words (D035-150). A SQL probe's output is its rows, which this surface never serves.
    let activity: Option<String> = col(&row, "probe_activity")?;
    let lake = activity.as_deref() == Some(sagat_domain::database::ICEBERG_PROBE_ACTIVITY);
    let output: Option<Vec<u8>> = if lake {
        sqlx::query_scalar("SELECT p.content FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.status='SUCCEEDED' AND s.activity=$3 AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0")
            .bind(tenant.as_uuid())
            .bind(run.info.run_id.as_uuid())
            .bind(sagat_domain::database::ICEBERG_PROBE_ACTIVITY)
            .fetch_optional(&store.pool)
            .await
            .map_err(store_error)?
    } else {
        None
    };
    Ok(Some(ConnectionProbe {
        run,
        revision,
        failure_code,
        lake,
        output,
        result: None,
    }))
}
