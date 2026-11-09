//! Tenant-scoped admission under the caller's publication/run transaction. No provider I/O.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    identity::{corrupt, store_error},
    quotas::lock_secrets,
};
use sagat_application::{error::StoreError, execution::RunInfo};
use sagat_domain::{
    database::{ConnectionId, OperationKind, RecordSource},
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
        actors.insert(authority_of(conn, info.tenant_id, &info.started_by).await?);
    }
    if info.parent.is_some() {
        let (starter, publisher): (String, String) = sqlx::query_as(
            "SELECT r.started_by,v.published_by FROM runs r JOIN workflow_versions v ON v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3")
            .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(info.root.as_uuid())
            .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
        actors.insert(if starter == Subject::engine().as_str() {
            publisher
        } else {
            authority_of(conn, info.tenant_id, &starter).await?
        });
    }
    Ok(actors.into_iter().collect())
}

/// The member whose authority a subject recorded on a run stands for. A person or a service is its
/// own authority. `system:schedule:<id>` belongs to no membership: a run a schedule started stands on
/// the subject that authorized the schedule, today (`FR-018-015`, decision D1950), so a database
/// step is admitted under, and loses its admission with, that member.
pub(crate) async fn authority_of(
    conn: &mut PgConnection,
    tenant: TenantId,
    subject: &str,
) -> Result<String, StoreError> {
    let Some(schedule) = Subject::schedule_of(subject) else {
        return Ok(subject.to_owned());
    };
    sqlx::query_scalar("SELECT authorized_by FROM schedules WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid())
        .bind(schedule)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
        .ok_or(StoreError::DatabaseUnavailable)
}

/// What the managed database steps of a workflow ask of the subject behind a run: `None` for a
/// workflow with none, otherwise whether any of them writes.
pub(crate) fn database_writes(definition: &WorkflowDefinition) -> Option<bool> {
    // An Iceberg write is a write (it needs `database:write`, as an `execute` does), and a workflow that holds one has
    // managed steps even when it holds no SQL one: its run takes the same locks and the same authority re-check.
    let lake = definition.nodes().any(|n| n.iceberg_write().is_some());
    let mut operations = definition.nodes().filter_map(|n| n.database_operation());
    let Some(first) = operations.next() else {
        return lake.then_some(true);
    };
    Some(
        lake || first.kind == OperationKind::Execute
            || operations.any(|op| op.kind == OperationKind::Execute),
    )
}

/// Resolved revisions only; connection settings and all credentials stay out of admission receipts. `lake_enabled` is the
/// deployment's `lake.enabled`: without it a run that would carry an Iceberg write is refused (feature 035, D035-137).
#[tracing::instrument(skip_all, name = "database.admit")]
#[allow(clippy::too_many_arguments)] // one admission scope, with optional existing run and the deployment's lake switch
pub(crate) async fn admit_definition(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    actors: &[&str],
    definition: &WorkflowDefinition,
    pinned: Option<RunId>,
    run_info: Option<&RunInfo>,
    lake_enabled: bool,
) -> Result<Vec<(NodeId, ConnectionId, u32)>, StoreError> {
    if !definition.nodes().any(|n| n.managed_kind().is_some()) {
        return Ok(Vec::new());
    }
    let started = std::time::Instant::now();
    let result = check_definition(
        conn,
        tenant,
        namespace,
        actors,
        definition,
        pinned,
        run_info,
        None,
        lake_enabled,
    )
    .await;
    let outcome = match &result {
        Ok(_) => "authorized",
        Err(
            StoreError::DatabaseUnavailable
            | StoreError::DatabaseLimit
            | StoreError::LakeRefused { .. },
        ) => "denied",
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
    lake_enabled: bool,
) -> Result<(), StoreError> {
    let actors = run_actors(conn, info).await?;
    // The nodes whose connections are asked: the node, and the SQL step whose retained result a lake write takes (G035-083). The rows
    // are already in the run, but where they came from is still asked when they are written: a connection paused, or a secret taken
    // from the actors, since the statement ran stops the write, as a revocation that has returned stops every later admission.
    let mut asked = vec![node.clone()];
    if let Some(RecordSource::DatabaseResult(source)) = definition
        .node(node)
        .and_then(sagat_domain::workflow::Node::iceberg_write)
        .map(sagat_domain::database::IcebergWrite::records)
    {
        asked.push(source.clone());
    }
    check_definition(
        conn,
        info.tenant_id,
        info.namespace_id,
        &actors.iter().map(String::as_str).collect::<Vec<_>>(),
        definition,
        Some(info.run_id),
        Some(info),
        Some(&asked),
        lake_enabled,
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
    selected: Option<&[NodeId]>,
    lake_enabled: bool,
) -> Result<Vec<(NodeId, ConnectionId, u32)>, StoreError> {
    let operations: Vec<_> = definition
        .nodes()
        .filter(|n| selected.is_none_or(|ids| ids.contains(n.id())))
        .filter_map(|n| n.database_operation().map(|op| (n.id(), op)))
        .collect();
    let lake_writes: Vec<_> = definition
        .nodes()
        .filter(|n| selected.is_none_or(|ids| ids.contains(n.id())))
        .filter_map(|n| n.iceberg_write().map(|write| (n.id(), write)))
        .collect();
    if operations.is_empty() && lake_writes.is_empty() {
        return Ok(Vec::new());
    }
    // An Iceberg write is admitted for publication and for the author's feedback always, and for a run only when the
    // deployment has turned the lake on (`lake.enabled`, off by default; D035-059, D035-137). Until then a run that would
    // carry one is refused, as a connection nobody may use is, instead of starting and waiting for a worker that is not there.
    if !lake_writes.is_empty()
        && !lake_enabled
        && (pinned.is_some() || run_info.is_some() || selected.is_some())
    {
        return Err(StoreError::DatabaseUnavailable);
    }
    // Same order as catalog/secret writes. Membership and existing ACL rows remain locked through
    // commit, so a revocation that has returned stops every subsequent admission.
    lock_secrets(conn, tenant, namespace).await?;
    let policy = super::policy::read(conn, tenant, namespace).await?;
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
    // Every Iceberg write is a write, so it needs `database:write` as an `execute` does.
    let writes = !lake_writes.is_empty()
        || operations
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
    let mut result = Vec::with_capacity(operations.len() + lake_writes.len());
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
            let found = resolve_connection(
                conn,
                (tenant, namespace),
                actors,
                &roles,
                &environment,
                key.0,
                revision,
            )
            .await?;
            resolved.entry(key).or_insert(found)
        };
        if pin.is_some_and(|p| p != (*id, *revision))
            || operation
                .connection
                .revision
                .is_some_and(|r| i64::from(r) != i64::from(*revision))
        {
            return Err(StoreError::DatabaseUnavailable);
        }
        // A SQL node can only use a SQL connection; a lake connection is unusable here.
        let limits = config.sql_limits().ok_or(StoreError::DatabaseUnavailable)?;
        let mut effective = operation.clone();
        effective.limits = effective.limits.intersect(limits).intersect(policy.limits);
        effective
            .validate()
            .map_err(|_| StoreError::DatabaseLimit)?;
        result.push((
            node.clone(),
            ConnectionId::new(*id),
            u32::try_from(*revision).map_err(|_| corrupt("connection revision"))?,
        ));
    }
    for (node, write) in lake_writes {
        let name = write.connection().name.as_str();
        // A run that exists uses the revision it was admitted with, whatever the connection has become since: a floating
        // write must not re-resolve at claim or replay (D035-130 E).
        let pin = if pinned.is_some() {
            Some(
                *pins
                    .get(node.as_str())
                    .ok_or(StoreError::DatabaseUnavailable)?,
            )
        } else {
            None
        };
        let pinned_revision = pin
            .map(|(_, r)| i64::from(r))
            .or(write.connection().revision.map(i64::from));
        let (id, revision, refusal) = {
            let key = (name, pinned_revision);
            let (id, revision, config) = if let Some(found) = resolved.get(&key) {
                found
            } else {
                let found = resolve_connection(
                    conn,
                    (tenant, namespace),
                    actors,
                    &roles,
                    &environment,
                    key.0,
                    key.1,
                )
                .await?;
                resolved.entry(key).or_insert(found)
            };
            // A lake write can only use a lake connection; a SQL connection is unusable here, as a lake one is to a SQL node.
            let iceberg = config.as_iceberg().ok_or(StoreError::DatabaseUnavailable)?;
            // The pin is the connection and the revision the run was admitted with; nothing else stands in for it.
            if pin.is_some_and(|p| p != (*id, *revision))
                || write
                    .connection()
                    .revision
                    .is_some_and(|r| i64::from(r) != i64::from(*revision))
            {
                return Err(StoreError::DatabaseUnavailable);
            }
            (*id, *revision, write.refusal_by(iceberg))
        };
        // A step that names a revision is judged by the current one too: a mode or a ceiling that was tightened since applies
        // to it, so naming an older revision cannot loosen what the connection says now. A run that already exists is not:
        // it keeps the revision it was admitted with, as a SQL run does, so the replay of its start and the claim of its step
        // are judged by that revision, and the way to stop it is to pause the connection, which refuses both.
        let refusal = match (refusal, pinned_revision) {
            (None, Some(_)) if pin.is_none() => {
                let key = (name, None);
                let (_, _, current) = if let Some(found) = resolved.get(&key) {
                    found
                } else {
                    let found = resolve_connection(
                        conn,
                        (tenant, namespace),
                        actors,
                        &roles,
                        &environment,
                        key.0,
                        key.1,
                    )
                    .await?;
                    resolved.entry(key).or_insert(found)
                };
                let current = current
                    .as_iceberg()
                    .ok_or(StoreError::DatabaseUnavailable)?;
                write.refusal_by(current)
            }
            (refusal, _) => refusal,
        };
        if let Some(refusal) = refusal {
            return Err(StoreError::LakeRefused {
                node: node.as_str().to_owned(),
                refusal,
            });
        }
        result.push((
            node.clone(),
            ConnectionId::new(id),
            u32::try_from(revision).map_err(|_| corrupt("connection revision"))?,
        ));
    }
    Ok(result)
}

/// An active connection of the namespace at the requested revision (the current one when none is asked for), once the
/// actors may use every secret it refers to and the namespace may reach what it reaches. Nothing leaves the database: no
/// provider is contacted. Whatever stops it is the same refusal, so a connection that is missing, paused, of another kind of
/// namespace or out of reach cannot be told apart.
async fn resolve_connection(
    conn: &mut PgConnection,
    (tenant, namespace): (TenantId, NamespaceId),
    actors: &[&str],
    roles: &BTreeMap<String, Role>,
    environment: &str,
    name: &str,
    revision: Option<i64>,
) -> Result<(Uuid, i32, sagat_domain::database::ConnectionConfig), StoreError> {
    let row = sqlx::query(
        "SELECT c.id,r.revision,r.config FROM database_connections c JOIN database_connection_revisions r ON r.tenant_id=c.tenant_id AND r.namespace_id=c.namespace_id AND r.connection_id=c.id WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.name=$3 AND c.status='ACTIVE' AND r.revision=coalesce($4,c.current_revision) FOR SHARE OF c")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name).bind(revision)
        .fetch_optional(&mut *conn).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
    let id: Uuid = row.try_get("id").map_err(store_error)?;
    let revision: i32 = row.try_get("revision").map_err(store_error)?;
    let config = super::config_from(&row.try_get("config").map_err(store_error)?)?;
    if config.requires_development() && environment != "dev" {
        return Err(StoreError::DatabaseUnavailable);
    }
    // Every secret the connection refers to: one for a SQL connection, the catalog's and the storage's for a lake one.
    let mut seen = BTreeSet::new();
    for secret_name in config.secrets() {
        if !seen.insert(secret_name.as_str().to_owned()) {
            continue;
        }
        let secret: Uuid = sqlx::query_scalar(
            "SELECT id FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND authority='openbao' FOR SHARE")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(secret_name.as_str())
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
    }
    Ok((id, revision, config))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::database_writes;
    use sagat_domain::{
        database::{
            ColumnMapping, ConnectionRef, DatabaseLimits, DatabaseOperation, IcebergWrite,
            IcebergWriteParts, LakeLimits, OperationKind, RecordSource, TableIdentifier,
            WriteIntent,
        },
        identity::Slug,
        workflow::{Expression, Node, NodeId, NodeKind, WorkflowDefinition},
    };

    fn id(text: &str) -> NodeId {
        NodeId::parse(text).expect("a node id")
    }

    fn connection(name: &str) -> ConnectionRef {
        ConnectionRef::new(Slug::parse(name).expect("name"), None).expect("a reference")
    }

    fn sql(node: &str, kind: OperationKind) -> Node {
        Node::database(
            id(node),
            DatabaseOperation::new(
                connection("orders-db"),
                kind,
                "SELECT 1",
                vec![],
                DatabaseLimits::default(),
            )
            .expect("an operation"),
        )
    }

    fn lake(node: &str) -> Node {
        let write = IcebergWrite::new(IcebergWriteParts {
            connection: connection("lake-main"),
            table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("a table"),
            reference: "main".to_owned(),
            intent: WriteIntent::Append,
            records: RecordSource::Inline(Expression::parse("input.orders").expect("expression")),
            mapping: vec![ColumnMapping::new("/id", "id").expect("a mapping")],
            ignore_unknown_fields: false,
            limits: LakeLimits::default(),
            revert: false,
        })
        .expect("a write");
        Node::iceberg(id(node), write)
    }

    /// The nodes in a line, then an end.
    fn line(nodes: Vec<Node>) -> WorkflowDefinition {
        let entry = nodes
            .first()
            .map_or_else(|| id("done"), |node| node.id().clone());
        let mut linked: Vec<Node> = Vec::new();
        let mut next = id("done");
        for node in nodes.into_iter().rev() {
            let current = node.id().clone();
            linked.push(node.with_next(next));
            next = current;
        }
        linked.reverse();
        linked.push(Node::new(id("done"), NodeKind::End { output: None }));
        WorkflowDefinition::new(Slug::parse("flow").expect("name"), entry, linked)
            .expect("a definition")
    }

    #[test]
    fn what_a_workflow_asks_of_the_subject_behind_a_run_counts_an_iceberg_write_as_a_write() {
        assert_eq!(database_writes(&line(vec![])), None, "no managed step");
        assert_eq!(
            database_writes(&line(vec![sql("read", OperationKind::Query)])),
            Some(false)
        );
        assert_eq!(
            database_writes(&line(vec![sql("write", OperationKind::Execute)])),
            Some(true)
        );
        assert_eq!(
            database_writes(&line(vec![lake("save")])),
            Some(true),
            "a workflow with an Iceberg write and no SQL step has managed steps, and writes"
        );
        assert_eq!(
            database_writes(&line(vec![sql("read", OperationKind::Query), lake("save")])),
            Some(true)
        );
    }
}
