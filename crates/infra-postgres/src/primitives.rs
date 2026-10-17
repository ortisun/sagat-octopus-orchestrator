//! PostgreSQL collection progress adapter.
//!
//! The methods here are intentionally not wired into publication or scheduling. They persist a
//! collection transition together with its journal, projections, usage and outbox. Checkpoint and
//! finish additionally require an immutable definition cache. Runtime orchestration stays gated
//! until the planner owns scoped routing and supplies semantic audit evidence for every transition.

use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionCheckpoint, CollectionDrain, CollectionDrainFinish,
    CollectionEntry, CollectionFinish, CollectionIterationDrain, CollectionProgress,
    CollectionScopeId, CollectionStore, CollectionTransition, PrimitiveStoreError, PrimitiveWrite,
};
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    CollectionDrainReason, ExecutionInstanceId, FailureCode, InstancePath,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::outbound::OutboundKind;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{NodeId, NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{insert_audit, store_error};
use crate::pool::BeginSafely;

/// Storage adapter for collection transition batches.
#[derive(Debug, Clone)]
pub struct PgCollectionStore {
    pool: PgPool,
    definitions: Option<DefinitionCache>,
}

impl PgCollectionStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            definitions: None,
        }
    }

    /// Enables immutable definition checks required before a scoped return can checkpoint.
    #[must_use]
    pub fn with_definition_cache(pool: PgPool, definitions: DefinitionCache) -> Self {
        Self {
            pool,
            definitions: Some(definitions),
        }
    }

    async fn definition(
        &self,
        conn: &mut PgConnection,
        tenant_id: Uuid,
        version_id: WorkflowVersionId,
    ) -> Result<std::sync::Arc<WorkflowDefinition>, PrimitiveStoreError> {
        let cache = self
            .definitions
            .as_ref()
            .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?;
        if let Some(definition) = cache.get(version_id) {
            return Ok(definition);
        }
        let document: Vec<u8> = sqlx::query_scalar(
            "SELECT document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(version_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        cache.get_or_parse(version_id, &document).map_err(|error| {
            tracing::error!(%error, %version_id, "stored collection workflow no longer parses");
            invalid_storage("stored workflow definition")
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct LockedRun {
    namespace_id: NamespaceId,
    version_id: WorkflowVersionId,
    run_version: u64,
}

fn path_parts(instance: &ExecutionInstanceId) -> (Vec<String>, Vec<i32>) {
    instance
        .path()
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
            )
        })
        .unzip()
}

fn stored_instance(
    path_nodes: Vec<String>,
    path_indices: Vec<i32>,
    node: String,
) -> Result<ExecutionInstanceId, PrimitiveStoreError> {
    if path_nodes.len() != path_indices.len() {
        return Err(invalid_storage("collection member path"));
    }
    let mut path = InstancePath::root();
    for (scope, index) in path_nodes.into_iter().zip(path_indices) {
        let scope =
            NodeId::parse_stored(&scope).map_err(|_| invalid_storage("collection member scope"))?;
        let index =
            u32::try_from(index).map_err(|_| invalid_storage("collection member iteration"))?;
        path = path
            .child(scope, index)
            .map_err(|_| invalid_storage("collection member path"))?;
    }
    let node =
        NodeId::parse_stored(&node).map_err(|_| invalid_storage("collection member node"))?;
    Ok(ExecutionInstanceId::in_path(path, node))
}

fn instance_json(instance: &ExecutionInstanceId) -> Value {
    json!({
        "path": instance.path().segments().iter().map(|segment| json!({
            "scope": segment.scope_node().as_str(),
            "index": segment.iteration_index(),
        })).collect::<Vec<_>>(),
        "node": instance.node().as_str(),
    })
}

fn payload_matches(content: &[u8], digest: sagat_domain::workflow::Digest) -> bool {
    Sha256::digest(content).as_slice() == digest.as_bytes()
}

