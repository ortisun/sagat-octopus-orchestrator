//! PostgreSQL transaction adapter for sequential-loop checkpoints and durable delays.
//!
//! Entry, checkpoints, elapsed delays, terminal routing and drain atomically update journal,
//! projection, retained payloads, budgets, timers, audit and outbox. Public workflow publication
//! remains closed until every scheduler failure path has the same durable evidence.

use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::loop_progress::{
    LoopCandidate, LoopCheckpointCommand, LoopDelayElapsedCommand, LoopDrainCommand,
    LoopDrainFinishCommand, LoopEntryCommand, LoopIterationDrainCommand, LoopNextActivity,
    LoopProgressError, LoopProgressStore, LoopTransition, LoopWrite,
};
use sagat_application::maintenance::{StepRef, Window};
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ExecutionInstanceId, FailureCode, InstancePath, LoopContinuation, LoopKind, RunId,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::outbound::OutboundKind;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{ActivityName, NodeId, NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Row};

use crate::PgRunStore;
use crate::identity::{insert_audit, store_error};
use crate::pool::BeginSafely;

#[derive(Debug, Clone)]
pub struct PgLoopProgressStore {
    pool: PgPool,
    definitions: DefinitionCache,
}

impl PgLoopProgressStore {
    #[must_use]
    pub fn new(pool: PgPool, definitions: DefinitionCache) -> Self {
        Self { pool, definitions }
    }

