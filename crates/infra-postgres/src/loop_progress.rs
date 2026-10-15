//! PostgreSQL transaction adapter for sequential-loop checkpoints and durable delays.
//!
//! Completion remains closed until the coordinator owns successor routing. Nonterminal checkpoints
//! and elapsed delays atomically update journal, projection, budgets, timer, audit and outbox.

use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::loop_progress::{
    LoopCheckpointCommand, LoopDelayElapsedCommand, LoopNextActivity, LoopProgressError,
    LoopProgressStore, LoopTransition, LoopWrite,
};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{ExecutionInstanceId, LoopContinuation};
use sagat_domain::identity::NamespaceId;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Row};

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

#[derive(Debug, Clone, Copy)]
struct LockedRun {
    namespace_id: NamespaceId,
    version_id: WorkflowVersionId,
    run_version: u64,
}

fn invalid_storage(detail: &'static str) -> LoopProgressError {
    LoopProgressError::Storage(StoreError::Internal {
        detail: detail.to_owned(),
    })
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
        "SELECT namespace_id, workflow_version_id, run_version, status FROM runs \
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
    if namespace_id != transition.namespace_id()
        || !matches!(status.as_str(), "RUNNING" | "WAITING")
    {
        return Err(LoopProgressError::InvalidTransition);
    }
    Ok(LockedRun {
        namespace_id,
        version_id: WorkflowVersionId::new(row.get("workflow_version_id")),
        run_version,
    })
}

fn check_revision(run: LockedRun, transition: &LoopTransition) -> Result<(), LoopProgressError> {
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
        "SELECT max_active_iterations FROM tenant_collection_policies \
         WHERE tenant_id = $1 FOR SHARE",
    )
    .bind(transition.tenant_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if requires_policy && tenant_limit.is_none() {
        return Err(LoopProgressError::PolicyMissing);
    }
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
    run: LockedRun,
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
    run: LockedRun,
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

async fn validate_payload(
    conn: &mut PgConnection,
    tenant_id: uuid::Uuid,
    digest: sagat_domain::workflow::Digest,
) -> Result<(), LoopProgressError> {
    let content: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant_id)
            .bind(digest.as_bytes().as_slice())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    if content
        .as_deref()
        .is_none_or(|bytes| !payload_matches(bytes, digest))
    {
        return Err(LoopProgressError::InvalidTransition);
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
    run: LockedRun,
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

#[allow(clippy::too_many_lines)]
impl LoopProgressStore for PgLoopProgressStore {
    async fn checkpoint(
        &self,
        command: &LoopCheckpointCommand,
    ) -> Result<LoopWrite, LoopProgressError> {
        if matches!(
            command.continuation,
            LoopContinuation::Complete | LoopContinuation::IterationLimit
        ) {
            return Err(LoopProgressError::RuntimeNotIntegrated);
        }
        if command.transition.audit().tenant_id() != command.transition.tenant_id() {
            return Err(LoopProgressError::AuditTenantMismatch);
        }
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
        check_revision(run, &command.transition)?;
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
        validate_payload(
            &mut tx,
            command.transition.tenant_id().as_uuid(),
            command.result_digest,
        )
        .await?;
        validate_payload(
            &mut tx,
            command.transition.tenant_id().as_uuid(),
            command.carry_digest,
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
        }

        let returned_sequence = run.run_version + 1;
        let checkpoint_sequence = returned_sequence + 1;
        let mut committed_version = checkpoint_sequence;
        if let Some(next) = &command.next_activity {
            insert_ready_activity(
                &mut tx,
                &command.transition,
                run,
                next_index,
                next,
                checkpoint_sequence + 1,
            )
            .await?;
            committed_version += 2;
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
        .bind(if command.next_activity.is_some() {
            "RUNNING"
        } else {
            "DELAYING"
        })
        .bind(command.carry_digest.as_bytes().as_slice())
        .bind(i32::try_from(next_index).unwrap_or(i32::MAX))
        .bind(
            command
                .next_activity
                .as_ref()
                .map(|_| i32::try_from(next_index).unwrap_or(i32::MAX)),
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
            run,
            returned_sequence,
            "loop.iteration_returned",
            json!({"index": command.index, "return_instance": instance_json(&command.return_instance)}),
        )
        .await?;
        append_event(
            &mut tx,
            &command.transition,
            run,
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
                run,
                checkpoint_sequence + 1,
                "loop.iteration_admitted",
                json!({"index": next_index, "entry": instance_json(&next.instance)}),
            )
            .await?;
            append_instance_event(
                &mut tx,
                &command.transition,
                run,
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
        finish_version(&mut tx, &command.transition, committed_version).await?;
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
        if command.transition.audit().tenant_id() != command.transition.tenant_id() {
            return Err(LoopProgressError::AuditTenantMismatch);
        }
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
        check_revision(run, &command.transition)?;
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
            run,
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
            run,
            elapsed_sequence,
            "loop.delay_elapsed",
            json!({"next_iteration": command.next_iteration}),
        )
        .await?;
        append_event(
            &mut tx,
            &command.transition,
            run,
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
            run,
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
        tx.commit().await.map_err(store_error)?;
        Ok(LoopWrite {
            applied: true,
            run_version: released_sequence,
        })
    }
}