async fn retain_payload_in_transaction(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    digest: sagat_domain::workflow::Digest,
    content: Option<&[u8]>,
) -> Result<Vec<u8>, PrimitiveStoreError> {
    if let Some(content) = content {
        if !payload_matches(content, digest) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        sqlx::query(
            "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
             ON CONFLICT (tenant_id,digest) DO NOTHING",
        )
        .bind(tenant_id.as_uuid())
        .bind(digest.as_bytes().as_slice())
        .bind(content)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    }
    let retained: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id=$1 AND digest=$2")
            .bind(tenant_id.as_uuid())
            .bind(digest.as_bytes().as_slice())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    let Some(retained) = retained else {
        return Err(PrimitiveStoreError::InvalidTransition);
    };
    if !payload_matches(&retained, digest) {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    if content.is_some_and(|value| value != retained) {
        return Err(PrimitiveStoreError::DuplicateMismatch);
    }
    Ok(retained)
}

fn number(value: u64, field: &'static str) -> Result<i64, PrimitiveStoreError> {
    i64::try_from(value).map_err(|_| {
        PrimitiveStoreError::Storage(StoreError::Internal {
            detail: format!("{field} does not fit PostgreSQL bigint"),
        })
    })
}

fn invalid_storage(detail: &'static str) -> PrimitiveStoreError {
    PrimitiveStoreError::Storage(StoreError::Internal {
        detail: detail.to_owned(),
    })
}

async fn lock_run(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<LockedRun, PrimitiveStoreError> {
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, run_version, status FROM runs \
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::Missing)?;
    let namespace_id = NamespaceId::new(row.get("namespace_id"));
    let status: String = row.get("status");
    let stored_version: i64 = row.get("run_version");
    let run_version = u64::try_from(stored_version).map_err(|_| invalid_storage("run version"))?;
    if namespace_id != transition.namespace_id()
        || !matches!(status.as_str(), "RUNNING" | "WAITING")
    {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    Ok(LockedRun {
        namespace_id,
        version_id: WorkflowVersionId::new(row.get("workflow_version_id")),
        run_version,
    })
}

#[derive(Debug, Clone)]
struct LockedDrainRun {
    run: LockedRun,
    status: String,
    stop_reason: Option<String>,
    stop_code: Option<String>,
}

async fn lock_drain_run(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<LockedDrainRun, PrimitiveStoreError> {
    let row = sqlx::query(
        "SELECT namespace_id,workflow_version_id,run_version,status,stop_reason,stop_code \
         FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::Missing)?;
    let run = LockedRun {
        namespace_id: NamespaceId::new(row.get("namespace_id")),
        version_id: WorkflowVersionId::new(row.get("workflow_version_id")),
        run_version: u64::try_from(row.get::<i64, _>("run_version"))
            .map_err(|_| invalid_storage("run version"))?,
    };
    if run.namespace_id != transition.namespace_id() {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    Ok(LockedDrainRun {
        run,
        status: row.get("status"),
        stop_reason: row.get("stop_reason"),
        stop_code: row.get("stop_code"),
    })
}

fn check_revision(
    run: LockedRun,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    if run.run_version != transition.expected_run_version() {
        return Err(PrimitiveStoreError::RevisionConflict {
            current: run.run_version,
        });
    }
    Ok(())
}

fn check_audit(transition: &CollectionTransition) -> Result<(), PrimitiveStoreError> {
    if transition
        .audit()
        .is_some_and(|audit| audit.tenant_id() != transition.tenant_id())
    {
        return Err(PrimitiveStoreError::AuditTenantMismatch);
    }
    Ok(())
}

async fn append_event(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    run: LockedRun,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), PrimitiveStoreError> {
    let (nodes, indices) = path_parts(transition.owner());
    let instance_id = instance_step_id(run.version_id, transition.run_id(), transition.owner());
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(transition.owner().node().as_str())
    .bind(detail)
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .bind(instance_id)
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn append_instance_event(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    run: LockedRun,
    instance: &ExecutionInstanceId,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), PrimitiveStoreError> {
    let (nodes, indices) = path_parts(instance);
    let instance_id = instance_step_id(run.version_id, transition.run_id(), instance);
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(instance.node().as_str())
    .bind(detail)
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .bind(instance_id)
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn finish_version(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    new_version: u64,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = $4, \
                status = CASE \
                    WHEN EXISTS (SELECT 1 FROM steps \
                                 WHERE tenant_id = $1 AND run_id = $2 \
                                   AND status IN ('PENDING', 'READY', 'LEASED', 'RUNNING', 'RETRY_WAIT')) \
                        THEN 'RUNNING' \
                    WHEN EXISTS (SELECT 1 FROM steps \
                                 WHERE tenant_id = $1 AND run_id = $2 AND status = 'WAITING') \
                        THEN 'WAITING' \
                    ELSE 'RUNNING' \
                END \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(new_version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if let Some(audit) = transition.audit() {
        insert_audit(conn, audit).await?;
    }
    Ok(())
}

async fn finish_terminal_version(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    new_version: u64,
) -> Result<(), PrimitiveStoreError> {
    let updated = sqlx::query(
        "UPDATE runs SET status='SUCCEEDED',run_version=$3,updated_at=$4,finished_at=$4 \
         WHERE tenant_id=$1 AND id=$2 AND finished_at IS NULL",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(new_version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    if let Some(audit) = transition.audit() {
        insert_audit(conn, audit).await?;
    }
    Ok(())
}

async fn outbox_run_finished(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    definition: &WorkflowDefinition,
    status: &'static str,
) -> Result<(), PrimitiveStoreError> {
    let message = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished',$4)",
    )
    .bind(message)
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(json!({
        "tenantId": transition.tenant_id().to_string(),
        "namespaceId": transition.namespace_id().to_string(),
        "runId": transition.run_id().to_string(),
        "status": status,
        "traceId": transition.trace_id(),
    }))
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    let body = json!({
        "id": message.to_string(),
        "type": "run.finished",
        "time": transition
            .occurred_at()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        "data": {
            "tenantId": transition.tenant_id().to_string(),
            "namespaceId": transition.namespace_id().to_string(),
            "runId": transition.run_id().to_string(),
            "workflow": definition.name().as_str(),
            "status": status,
            "traceId": transition.trace_id(),
        },
    });
    crate::outbound::fan_out(
        conn,
        transition.tenant_id(),
        transition.namespace_id(),
        OutboundKind::RunFinished,
        &message.to_string(),
        &serde_json::to_vec(&body).unwrap_or_default(),
        transition.occurred_at(),
    )
    .await?;
    Ok(())
}

async fn mark_telemetry_pending(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    head: u64,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query(
        "INSERT INTO telemetry_pending \
         (tenant_id,run_id,head_seq,applied_seq,behind_since) \
         VALUES ($1,$2,$3,0,clock_timestamp()) \
         ON CONFLICT (tenant_id,run_id) DO UPDATE SET \
           behind_since=CASE \
             WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
             THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
           head_seq=GREATEST(telemetry_pending.head_seq,EXCLUDED.head_seq)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(head, "telemetry journal sequence")?)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn finish_drain_version(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    new_version: u64,
) -> Result<(), PrimitiveStoreError> {
    let updated = sqlx::query(
        "UPDATE runs SET run_version=$3,updated_at=$4 \
         WHERE tenant_id=$1 AND id=$2 AND status='CANCELLING'",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(new_version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    if let Some(audit) = transition.audit() {
        insert_audit(conn, audit).await?;
    }
    Ok(())
}

async fn append_run_event(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,node,detail,actor,trace_id,occurred_at, \
          instance_id,instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,$3,$4,NULL,$5,$6,$7,$8,NULL,NULL,NULL)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(detail)
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn shared_policy_fence(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(format!("primitive-policy:{}", transition.tenant_id()))
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn check_scheduler_fence(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    let authority = transition.scheduler();
    let lease = authority.lease();
    if lease.shard != Shard::of(transition.run_id().as_uuid()) {
        return Err(PrimitiveStoreError::SchedulerFenceLost);
    }
    let held: Option<i16> = sqlx::query_scalar(
        "SELECT shard FROM scheduler_shards \
         WHERE shard = $1 AND generation = $2 AND owner = $3 \
           AND expires_at > clock_timestamp() FOR SHARE",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(number(lease.generation, "scheduler generation")?)
    .bind(authority.owner())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if held.is_none() {
        return Err(PrimitiveStoreError::SchedulerFenceLost);
    }
    Ok(())
}

async fn tenant_active_limit(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<u32, PrimitiveStoreError> {
    shared_policy_fence(conn, transition).await?;
    let limit: Option<i32> = sqlx::query_scalar(
        "SELECT least(policy.max_active_iterations, coalesce(ceiling.max_value, 1024))::integer \
           FROM tenant_collection_policies policy \
           LEFT JOIN quota_ceilings ceiling ON ceiling.tenant_id = policy.tenant_id \
            AND ceiling.resource = 'iteration_concurrency' \
          WHERE policy.tenant_id = $1 FOR SHARE OF policy",
    )
    .bind(transition.tenant_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    limit
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PrimitiveStoreError::PolicyMissing)
}

#[allow(clippy::too_many_lines)]
impl CollectionStore for PgCollectionStore {
    async fn retain_collection_payload(
        &self,
        tenant_id: TenantId,
        content: &[u8],
    ) -> Result<sagat_domain::workflow::Digest, PrimitiveStoreError> {
        let digest = sagat_domain::workflow::Digest::new(Sha256::digest(content).into());
        sqlx::query(
            "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
             ON CONFLICT (tenant_id,digest) DO NOTHING",
        )
        .bind(tenant_id.as_uuid())
        .bind(digest.as_bytes().as_slice())
        .bind(content)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        let retained: Vec<u8> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id=$1 AND digest=$2")
                .bind(tenant_id.as_uuid())
                .bind(digest.as_bytes().as_slice())
                .fetch_one(&self.pool)
                .await
                .map_err(store_error)?;
        if retained != content {
            return Err(PrimitiveStoreError::DuplicateMismatch);
        }
        Ok(digest)
    }

    async fn collection_progress(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<CollectionProgress>, PrimitiveStoreError> {
        let (owner_nodes, owner_indices) = path_parts(owner);
        let Some(scope) = sqlx::query(
            "SELECT s.scope_id,s.snapshot_digest,p.content FROM iteration_scopes s JOIN payloads p \
               ON p.tenant_id=s.tenant_id AND p.digest=s.snapshot_digest \
             WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_path_nodes=$3 \
               AND s.owner_path_indices=$4 AND s.owner_node=$5",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(owner_nodes)
        .bind(owner_indices)
        .bind(owner.node().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let scope_id = CollectionScopeId::new(scope.get("scope_id"));
        let snapshot: Vec<u8> = scope.get("content");
        let snapshot_digest: Vec<u8> = scope.get("snapshot_digest");
        if Sha256::digest(&snapshot).to_vec() != snapshot_digest {
            return Err(invalid_storage("collection snapshot payload"));
        }
        let item_rows = sqlx::query(
            "SELECT i.iteration_index,i.item_digest,p.content \
             FROM collection_scope_items i JOIN payloads p \
               ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest \
             WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.scope_id=$3 \
             ORDER BY i.iteration_index",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(scope_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut items = Vec::with_capacity(item_rows.len());
        for (position, row) in item_rows.into_iter().enumerate() {
            let index = usize::try_from(row.get::<i32, _>("iteration_index"))
                .map_err(|_| invalid_storage("collection item index"))?;
            let content: Vec<u8> = row.get("content");
            let digest: Vec<u8> = row.get("item_digest");
            if index != position || Sha256::digest(&content).to_vec() != digest {
                return Err(invalid_storage("collection item payload"));
            }
            items.push(content);
        }
        let result_rows = sqlx::query(
            "SELECT c.iteration_index,c.result_digest,p.content \
             FROM iteration_checkpoints c JOIN payloads p \
               ON p.tenant_id=c.tenant_id AND p.digest=c.result_digest \
             WHERE c.tenant_id=$1 AND c.run_id=$2 AND c.scope_id=$3 \
             ORDER BY c.iteration_index",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(scope_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut results = Vec::with_capacity(result_rows.len());
        for row in result_rows {
            let content: Vec<u8> = row.get("content");
            let digest: Vec<u8> = row.get("result_digest");
            if Sha256::digest(&content).to_vec() != digest {
                return Err(invalid_storage("collection result payload"));
            }
            results.push((
                u32::try_from(row.get::<i32, _>("iteration_index"))
                    .map_err(|_| invalid_storage("collection result index"))?,
                content,
            ));
        }
        let snapshot_items = serde_json::from_slice::<Vec<Value>>(&snapshot)
            .map_err(|_| invalid_storage("collection snapshot payload"))?
            .into_iter()
            .map(|item| {
                serde_json::to_vec(&item)
                    .map_err(|_| invalid_storage("collection snapshot payload"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if snapshot_items != items {
            return Err(invalid_storage("collection membership projection"));
        }
        Ok(Some(CollectionProgress {
            scope_id,
            snapshot,
            items,
            results,
        }))
    }

    async fn enter_collection(
        &self,
        entry: &CollectionEntry,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&entry.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &entry.transition).await?;
        check_scheduler_fence(&mut tx, &entry.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(entry.transition.owner());
        if let Some(row) = sqlx::query(
            "SELECT scope_id, snapshot_digest, item_count, max_items, max_concurrency \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 \
               AND owner_path_nodes = $3 AND owner_path_indices = $4 AND owner_node = $5",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        {
            let matches = row.get::<Uuid, _>("scope_id") == entry.scope_id.as_uuid()
                && row.get::<Vec<u8>, _>("snapshot_digest")
                    == entry.snapshot_digest.as_bytes().as_slice()
                && row.get::<i32, _>("item_count") == i32::try_from(entry.item_count).unwrap_or(-1)
                && row.get::<i32, _>("max_items")
                    == i32::try_from(entry.limits.max_items()).unwrap_or(-1)
                && row.get::<i32, _>("max_concurrency")
                    == i32::from(entry.limits.max_concurrency());
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        check_revision(run, &entry.transition)?;
        tenant_active_limit(&mut tx, &entry.transition).await?;

        let snapshot_content = retain_payload_in_transaction(
            &mut tx,
            entry.transition.tenant_id(),
            entry.snapshot_digest,
            entry.snapshot_content(),
        )
        .await?;
        let snapshot: Value = serde_json::from_slice(&snapshot_content)
            .map_err(|_| PrimitiveStoreError::InvalidTransition)?;
        let Value::Array(items) = snapshot else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if items.len() != usize::try_from(entry.item_count).unwrap_or(usize::MAX) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let item_payloads = items
            .iter()
            .map(|item| {
                serde_json::to_vec(item)
                    .map(|bytes| (Sha256::digest(&bytes).to_vec(), bytes))
                    .map_err(|_| invalid_storage("collection snapshot item"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let item_digests = item_payloads
            .iter()
            .map(|(digest, _)| digest.clone())
            .collect::<Vec<_>>();
        let item_contents = item_payloads
            .into_iter()
            .map(|(_, content)| content)
            .collect::<Vec<_>>();
        if !item_digests.is_empty() {
            sqlx::query(
                "INSERT INTO payloads (tenant_id, digest, content) \
                 SELECT $1, item.digest, item.content \
                 FROM unnest($2::bytea[], $3::bytea[]) AS item(digest, content) \
                 ON CONFLICT (tenant_id, digest) DO NOTHING",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(&item_digests)
            .bind(&item_contents)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            let mismatched: bool = sqlx::query_scalar(
                "SELECT EXISTS ( \
                   SELECT 1 FROM unnest($2::bytea[], $3::bytea[]) AS item(digest, content) \
                   JOIN payloads p ON p.tenant_id = $1 AND p.digest = item.digest \
                   WHERE p.content <> item.content)",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(&item_digests)
            .bind(&item_contents)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if mismatched {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }

        let usage = sqlx::query(
            "SELECT max_instances, admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let consumed = i64::from(usage.get::<i32, _>("admitted_instances"))
            + i64::from(usage.get::<i32, _>("reserved_instances"));
        // The lifetime budget is the smaller of what the run was created with (the platform
        // ceiling lowered by the author or by tenant policy) and what the pinned definition says.
        // Every iteration will be admitted unless the scope fails first, so a collection that
        // cannot afford all of them is refused before it starts, not half way through.
        let ceiling = i64::from(usage.get::<i32, _>("max_instances"))
            .min(i64::from(entry.limits.max_instances()));
        let demand = i64::from(entry.item_count) * i64::from(entry.iteration_charge());
        // The owner is still pending and is charged when this transaction parks it, so it is the
        // first of the instances the collection needs.
        if consumed + 1 + demand > ceiling {
            return Err(PrimitiveStoreError::InstanceBudgetExceeded);
        }
        sqlx::query(
            "UPDATE run_instance_usage SET max_instances = least(max_instances, $3), \
                    max_depth = least(max_depth, $4) \
             WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(i32::try_from(entry.limits.max_instances()).unwrap_or(i32::MAX))
        .bind(i16::from(entry.limits.max_depth()))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let parked = sqlx::query(
            "UPDATE steps SET status = 'WAITING', waiting_kind = 'COLLECTION', updated_at = $6 \
             WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = $3 \
               AND instance_path_indices = $4 AND node = $5 AND status = 'PENDING'",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .bind(entry.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if parked.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        sqlx::query(
            "INSERT INTO iteration_scopes \
             (tenant_id, run_id, scope_id, namespace_id, owner_path_nodes, owner_path_indices, \
              owner_node, kind, lifecycle, snapshot_digest, item_count, max_items, max_concurrency, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'COLLECTION', 'OPEN', $8, $9, $10, $11, $12, $12)",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(entry.scope_id.as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(owner_nodes)
        .bind(owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .bind(entry.snapshot_digest.as_bytes().as_slice())
        .bind(i32::try_from(entry.item_count).unwrap_or(i32::MAX))
        .bind(i32::try_from(entry.limits.max_items()).unwrap_or(i32::MAX))
        .bind(i32::from(entry.limits.max_concurrency()))
        .bind(entry.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if !item_digests.is_empty() {
            let indices = (0..item_digests.len())
                .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO collection_scope_items \
                 (tenant_id, run_id, scope_id, iteration_index, item_digest) \
                 SELECT $1, $2, $3, item_index, item_digest \
                 FROM unnest($4::integer[], $5::bytea[]) AS item(item_index, item_digest)",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(entry.transition.run_id().as_uuid())
            .bind(entry.scope_id.as_uuid())
            .bind(indices)
            .bind(item_digests)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let new_version = run.run_version + 1;
        append_event(
            &mut tx,
            &entry.transition,
            run,
            new_version,
            "collection.entered",
            json!({
                "item_count": entry.item_count,
                "max_items": entry.limits.max_items(),
                "max_concurrency": entry.limits.max_concurrency(),
                "snapshot_digest": entry.snapshot_digest.to_hex(),
            }),
        )
        .await?;
        finish_version(&mut tx, &entry.transition, new_version).await?;
        mark_telemetry_pending(&mut tx, &entry.transition, new_version).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: new_version,
        })
    }

    async fn admit_collection_activity(
        &self,
        admission: &CollectionAdmission,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&admission.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &admission.transition).await?;
        check_scheduler_fence(&mut tx, &admission.transition).await?;
        let definition = self
            .definition(
                &mut tx,
                admission.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        let Some(owner_definition) = definition.node_for_instance(admission.transition.owner())
        else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let NodeKind::ForEach { body, .. } = owner_definition.kind() else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let Some(body) = definition.body(body) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let Some(entry_definition) = body.node(body.entrypoint()) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let Some(expected_reservations) = body.all_nodes().count().checked_sub(1) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let expected_reservations = u16::try_from(expected_reservations)
            .map_err(|_| PrimitiveStoreError::InvalidTransition)?;
        if admission.entry.node() != body.entrypoint()
            || !matches!(
                entry_definition.kind(),
                NodeKind::Activity { activity, .. } if activity == &admission.activity
            )
            || admission.reserved_instances != expected_reservations
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let (owner_nodes, owner_indices) = path_parts(admission.transition.owner());
        let stored_owner = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(stored_owner.get("namespace_id")) != run.namespace_id
            || stored_owner.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || stored_owner.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || stored_owner.get::<String, _>("owner_node")
                != admission.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if let Some(row) = sqlx::query(
            "SELECT entry_instance_id, item_digest, activity, reserved_instances, outbox_id \
             FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2 \
               AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        {
            let expected_id = instance_step_id(
                run.version_id,
                admission.transition.run_id(),
                &admission.entry,
            );
            let matches = row.get::<Uuid, _>("entry_instance_id") == expected_id
                && row.get::<Vec<u8>, _>("item_digest")
                    == admission.item_digest.as_bytes().as_slice()
                && row.get::<String, _>("activity") == admission.activity.as_str()
                && row.get::<i32, _>("reserved_instances")
                    == i32::from(admission.reserved_instances)
                && row.get::<Uuid, _>("outbox_id") == admission.outbox_id;
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        check_revision(run, &admission.transition)?;
        let active_limit = tenant_active_limit(&mut tx, &admission.transition).await?;
        let active: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if u32::try_from(active).unwrap_or(u32::MAX) >= active_limit {
            return Err(PrimitiveStoreError::AtCapacity);
        }
        let usage = sqlx::query(
            "SELECT max_instances, admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let charge = 1_i64 + i64::from(admission.reserved_instances);
        let consumed = i64::from(usage.get::<i32, _>("admitted_instances"))
            + i64::from(usage.get::<i32, _>("reserved_instances"));
        if consumed + charge > i64::from(usage.get::<i32, _>("max_instances")) {
            return Err(PrimitiveStoreError::InstanceBudgetExceeded);
        }
        let scope = sqlx::query(
            "SELECT lifecycle, item_count, next_admission_index, active_count, max_concurrency \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("next_admission_index")
                != i32::try_from(admission.index).unwrap_or(i32::MAX)
            || admission.index >= u32::try_from(scope.get::<i32, _>("item_count")).unwrap_or(0)
            || scope.get::<i32, _>("active_count") >= scope.get::<i32, _>("max_concurrency")
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let expected_item: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT item_digest FROM collection_scope_items \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if expected_item.as_deref() != Some(admission.item_digest.as_bytes().as_slice()) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        retain_payload_in_transaction(
            &mut tx,
            admission.transition.tenant_id(),
            admission.item_digest,
            admission.item_content(),
        )
        .await?;
        let (entry_nodes, entry_indices) = path_parts(&admission.entry);
        let entry_id = instance_step_id(
            run.version_id,
            admission.transition.run_id(),
            &admission.entry,
        );
        let admitted_sequence = run.run_version + 1;
        let released_sequence = admitted_sequence + 1;
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
              last_hinted_at, hint_count, updated_at, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, $4, $5, 'READY', $6, $7, $7, 1, $7, $8, $9)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(run.namespace_id.as_uuid())
        .bind(admission.activity.as_str())
        .bind(admission.transition.occurred_at())
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO collection_iterations \
             (tenant_id, run_id, scope_id, iteration_index, entry_path_nodes, entry_path_indices, \
              entry_node, entry_instance_id, activity, item_digest, lifecycle, slot_held, \
              reserved_instances, remaining_reservations, admitted_sequence, outbox_id, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'ADMITTED', true, $11, $11, \
                     $12, $13, $14, $14)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(admission.activity.as_str())
        .bind(admission.item_digest.as_bytes().as_slice())
        .bind(i32::from(admission.reserved_instances))
        .bind(number(admitted_sequence, "journal sequence")?)
        .bind(admission.outbox_id)
        .bind(admission.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET next_admission_index = next_admission_index + 1, \
                    admitted_count = admitted_count + 1, active_count = active_count + 1, updated_at = $4 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(admission.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances = admitted_instances + 1, \
                    reserved_instances = reserved_instances + $3 \
             WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(i32::from(admission.reserved_instances))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations = active_iterations + 1 WHERE tenant_id = $1",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &admission.transition,
            run,
            admitted_sequence,
            "collection.iteration_admitted",
            json!({
                "index": admission.index,
                "entry": instance_json(&admission.entry),
                "item_digest": admission.item_digest.to_hex(),
            }),
        )
        .await?;
        append_instance_event(
            &mut tx,
            &admission.transition,
            run,
            &admission.entry,
            released_sequence,
            "step.released",
            json!({}),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4, $5)",
        )
        .bind(admission.outbox_id)
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.activity.as_str())
        .bind(json!({
            "tenantId": admission.transition.tenant_id().to_string(),
            "namespaceId": admission.transition.namespace_id().to_string(),
            "runId": admission.transition.run_id().to_string(),
            "node": admission.entry.node().as_str(),
            "instancePath": instance_json(&admission.entry)["path"],
            "activity": admission.activity.as_str(),
            "traceId": admission.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_version(&mut tx, &admission.transition, released_sequence).await?;
        mark_telemetry_pending(&mut tx, &admission.transition, released_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: released_sequence,
        })
    }

    async fn checkpoint_collection_iteration(
        &self,
        checkpoint: &CollectionCheckpoint,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&checkpoint.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &checkpoint.transition).await?;
        check_scheduler_fence(&mut tx, &checkpoint.transition).await?;

        let _: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage \
             WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let usage = sqlx::query(
            "SELECT admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;

        let (owner_nodes, owner_indices) = path_parts(checkpoint.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, lifecycle, \
                    active_count \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
             FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != checkpoint.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let iteration = sqlx::query(
            "SELECT lifecycle, slot_held, remaining_reservations, entry_path_nodes, \
                    entry_path_indices, return_path_nodes, return_path_indices, return_node, \
                    return_instance_id, result_digest, checkpoint_sequence \
             FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2 \
               AND scope_id = $3 AND iteration_index = $4 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;

        let definition = self
            .definition(
                &mut tx,
                checkpoint.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        if !matches!(
            definition
                .node_for_instance(checkpoint.transition.owner())
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::ForEach { .. })
        ) || !matches!(
            definition
                .node_for_instance(&checkpoint.return_instance)
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::Return { .. })
        ) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let return_id = instance_step_id(
            run.version_id,
            checkpoint.transition.run_id(),
            &checkpoint.return_instance,
        );
        let (return_nodes, return_indices) = path_parts(&checkpoint.return_instance);
        if iteration.get::<String, _>("lifecycle") == "CHECKPOINTED" {
            let saved = sqlx::query(
                "SELECT return_instance_id, result_digest, outbox_id FROM iteration_checkpoints \
                 WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
            )
            .bind(checkpoint.transition.tenant_id().as_uuid())
            .bind(checkpoint.transition.run_id().as_uuid())
            .bind(checkpoint.scope_id.as_uuid())
            .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            .ok_or_else(|| invalid_storage("checkpoint projection"))?;
            let matches = saved.get::<Uuid, _>("return_instance_id") == return_id
                && saved.get::<Vec<u8>, _>("result_digest")
                    == checkpoint.result_digest.as_bytes().as_slice()
                && saved.get::<Uuid, _>("outbox_id") == checkpoint.outbox_id
                && iteration.get::<Option<Vec<String>>, _>("return_path_nodes")
                    == Some(return_nodes)
                && iteration.get::<Option<Vec<i32>>, _>("return_path_indices")
                    == Some(return_indices)
                && iteration.get::<Option<String>, _>("return_node")
                    == Some(checkpoint.return_instance.node().as_str().to_owned());
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }

        check_revision(run, &checkpoint.transition)?;
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("active_count") <= 0
            || iteration.get::<String, _>("lifecycle") != "ADMITTED"
            || !iteration.get::<bool, _>("slot_held")
            || iteration.get::<i32, _>("remaining_reservations") <= 0
            || usage.get::<i32, _>("reserved_instances") <= 0
            || iteration.get::<Vec<String>, _>("entry_path_nodes") != return_nodes
            || iteration.get::<Vec<i32>, _>("entry_path_indices") != return_indices
            || iteration
                .get::<Option<i64>, _>("checkpoint_sequence")
                .is_some()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        retain_payload_in_transaction(
            &mut tx,
            checkpoint.transition.tenant_id(),
            checkpoint.result_digest,
            checkpoint.result_content(),
        )
        .await?;

        let member_steps = sqlx::query(
            "SELECT node, step_id, status, routed, attempt, generation, lease_token_hash, \
                    waiting_kind \
             FROM steps WHERE tenant_id = $1 AND run_id = $2 \
               AND instance_path_nodes = $3 AND instance_path_indices = $4 \
             ORDER BY node FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut found_return = false;
        for member in member_steps {
            let node: String = member.get("node");
            if node == checkpoint.return_instance.node().as_str() {
                let valid_return = !found_return
                    && member.get::<Uuid, _>("step_id") == return_id
                    && member.get::<String, _>("status") == "PENDING"
                    && !member.get::<bool, _>("routed")
                    && member.get::<i32, _>("attempt") == 0
                    && member.get::<i64, _>("generation") == 0
                    && member
                        .get::<Option<Vec<u8>>, _>("lease_token_hash")
                        .is_none()
                    && member.get::<Option<String>, _>("waiting_kind").is_none();
                if !valid_return {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
                found_return = true;
            } else if !matches!(
                member.get::<String, _>("status").as_str(),
                "SUCCEEDED" | "SKIPPED"
            ) || !member.get::<bool, _>("routed")
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }
        if !found_return {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let completed = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', routed = true, updated_at = $8 \
             WHERE tenant_id = $1 AND run_id = $2 AND namespace_id = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5 AND node = $6 \
               AND step_id = $7 AND status = 'PENDING' AND attempt = 0 AND generation = 0 \
               AND NOT routed AND lease_token_hash IS NULL AND waiting_kind IS NULL",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .bind(checkpoint.return_instance.node().as_str())
        .bind(return_id)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if completed.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let returned_sequence = run.run_version + 1;
        let checkpoint_sequence = returned_sequence + 1;
        sqlx::query(
            "UPDATE collection_iterations SET lifecycle = 'CHECKPOINTED', slot_held = false, \
                    remaining_reservations = remaining_reservations - 1, \
                    return_path_nodes = $5, return_path_indices = $6, return_node = $7, \
                    return_instance_id = $8, result_digest = $9, checkpoint_sequence = $10, \
                    updated_at = $11 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .bind(&return_nodes)
        .bind(&return_indices)
        .bind(checkpoint.return_instance.node().as_str())
        .bind(return_id)
        .bind(checkpoint.result_digest.as_bytes().as_slice())
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO iteration_checkpoints \
             (tenant_id, run_id, scope_id, iteration_index, return_instance_id, result_digest, \
              journal_sequence, outbox_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .bind(return_id)
        .bind(checkpoint.result_digest.as_bytes().as_slice())
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(checkpoint.outbox_id)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET completed_count = completed_count + 1, \
                    active_count = active_count - 1, updated_at = $4 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let tenant_released = sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations = active_iterations - 1 \
             WHERE tenant_id = $1 AND active_iterations > 0",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let reservation_consumed = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances = admitted_instances + 1, \
                    reserved_instances = reserved_instances - 1 \
             WHERE tenant_id = $1 AND run_id = $2 AND reserved_instances > 0",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if tenant_released.rows_affected() != 1 || reservation_consumed.rows_affected() != 1 {
            return Err(invalid_storage("collection usage projection"));
        }

        append_event(
            &mut tx,
            &checkpoint.transition,
            run,
            returned_sequence,
            "collection.iteration_returned",
            json!({
                "index": checkpoint.index,
                "return_instance": instance_json(&checkpoint.return_instance),
            }),
        )
        .await?;
        append_event(
            &mut tx,
            &checkpoint.transition,
            run,
            checkpoint_sequence,
            "collection.iteration_checkpointed",
            json!({
                "index": checkpoint.index,
                "return_instance": instance_json(&checkpoint.return_instance),
                "result_digest": checkpoint.result_digest.to_hex(),
            }),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox \
             (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'collection.iteration.checkpointed', $4, $5)",
        )
        .bind(checkpoint.outbox_id)
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.transition.owner().node().as_str())
        .bind(json!({
            "tenantId": checkpoint.transition.tenant_id().to_string(),
            "namespaceId": checkpoint.transition.namespace_id().to_string(),
            "runId": checkpoint.transition.run_id().to_string(),
            "scopeId": checkpoint.scope_id.as_uuid().to_string(),
            "index": checkpoint.index,
            "traceId": checkpoint.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_version(&mut tx, &checkpoint.transition, checkpoint_sequence).await?;
        mark_telemetry_pending(&mut tx, &checkpoint.transition, checkpoint_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: checkpoint_sequence,
        })
    }

    async fn start_collection_drain(
        &self,
        drain: &CollectionDrain,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&drain.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let locked = lock_drain_run(&mut tx, &drain.transition).await?;
        check_scheduler_fence(&mut tx, &drain.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(drain.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id,owner_path_nodes,owner_path_indices,owner_node,lifecycle, \
                    drain_kind,drain_iteration,drain_failure_code,drain_sequence,drain_outbox_id \
             FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != locked.run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != drain.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let storage_quota_code = FailureCode::storage_quota_exceeded();
        let (kind, iteration, code) = match &drain.reason {
            CollectionDrainReason::Failed { iteration, code } => (
                "FAILED",
                Some(
                    i32::try_from(*iteration)
                        .map_err(|_| PrimitiveStoreError::InvalidTransition)?,
                ),
                Some(code.as_str()),
            ),
            CollectionDrainReason::StorageQuotaExceeded => {
                ("STORAGE_QUOTA", None, Some(storage_quota_code.as_str()))
            }
            CollectionDrainReason::Cancelled => ("CANCELLED", None, None),
        };
        if matches!(
            scope.get::<String, _>("lifecycle").as_str(),
            "DRAINING" | "FAILED" | "CANCELLED"
        ) {
            let matches = scope.get::<Option<String>, _>("drain_kind").as_deref() == Some(kind)
                && scope.get::<Option<i32>, _>("drain_iteration") == iteration
                && scope
                    .get::<Option<String>, _>("drain_failure_code")
                    .as_deref()
                    == code
                && scope.get::<Option<Uuid>, _>("drain_outbox_id") == Some(drain.outbox_id);
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            let sequence = scope
                .get::<Option<i64>, _>("drain_sequence")
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| invalid_storage("collection drain sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: sequence,
            });
        }
        if scope.get::<String, _>("lifecycle") != "OPEN" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if matches!(drain.reason, CollectionDrainReason::StorageQuotaExceeded)
            && !drain.transition.audit().is_some_and(|audit| {
                audit.action() == AuditAction::RunStorageQuotaExceeded
                    && audit.resource_type() == ResourceType::Run
                    && audit.resource_id() == drain.transition.run_id().to_string()
                    && audit.actor() == drain.transition.actor()
                    && audit.actor() == &Subject::engine()
            })
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        check_revision(locked.run, &drain.transition)?;

        let mut sequence = locked.run.run_version;
        match &drain.reason {
            CollectionDrainReason::Cancelled => {
                if locked.status != "CANCELLING"
                    || locked.stop_reason.as_deref() != Some("CANCELLED")
                    || locked.stop_code.is_some()
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
            CollectionDrainReason::Failed { iteration, code } => {
                let failed: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM collection_iterations i JOIN steps s \
                       ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id \
                      AND s.instance_path_nodes=i.entry_path_nodes \
                      AND s.instance_path_indices=i.entry_path_indices \
                     WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.scope_id=$3 \
                       AND i.iteration_index=$4 AND i.lifecycle='ADMITTED' \
                       AND s.status='FAILED' AND s.failure_code=$5)",
                )
                .bind(drain.transition.tenant_id().as_uuid())
                .bind(drain.transition.run_id().as_uuid())
                .bind(drain.scope_id.as_uuid())
                .bind(i32::try_from(*iteration).unwrap_or(i32::MAX))
                .bind(code.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                if !failed {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
                match (
                    locked.status.as_str(),
                    locked.stop_reason.as_deref(),
                    locked.stop_code.as_deref(),
                ) {
                    ("RUNNING" | "WAITING", None, None) => {
                        sequence += 1;
                        append_run_event(
                            &mut tx,
                            &drain.transition,
                            sequence,
                            "run.stop_requested",
                            json!({"reason":"FAILED","code":code.as_str()}),
                        )
                        .await?;
                        sqlx::query(
                            "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3 \
                             WHERE tenant_id=$1 AND id=$2",
                        )
                        .bind(drain.transition.tenant_id().as_uuid())
                        .bind(drain.transition.run_id().as_uuid())
                        .bind(code.as_str())
                        .execute(&mut *tx)
                        .await
                        .map_err(store_error)?;
                    }
                    ("CANCELLING", Some("FAILED"), Some(stored)) if stored == code.as_str() => {}
                    _ => return Err(PrimitiveStoreError::InvalidTransition),
                }
            }
            CollectionDrainReason::StorageQuotaExceeded => match (
                locked.status.as_str(),
                locked.stop_reason.as_deref(),
                locked.stop_code.as_deref(),
            ) {
                ("RUNNING" | "WAITING", None, None) => {
                    sequence += 1;
                    append_run_event(
                        &mut tx,
                        &drain.transition,
                        sequence,
                        "run.stop_requested",
                        json!({
                            "reason":"FAILED",
                            "code":storage_quota_code.as_str()
                        }),
                    )
                    .await?;
                    sqlx::query(
                        "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3 \
                         WHERE tenant_id=$1 AND id=$2",
                    )
                    .bind(drain.transition.tenant_id().as_uuid())
                    .bind(drain.transition.run_id().as_uuid())
                    .bind(storage_quota_code.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                }
                ("CANCELLING", Some("FAILED"), Some(stored))
                    if stored == storage_quota_code.as_str() => {}
                _ => return Err(PrimitiveStoreError::InvalidTransition),
            },
        }
        let cancellable = sqlx::query(
            "SELECT s.node,s.instance_path_nodes,s.instance_path_indices \
             FROM collection_iterations i JOIN steps s \
               ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id \
              AND s.instance_path_nodes=i.entry_path_nodes \
              AND s.instance_path_indices=i.entry_path_indices \
             WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.scope_id=$3 \
               AND i.lifecycle='ADMITTED' AND s.status IN ('LEASED','RUNNING') \
               AND s.cancel_requested=false \
             ORDER BY s.instance_path_nodes,s.instance_path_indices,s.node FOR UPDATE OF s",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let cancellable = cancellable
            .into_iter()
            .map(|row| {
                stored_instance(
                    row.get("instance_path_nodes"),
                    row.get("instance_path_indices"),
                    row.get("node"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let final_sequence = sequence
            .checked_add(1)
            .and_then(|value| {
                u64::try_from(cancellable.len())
                    .ok()
                    .and_then(|count| value.checked_add(count))
            })
            .ok_or_else(|| invalid_storage("journal sequence"))?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| invalid_storage("journal sequence"))?;
        sqlx::query(
            "UPDATE iteration_scopes SET lifecycle='DRAINING',drain_kind=$4,drain_iteration=$5, \
                    drain_failure_code=$6,drain_sequence=$7,drain_outbox_id=$8,updated_at=$9 \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .bind(kind)
        .bind(iteration)
        .bind(code)
        .bind(number(final_sequence, "journal sequence")?)
        .bind(drain.outbox_id)
        .bind(drain.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let detail = match &drain.reason {
            CollectionDrainReason::Failed { iteration, code } => {
                json!({"reason":"failed","iteration":iteration,"code":code.as_str()})
            }
            CollectionDrainReason::StorageQuotaExceeded => {
                json!({"reason":"storage_quota_exceeded"})
            }
            CollectionDrainReason::Cancelled => json!({"reason":"cancelled"}),
        };
        append_event(
            &mut tx,
            &drain.transition,
            locked.run,
            sequence,
            "collection.drain_started",
            detail,
        )
        .await?;
        for instance in &cancellable {
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| invalid_storage("journal sequence"))?;
            append_instance_event(
                &mut tx,
                &drain.transition,
                locked.run,
                instance,
                sequence,
                "step.cancel_requested",
                json!({}),
            )
            .await?;
            let (path_nodes, path_indices) = path_parts(instance);
            let updated = sqlx::query(
                "UPDATE steps SET cancel_requested=true,updated_at=$6 \
                 WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
                   AND instance_path_indices=$4 AND node=$5 \
                   AND status IN ('LEASED','RUNNING') AND cancel_requested=false",
            )
            .bind(drain.transition.tenant_id().as_uuid())
            .bind(drain.transition.run_id().as_uuid())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(instance.node().as_str())
            .bind(drain.transition.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if updated.rows_affected() != 1 {
                return Err(PrimitiveStoreError::RevisionConflict {
                    current: locked.run.run_version,
                });
            }
        }
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'collection.drain.started',$4,$5)",
        )
        .bind(drain.outbox_id)
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.transition.owner().node().as_str())
        .bind(json!({
            "tenantId":drain.transition.tenant_id().to_string(),
            "namespaceId":drain.transition.namespace_id().to_string(),
            "runId":drain.transition.run_id().to_string(),
            "scopeId":drain.scope_id.as_uuid().to_string(),
            "reason":kind,
            "traceId":drain.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        debug_assert_eq!(sequence, final_sequence);
        finish_drain_version(&mut tx, &drain.transition, final_sequence).await?;
        mark_telemetry_pending(&mut tx, &drain.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: final_sequence,
        })
    }

    async fn drain_collection_iteration(
        &self,
        drain: &CollectionIterationDrain,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&drain.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let locked = lock_drain_run(&mut tx, &drain.transition).await?;
        check_scheduler_fence(&mut tx, &drain.transition).await?;
        let _: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1 FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let _: i32 = sqlx::query_scalar(
            "SELECT reserved_instances FROM run_instance_usage \
             WHERE tenant_id=$1 AND run_id=$2 FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let (owner_nodes, owner_indices) = path_parts(drain.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id,owner_path_nodes,owner_path_indices,owner_node,lifecycle, \
                    active_count,drain_kind,drain_iteration,drain_failure_code \
             FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != locked.run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != drain.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let iteration = sqlx::query(
            "SELECT lifecycle,slot_held,remaining_reservations,entry_path_nodes,entry_path_indices, \
                    entry_node,drained_sequence,drain_outbox_id \
             FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 \
               AND iteration_index=$4 FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .bind(i32::try_from(drain.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if iteration.get::<String, _>("lifecycle") == "DRAINED" {
            if iteration.get::<Option<Uuid>, _>("drain_outbox_id") != Some(drain.outbox_id) {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            let sequence = iteration
                .get::<Option<i64>, _>("drained_sequence")
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| invalid_storage("collection iteration drain sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: sequence,
            });
        }
        if scope.get::<String, _>("lifecycle") != "DRAINING" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if locked.status != "CANCELLING" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        check_revision(locked.run, &drain.transition)?;
        if iteration.get::<String, _>("lifecycle") != "ADMITTED"
            || !iteration.get::<bool, _>("slot_held")
            || scope.get::<i32, _>("active_count") <= 0
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let path_nodes = iteration.get::<Vec<String>, _>("entry_path_nodes");
        let path_indices = iteration.get::<Vec<i32>, _>("entry_path_indices");
        let entry_node = iteration.get::<String, _>("entry_node");
        let remaining = iteration.get::<i32, _>("remaining_reservations");
        let member_steps = sqlx::query(
            "SELECT node,status,attempt FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND instance_path_nodes=$3 AND instance_path_indices=$4 ORDER BY node FOR UPDATE",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        if member_steps.is_empty() {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        for step in &member_steps {
            let node = NodeId::parse_stored(&step.get::<String, _>("node"))
                .map_err(|_| invalid_storage("collection member node"))?;
            let status = step.get::<String, _>("status");
            let terminal = matches!(
                status.as_str(),
                "SUCCEEDED" | "FAILED" | "CANCELLED" | "SKIPPED"
            );
            let safe_unstarted = matches!(status.as_str(), "PENDING" | "READY")
                && step.get::<i32, _>("attempt") == 0;
            if (node.is_derived() && !matches!(status.as_str(), "SUCCEEDED" | "SKIPPED"))
                || (!terminal && !safe_unstarted)
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }
        let failure_entry = scope.get::<Option<String>, _>("drain_kind").as_deref()
            == Some("FAILED")
            && scope.get::<Option<i32>, _>("drain_iteration")
                == Some(i32::try_from(drain.index).unwrap_or(i32::MAX));
        let failure_code = scope.get::<Option<String>, _>("drain_failure_code");
        sqlx::query(
            "UPDATE steps SET status=CASE WHEN $5 AND node=$6 THEN 'FAILED' ELSE 'CANCELLED' END, \
                    failure_code=CASE WHEN $5 AND node=$6 THEN $7 ELSE failure_code END, \
                    routed=true,waiting_kind=NULL,due_at=NULL,event_type=NULL,correlation_key=NULL, \
                    next_attempt_at=NULL,cancel_requested=false,updated_at=$8 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND status IN ('PENDING','READY') AND attempt=0",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(failure_entry)
        .bind(&entry_node)
        .bind(failure_code.as_deref())
        .bind(drain.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let sequence = locked.run.run_version + 1;
        sqlx::query(
            "UPDATE collection_iterations SET lifecycle='DRAINED',slot_held=false, \
                    remaining_reservations=0,drained_sequence=$5,drain_outbox_id=$6,updated_at=$7 \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=$4",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .bind(i32::try_from(drain.index).unwrap_or(i32::MAX))
        .bind(number(sequence, "journal sequence")?)
        .bind(drain.outbox_id)
        .bind(drain.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET completed_count=completed_count+1,active_count=active_count-1, \
                    updated_at=$4 WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .bind(drain.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let tenant_updated = sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations=active_iterations-1 \
             WHERE tenant_id=$1 AND active_iterations>0",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let usage_updated = sqlx::query(
            "UPDATE run_instance_usage SET reserved_instances=reserved_instances-$3 \
             WHERE tenant_id=$1 AND run_id=$2 AND reserved_instances >= $3",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(remaining)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if tenant_updated.rows_affected() != 1 || usage_updated.rows_affected() != 1 {
            return Err(invalid_storage("collection drain usage projection"));
        }
        append_event(
            &mut tx,
            &drain.transition,
            locked.run,
            sequence,
            "collection.iteration_drained",
            json!({"index":drain.index}),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'collection.iteration.drained',$4,$5)",
        )
        .bind(drain.outbox_id)
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.transition.owner().node().as_str())
        .bind(json!({
            "tenantId":drain.transition.tenant_id().to_string(),
            "namespaceId":drain.transition.namespace_id().to_string(),
            "runId":drain.transition.run_id().to_string(),
            "scopeId":drain.scope_id.as_uuid().to_string(),
            "index":drain.index,
            "traceId":drain.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_drain_version(&mut tx, &drain.transition, sequence).await?;
        mark_telemetry_pending(&mut tx, &drain.transition, sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: sequence,
        })
    }

    async fn finish_collection_drain(
        &self,
        finish: &CollectionDrainFinish,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&finish.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let locked = lock_drain_run(&mut tx, &finish.transition).await?;
        check_scheduler_fence(&mut tx, &finish.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(finish.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id,owner_path_nodes,owner_path_indices,owner_node,lifecycle, \
                    active_count,drain_kind,drain_failure_code,finish_sequence,finish_outbox_id \
             FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != locked.run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != finish.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let lifecycle = scope.get::<String, _>("lifecycle");
        if matches!(lifecycle.as_str(), "FAILED" | "CANCELLED") {
            if scope.get::<Option<Uuid>, _>("finish_outbox_id") != Some(finish.outbox_id) {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            let sequence = scope
                .get::<Option<i64>, _>("finish_sequence")
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| invalid_storage("collection finish sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: sequence + 1,
            });
        }
        if locked.status != "CANCELLING" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        check_revision(locked.run, &finish.transition)?;
        if lifecycle != "DRAINING" || scope.get::<i32, _>("active_count") != 0 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let unsettled: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM collection_iterations \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND lifecycle='ADMITTED')",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let other_active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND NOT (instance_path_nodes=$3 AND instance_path_indices=$4 AND node=$5) \
               AND status NOT IN ('SUCCEEDED','FAILED','CANCELLED','SKIPPED'))",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(finish.transition.owner().node().as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if unsettled || other_active {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let kind = scope
            .get::<Option<String>, _>("drain_kind")
            .ok_or_else(|| invalid_storage("collection drain reason"))?;
        let code = scope.get::<Option<String>, _>("drain_failure_code");
        let (outcome, terminal_status, expected_reason) = match (kind.as_str(), code.as_deref()) {
            ("FAILED", Some(code)) => (json!({"outcome":"failed","code":code}), "FAILED", "FAILED"),
            ("STORAGE_QUOTA", Some(code))
                if code == FailureCode::storage_quota_exceeded().as_str() =>
            {
                (json!({"outcome":"failed","code":code}), "FAILED", "FAILED")
            }
            ("CANCELLED", None) => (json!({"outcome":"cancelled"}), "CANCELLED", "CANCELLED"),
            _ => return Err(invalid_storage("collection drain reason")),
        };
        if locked.stop_reason.as_deref() != Some(expected_reason)
            || (expected_reason == "FAILED" && locked.stop_code.as_deref() != code.as_deref())
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let owner = sqlx::query(
            "UPDATE steps SET status=$6,waiting_kind=NULL,failure_code=$7,updated_at=$8 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND node=$5 \
               AND status='WAITING' AND waiting_kind='COLLECTION'",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(finish.transition.owner().node().as_str())
        .bind(terminal_status)
        .bind(code.as_deref())
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if owner.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let finish_sequence = locked.run.run_version + 1;
        let final_sequence = finish_sequence + 1;
        sqlx::query(
            "UPDATE iteration_scopes SET lifecycle=$4,finish_sequence=$5,finish_outbox_id=$6, \
                    updated_at=$7 WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .bind(terminal_status)
        .bind(number(finish_sequence, "journal sequence")?)
        .bind(finish.outbox_id)
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &finish.transition,
            locked.run,
            finish_sequence,
            "collection.finished",
            outcome,
        )
        .await?;
        append_run_event(
            &mut tx,
            &finish.transition,
            final_sequence,
            "run.finished",
            json!({"status":terminal_status}),
        )
        .await?;
        let run_finished = sqlx::query(
            "UPDATE runs SET status=$3,run_version=$4,updated_at=$5,finished_at=$5 \
             WHERE tenant_id=$1 AND id=$2 AND status='CANCELLING'",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(terminal_status)
        .bind(number(final_sequence, "run version")?)
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if run_finished.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if let Some(audit) = finish.transition.audit() {
            insert_audit(&mut tx, audit).await?;
        }
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'collection.finished',$4,$5)",
        )
        .bind(finish.outbox_id)
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.transition.owner().node().as_str())
        .bind(json!({
            "tenantId":finish.transition.tenant_id().to_string(),
            "namespaceId":finish.transition.namespace_id().to_string(),
            "runId":finish.transition.run_id().to_string(),
            "scopeId":finish.scope_id.as_uuid().to_string(),
            "status":terminal_status,
            "traceId":finish.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let definition = self
            .definition(
                &mut tx,
                finish.transition.tenant_id().as_uuid(),
                locked.run.version_id,
            )
            .await?;
        outbox_run_finished(&mut tx, &finish.transition, &definition, terminal_status).await?;
        mark_telemetry_pending(&mut tx, &finish.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: final_sequence,
        })
    }

    async fn finish_collection(
        &self,
        finish: &CollectionFinish,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&finish.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let locked_run = lock_drain_run(&mut tx, &finish.transition).await?;
        check_scheduler_fence(&mut tx, &finish.transition).await?;
        if !matches!(
            locked_run.status.as_str(),
            "RUNNING" | "WAITING" | "SUCCEEDED"
        ) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let run = locked_run.run;
        let _: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage \
             WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let _: i32 = sqlx::query_scalar(
            "SELECT admitted_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;

        let (owner_nodes, owner_indices) = path_parts(finish.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, lifecycle, \
                    item_count, next_admission_index, admitted_count, completed_count, \
                    active_count, result_digest, finish_sequence, finish_outbox_id \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
             FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != finish.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let definition = self
            .definition(
                &mut tx,
                finish.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        if scope.get::<String, _>("lifecycle") == "SUCCEEDED" {
            let matches = scope.get::<Option<Vec<u8>>, _>("result_digest")
                == Some(finish.result_digest.as_bytes().to_vec())
                && scope.get::<Option<Uuid>, _>("finish_outbox_id") == Some(finish.outbox_id)
                && scope.get::<Option<i64>, _>("finish_sequence").is_some();
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        if locked_run.status == "SUCCEEDED" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        check_revision(run, &finish.transition)?;
        let item_count = scope.get::<i32, _>("item_count");
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("next_admission_index") != item_count
            || scope.get::<i32, _>("admitted_count") != item_count
            || scope.get::<i32, _>("completed_count") != item_count
            || scope.get::<i32, _>("active_count") != 0
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let unsettled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM collection_iterations \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
               AND (lifecycle <> 'CHECKPOINTED' OR slot_held OR checkpoint_sequence IS NULL)",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let checkpoint_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM iteration_checkpoints \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if unsettled != 0 || checkpoint_count != i64::from(item_count) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let aggregate_content = retain_payload_in_transaction(
            &mut tx,
            finish.transition.tenant_id(),
            finish.result_digest,
            finish.result_content(),
        )
        .await?;
        let result_rows = sqlx::query(
            "SELECT c.iteration_index, c.result_digest, p.content FROM iteration_checkpoints c \
             JOIN payloads p ON p.tenant_id = c.tenant_id AND p.digest = c.result_digest \
             WHERE c.tenant_id = $1 AND c.run_id = $2 AND c.scope_id = $3 \
             ORDER BY c.iteration_index FOR UPDATE OF c",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let ordered_results = result_rows
            .iter()
            .map(|row| {
                let digest = row.get::<Vec<u8>, _>("result_digest");
                let content = row.get::<Vec<u8>, _>("content");
                if Sha256::digest(&content).as_slice() != digest {
                    return Err(invalid_storage("checkpoint result payload digest"));
                }
                serde_json::from_slice(&content)
                    .map_err(|_| invalid_storage("checkpoint result payload"))
            })
            .collect::<Result<Vec<Value>, PrimitiveStoreError>>()?;
        let aggregate: Value = serde_json::from_slice(&aggregate_content)
            .map_err(|_| invalid_storage("collection aggregate payload"))?;
        let Some(owner_definition) = definition.node_for_instance(finish.transition.owner()) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let NodeKind::ForEach { result, .. } = owner_definition.kind() else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if aggregate.as_array().map(Vec::len) != Some(result_rows.len()) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if let Some(sources) = finish.source_digests() {
            if sources.len() != result_rows.len() {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            for ((index, digest), row) in sources.iter().zip(&result_rows) {
                let stored_index = u32::try_from(row.get::<i32, _>("iteration_index"))
                    .map_err(|_| invalid_storage("collection checkpoint index"))?;
                let stored_digest = row.get::<Vec<u8>, _>("result_digest");
                if *index != stored_index
                    || digest.as_bytes().as_slice() != stored_digest.as_slice()
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
        } else if result.as_str() != "bodyResult" {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if result.as_str() == "bodyResult" && aggregate != Value::Array(ordered_results) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let successor_node = owner_definition
            .next()
            .cloned()
            .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?;
        let successor =
            ExecutionInstanceId::in_path(finish.transition.owner().path().clone(), successor_node);
        let (successor_activity, successor_is_end) = match definition
            .node_for_instance(&successor)
            .map(sagat_domain::workflow::Node::kind)
        {
            Some(NodeKind::Activity { activity, .. }) => (Some(activity.clone()), false),
            Some(NodeKind::ForEach { .. }) => (None, false),
            Some(NodeKind::End { .. }) => (None, true),
            Some(_) => return Err(PrimitiveStoreError::RuntimeNotIntegrated),
            None => return Err(PrimitiveStoreError::InvalidTransition),
        };
        let owner_finished = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', waiting_kind = NULL, routed = true, \
                    updated_at = $6 \
             WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = $3 \
               AND instance_path_indices = $4 AND node = $5 \
               AND status = 'WAITING' AND waiting_kind = 'COLLECTION' AND NOT routed",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(finish.transition.owner().node().as_str())
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if owner_finished.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let finish_sequence = run.run_version + 1;
        let routed_sequence = finish_sequence + 1;
        let final_sequence = if successor_is_end {
            routed_sequence + 2
        } else if successor_activity.is_some() {
            routed_sequence + 1
        } else {
            routed_sequence
        };
        let (successor_nodes, successor_indices) = path_parts(&successor);
        if let Some(activity) = &successor_activity {
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
                  last_hinted_at, hint_count, updated_at, \
                  instance_path_nodes, instance_path_indices) \
                 VALUES ($1, $2, $3, $4, $5, 'READY', $6, $7, $7, 1, $7, $8, $9)",
            )
            .bind(finish.transition.tenant_id().as_uuid())
            .bind(finish.transition.run_id().as_uuid())
            .bind(successor.node().as_str())
            .bind(instance_step_id(
                run.version_id,
                finish.transition.run_id(),
                &successor,
            ))
            .bind(run.namespace_id.as_uuid())
            .bind(activity.as_str())
            .bind(finish.transition.occurred_at())
            .bind(&successor_nodes)
            .bind(&successor_indices)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        } else if successor_is_end {
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id,run_id,node,step_id,namespace_id,status,routed,updated_at, \
                  instance_path_nodes,instance_path_indices) \
                 VALUES ($1,$2,$3,$4,$5,'SUCCEEDED',true,$6,$7,$8)",
            )
            .bind(finish.transition.tenant_id().as_uuid())
            .bind(finish.transition.run_id().as_uuid())
            .bind(successor.node().as_str())
            .bind(instance_step_id(
                run.version_id,
                finish.transition.run_id(),
                &successor,
            ))
            .bind(run.namespace_id.as_uuid())
            .bind(finish.transition.occurred_at())
            .bind(&successor_nodes)
            .bind(&successor_indices)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        } else {
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
                  instance_path_nodes, instance_path_indices) \
                 VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, $7, $8)",
            )
            .bind(finish.transition.tenant_id().as_uuid())
            .bind(finish.transition.run_id().as_uuid())
            .bind(successor.node().as_str())
            .bind(instance_step_id(
                run.version_id,
                finish.transition.run_id(),
                &successor,
            ))
            .bind(run.namespace_id.as_uuid())
            .bind(finish.transition.occurred_at())
            .bind(&successor_nodes)
            .bind(&successor_indices)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        sqlx::query(
            "UPDATE iteration_scopes SET lifecycle = 'SUCCEEDED', result_digest = $4, \
                    finish_sequence = $5, finish_outbox_id = $6, updated_at = $7 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .bind(finish.result_digest.as_bytes().as_slice())
        .bind(number(finish_sequence, "journal sequence")?)
        .bind(finish.outbox_id)
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &finish.transition,
            run,
            finish_sequence,
            "collection.finished",
            json!({
                "outcome": "succeeded",
                "result_digest": finish.result_digest.to_hex(),
            }),
        )
        .await?;
        append_event(
            &mut tx,
            &finish.transition,
            run,
            routed_sequence,
            "step.routed",
            json!({ "activated": [successor.node().as_str()] }),
        )
        .await?;
        if let Some(activity) = &successor_activity {
            append_instance_event(
                &mut tx,
                &finish.transition,
                run,
                &successor,
                final_sequence,
                "step.released",
                json!({}),
            )
            .await?;
            sqlx::query(
                "INSERT INTO event_outbox \
                 (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
                 VALUES ($1, $2, $3, 'task.ready', $4, $5)",
            )
            .bind(Uuid::now_v7())
            .bind(finish.transition.tenant_id().as_uuid())
            .bind(finish.transition.run_id().as_uuid())
            .bind(activity.as_str())
            .bind(json!({
                "tenantId": finish.transition.tenant_id().to_string(),
                "namespaceId": finish.transition.namespace_id().to_string(),
                "runId": finish.transition.run_id().to_string(),
                "node": successor.node().as_str(),
                "instancePath": instance_json(&successor)["path"],
                "activity": activity.as_str(),
                "traceId": finish.transition.trace_id(),
            }))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        } else if successor_is_end {
            append_instance_event(
                &mut tx,
                &finish.transition,
                run,
                &successor,
                routed_sequence + 1,
                "end.reached",
                json!({}),
            )
            .await?;
            append_run_event(
                &mut tx,
                &finish.transition,
                routed_sequence + 2,
                "run.finished",
                json!({ "status": "SUCCEEDED" }),
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO event_outbox \
             (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'collection.finished', $4, $5)",
        )
        .bind(finish.outbox_id)
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.transition.owner().node().as_str())
        .bind(json!({
            "tenantId": finish.transition.tenant_id().to_string(),
            "namespaceId": finish.transition.namespace_id().to_string(),
            "runId": finish.transition.run_id().to_string(),
            "scopeId": finish.scope_id.as_uuid().to_string(),
            "status": "SUCCEEDED",
            "traceId": finish.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if successor_is_end {
            outbox_run_finished(&mut tx, &finish.transition, &definition, "SUCCEEDED").await?;
            finish_terminal_version(&mut tx, &finish.transition, final_sequence).await?;
        } else {
            finish_version(&mut tx, &finish.transition, final_sequence).await?;
        }
        mark_telemetry_pending(&mut tx, &finish.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: final_sequence,
        })
    }
}

impl CollectionStore for PgRunStore {
    async fn retain_collection_payload(
        &self,
        tenant_id: TenantId,
        content: &[u8],
    ) -> Result<sagat_domain::workflow::Digest, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .retain_collection_payload(tenant_id, content)
            .await
    }

    async fn collection_progress(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<CollectionProgress>, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .collection_progress(tenant_id, run_id, owner)
            .await
    }

    async fn enter_collection(
        &self,
        entry: &CollectionEntry,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .enter_collection(entry)
            .await
    }

    async fn admit_collection_activity(
        &self,
        admission: &CollectionAdmission,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .admit_collection_activity(admission)
            .await
    }

    async fn checkpoint_collection_iteration(
        &self,
        checkpoint: &CollectionCheckpoint,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .checkpoint_collection_iteration(checkpoint)
            .await
    }

    async fn start_collection_drain(
        &self,
        drain: &CollectionDrain,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .start_collection_drain(drain)
            .await
    }

    async fn drain_collection_iteration(
        &self,
        drain: &CollectionIterationDrain,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .drain_collection_iteration(drain)
            .await
    }

    async fn finish_collection_drain(
        &self,
        finish: &CollectionDrainFinish,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .finish_collection_drain(finish)
            .await
    }

    async fn finish_collection(
        &self,
        finish: &CollectionFinish,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .finish_collection(finish)
            .await
    }
}