    async fn definition(
        &self,
        conn: &mut PgConnection,
        tenant_id: uuid::Uuid,
        version_id: WorkflowVersionId,
    ) -> Result<std::sync::Arc<WorkflowDefinition>, LoopProgressError> {
        if let Some(definition) = self.definitions.get(version_id) {
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
        .ok_or(LoopProgressError::Missing)?;
        self.definitions
            .get_or_parse(version_id, &document)
            .map_err(|error| {
                tracing::error!(%error, %version_id, "stored loop workflow no longer parses");
                invalid_storage("stored loop workflow definition")
            })
    }
}

#[derive(Debug, Clone)]
struct LockedRun {
    namespace_id: NamespaceId,
    version_id: WorkflowVersionId,
    run_version: u64,
    active: bool,
    status: String,
    stop_reason: Option<String>,
    stop_code: Option<String>,
}

fn invalid_storage(detail: &'static str) -> LoopProgressError {
    LoopProgressError::Storage(StoreError::Internal {
        detail: detail.to_owned(),
    })
}

fn check_audit(transition: &LoopTransition) -> Result<(), LoopProgressError> {
    let audit = transition.audit();
    if audit.tenant_id() != transition.tenant_id()
        || audit.actor() != transition.actor()
        || audit.action() != AuditAction::RunUpdated
        || audit.resource_type() != ResourceType::Run
        || audit.resource_id() != transition.run_id().to_string()
    {
        return Err(LoopProgressError::AuditTenantMismatch);
    }
    Ok(())
}

fn number(value: u64, field: &'static str) -> Result<i64, LoopProgressError> {
    i64::try_from(value).map_err(|_| {
        LoopProgressError::Storage(StoreError::Internal {
            detail: format!("{field} does not fit PostgreSQL bigint"),
        })
    })
}

fn micros(value: time::OffsetDateTime) -> Result<i64, LoopProgressError> {
    i64::try_from(value.unix_timestamp_nanos() / 1_000)
        .map_err(|_| invalid_storage("loop timestamp"))
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
) -> Result<ExecutionInstanceId, LoopProgressError> {
    if path_nodes.len() != path_indices.len() {
        return Err(invalid_storage("loop member path"));
    }
    let mut path = InstancePath::root();
    for (scope, index) in path_nodes.into_iter().zip(path_indices) {
        let scope =
            NodeId::parse_stored(&scope).map_err(|_| invalid_storage("loop member scope"))?;
        let index = u32::try_from(index).map_err(|_| invalid_storage("loop member iteration"))?;
        path = path
            .child(scope, index)
            .map_err(|_| invalid_storage("loop member path"))?;
    }
    let node = NodeId::parse_stored(&node).map_err(|_| invalid_storage("loop member node"))?;
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

async fn lock_run(
    conn: &mut PgConnection,
    transition: &LoopTransition,
) -> Result<LockedRun, LoopProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, run_version, status,stop_reason,stop_code \
         FROM runs \
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(LoopProgressError::Missing)?;
    let namespace_id = NamespaceId::new(row.get("namespace_id"));
    let status: String = row.get("status");
    let stored_version: i64 = row.get("run_version");
    let run_version = u64::try_from(stored_version).map_err(|_| invalid_storage("run version"))?;
    if namespace_id != transition.namespace_id() {
        return Err(LoopProgressError::InvalidTransition);
    }
    Ok(LockedRun {
        namespace_id,
        version_id: WorkflowVersionId::new(row.get("workflow_version_id")),
        run_version,
        active: matches!(status.as_str(), "RUNNING" | "WAITING"),
        status,
        stop_reason: row.get("stop_reason"),
        stop_code: row.get("stop_code"),
    })
}

fn check_run_active(run: &LockedRun) -> Result<(), LoopProgressError> {
    if run.active {
        Ok(())
    } else {
        Err(LoopProgressError::InvalidTransition)
    }
}

fn check_revision(run: &LockedRun, transition: &LoopTransition) -> Result<(), LoopProgressError> {
    if run.run_version != transition.expected_run_version() {
        return Err(LoopProgressError::RevisionConflict {
            current: run.run_version,
        });
    }
    Ok(())
}

async fn check_scheduler_fence(
    conn: &mut PgConnection,
    transition: &LoopTransition,
) -> Result<(), LoopProgressError> {
    let authority = transition.scheduler();
    let lease = authority.lease();
    if lease.shard != Shard::of(transition.run_id().as_uuid()) {
        return Err(LoopProgressError::SchedulerFenceLost);
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
        return Err(LoopProgressError::SchedulerFenceLost);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct LockedUsage {
    policy_configured: bool,
    tenant_active: i32,
    tenant_limit: i32,
    admitted: i32,
    reserved: i32,
    max_instances: i32,
}

async fn lock_usage(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    requires_policy: bool,
) -> Result<LockedUsage, LoopProgressError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(format!("primitive-policy:{}", transition.tenant_id()))
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    let tenant_limit: Option<i32> = sqlx::query_scalar(
        "SELECT least(policy.max_active_iterations::bigint, \
                      coalesce(ceiling.max_value, 1024))::integer \
         FROM tenant_collection_policies policy \
         LEFT JOIN quota_ceilings ceiling \
           ON ceiling.tenant_id=policy.tenant_id AND ceiling.resource='iteration_concurrency' \
         WHERE policy.tenant_id=$1 FOR SHARE OF policy",
    )
    .bind(transition.tenant_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if requires_policy && tenant_limit.is_none() {
        return Err(LoopProgressError::PolicyMissing);
    }
    let policy_configured = tenant_limit.is_some();
    let tenant_limit = tenant_limit.unwrap_or(i32::MAX);
    let tenant_active: Option<i32> = sqlx::query_scalar(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let tenant_active = tenant_active.ok_or_else(|| invalid_storage("tenant iteration usage"))?;
    let run = sqlx::query(
        "SELECT admitted_instances, reserved_instances, max_instances \
         FROM run_instance_usage WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or_else(|| invalid_storage("run instance usage"))?;
    Ok(LockedUsage {
        policy_configured,
        tenant_active,
        tenant_limit,
        admitted: run.get("admitted_instances"),
        reserved: run.get("reserved_instances"),
        max_instances: run.get("max_instances"),
    })
}

async fn append_event(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    run: &LockedRun,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), LoopProgressError> {
    let (nodes, indices) = path_parts(transition.owner());
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
    .bind(instance_step_id(
        run.version_id,
        transition.run_id(),
        transition.owner(),
    ))
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn append_instance_event(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    run: &LockedRun,
    instance: &ExecutionInstanceId,
    sequence: u64,
    event_type: &str,
) -> Result<(), LoopProgressError> {
    let (nodes, indices) = path_parts(instance);
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, $6, $7, $8, $9, $10, $11)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(instance.node().as_str())
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .bind(instance_step_id(
        run.version_id,
        transition.run_id(),
        instance,
    ))
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn retain_payload_in_transaction(
    conn: &mut PgConnection,
    tenant_id: uuid::Uuid,
    digest: sagat_domain::workflow::Digest,
    content: Option<&[u8]>,
) -> Result<(), LoopProgressError> {
    if let Some(content) = content {
        if !payload_matches(content, digest) {
            return Err(LoopProgressError::InvalidTransition);
        }
        sqlx::query(
            "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
             ON CONFLICT (tenant_id,digest) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(digest.as_bytes().as_slice())
        .bind(content)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    }
    let retained: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant_id)
            .bind(digest.as_bytes().as_slice())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    if retained
        .as_deref()
        .is_none_or(|bytes| !payload_matches(bytes, digest))
    {
        return Err(LoopProgressError::InvalidTransition);
    }
    if content.is_some_and(|bytes| retained.as_deref() != Some(bytes)) {
        return Err(LoopProgressError::DuplicateMismatch);
    }
    Ok(())
}

fn continuation_parts(
    continuation: LoopContinuation,
) -> (&'static str, Option<time::OffsetDateTime>) {
    match continuation {
        LoopContinuation::Complete => ("COMPLETE", None),
        LoopContinuation::Continue => ("CONTINUE", None),
        LoopContinuation::DelayUntil(due_at) => ("DELAY", Some(due_at)),
        LoopContinuation::IterationLimit => ("ITERATION_LIMIT", None),
    }
}

fn continuation_json(continuation: LoopContinuation) -> Result<Value, LoopProgressError> {
    Ok(match continuation {
        LoopContinuation::Complete => json!({"kind": "complete"}),
        LoopContinuation::Continue => json!({"kind": "continue"}),
        LoopContinuation::DelayUntil(due_at) => {
            json!({"kind": "delay", "at_us": micros(due_at)?})
        }
        LoopContinuation::IterationLimit => json!({"kind": "iteration_limit"}),
    })
}

async fn insert_ready_activity(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    run: &LockedRun,
    index: u32,
    next: &LoopNextActivity,
    admitted_sequence: u64,
) -> Result<(), LoopProgressError> {
    let (nodes, indices) = path_parts(&next.instance);
    let step_id = instance_step_id(run.version_id, transition.run_id(), &next.instance);
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
          last_hinted_at, hint_count, updated_at, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'READY', $6, $7, $7, 1, $7, $8, $9)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(next.instance.node().as_str())
    .bind(step_id)
    .bind(run.namespace_id.as_uuid())
    .bind(next.activity.as_str())
    .bind(transition.occurred_at())
    .bind(&nodes)
    .bind(&indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    sqlx::query(
        "INSERT INTO loop_iterations \
         (tenant_id, run_id, owner_instance_id, iteration_index, entry_instance_id, \
          entry_path_nodes, entry_path_indices, entry_node, activity, lifecycle, slot_held, \
          reserved_instances, remaining_reservations, admitted_sequence, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'ADMITTED', true, $10, $10, $11, $12, $12)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(instance_step_id(
        run.version_id,
        transition.run_id(),
        transition.owner(),
    ))
    .bind(i32::try_from(index).unwrap_or(i32::MAX))
    .bind(step_id)
    .bind(nodes)
    .bind(indices)
    .bind(next.instance.node().as_str())
    .bind(next.activity.as_str())
    .bind(i32::from(next.reserved_instances))
    .bind(number(admitted_sequence, "journal sequence")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_task_outbox(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    next: &LoopNextActivity,
    outbox_id: uuid::Uuid,
) -> Result<(), LoopProgressError> {
    sqlx::query(
        "INSERT INTO event_outbox \
         (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
         VALUES ($1, $2, $3, 'task.ready', $4, $5)",
    )
    .bind(outbox_id)
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(next.activity.as_str())
    .bind(json!({
        "tenantId": transition.tenant_id().to_string(),
        "namespaceId": transition.namespace_id().to_string(),
        "runId": transition.run_id().to_string(),
        "node": next.instance.node().as_str(),
        "instancePath": instance_json(&next.instance)["path"],
        "activity": next.activity.as_str(),
        "traceId": transition.trace_id(),
    }))
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct LoopSuccessor {
    instance: ExecutionInstanceId,
    activity: Option<ActivityName>,
    is_end: bool,
}

fn terminal_successor(
    definition: &WorkflowDefinition,
    owner: &ExecutionInstanceId,
    failed: bool,
) -> Result<Option<LoopSuccessor>, LoopProgressError> {
    let declared = definition
        .node_for_instance(owner)
        .ok_or(LoopProgressError::InvalidTransition)?;
    if !matches!(
        declared.kind(),
        NodeKind::While { .. } | NodeKind::RepeatUntil { .. }
    ) {
        return Err(LoopProgressError::InvalidTransition);
    }
    let target = if failed {
        declared.on_error()
    } else {
        declared.next()
    };
    let Some(target) = target.cloned() else {
        return if failed {
            Ok(None)
        } else {
            Err(LoopProgressError::InvalidTransition)
        };
    };
    let instance = ExecutionInstanceId::in_path(owner.path().clone(), target);
    let (activity, is_end) = match definition
        .node_for_instance(&instance)
        .map(sagat_domain::workflow::Node::kind)
    {
        Some(NodeKind::Activity { activity, .. }) => (Some(activity.clone()), false),
        Some(NodeKind::End { .. }) => (None, true),
        Some(NodeKind::Return { .. }) | None => {
            return Err(LoopProgressError::InvalidTransition);
        }
        Some(_) => (None, false),
    };
    Ok(Some(LoopSuccessor {
        instance,
        activity,
        is_end,
    }))
}

async fn insert_loop_hint(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    outbox_id: uuid::Uuid,
    event_type: &'static str,
    status: &'static str,
) -> Result<(), LoopProgressError> {
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(outbox_id)
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(event_type)
    .bind(json!({
        "tenantId": transition.tenant_id().to_string(),
        "namespaceId": transition.namespace_id().to_string(),
        "runId": transition.run_id().to_string(),
        "node": transition.owner().node().as_str(),
        "status": status,
        "traceId": transition.trace_id(),
    }))
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn append_run_event(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    sequence: u64,
    event_type: &'static str,
    detail: Value,
) -> Result<(), LoopProgressError> {
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

async fn route_loop_successor(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    run: &LockedRun,
    successor: &LoopSuccessor,
    routed_sequence: u64,
    outbox_id: uuid::Uuid,
) -> Result<(u64, bool), LoopProgressError> {
    let (nodes, indices) = path_parts(&successor.instance);
    let step_id = instance_step_id(run.version_id, transition.run_id(), &successor.instance);
    if let Some(activity) = &successor.activity {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,activity,ready_at,last_hinted_at, \
              hint_count,updated_at,instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'READY',$6,$7,$7,1,$7,$8,$9)",
        )
        .bind(transition.tenant_id().as_uuid())
        .bind(transition.run_id().as_uuid())
        .bind(successor.instance.node().as_str())
        .bind(step_id)
        .bind(run.namespace_id.as_uuid())
        .bind(activity.as_str())
        .bind(transition.occurred_at())
        .bind(&nodes)
        .bind(&indices)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    } else if successor.is_end {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,routed,updated_at, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'SUCCEEDED',true,$6,$7,$8)",
        )
        .bind(transition.tenant_id().as_uuid())
        .bind(transition.run_id().as_uuid())
        .bind(successor.instance.node().as_str())
        .bind(step_id)
        .bind(run.namespace_id.as_uuid())
        .bind(transition.occurred_at())
        .bind(&nodes)
        .bind(&indices)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    } else {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'PENDING',$6,$7,$8)",
        )
        .bind(transition.tenant_id().as_uuid())
        .bind(transition.run_id().as_uuid())
        .bind(successor.instance.node().as_str())
        .bind(step_id)
        .bind(run.namespace_id.as_uuid())
        .bind(transition.occurred_at())
        .bind(&nodes)
        .bind(&indices)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    }
    append_event(
        conn,
        transition,
        run,
        routed_sequence,
        "step.routed",
        json!({"activated": [successor.instance.node().as_str()]}),
    )
    .await?;
    if successor.activity.is_some() {
        append_instance_event(
            conn,
            transition,
            run,
            &successor.instance,
            routed_sequence + 1,
            "step.released",
        )
        .await?;
        let next = LoopNextActivity {
            instance: successor.instance.clone(),
            activity: successor
                .activity
                .clone()
                .ok_or(LoopProgressError::InvalidTransition)?,
            reserved_instances: 1,
        };
        insert_task_outbox(conn, transition, &next, outbox_id).await?;
        Ok((routed_sequence + 1, false))
    } else if successor.is_end {
        append_instance_event(
            conn,
            transition,
            run,
            &successor.instance,
            routed_sequence + 1,
            "end.reached",
        )
        .await?;
        append_run_event(
            conn,
            transition,
            routed_sequence + 2,
            "run.finished",
            json!({"status":"SUCCEEDED"}),
        )
        .await?;
        insert_loop_hint(conn, transition, outbox_id, "run.finished", "SUCCEEDED").await?;
        Ok((routed_sequence + 2, true))
    } else {
        insert_loop_hint(conn, transition, outbox_id, "loop.finished", "SUCCEEDED").await?;
        Ok((routed_sequence, false))
    }
}

async fn finish_version(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    version: u64,
) -> Result<(), LoopProgressError> {
    sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = $4, \
                status = CASE \
                    WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 \
                                 AND status IN ('PENDING','READY','LEASED','RUNNING','RETRY_WAIT')) \
                        THEN 'RUNNING' \
                    WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 \
                                 AND status = 'WAITING') THEN 'WAITING' \
                    ELSE 'RUNNING' END \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    insert_audit(conn, transition.audit()).await?;
    Ok(())
}

async fn finish_succeeded_version(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    version: u64,
) -> Result<(), LoopProgressError> {
    let updated = sqlx::query(
        "UPDATE runs SET status='SUCCEEDED',run_version=$3,updated_at=$4,finished_at=$4 \
         WHERE tenant_id=$1 AND id=$2 AND finished_at IS NULL",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(LoopProgressError::InvalidTransition);
    }
    insert_audit(conn, transition.audit()).await?;
    Ok(())
}

async fn finish_iteration_limit(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    version: u64,
) -> Result<(), LoopProgressError> {
    let failure = FailureCode::iteration_limit();
    let updated = sqlx::query(
        "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3, \
                run_version=$4,updated_at=$5 \
         WHERE tenant_id=$1 AND id=$2 AND status IN ('RUNNING','WAITING') \
           AND stop_reason IS NULL AND stop_code IS NULL AND finished_at IS NULL",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(failure.as_str())
    .bind(number(version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(LoopProgressError::InvalidTransition);
    }
    insert_audit(conn, transition.audit()).await?;
    Ok(())
}

async fn finish_drain_version(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    version: u64,
) -> Result<(), LoopProgressError> {
    let updated = sqlx::query(
        "UPDATE runs SET run_version=$3,updated_at=$4 \
         WHERE tenant_id=$1 AND id=$2 AND status='CANCELLING'",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(LoopProgressError::InvalidTransition);
    }
    insert_audit(conn, transition.audit()).await?;
    Ok(())
}

async fn outbox_run_finished(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    definition: &WorkflowDefinition,
    status: &'static str,
) -> Result<(), LoopProgressError> {
    let message = uuid::Uuid::now_v7();
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
    .await
    .map_err(LoopProgressError::Storage)?;
    Ok(())
}

async fn mark_telemetry_pending(
    conn: &mut PgConnection,
    transition: &LoopTransition,
    head: u64,
) -> Result<(), LoopProgressError> {
    sqlx::query(
        "INSERT INTO telemetry_pending (tenant_id,run_id,head_seq,applied_seq,behind_since) \
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

fn validate_definition(
    definition: &WorkflowDefinition,
    transition: &LoopTransition,
    returned: Option<&ExecutionInstanceId>,
    next: Option<&LoopNextActivity>,
) -> Result<(), LoopProgressError> {
    if !matches!(
        definition
            .node_for_instance(transition.owner())
            .map(sagat_domain::workflow::Node::kind),
        Some(NodeKind::While { .. } | NodeKind::RepeatUntil { .. })
    ) {
        return Err(LoopProgressError::InvalidTransition);
    }
    if returned.is_some_and(|instance| {
        !matches!(
            definition
                .node_for_instance(instance)
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::Return { .. })
        )
    }) {
        return Err(LoopProgressError::InvalidTransition);
    }
    if next.is_some_and(|next| {
        !matches!(
            definition
                .node_for_instance(&next.instance)
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::Activity { activity, .. }) if activity == &next.activity
        )
    }) {
        return Err(LoopProgressError::InvalidTransition);
    }
    Ok(())
}

fn loop_kind_parts(kind: LoopKind) -> (&'static str, &'static str) {
    match kind {
        LoopKind::While => ("WHILE", "while"),
        LoopKind::RepeatUntil => ("REPEAT_UNTIL", "repeatUntil"),
    }
}

fn validate_entry_definition(
    definition: &WorkflowDefinition,
    command: &LoopEntryCommand,
    first: Option<&LoopNextActivity>,
) -> Result<(), LoopProgressError> {
    let owner = definition
        .node_for_instance(command.transition.owner())
        .ok_or(LoopProgressError::InvalidTransition)?;
    let (kind, body, max_iterations) = match owner.kind() {
        NodeKind::While {
            body,
            max_iterations,
            ..
        } => (LoopKind::While, body, *max_iterations),
        NodeKind::RepeatUntil {
            body,
            max_iterations,
            ..
        } => (LoopKind::RepeatUntil, body, *max_iterations),
        _ => return Err(LoopProgressError::InvalidTransition),
    };
    let Some(timeout) = owner.timeout() else {
        return Err(LoopProgressError::InvalidTransition);
    };
    if kind != command.kind
        || max_iterations != command.max_iterations
        || command.transition.occurred_at() + timeout != command.deadline
    {
        return Err(LoopProgressError::InvalidTransition);
    }
    if let Some(first) = first {
        let body = definition
            .body(body)
            .ok_or(LoopProgressError::InvalidTransition)?;
        let entry = body
            .node(body.entrypoint())
            .ok_or(LoopProgressError::InvalidTransition)?;
        let expected_reservations = body
            .all_nodes()
            .count()
            .checked_sub(1)
            .and_then(|count| u16::try_from(count).ok())
            .ok_or(LoopProgressError::InvalidTransition)?;
        let expected_path = command
            .transition
            .owner()
            .path()
            .child(command.transition.owner().node().clone(), 0)
            .map_err(|_| LoopProgressError::InvalidTransition)?;
        if first.instance.path() != &expected_path
            || first.instance.node() != body.entrypoint()
            || first.reserved_instances != expected_reservations
            || !matches!(
                entry.kind(),
                NodeKind::Activity { activity, .. } if activity == &first.activity
            )
        {
            return Err(LoopProgressError::InvalidTransition);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
impl LoopProgressStore for PgLoopProgressStore {
    async fn loop_candidates(
        &self,
        window: &Window,
    ) -> Result<Vec<LoopCandidate>, LoopProgressError> {
        let limit = i64::from(window.limit);
        let shards = window.shards.indexes();
        let excluded = window
            .exclude
            .iter()
            .map(|tenant| tenant.as_uuid())
            .collect::<Vec<_>>();
        let rows = sqlx::query(
            "SELECT candidate.tenant_id,candidate.run_id,candidate.node, \
                    candidate.instance_path_nodes,candidate.instance_path_indices, \
                    candidate.timer_id \
             FROM unnest($2::smallint[]) AS shard(value) \
             CROSS JOIN LATERAL ( \
               SELECT work.tenant_id,work.run_id,work.node,work.instance_path_nodes, \
                      work.instance_path_indices,work.timer_id,work.updated_at \
               FROM ( \
                 SELECT step.tenant_id,step.run_id,step.node,step.instance_path_nodes, \
                        step.instance_path_indices,NULL::uuid AS timer_id,step.updated_at \
                   FROM steps step JOIN runs run \
                     ON run.tenant_id=step.tenant_id AND run.id=step.run_id \
                   JOIN workflow_versions version \
                     ON version.tenant_id=run.tenant_id AND version.id=run.workflow_version_id \
                  WHERE run_shard(step.run_id)=shard.value AND run.status='RUNNING' \
                    AND step.status='PENDING' AND cardinality(step.instance_path_nodes)=0 \
                    AND step.tenant_id <> ALL($3::uuid[]) \
                    AND EXISTS ( \
                      SELECT 1 FROM jsonb_array_elements( \
                        (convert_from(version.document,'UTF8')::jsonb)->'nodes') declared \
                      WHERE declared->>'id'=step.node \
                        AND declared->>'type' IN ('while','repeatUntil')) \
                 UNION ALL \
                 SELECT scope.tenant_id,scope.run_id,scope.owner_node, \
                        scope.owner_path_nodes,scope.owner_path_indices,timer.timer_id, \
                        scope.updated_at \
                   FROM loop_scopes scope \
                   JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
                   JOIN workflow_versions version \
                     ON version.tenant_id=run.tenant_id AND version.id=run.workflow_version_id \
                   CROSS JOIN LATERAL ( \
                     SELECT min(declared->>'body') AS body_name \
                       FROM jsonb_array_elements( \
                         (convert_from(version.document,'UTF8')::jsonb)->'nodes') declared \
                      WHERE declared->>'id'=scope.owner_node \
                        AND declared->>'type' IN ('while','repeatUntil') \
                     HAVING count(*)=1) owner_definition \
                   CROSS JOIN LATERAL ( \
                     SELECT min(declared->>'id') AS return_node \
                       FROM jsonb_array_elements( \
                         (convert_from(version.document,'UTF8')::jsonb)->'bodies' \
                           ->owner_definition.body_name->'nodes') declared \
                      WHERE declared->>'type'='return' \
                     HAVING count(*)=1) return_definition \
                   LEFT JOIN loop_timers timer \
                     ON timer.tenant_id=scope.tenant_id AND timer.run_id=scope.run_id \
                    AND timer.owner_instance_id=scope.owner_instance_id \
                    AND timer.lifecycle='WAITING' \
                  WHERE run_shard(scope.run_id)=shard.value \
                    AND run.status IN ('RUNNING','WAITING') \
                    AND scope.tenant_id <> ALL($3::uuid[]) \
                    AND ( \
                      (scope.lifecycle='RUNNING' AND EXISTS ( \
                        SELECT 1 FROM loop_iterations member JOIN steps returned \
                          ON returned.tenant_id=member.tenant_id \
                         AND returned.run_id=member.run_id \
                         AND returned.instance_path_nodes=member.entry_path_nodes \
                         AND returned.instance_path_indices=member.entry_path_indices \
                         AND returned.node=return_definition.return_node \
                         WHERE member.tenant_id=scope.tenant_id \
                           AND member.run_id=scope.run_id \
                           AND member.owner_instance_id=scope.owner_instance_id \
                           AND member.lifecycle='ADMITTED' AND returned.status='PENDING')) \
                      OR (scope.lifecycle='DELAYING' AND timer.due_at <= clock_timestamp())) \
               ) work ORDER BY work.updated_at LIMIT $1 \
             ) candidate ORDER BY candidate.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(excluded)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                let nodes = row.get::<Vec<String>, _>("instance_path_nodes");
                let indices = row.get::<Vec<i32>, _>("instance_path_indices");
                if nodes.len() != indices.len() {
                    return Err(invalid_storage("loop candidate path"));
                }
                let mut path = InstancePath::root();
                for (node, index) in nodes.into_iter().zip(indices) {
                    path = path
                        .child(
                            NodeId::parse_stored(&node)
                                .map_err(|_| invalid_storage("loop candidate path"))?,
                            u32::try_from(index)
                                .map_err(|_| invalid_storage("loop candidate path"))?,
                        )
                        .map_err(|_| invalid_storage("loop candidate path"))?;
                }
                let node = row.get::<String, _>("node");
                Ok(LoopCandidate {
                    step: StepRef {
                        tenant_id: TenantId::new(row.get("tenant_id")),
                        run_id: RunId::new(row.get("run_id")),
                        node: NodeId::parse_stored(&node)
                            .map_err(|_| invalid_storage("loop candidate node"))?,
                        instance_path: path,
                    },
                    timer_id: row.get("timer_id"),
                })
            })
            .collect()
    }

    async fn enter_loop(&self, command: &LoopEntryCommand) -> Result<LoopWrite, LoopProgressError> {
        let valid_phase = matches!(
            (command.kind, command.initial_condition),
            (LoopKind::While, Some(_)) | (LoopKind::RepeatUntil, None)
        );
        let needs_body =
            command.kind == LoopKind::RepeatUntil || command.initial_condition == Some(true);
        if !valid_phase
            || command.max_iterations == 0
            || command.max_iterations > 10_000
            || needs_body != command.first_activity.is_some()
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        check_audit(&command.transition)?;

        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let usage = lock_usage(&mut tx, &command.transition, false).await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let (owner_nodes, owner_indices) = path_parts(command.transition.owner());
        let (kind_db, kind_event) = loop_kind_parts(command.kind);
        let definition = self
            .definition(
                &mut tx,
                command.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        validate_entry_definition(&definition, command, command.first_activity.as_ref())?;
        let terminal = if needs_body {
            None
        } else {
            terminal_successor(&definition, command.transition.owner(), false)?
        };

        if let Some(scope) = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, kind, \
                    lifecycle, max_iterations, deadline_at, initial_carry_digest, initial_condition, \
                    entered_sequence, entry_outbox_id \
             FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
             FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        {
            let common_matches = scope.get::<uuid::Uuid, _>("namespace_id")
                == command.transition.namespace_id().as_uuid()
                && scope.get::<Vec<String>, _>("owner_path_nodes") == owner_nodes
                && scope.get::<Vec<i32>, _>("owner_path_indices") == owner_indices
                && scope.get::<String, _>("owner_node")
                    == command.transition.owner().node().as_str()
                && scope.get::<String, _>("kind") == kind_db
                && scope.get::<i32, _>("max_iterations")
                    == i32::try_from(command.max_iterations).unwrap_or(i32::MAX)
                && scope.get::<time::OffsetDateTime, _>("deadline_at") == command.deadline
                && scope.get::<Vec<u8>, _>("initial_carry_digest")
                    == command.initial_carry_digest.as_bytes().as_slice()
                && scope.get::<Option<bool>, _>("initial_condition") == command.initial_condition
                && scope.get::<Option<uuid::Uuid>, _>("entry_outbox_id")
                    == Some(command.outbox_id);
            if !common_matches {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let entered = u64::try_from(scope.get::<i64, _>("entered_sequence"))
                .map_err(|_| invalid_storage("loop entered sequence"))?;
            let committed = if let Some(first) = command.first_activity.as_ref() {
                let iteration = sqlx::query(
                    "SELECT entry_instance_id, entry_path_nodes, entry_path_indices, entry_node, \
                            activity, reserved_instances, admitted_sequence \
                     FROM loop_iterations WHERE tenant_id=$1 AND run_id=$2 \
                       AND owner_instance_id=$3 AND iteration_index=0",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(owner_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?
                .ok_or_else(|| invalid_storage("first loop iteration projection"))?;
                let (entry_nodes, entry_indices) = path_parts(&first.instance);
                let entry_id =
                    instance_step_id(run.version_id, command.transition.run_id(), &first.instance);
                if scope.get::<String, _>("lifecycle") != "RUNNING"
                    || iteration.get::<uuid::Uuid, _>("entry_instance_id") != entry_id
                    || iteration.get::<Vec<String>, _>("entry_path_nodes") != entry_nodes
                    || iteration.get::<Vec<i32>, _>("entry_path_indices") != entry_indices
                    || iteration.get::<String, _>("entry_node")
                        != first.instance.node().as_str()
                    || iteration.get::<String, _>("activity") != first.activity.as_str()
                    || iteration.get::<i32, _>("reserved_instances")
                        != i32::from(first.reserved_instances)
                {
                    return Err(LoopProgressError::DuplicateMismatch);
                }
                let admitted = u64::try_from(iteration.get::<i64, _>("admitted_sequence"))
                    .map_err(|_| invalid_storage("loop admitted sequence"))?;
                entered
                    .checked_add(2)
                    .filter(|_| admitted == entered + 1)
                    .ok_or_else(|| invalid_storage("loop entry journal sequence"))?
            } else {
                let successor = terminal
                    .as_ref()
                    .ok_or(LoopProgressError::InvalidTransition)?;
                let expected_status = if successor.is_end {
                    "SUCCEEDED"
                } else if successor.activity.is_some() {
                    "READY"
                } else {
                    "PENDING"
                };
                let successor_id =
                    instance_step_id(run.version_id, command.transition.run_id(), &successor.instance);
                let projection_matches: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
                       AND step_id=$3 AND status=$4) \
                     AND NOT EXISTS (SELECT 1 FROM loop_iterations WHERE tenant_id=$1 \
                       AND run_id=$2 AND owner_instance_id=$5) \
                     AND EXISTS (SELECT 1 FROM event_outbox WHERE tenant_id=$1 AND id=$6 \
                       AND aggregate_id=$2)",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(successor_id)
                .bind(expected_status)
                .bind(owner_id)
                .bind(command.outbox_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                if scope.get::<String, _>("lifecycle") != "SUCCEEDED" || !projection_matches {
                    return Err(LoopProgressError::DuplicateMismatch);
                }
                entered + if successor.is_end { 3 } else if successor.activity.is_some() { 2 } else { 1 }
            };
            if committed > run.run_version {
                return Err(invalid_storage("loop entry committed version"));
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: committed,
            });
        }

        check_run_active(&run)?;
        check_revision(&run, &command.transition)?;
        if !usage.policy_configured {
            return Err(LoopProgressError::PolicyMissing);
        }
        if needs_body && usage.tenant_active >= usage.tenant_limit {
            return Err(LoopProgressError::AtCapacity);
        }
        let first_charge = command
            .first_activity
            .as_ref()
            .map_or(0_i64, |first| 1 + i64::from(first.reserved_instances));
        let owner_charge = i64::from(command.transition.owner().path().segments().is_empty());
        let terminal_charge = i64::from(terminal.is_some());
        if i64::from(usage.admitted)
            + i64::from(usage.reserved)
            + owner_charge
            + first_charge
            + terminal_charge
            > i64::from(usage.max_instances)
        {
            return Err(LoopProgressError::InstanceBudgetExceeded);
        }
        retain_payload_in_transaction(
            &mut tx,
            command.transition.tenant_id().as_uuid(),
            command.initial_carry_digest,
            command.initial_carry_content(),
        )
        .await?;
        let before_deadline: bool =
            sqlx::query_scalar("SELECT $1::timestamptz > clock_timestamp()")
                .bind(command.deadline)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
        if !before_deadline {
            return Err(LoopProgressError::InvalidTransition);
        }

        let entered_sequence = run
            .run_version
            .checked_add(1)
            .ok_or_else(|| invalid_storage("loop entered sequence"))?;
        let owner_status = if needs_body { "WAITING" } else { "SUCCEEDED" };
        let owner_wait = needs_body.then_some("LOOP");
        let owner_routed = !needs_body;
        let parked = sqlx::query(
            "UPDATE steps SET status=$6,waiting_kind=$7,routed=$8,updated_at=$9 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND node=$5 AND status='PENDING' AND NOT routed",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(command.transition.owner().node().as_str())
        .bind(owner_status)
        .bind(owner_wait)
        .bind(owner_routed)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if parked.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }

        sqlx::query(
            "INSERT INTO loop_scopes \
             (tenant_id,namespace_id,run_id,owner_instance_id,owner_path_nodes,owner_path_indices, \
              owner_node,kind,lifecycle,max_iterations,deadline_at,initial_carry_digest, \
              current_carry_digest,next_iteration,active_iteration,entered_sequence, \
              initial_condition,entry_outbox_id,created_at,updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12,0,$13,$14,$15,$16,$17,$17)",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.namespace_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(command.transition.owner().node().as_str())
        .bind(kind_db)
        .bind(if needs_body { "RUNNING" } else { "SUCCEEDED" })
        .bind(i32::try_from(command.max_iterations).unwrap_or(i32::MAX))
        .bind(command.deadline)
        .bind(command.initial_carry_digest.as_bytes().as_slice())
        .bind(needs_body.then_some(0_i32))
        .bind(number(entered_sequence, "journal sequence")?)
        .bind(command.initial_condition)
        .bind(command.outbox_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &command.transition,
            &run,
            entered_sequence,
            "loop.entered",
            json!({
                "kind": kind_event,
                "max_iterations": command.max_iterations,
                "entered_at_us": micros(command.transition.occurred_at())?,
                "deadline_us": micros(command.deadline)?,
                "initial_carry_digest": command.initial_carry_digest.to_hex(),
                "initial_condition": command.initial_condition,
            }),
        )
        .await?;

        let (committed_version, terminal_run) = if let Some(first) = command.first_activity.as_ref()
        {
            let admitted_sequence = entered_sequence + 1;
            let released_sequence = entered_sequence + 2;
            insert_ready_activity(
                &mut tx,
                &command.transition,
                &run,
                0,
                first,
                admitted_sequence,
            )
            .await?;
            let usage_updated = sqlx::query(
                "UPDATE run_instance_usage SET admitted_instances=admitted_instances+1, \
                        reserved_instances=reserved_instances+$3 \
                 WHERE tenant_id=$1 AND run_id=$2",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(i32::from(first.reserved_instances))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            let tenant_updated = sqlx::query(
                "UPDATE tenant_iteration_usage SET active_iterations=active_iterations+1 \
                 WHERE tenant_id=$1",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if usage_updated.rows_affected() != 1 || tenant_updated.rows_affected() != 1 {
                return Err(invalid_storage("loop entry usage projection"));
            }
            append_event(
                &mut tx,
                &command.transition,
                &run,
                admitted_sequence,
                "loop.iteration_admitted",
                json!({"index":0,"entry":instance_json(&first.instance)}),
            )
            .await?;
            append_instance_event(
                &mut tx,
                &command.transition,
                &run,
                &first.instance,
                released_sequence,
                "step.released",
            )
            .await?;
            insert_task_outbox(&mut tx, &command.transition, first, command.outbox_id).await?;
            (released_sequence, false)
        } else {
            let successor = terminal
                .as_ref()
                .ok_or(LoopProgressError::InvalidTransition)?;
            route_loop_successor(
                &mut tx,
                &command.transition,
                &run,
                successor,
                entered_sequence + 1,
                command.outbox_id,
            )
            .await?
        };
        if terminal_run {
            finish_succeeded_version(&mut tx, &command.transition, committed_version).await?;
        } else {
            finish_version(&mut tx, &command.transition, committed_version).await?;
        }
        mark_telemetry_pending(&mut tx, &command.transition, committed_version).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: committed_version,
        })
    }

    async fn checkpoint(
        &self,
        command: &LoopCheckpointCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        check_audit(&command.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let usage = lock_usage(
            &mut tx,
            &command.transition,
            command.next_activity.is_some(),
        )
        .await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, kind, \
                    lifecycle, max_iterations, deadline_at, deadline_at >= clock_timestamp() AS before_deadline, \
                    next_iteration, active_iteration, current_carry_digest \
             FROM loop_scopes WHERE tenant_id = $1 AND run_id = $2 AND owner_instance_id = $3 \
             FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let iteration = sqlx::query(
            "SELECT lifecycle, slot_held, remaining_reservations, entry_path_nodes, \
                    entry_path_indices, return_instance_id, checkpoint_sequence \
             FROM loop_iterations WHERE tenant_id = $1 AND run_id = $2 \
               AND owner_instance_id = $3 AND iteration_index = $4 FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let (continuation, delay_due_at) = continuation_parts(command.continuation);
        let (owner_nodes, owner_indices) = path_parts(command.transition.owner());
        let return_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            &command.return_instance,
        );
        if iteration.get::<String, _>("lifecycle") == "CHECKPOINTED" {
            let saved = sqlx::query(
                "SELECT return_instance_id, result_digest, carry_digest, condition, checked_at, \
                        continuation, delay_due_at, timer_id, outbox_id, committed_run_version \
                 FROM loop_checkpoints WHERE tenant_id = $1 AND run_id = $2 \
                   AND owner_instance_id = $3 AND iteration_index = $4",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(owner_id)
            .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            .ok_or_else(|| invalid_storage("loop checkpoint projection"))?;
            let next_matches = if let Some(next) = &command.next_activity {
                let next_id =
                    instance_step_id(run.version_id, command.transition.run_id(), &next.instance);
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM loop_iterations WHERE tenant_id = $1 AND run_id = $2 \
                       AND owner_instance_id = $3 AND iteration_index = $4 \
                       AND entry_instance_id = $5 AND activity = $6 AND reserved_instances = $7)",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(owner_id)
                .bind(i32::try_from(command.index + 1).unwrap_or(i32::MAX))
                .bind(next_id)
                .bind(next.activity.as_str())
                .bind(i32::from(next.reserved_instances))
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?
            } else {
                true
            };
            let matches = saved.get::<uuid::Uuid, _>("return_instance_id") == return_id
                && saved.get::<Vec<u8>, _>("result_digest") == command.result_digest.as_bytes()
                && saved.get::<Vec<u8>, _>("carry_digest") == command.carry_digest.as_bytes()
                && saved.get::<bool, _>("condition") == command.condition
                && saved.get::<time::OffsetDateTime, _>("checked_at") == command.checked_at
                && saved.get::<String, _>("continuation") == continuation
                && saved.get::<Option<time::OffsetDateTime>, _>("delay_due_at") == delay_due_at
                && saved.get::<Option<uuid::Uuid>, _>("timer_id") == command.timer_id
                && saved.get::<uuid::Uuid, _>("outbox_id") == command.outbox_id
                && next_matches;
            if !matches {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let committed = u64::try_from(saved.get::<i64, _>("committed_run_version"))
                .map_err(|_| invalid_storage("loop checkpoint run version"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: committed,
            });
        }
        check_run_active(&run)?;
        check_revision(&run, &command.transition)?;
        let next_index = command
            .index
            .checked_add(1)
            .ok_or(LoopProgressError::InvalidTransition)?;
        let max_iterations = u32::try_from(scope.get::<i32, _>("max_iterations"))
            .map_err(|_| invalid_storage("loop max iterations"))?;
        let deadline: time::OffsetDateTime = scope.get("deadline_at");
        let needs_another = match scope.get::<String, _>("kind").as_str() {
            "WHILE" => command.condition,
            "REPEAT_UNTIL" => !command.condition,
            _ => return Err(invalid_storage("loop kind")),
        };
        let continuation_valid = if !needs_another {
            matches!(command.continuation, LoopContinuation::Complete)
        } else if next_index >= max_iterations {
            matches!(command.continuation, LoopContinuation::IterationLimit)
        } else {
            match command.continuation {
                LoopContinuation::Continue => true,
                LoopContinuation::DelayUntil(due_at) => {
                    due_at > command.checked_at && due_at <= deadline
                }
                LoopContinuation::Complete | LoopContinuation::IterationLimit => false,
            }
        };
        let (return_nodes, return_indices) = path_parts(&command.return_instance);
        if scope.get::<uuid::Uuid, _>("namespace_id") != command.transition.namespace_id().as_uuid()
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != command.transition.owner().node().as_str()
            || scope.get::<String, _>("lifecycle") != "RUNNING"
            || scope.get::<Option<i32>, _>("active_iteration")
                != Some(i32::try_from(command.index).unwrap_or(i32::MAX))
            || !scope.get::<bool, _>("before_deadline")
            || command.checked_at > deadline
            || !continuation_valid
            || iteration.get::<String, _>("lifecycle") != "ADMITTED"
            || !iteration.get::<bool, _>("slot_held")
            || iteration.get::<i32, _>("remaining_reservations") <= 0
            || usage.reserved <= 0
            || iteration.get::<Vec<String>, _>("entry_path_nodes") != return_nodes
            || iteration.get::<Vec<i32>, _>("entry_path_indices") != return_indices
            || iteration
                .get::<Option<i64>, _>("checkpoint_sequence")
                .is_some()
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        let definition = self
            .definition(
                &mut tx,
                command.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        validate_definition(
            &definition,
            &command.transition,
            Some(&command.return_instance),
            command.next_activity.as_ref(),
        )?;
        let owner_definition = definition
            .node_for_instance(command.transition.owner())
            .ok_or(LoopProgressError::InvalidTransition)?;
        let (definition_kind, definition_limit) = match owner_definition.kind() {
            NodeKind::While { max_iterations, .. } => ("WHILE", *max_iterations),
            NodeKind::RepeatUntil { max_iterations, .. } => ("REPEAT_UNTIL", *max_iterations),
            _ => return Err(LoopProgressError::InvalidTransition),
        };
        if scope.get::<String, _>("kind") != definition_kind || max_iterations != definition_limit {
            return Err(LoopProgressError::InvalidTransition);
        }
        let terminal = match command.continuation {
            LoopContinuation::Complete => {
                terminal_successor(&definition, command.transition.owner(), false)?
            }
            LoopContinuation::IterationLimit => {
                terminal_successor(&definition, command.transition.owner(), true)?
            }
            LoopContinuation::Continue | LoopContinuation::DelayUntil(_) => None,
        };
        retain_payload_in_transaction(
            &mut tx,
            command.transition.tenant_id().as_uuid(),
            command.result_digest,
            command.result_content(),
        )
        .await?;
        retain_payload_in_transaction(
            &mut tx,
            command.transition.tenant_id().as_uuid(),
            command.carry_digest,
            command.carry_content(),
        )
        .await?;
        let member_steps = sqlx::query(
            "SELECT node, step_id, status, routed, attempt, generation, lease_token_hash, waiting_kind \
             FROM steps WHERE tenant_id = $1 AND run_id = $2 \
               AND instance_path_nodes = $3 AND instance_path_indices = $4 ORDER BY node FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut found_return = false;
        for member in member_steps {
            if member.get::<String, _>("node") == command.return_instance.node().as_str() {
                let valid = !found_return
                    && member.get::<uuid::Uuid, _>("step_id") == return_id
                    && member.get::<String, _>("status") == "PENDING"
                    && !member.get::<bool, _>("routed")
                    && member.get::<i32, _>("attempt") == 0
                    && member.get::<i64, _>("generation") == 0
                    && member
                        .get::<Option<Vec<u8>>, _>("lease_token_hash")
                        .is_none()
                    && member.get::<Option<String>, _>("waiting_kind").is_none();
                if !valid {
                    return Err(LoopProgressError::InvalidTransition);
                }
                found_return = true;
            } else if !matches!(
                member.get::<String, _>("status").as_str(),
                "SUCCEEDED" | "SKIPPED"
            ) || !member.get::<bool, _>("routed")
            {
                return Err(LoopProgressError::InvalidTransition);
            }
        }
        if !found_return {
            return Err(LoopProgressError::InvalidTransition);
        }
        if let Some(next) = &command.next_activity {
            let charge = 1_i64 + i64::from(next.reserved_instances);
            let consumed = i64::from(usage.admitted) + i64::from(usage.reserved);
            if consumed + charge > i64::from(usage.max_instances) {
                return Err(LoopProgressError::InstanceBudgetExceeded);
            }
        } else if terminal.is_some()
            && i64::from(usage.admitted) + i64::from(usage.reserved) + 1
                > i64::from(usage.max_instances)
        {
            return Err(LoopProgressError::InstanceBudgetExceeded);
        }

        let returned_sequence = run.run_version + 1;
        let checkpoint_sequence = returned_sequence + 1;
        let mut committed_version = checkpoint_sequence;
        if let Some(next) = &command.next_activity {
            insert_ready_activity(
                &mut tx,
                &command.transition,
                &run,
                next_index,
                next,
                checkpoint_sequence + 1,
            )
            .await?;
            committed_version += 2;
        } else if let Some(successor) = &terminal {
            committed_version += if successor.is_end {
                3
            } else if successor.activity.is_some() {
                2
            } else {
                1
            };
        } else if matches!(command.continuation, LoopContinuation::IterationLimit) {
            committed_version += 1;
        }
        let completed_return = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', routed = true, updated_at = $8 \
             WHERE tenant_id = $1 AND run_id = $2 AND namespace_id = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5 AND node = $6 \
               AND step_id = $7 AND status = 'PENDING' AND attempt = 0 AND generation = 0 \
               AND NOT routed AND lease_token_hash IS NULL AND waiting_kind IS NULL",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .bind(command.return_instance.node().as_str())
        .bind(return_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if completed_return.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        if matches!(
            command.continuation,
            LoopContinuation::Complete | LoopContinuation::IterationLimit
        ) {
            let failed = matches!(command.continuation, LoopContinuation::IterationLimit);
            let owner_finished = sqlx::query(
                "UPDATE steps SET status=$6,waiting_kind=NULL,routed=$7,failure_code=$8, \
                        updated_at=$9 \
                 WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
                   AND instance_path_indices=$4 AND node=$5 AND status='WAITING' \
                   AND waiting_kind='LOOP' AND NOT routed",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(&owner_nodes)
            .bind(&owner_indices)
            .bind(command.transition.owner().node().as_str())
            .bind(if failed { "FAILED" } else { "SUCCEEDED" })
            .bind(terminal.is_some())
            .bind(failed.then(|| FailureCode::iteration_limit().as_str().to_owned()))
            .bind(command.transition.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if owner_finished.rows_affected() != 1 {
                return Err(LoopProgressError::InvalidTransition);
            }
        }
        let iteration_updated = sqlx::query(
            "UPDATE loop_iterations SET lifecycle = 'CHECKPOINTED', slot_held = false, \
                    remaining_reservations = remaining_reservations - 1, return_instance_id = $5, \
                    return_node = $6, result_digest = $7, carry_digest = $8, condition = $9, \
                    checked_at = $10, checkpoint_sequence = $11, updated_at = $12 \
             WHERE tenant_id = $1 AND run_id = $2 AND owner_instance_id = $3 AND iteration_index = $4",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
        .bind(return_id)
        .bind(command.return_instance.node().as_str())
        .bind(command.result_digest.as_bytes().as_slice())
        .bind(command.carry_digest.as_bytes().as_slice())
        .bind(command.condition)
        .bind(command.checked_at)
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if iteration_updated.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let checkpoint_inserted = sqlx::query(
            "INSERT INTO loop_checkpoints \
             (tenant_id, run_id, owner_instance_id, iteration_index, return_instance_id, \
              result_digest, carry_digest, condition, checked_at, continuation, delay_due_at, \
              timer_id, journal_sequence, outbox_id, committed_run_version, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
        .bind(return_id)
        .bind(command.result_digest.as_bytes().as_slice())
        .bind(command.carry_digest.as_bytes().as_slice())
        .bind(command.condition)
        .bind(command.checked_at)
        .bind(continuation)
        .bind(delay_due_at)
        .bind(command.timer_id)
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(command.outbox_id)
        .bind(number(committed_version, "run version")?)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if checkpoint_inserted.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let scope_updated = sqlx::query(
            "UPDATE loop_scopes SET lifecycle = $4, current_carry_digest = $5, \
                    next_iteration = $6, active_iteration = $7, delay_due_at = $8, \
                    last_checkpoint_sequence = $9, updated_at = $10 \
             WHERE tenant_id = $1 AND run_id = $2 AND owner_instance_id = $3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(match command.continuation {
            LoopContinuation::Continue => "RUNNING",
            LoopContinuation::DelayUntil(_) => "DELAYING",
            LoopContinuation::Complete => "SUCCEEDED",
            LoopContinuation::IterationLimit => "FAILED",
        })
        .bind(command.carry_digest.as_bytes().as_slice())
        .bind(i32::try_from(next_index).unwrap_or(i32::MAX))
        .bind(
            matches!(command.continuation, LoopContinuation::Continue)
                .then(|| i32::try_from(next_index).unwrap_or(i32::MAX)),
        )
        .bind(delay_due_at)
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if scope_updated.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let next_reserved = command
            .next_activity
            .as_ref()
            .map_or(0_i32, |next| i32::from(next.reserved_instances));
        let admitted_delta = if command.next_activity.is_some() {
            2_i32
        } else {
            1_i32
        };
        let usage_updated = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances = admitted_instances + $3, \
                    reserved_instances = reserved_instances - 1 + $4 \
             WHERE tenant_id = $1 AND run_id = $2 AND reserved_instances > 0",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(admitted_delta)
        .bind(next_reserved)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if usage_updated.rows_affected() != 1 {
            return Err(invalid_storage("loop run usage projection"));
        }
        if command.next_activity.is_none() {
            let released = sqlx::query(
                "UPDATE tenant_iteration_usage SET active_iterations = active_iterations - 1 \
                 WHERE tenant_id = $1 AND active_iterations > 0",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if released.rows_affected() != 1 {
                return Err(invalid_storage("loop tenant usage projection"));
            }
        }
        append_event(
            &mut tx,
            &command.transition,
            &run,
            returned_sequence,
            "loop.iteration_returned",
            json!({"index": command.index, "return_instance": instance_json(&command.return_instance)}),
        )
        .await?;
        append_event(
            &mut tx,
            &command.transition,
            &run,
            checkpoint_sequence,
            "loop.iteration_checkpointed",
            json!({
                "index": command.index,
                "return_instance": instance_json(&command.return_instance),
                "result_digest": command.result_digest.to_hex(),
                "carry_digest": command.carry_digest.to_hex(),
                "condition": command.condition,
                "checked_at_us": micros(command.checked_at)?,
                "continuation": continuation_json(command.continuation)?,
            }),
        )
        .await?;
        if let Some(next) = &command.next_activity {
            append_event(
                &mut tx,
                &command.transition,
                &run,
                checkpoint_sequence + 1,
                "loop.iteration_admitted",
                json!({"index": next_index, "entry": instance_json(&next.instance)}),
            )
            .await?;
            append_instance_event(
                &mut tx,
                &command.transition,
                &run,
                &next.instance,
                checkpoint_sequence + 2,
                "step.released",
            )
            .await?;
            insert_task_outbox(&mut tx, &command.transition, next, command.outbox_id).await?;
        } else if let Some(timer_id) = command.timer_id {
            let due_at = delay_due_at.ok_or(LoopProgressError::InvalidTransition)?;
            sqlx::query(
                "INSERT INTO loop_timers \
                 (tenant_id, run_id, owner_instance_id, timer_id, checkpoint_iteration, \
                  next_iteration, due_at, lifecycle, scheduled_sequence, scheduled_outbox_id, \
                  created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'WAITING',$8,$9,$10,$10)",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(owner_id)
            .bind(timer_id)
            .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
            .bind(i32::try_from(next_index).unwrap_or(i32::MAX))
            .bind(due_at)
            .bind(number(checkpoint_sequence, "journal sequence")?)
            .bind(command.outbox_id)
            .bind(command.transition.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO event_outbox \
                 (id, tenant_id, aggregate_id, event_type, payload) \
                 VALUES ($1,$2,$3,'loop.timer.scheduled',$4)",
            )
            .bind(command.outbox_id)
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(json!({
                "runId": command.transition.run_id().to_string(),
                "timerId": timer_id.to_string(),
                "dueAtMicros": micros(due_at)?,
                "traceId": command.transition.trace_id(),
            }))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let terminal_run = if let Some(successor) = &terminal {
            let (routed_version, finished) = route_loop_successor(
                &mut tx,
                &command.transition,
                &run,
                successor,
                checkpoint_sequence + 1,
                command.outbox_id,
            )
            .await?;
            if routed_version != committed_version {
                return Err(invalid_storage("loop terminal journal sequence"));
            }
            finished
        } else if matches!(command.continuation, LoopContinuation::IterationLimit) {
            let stop_sequence = checkpoint_sequence + 1;
            let failure = FailureCode::iteration_limit();
            append_run_event(
                &mut tx,
                &command.transition,
                stop_sequence,
                "run.stop_requested",
                json!({"reason":"FAILED","code":failure.as_str()}),
            )
            .await?;
            insert_loop_hint(
                &mut tx,
                &command.transition,
                command.outbox_id,
                "loop.iteration_limit",
                "FAILED",
            )
            .await?;
            false
        } else {
            false
        };
        if terminal_run {
            finish_succeeded_version(&mut tx, &command.transition, committed_version).await?;
        } else if matches!(command.continuation, LoopContinuation::IterationLimit)
            && terminal.is_none()
        {
            finish_iteration_limit(&mut tx, &command.transition, committed_version).await?;
        } else {
            finish_version(&mut tx, &command.transition, committed_version).await?;
        }
        mark_telemetry_pending(&mut tx, &command.transition, committed_version).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: committed_version,
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn fire_delay(
        &self,
        command: &LoopDelayElapsedCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        check_audit(&command.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let usage = lock_usage(&mut tx, &command.transition, true).await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, lifecycle, \
                    next_iteration, delay_due_at FROM loop_scopes \
             WHERE tenant_id = $1 AND run_id = $2 AND owner_instance_id = $3 FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let timer = sqlx::query(
            "SELECT run_id, owner_instance_id, lifecycle, next_iteration, due_at, \
                    due_at <= clock_timestamp() AS due, fired_sequence, fired_outbox_id \
             FROM loop_timers WHERE tenant_id = $1 AND timer_id = $2 FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.timer_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        if timer.get::<String, _>("lifecycle") == "FIRED" {
            let next_id = instance_step_id(
                run.version_id,
                command.transition.run_id(),
                &command.next_activity.instance,
            );
            let matches = timer.get::<i32, _>("next_iteration")
                == i32::try_from(command.next_iteration).unwrap_or(i32::MAX)
                && timer.get::<Option<uuid::Uuid>, _>("fired_outbox_id") == Some(command.outbox_id)
                && sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM loop_iterations WHERE tenant_id=$1 AND run_id=$2 \
                     AND owner_instance_id=$3 AND iteration_index=$4 AND entry_instance_id=$5 \
                     AND activity=$6 AND reserved_instances=$7)",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(owner_id)
                .bind(i32::try_from(command.next_iteration).unwrap_or(i32::MAX))
                .bind(next_id)
                .bind(command.next_activity.activity.as_str())
                .bind(i32::from(command.next_activity.reserved_instances))
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            if !matches {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let fired = u64::try_from(
                timer
                    .get::<Option<i64>, _>("fired_sequence")
                    .ok_or_else(|| invalid_storage("fired loop timer sequence"))?,
            )
            .map_err(|_| invalid_storage("fired loop timer sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: fired + 2,
            });
        }
        check_run_active(&run)?;
        check_revision(&run, &command.transition)?;
        let due_at: time::OffsetDateTime = timer.get("due_at");
        let (owner_nodes, owner_indices) = path_parts(command.transition.owner());
        if scope.get::<uuid::Uuid, _>("namespace_id") != command.transition.namespace_id().as_uuid()
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != command.transition.owner().node().as_str()
            || timer.get::<uuid::Uuid, _>("run_id") != command.transition.run_id().as_uuid()
            || timer.get::<uuid::Uuid, _>("owner_instance_id") != owner_id
            || timer.get::<String, _>("lifecycle") != "WAITING"
            || timer.get::<i32, _>("next_iteration")
                != i32::try_from(command.next_iteration).unwrap_or(i32::MAX)
            || scope.get::<String, _>("lifecycle") != "DELAYING"
            || scope.get::<i32, _>("next_iteration")
                != i32::try_from(command.next_iteration).unwrap_or(i32::MAX)
            || scope.get::<Option<time::OffsetDateTime>, _>("delay_due_at") != Some(due_at)
            || !timer.get::<bool, _>("due")
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        if usage.tenant_active >= usage.tenant_limit {
            return Err(LoopProgressError::AtCapacity);
        }
        let charge = 1_i64 + i64::from(command.next_activity.reserved_instances);
        if i64::from(usage.admitted) + i64::from(usage.reserved) + charge
            > i64::from(usage.max_instances)
        {
            return Err(LoopProgressError::InstanceBudgetExceeded);
        }
        let definition = self
            .definition(
                &mut tx,
                command.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        validate_definition(
            &definition,
            &command.transition,
            None,
            Some(&command.next_activity),
        )?;
        let elapsed_sequence = run.run_version + 1;
        let admitted_sequence = elapsed_sequence + 1;
        let released_sequence = admitted_sequence + 1;
        insert_ready_activity(
            &mut tx,
            &command.transition,
            &run,
            command.next_iteration,
            &command.next_activity,
            admitted_sequence,
        )
        .await?;
        let timer_fired = sqlx::query(
            "UPDATE loop_timers SET lifecycle='FIRED', fired_sequence=$3, fired_outbox_id=$4, \
                    updated_at=$5 WHERE tenant_id=$1 AND timer_id=$2",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.timer_id)
        .bind(number(elapsed_sequence, "journal sequence")?)
        .bind(command.outbox_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if timer_fired.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let scope_advanced = sqlx::query(
            "UPDATE loop_scopes SET lifecycle='RUNNING', active_iteration=$4, delay_due_at=NULL, \
                    updated_at=$5 WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.next_iteration).unwrap_or(i32::MAX))
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if scope_advanced.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let run_usage = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances=admitted_instances + 1, \
                    reserved_instances=reserved_instances + $3 \
             WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(i32::from(command.next_activity.reserved_instances))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let tenant_usage = sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations=active_iterations + 1 \
             WHERE tenant_id=$1",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if run_usage.rows_affected() != 1 || tenant_usage.rows_affected() != 1 {
            return Err(invalid_storage("loop admission usage projection"));
        }
        append_event(
            &mut tx,
            &command.transition,
            &run,
            elapsed_sequence,
            "loop.delay_elapsed",
            json!({"next_iteration": command.next_iteration}),
        )
        .await?;
        append_event(
            &mut tx,
            &command.transition,
            &run,
            admitted_sequence,
            "loop.iteration_admitted",
            json!({
                "index": command.next_iteration,
                "entry": instance_json(&command.next_activity.instance),
            }),
        )
        .await?;
        append_instance_event(
            &mut tx,
            &command.transition,
            &run,
            &command.next_activity.instance,
            released_sequence,
            "step.released",
        )
        .await?;
        insert_task_outbox(
            &mut tx,
            &command.transition,
            &command.next_activity,
            command.outbox_id,
        )
        .await?;
        finish_version(&mut tx, &command.transition, released_sequence).await?;
        mark_telemetry_pending(&mut tx, &command.transition, released_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: released_sequence,
        })
    }

    async fn start_drain(
        &self,
        command: &LoopDrainCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        check_audit(&command.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let _usage = lock_usage(&mut tx, &command.transition, false).await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let scope = sqlx::query(
            "SELECT namespace_id,owner_path_nodes,owner_path_indices,owner_node,lifecycle, \
                    active_iteration,deadline_at,drain_kind,drain_iteration,drain_failure_code, \
                    drain_sequence,drain_outbox_id \
             FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
             FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let (owner_nodes, owner_indices) = path_parts(command.transition.owner());
        if NamespaceId::new(scope.get("namespace_id")) != run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != command.transition.owner().node().as_str()
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        let timeout_code = FailureCode::run_timed_out();
        let (kind, iteration, code) = match &command.reason {
            sagat_domain::execution::LoopDrainReason::Failed { iteration, code } => (
                "FAILED",
                Some(i32::try_from(*iteration).map_err(|_| LoopProgressError::InvalidTransition)?),
                Some(code.as_str()),
            ),
            sagat_domain::execution::LoopDrainReason::TimedOut => {
                ("TIMED_OUT", None, Some(timeout_code.as_str()))
            }
            sagat_domain::execution::LoopDrainReason::Cancelled => ("CANCELLED", None, None),
        };
        let lifecycle = scope.get::<String, _>("lifecycle");
        if matches!(lifecycle.as_str(), "DRAINING" | "FAILED" | "CANCELLED")
            && scope.get::<Option<String>, _>("drain_kind").is_some()
        {
            let matches = scope.get::<Option<String>, _>("drain_kind").as_deref() == Some(kind)
                && scope.get::<Option<i32>, _>("drain_iteration") == iteration
                && scope
                    .get::<Option<String>, _>("drain_failure_code")
                    .as_deref()
                    == code
                && scope.get::<Option<uuid::Uuid>, _>("drain_outbox_id") == Some(command.outbox_id);
            if !matches {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let sequence = u64::try_from(
                scope
                    .get::<Option<i64>, _>("drain_sequence")
                    .ok_or_else(|| invalid_storage("loop drain sequence"))?,
            )
            .map_err(|_| invalid_storage("loop drain sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: sequence,
            });
        }
        if !matches!(lifecycle.as_str(), "READY" | "RUNNING" | "DELAYING") {
            return Err(LoopProgressError::InvalidTransition);
        }
        check_revision(&run, &command.transition)?;

        let mut sequence = run.run_version;
        match &command.reason {
            sagat_domain::execution::LoopDrainReason::Cancelled => {
                if run.status != "CANCELLING"
                    || run.stop_reason.as_deref() != Some("CANCELLED")
                    || run.stop_code.is_some()
                {
                    return Err(LoopProgressError::InvalidTransition);
                }
            }
            sagat_domain::execution::LoopDrainReason::TimedOut => {
                let deadline: time::OffsetDateTime = scope.get("deadline_at");
                if command.transition.occurred_at() < deadline {
                    return Err(LoopProgressError::InvalidTransition);
                }
                sequence += 1;
                append_run_event(
                    &mut tx,
                    &command.transition,
                    sequence,
                    "run.stop_requested",
                    json!({"reason":"FAILED","code":timeout_code.as_str()}),
                )
                .await?;
                let stopped = sqlx::query(
                    "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3 \
                     WHERE tenant_id=$1 AND id=$2 AND status IN ('RUNNING','WAITING') \
                       AND stop_reason IS NULL AND stop_code IS NULL",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(timeout_code.as_str())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                if stopped.rows_affected() != 1 {
                    return Err(LoopProgressError::InvalidTransition);
                }
            }
            sagat_domain::execution::LoopDrainReason::Failed { iteration, code } => {
                if scope.get::<Option<i32>, _>("active_iteration")
                    != Some(i32::try_from(*iteration).unwrap_or(i32::MAX))
                {
                    return Err(LoopProgressError::InvalidTransition);
                }
                let iteration_path = command
                    .transition
                    .owner()
                    .path()
                    .child(command.transition.owner().node().clone(), *iteration)
                    .map_err(|_| LoopProgressError::InvalidTransition)?;
                let (path_nodes, path_indices): (Vec<_>, Vec<_>) = iteration_path
                    .segments()
                    .iter()
                    .map(|segment| {
                        (
                            segment.scope_node().as_str().to_owned(),
                            i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
                        )
                    })
                    .unzip();
                let members = sqlx::query(
                    "SELECT node,status,failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 \
                       AND instance_path_nodes=$3 AND instance_path_indices=$4 FOR UPDATE",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(&path_nodes)
                .bind(&path_indices)
                .fetch_all(&mut *tx)
                .await
                .map_err(store_error)?;
                let definition = self
                    .definition(
                        &mut tx,
                        command.transition.tenant_id().as_uuid(),
                        run.version_id,
                    )
                    .await?;
                let proven = members.iter().any(|member| {
                    let status = member.get::<String, _>("status");
                    let member_code = member.get::<Option<String>, _>("failure_code");
                    let failed =
                        status == "FAILED" && member_code.as_deref() == Some(code.as_str());
                    let expression_boundary = code == &FailureCode::expression_error()
                        && status == "PENDING"
                        && NodeId::parse_stored(&member.get::<String, _>("node")).is_ok_and(
                            |node| {
                                matches!(
                                    definition
                                        .node_for_instance(&ExecutionInstanceId::in_path(
                                            iteration_path.clone(),
                                            node,
                                        ))
                                        .map(sagat_domain::workflow::Node::kind),
                                    Some(NodeKind::Return { .. })
                                )
                            },
                        );
                    failed || expression_boundary
                });
                if !proven {
                    return Err(LoopProgressError::InvalidTransition);
                }
                sequence += 1;
                append_run_event(
                    &mut tx,
                    &command.transition,
                    sequence,
                    "run.stop_requested",
                    json!({"reason":"FAILED","code":code.as_str()}),
                )
                .await?;
                let stopped = sqlx::query(
                    "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3 \
                     WHERE tenant_id=$1 AND id=$2 AND status IN ('RUNNING','WAITING') \
                       AND stop_reason IS NULL AND stop_code IS NULL",
                )
                .bind(command.transition.tenant_id().as_uuid())
                .bind(command.transition.run_id().as_uuid())
                .bind(code.as_str())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                if stopped.rows_affected() != 1 {
                    return Err(LoopProgressError::InvalidTransition);
                }
            }
        }
        if lifecycle == "DELAYING" {
            let cancelled = sqlx::query(
                "UPDATE loop_timers SET lifecycle='CANCELLED',updated_at=$4 \
                 WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
                   AND lifecycle='WAITING'",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(owner_id)
            .bind(command.transition.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if cancelled.rows_affected() != 1 {
                return Err(LoopProgressError::InvalidTransition);
            }
        }
        let cancellable = sqlx::query(
            "SELECT step.node,step.instance_path_nodes,step.instance_path_indices \
             FROM loop_iterations iteration JOIN steps step \
               ON step.tenant_id=iteration.tenant_id AND step.run_id=iteration.run_id \
              AND step.instance_path_nodes=iteration.entry_path_nodes \
              AND step.instance_path_indices=iteration.entry_path_indices \
             WHERE iteration.tenant_id=$1 AND iteration.run_id=$2 \
               AND iteration.owner_instance_id=$3 AND iteration.lifecycle='ADMITTED' \
               AND step.status IN ('LEASED','RUNNING') AND step.cancel_requested=false \
             ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node \
             FOR UPDATE OF step",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|row| {
            stored_instance(
                row.get("instance_path_nodes"),
                row.get("instance_path_indices"),
                row.get("node"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| invalid_storage("journal sequence"))?;
        let drain_sequence = sequence;
        let final_sequence = sequence
            .checked_add(
                u64::try_from(cancellable.len())
                    .map_err(|_| invalid_storage("journal sequence"))?,
            )
            .ok_or_else(|| invalid_storage("journal sequence"))?;
        let updated = sqlx::query(
            "UPDATE loop_scopes SET lifecycle='DRAINING',delay_due_at=NULL,drain_kind=$4, \
                    drain_iteration=$5,drain_failure_code=$6,drain_sequence=$7,drain_outbox_id=$8, \
                    updated_at=$9 WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(kind)
        .bind(iteration)
        .bind(code)
        .bind(number(final_sequence, "journal sequence")?)
        .bind(command.outbox_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        append_event(
            &mut tx,
            &command.transition,
            &run,
            drain_sequence,
            "loop.drain_started",
            match &command.reason {
                sagat_domain::execution::LoopDrainReason::Failed { iteration, code } => {
                    json!({"reason":"failed","iteration":iteration,"code":code.as_str()})
                }
                sagat_domain::execution::LoopDrainReason::TimedOut => {
                    json!({"reason":"timed_out"})
                }
                sagat_domain::execution::LoopDrainReason::Cancelled => {
                    json!({"reason":"cancelled"})
                }
            },
        )
        .await?;
        for instance in &cancellable {
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| invalid_storage("journal sequence"))?;
            append_instance_event(
                &mut tx,
                &command.transition,
                &run,
                instance,
                sequence,
                "step.cancel_requested",
            )
            .await?;
            let (path_nodes, path_indices) = path_parts(instance);
            let updated = sqlx::query(
                "UPDATE steps SET cancel_requested=true,updated_at=$6 \
                 WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
                   AND instance_path_indices=$4 AND node=$5 \
                   AND status IN ('LEASED','RUNNING') AND cancel_requested=false",
            )
            .bind(command.transition.tenant_id().as_uuid())
            .bind(command.transition.run_id().as_uuid())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(instance.node().as_str())
            .bind(command.transition.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if updated.rows_affected() != 1 {
                return Err(LoopProgressError::RevisionConflict {
                    current: run.run_version,
                });
            }
        }
        debug_assert_eq!(sequence, final_sequence);
        insert_loop_hint(
            &mut tx,
            &command.transition,
            command.outbox_id,
            "loop.drain_started",
            "DRAINING",
        )
        .await?;
        finish_drain_version(&mut tx, &command.transition, final_sequence).await?;
        mark_telemetry_pending(&mut tx, &command.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: final_sequence,
        })
    }

    async fn drain_iteration(
        &self,
        command: &LoopIterationDrainCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        check_audit(&command.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let _usage = lock_usage(&mut tx, &command.transition, false).await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let scope = sqlx::query(
            "SELECT lifecycle,active_iteration,drain_kind,drain_iteration,drain_failure_code \
             FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
             FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let iteration = sqlx::query(
            "SELECT lifecycle,slot_held,remaining_reservations,entry_path_nodes, \
                    entry_path_indices,entry_node,drained_sequence,drain_outbox_id \
             FROM loop_iterations WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
               AND iteration_index=$4 FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        if iteration.get::<String, _>("lifecycle") == "DRAINED" {
            if iteration.get::<Option<uuid::Uuid>, _>("drain_outbox_id") != Some(command.outbox_id)
            {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let sequence = u64::try_from(
                iteration
                    .get::<Option<i64>, _>("drained_sequence")
                    .ok_or_else(|| invalid_storage("loop iteration drain sequence"))?,
            )
            .map_err(|_| invalid_storage("loop iteration drain sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: sequence,
            });
        }
        check_revision(&run, &command.transition)?;
        if run.status != "CANCELLING" {
            return Err(LoopProgressError::InvalidTransition);
        }
        if scope.get::<String, _>("lifecycle") != "DRAINING"
            || scope.get::<Option<i32>, _>("active_iteration")
                != Some(i32::try_from(command.index).unwrap_or(i32::MAX))
            || iteration.get::<String, _>("lifecycle") != "ADMITTED"
            || !iteration.get::<bool, _>("slot_held")
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        let path_nodes = iteration.get::<Vec<String>, _>("entry_path_nodes");
        let path_indices = iteration.get::<Vec<i32>, _>("entry_path_indices");
        let entry_node = iteration.get::<String, _>("entry_node");
        let remaining = iteration.get::<i32, _>("remaining_reservations");
        let members = sqlx::query(
            "SELECT node,status,attempt FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND instance_path_nodes=$3 AND instance_path_indices=$4 ORDER BY node FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        if members.is_empty() || remaining < 0 {
            return Err(LoopProgressError::InvalidTransition);
        }
        for member in &members {
            let node = NodeId::parse_stored(&member.get::<String, _>("node"))
                .map_err(|_| invalid_storage("loop member node"))?;
            let status = member.get::<String, _>("status");
            let terminal = matches!(
                status.as_str(),
                "SUCCEEDED" | "FAILED" | "CANCELLED" | "SKIPPED"
            );
            let safe_unstarted = matches!(status.as_str(), "PENDING" | "READY")
                && member.get::<i32, _>("attempt") == 0;
            if (node.is_derived() && !matches!(status.as_str(), "SUCCEEDED" | "SKIPPED"))
                || (!terminal && !safe_unstarted)
            {
                return Err(LoopProgressError::InvalidTransition);
            }
        }
        let failed_entry = scope.get::<Option<String>, _>("drain_kind").as_deref()
            == Some("FAILED")
            && scope.get::<Option<i32>, _>("drain_iteration")
                == Some(i32::try_from(command.index).unwrap_or(i32::MAX));
        let failure_code = scope.get::<Option<String>, _>("drain_failure_code");
        sqlx::query(
            "UPDATE steps SET status=CASE WHEN $5 AND node=$6 THEN 'FAILED' ELSE 'CANCELLED' END, \
                    failure_code=CASE WHEN $5 AND node=$6 THEN $7 ELSE failure_code END, \
                    routed=true,waiting_kind=NULL,due_at=NULL,event_type=NULL,correlation_key=NULL, \
                    next_attempt_at=NULL,cancel_requested=false,updated_at=$8 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND status IN ('PENDING','READY') AND attempt=0",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(failed_entry)
        .bind(&entry_node)
        .bind(failure_code.as_deref())
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let sequence = run.run_version + 1;
        let updated = sqlx::query(
            "UPDATE loop_iterations SET lifecycle='DRAINED',slot_held=false, \
                    remaining_reservations=0,drained_sequence=$5,drain_outbox_id=$6,updated_at=$7 \
             WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 AND iteration_index=$4",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(i32::try_from(command.index).unwrap_or(i32::MAX))
        .bind(number(sequence, "journal sequence")?)
        .bind(command.outbox_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let scope_updated = sqlx::query(
            "UPDATE loop_scopes SET active_iteration=NULL,updated_at=$4 \
             WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let tenant_updated = sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations=active_iterations-1 \
             WHERE tenant_id=$1 AND active_iterations>0",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let usage_updated = sqlx::query(
            "UPDATE run_instance_usage SET reserved_instances=reserved_instances-$3 \
             WHERE tenant_id=$1 AND run_id=$2 AND reserved_instances >= $3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(remaining)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1
            || scope_updated.rows_affected() != 1
            || tenant_updated.rows_affected() != 1
            || usage_updated.rows_affected() != 1
        {
            return Err(invalid_storage("loop drain projection"));
        }
        append_event(
            &mut tx,
            &command.transition,
            &run,
            sequence,
            "loop.iteration_drained",
            json!({"index":command.index}),
        )
        .await?;
        insert_loop_hint(
            &mut tx,
            &command.transition,
            command.outbox_id,
            "loop.iteration.drained",
            "DRAINING",
        )
        .await?;
        finish_drain_version(&mut tx, &command.transition, sequence).await?;
        mark_telemetry_pending(&mut tx, &command.transition, sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: sequence,
        })
    }

    async fn finish_drain(
        &self,
        command: &LoopDrainFinishCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        check_audit(&command.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &command.transition).await?;
        check_scheduler_fence(&mut tx, &command.transition).await?;
        let owner_id = instance_step_id(
            run.version_id,
            command.transition.run_id(),
            command.transition.owner(),
        );
        let scope = sqlx::query(
            "SELECT lifecycle,active_iteration,drain_kind,drain_failure_code, \
                    finish_sequence,finish_outbox_id \
             FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 \
             FOR UPDATE",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(LoopProgressError::Missing)?;
        let lifecycle = scope.get::<String, _>("lifecycle");
        if matches!(lifecycle.as_str(), "FAILED" | "CANCELLED") {
            if scope.get::<Option<uuid::Uuid>, _>("finish_outbox_id") != Some(command.outbox_id) {
                return Err(LoopProgressError::DuplicateMismatch);
            }
            let sequence = u64::try_from(
                scope
                    .get::<Option<i64>, _>("finish_sequence")
                    .ok_or_else(|| invalid_storage("loop finish sequence"))?,
            )
            .map_err(|_| invalid_storage("loop finish sequence"))?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(LoopWrite {
                applied: false,
                run_version: sequence + 1,
            });
        }
        check_revision(&run, &command.transition)?;
        if run.status != "CANCELLING"
            || lifecycle != "DRAINING"
            || scope.get::<Option<i32>, _>("active_iteration").is_some()
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        let unsettled: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM loop_iterations \
             WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3 AND lifecycle='ADMITTED')",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let (owner_nodes, owner_indices) = path_parts(command.transition.owner());
        let other_active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND NOT (instance_path_nodes=$3 AND instance_path_indices=$4 AND node=$5) \
               AND status NOT IN ('SUCCEEDED','FAILED','CANCELLED','SKIPPED'))",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(command.transition.owner().node().as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let chained: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chain_segments WHERE tenant_id=$1 AND run_id=$2)",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if unsettled || other_active || chained {
            return Err(LoopProgressError::InvalidTransition);
        }
        let kind = scope
            .get::<Option<String>, _>("drain_kind")
            .ok_or_else(|| invalid_storage("loop drain reason"))?;
        let code = scope.get::<Option<String>, _>("drain_failure_code");
        let (detail, status, expected_reason) = match (kind.as_str(), code.as_deref()) {
            ("FAILED" | "TIMED_OUT", Some(code)) => {
                (json!({"outcome":"failed","code":code}), "FAILED", "FAILED")
            }
            ("CANCELLED", None) => (json!({"outcome":"cancelled"}), "CANCELLED", "CANCELLED"),
            _ => return Err(invalid_storage("loop drain reason")),
        };
        if run.stop_reason.as_deref() != Some(expected_reason)
            || (expected_reason == "FAILED" && run.stop_code.as_deref() != code.as_deref())
        {
            return Err(LoopProgressError::InvalidTransition);
        }
        let owner = sqlx::query(
            "UPDATE steps SET status=$6,waiting_kind=NULL,failure_code=$7,updated_at=$8 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND node=$5 \
               AND status='WAITING' AND waiting_kind='LOOP'",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(command.transition.owner().node().as_str())
        .bind(status)
        .bind(code.as_deref())
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if owner.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        let finish_sequence = run.run_version + 1;
        let final_sequence = finish_sequence + 1;
        let scope_updated = sqlx::query(
            "UPDATE loop_scopes SET lifecycle=$4,finish_sequence=$5,finish_outbox_id=$6, \
                    updated_at=$7 WHERE tenant_id=$1 AND run_id=$2 AND owner_instance_id=$3",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(owner_id)
        .bind(status)
        .bind(number(finish_sequence, "journal sequence")?)
        .bind(command.outbox_id)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if scope_updated.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        append_event(
            &mut tx,
            &command.transition,
            &run,
            finish_sequence,
            "loop.finished",
            detail,
        )
        .await?;
        append_run_event(
            &mut tx,
            &command.transition,
            final_sequence,
            "run.finished",
            json!({"status":status}),
        )
        .await?;
        let run_updated = sqlx::query(
            "UPDATE runs SET status=$3,run_version=$4,updated_at=$5,finished_at=$5 \
             WHERE tenant_id=$1 AND id=$2 AND status='CANCELLING'",
        )
        .bind(command.transition.tenant_id().as_uuid())
        .bind(command.transition.run_id().as_uuid())
        .bind(status)
        .bind(number(final_sequence, "run version")?)
        .bind(command.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if run_updated.rows_affected() != 1 {
            return Err(LoopProgressError::InvalidTransition);
        }
        insert_audit(&mut tx, command.transition.audit()).await?;
        insert_loop_hint(
            &mut tx,
            &command.transition,
            command.outbox_id,
            "loop.finished",
            status,
        )
        .await?;
        let definition = self
            .definition(
                &mut tx,
                command.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        outbox_run_finished(&mut tx, &command.transition, &definition, status).await?;
        mark_telemetry_pending(&mut tx, &command.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: final_sequence,
        })
    }
}

impl LoopProgressStore for PgRunStore {
    async fn loop_candidates(
        &self,
        window: &Window,
    ) -> Result<Vec<LoopCandidate>, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .loop_candidates(window)
            .await
    }

    async fn enter_loop(&self, command: &LoopEntryCommand) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .enter_loop(command)
            .await
    }

    async fn checkpoint(
        &self,
        command: &LoopCheckpointCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .checkpoint(command)
            .await
    }

    async fn fire_delay(
        &self,
        command: &LoopDelayElapsedCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .fire_delay(command)
            .await
    }

    async fn start_drain(
        &self,
        command: &LoopDrainCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .start_drain(command)
            .await
    }

    async fn drain_iteration(
        &self,
        command: &LoopIterationDrainCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .drain_iteration(command)
            .await
    }

    async fn finish_drain(
        &self,
        command: &LoopDrainFinishCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        PgLoopProgressStore::new(self.pool.clone(), self.cache.clone())
            .finish_drain(command)
            .await
    }
}
