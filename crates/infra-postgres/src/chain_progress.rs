//! PostgreSQL transaction adapter for private execution-chain continuation.
//!
//! The adapter supports an activity entrypoint and unchanged retained input. More general input
//! evaluation, child-chain reconciliation and non-activity planning remain closed at the port.

use sagat_application::chain_progress::{
    ChainCommandStore, ChainFinalizationCommand, ChainFinalizationWrite, ChainHandoffCommand,
    ChainHandoffWrite, ChainInitialization, ChainInspection, ChainProgressError,
    ChainProgressStore, ChainSegmentView, ChildChainSettlementCommand, ChildChainSettlementWrite,
    LockedChainHead,
};
use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::runs::Lock;
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::execution::{
    ChainFinalOutcome, ChainScope, ChildOutcome, ContinuationTransfer, ExecutionInstanceId,
    RunEvent, RunId, RunState, RunStatus, SegmentCompletionKind,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::quota::{self, QuotaResource};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, NodeId, NodeKind, WorkflowDefinition};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Row};

use crate::event_codec::{decode, encode};
use crate::identity::{insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::{PgRunStore, PgRunTx};

#[derive(Debug, Clone)]
pub struct PgChainProgressStore {
    pool: PgPool,
    definitions: DefinitionCache,
}

impl PgChainProgressStore {
    #[must_use]
    pub fn new(pool: PgPool, definitions: DefinitionCache) -> Self {
        Self { pool, definitions }
    }

    /// Replays the stored root journal and compares its chain state with durable projections.
    ///
    /// This verifier is intentionally limited to root-only chains supported by this private
    /// adapter. A scoped journal fails closed until canonical graph restoration supports links.
    ///
    /// # Errors
    /// Returns [`ChainProgressError::CorruptProjection`] for a journal gap, undecodable or scoped
    /// event, invalid replay, or any projection difference.
    pub async fn verify_chain_journal(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
    ) -> Result<(), ChainProgressError> {
        let rows = sqlx::query(
            "SELECT seq,event_type,node,detail,instance_path_nodes,instance_path_indices \
             FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut events = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let expected = i64::try_from(index + 1).map_err(|_| corrupt("journal sequence"))?;
            let sequence: i64 = row.get("seq");
            let path_nodes: Option<Vec<String>> = row.get("instance_path_nodes");
            let path_indices: Option<Vec<i32>> = row.get("instance_path_indices");
            if sequence != expected
                || path_nodes.as_deref().is_some_and(|path| !path.is_empty())
                || path_indices.as_deref().is_some_and(|path| !path.is_empty())
            {
                return Err(ChainProgressError::CorruptProjection);
            }
            let event_type: String = row.get("event_type");
            let node: Option<String> = row.get("node");
            let detail: serde_json::Value = row.get("detail");
            events.push(
                decode(&event_type, node.as_deref(), &detail)
                    .ok_or(ChainProgressError::CorruptProjection)?,
            );
        }
        let replayed =
            RunState::replay(run_id, &events).map_err(|_| ChainProgressError::CorruptProjection)?;
        let projection = sqlx::query(
            "SELECT r.status,r.run_version,ec.chain_id,ec.max_segments,ec.original_deadline_at, \
                    cs.segment_index,cs.predecessor_run_id,cs.successor_run_id,cs.completion_kind, \
                    CASE WHEN cs.completion_kind='CONTINUED' THEN cs.transfer_input_digest \
                         ELSE incoming.transfer_input_digest END AS transfer_input_digest, \
                    CASE WHEN cs.completion_kind='CONTINUED' THEN cs.transfer_state_digest \
                         ELSE incoming.transfer_state_digest END AS transfer_state_digest \
             FROM runs r JOIN chain_segments cs ON cs.tenant_id=r.tenant_id AND cs.run_id=r.id \
             JOIN execution_chains ec ON ec.tenant_id=cs.tenant_id AND ec.chain_id=cs.chain_id \
             LEFT JOIN chain_segments incoming ON incoming.tenant_id=cs.tenant_id \
                  AND incoming.chain_id=cs.chain_id AND incoming.successor_run_id=cs.run_id \
             WHERE r.tenant_id=$1 AND r.id=$2",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        let chain = replayed
            .chain()
            .ok_or(ChainProgressError::CorruptProjection)?;
        let status: String = projection.get("status");
        let completion: Option<String> = projection.get("completion_kind");
        let projected_completion = match completion.as_deref() {
            None => None,
            Some("FINAL") => Some(SegmentCompletionKind::Final),
            Some("CONTINUED") => Some(SegmentCompletionKind::Continued),
            _ => return Err(ChainProgressError::CorruptProjection),
        };
        let input = projection
            .get::<Option<Vec<u8>>, _>("transfer_input_digest")
            .map(|value| digest(value, "chain transfer input"))
            .transpose()?;
        let state = projection
            .get::<Option<Vec<u8>>, _>("transfer_state_digest")
            .map(|value| digest(value, "chain transfer state"))
            .transpose()?;
        let transfer = input.map(|value| ContinuationTransfer::new(value, state));
        let projected_status = match status.as_str() {
            "RUNNING" => RunStatus::Running,
            "WAITING" => RunStatus::Waiting,
            "SUCCEEDED" => RunStatus::Succeeded,
            "FAILED" => RunStatus::Failed,
            "CANCELLED" => RunStatus::Cancelled,
            "TIMED_OUT" => RunStatus::TimedOut,
            "COMPENSATED" => RunStatus::Compensated,
            "PARTIAL" => RunStatus::Partial,
            "COMPENSATION_FAILED" => RunStatus::CompensationFailed,
            _ => return Err(ChainProgressError::CorruptProjection),
        };
        if replayed.status() != projected_status
            || replayed.version()
                != stored_u64(projection.get("run_version"), "projected run version")?
            || chain.chain_id().as_uuid() != projection.get::<uuid::Uuid, _>("chain_id")
            || i32::from(chain.max_segments()) != projection.get::<i32, _>("max_segments")
            || chain.original_deadline()
                != projection.get::<time::OffsetDateTime, _>("original_deadline_at")
            || i32::from(chain.segment_index().get()) != projection.get::<i32, _>("segment_index")
            || chain.predecessor().map(RunId::as_uuid)
                != projection.get::<Option<uuid::Uuid>, _>("predecessor_run_id")
            || chain.successor().map(RunId::as_uuid)
                != projection.get::<Option<uuid::Uuid>, _>("successor_run_id")
            || chain.completion_kind() != projected_completion
            || chain.transfer() != transfer.as_ref()
        {
            return Err(ChainProgressError::CorruptProjection);
        }
        Ok(())
    }

    async fn definition(
        &self,
        conn: &mut PgConnection,
        tenant_id: TenantId,
        version_id: sagat_domain::definition::WorkflowVersionId,
    ) -> Result<std::sync::Arc<WorkflowDefinition>, ChainProgressError> {
        if let Some(definition) = self.definitions.get(version_id) {
            return Ok(definition);
        }
        let document: Vec<u8> = sqlx::query_scalar(
            "SELECT document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(version_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        self.definitions
            .get_or_parse(version_id, &document)
            .map_err(|error| {
                tracing::error!(%error, %version_id, "stored continuation workflow no longer parses");
                corrupt("stored continuation workflow definition")
            })
    }
}

#[derive(Debug, Clone)]
struct LockedChain {
    namespace_id: NamespaceId,
    current_run_id: RunId,
    version_id: sagat_domain::definition::WorkflowVersionId,
    started_by: String,
    max_segments: u16,
    original_deadline: time::OffsetDateTime,
    segment_count: u16,
    revision: u64,
    cancel_requested: bool,
    final_outcome: Option<String>,
}

#[derive(Debug, Clone)]
struct LockedRun {
    workflow_id: sagat_domain::definition::WorkflowId,
    version_id: sagat_domain::definition::WorkflowVersionId,
    run_version: u64,
    input_digest: Digest,
    parent_run_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone)]
struct FinalizationRun {
    run_version: u64,
    status: String,
    finished_at: Option<time::OffsetDateTime>,
    trace_id: Option<String>,
}

fn corrupt(detail: &'static str) -> ChainProgressError {
    ChainProgressError::Storage(StoreError::Internal {
        detail: detail.to_owned(),
    })
}

fn stored_u64(value: i64, field: &'static str) -> Result<u64, ChainProgressError> {
    u64::try_from(value).map_err(|_| corrupt(field))
}

fn stored_u16(value: i32, field: &'static str) -> Result<u16, ChainProgressError> {
    u16::try_from(value).map_err(|_| corrupt(field))
}

fn number(value: u64, field: &'static str) -> Result<i64, ChainProgressError> {
    i64::try_from(value).map_err(|_| corrupt(field))
}

fn digest(bytes: Vec<u8>, field: &'static str) -> Result<Digest, ChainProgressError> {
    bytes
        .try_into()
        .map(Digest::new)
        .map_err(|_| corrupt(field))
}

fn validate_audit(command: &ChainHandoffCommand) -> Result<(), ChainProgressError> {
    let audit = command.audit();
    if audit.tenant_id() != command.scope().tenant_id()
        || audit.actor() != command.actor()
        || audit.action() != AuditAction::RunContinued
        || audit.resource_type() != ResourceType::Run
        || audit.resource_id() != command.expected_head().to_string()
        || audit.trace_id() != command.trace_id()
        || audit.created_at() != command.occurred_at()
    {
        return Err(ChainProgressError::AuditMismatch);
    }
    Ok(())
}

fn validate_finalization_audit(
    command: &ChainFinalizationCommand,
) -> Result<(), ChainProgressError> {
    let audit = command.audit();
    if audit.tenant_id() != command.scope().tenant_id()
        || audit.actor() != &Subject::engine()
        || audit.action() != AuditAction::RunFinished
        || audit.resource_type() != ResourceType::Run
        || audit.resource_id() != command.expected_head().to_string()
        || audit.created_at() != command.occurred_at()
    {
        return Err(ChainProgressError::AuditMismatch);
    }
    Ok(())
}

fn final_outcome(status: RunStatus) -> Result<ChainFinalOutcome, ChainProgressError> {
    match status {
        RunStatus::Succeeded => Ok(ChainFinalOutcome::Succeeded),
        RunStatus::Failed => Ok(ChainFinalOutcome::Failed),
        RunStatus::Cancelled => Ok(ChainFinalOutcome::Cancelled),
        RunStatus::TimedOut => Ok(ChainFinalOutcome::TimedOut),
        RunStatus::Compensated => Ok(ChainFinalOutcome::Compensated),
        RunStatus::Partial => Ok(ChainFinalOutcome::Partial),
        RunStatus::CompensationFailed => Ok(ChainFinalOutcome::CompensationFailed),
        RunStatus::Running
        | RunStatus::Waiting
        | RunStatus::Paused
        | RunStatus::Cancelling
        | RunStatus::Compensating
        | RunStatus::ManualReview => Err(ChainProgressError::RuntimeNotIntegrated),
    }
}

fn restore_final_outcome(value: Option<&str>) -> Result<Option<ChainFinalOutcome>, StoreError> {
    value
        .map(|value| match value {
            "SUCCEEDED" => Ok(ChainFinalOutcome::Succeeded),
            "FAILED" => Ok(ChainFinalOutcome::Failed),
            "CANCELLED" => Ok(ChainFinalOutcome::Cancelled),
            "TIMED_OUT" => Ok(ChainFinalOutcome::TimedOut),
            "COMPENSATED" => Ok(ChainFinalOutcome::Compensated),
            "PARTIAL" => Ok(ChainFinalOutcome::Partial),
            "COMPENSATION_FAILED" => Ok(ChainFinalOutcome::CompensationFailed),
            _ => Err(StoreError::Internal {
                detail: "stored chain final outcome is invalid".to_owned(),
            }),
        })
        .transpose()
}

async fn replay_root_journal(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    run_id: RunId,
) -> Result<RunState, ChainProgressError> {
    let rows = sqlx::query(
        "SELECT seq,event_type,node,detail,instance_path_nodes,instance_path_indices \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(tenant_id.as_uuid())
    .bind(run_id.as_uuid())
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let mut events = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let expected = i64::try_from(index + 1).map_err(|_| corrupt("journal sequence"))?;
        let path_nodes: Option<Vec<String>> = row.get("instance_path_nodes");
        let path_indices: Option<Vec<i32>> = row.get("instance_path_indices");
        if row.get::<i64, _>("seq") != expected
            || path_nodes.as_deref().is_some_and(|path| !path.is_empty())
            || path_indices.as_deref().is_some_and(|path| !path.is_empty())
        {
            return Err(ChainProgressError::CorruptProjection);
        }
        let event_type: String = row.get("event_type");
        let node: Option<String> = row.get("node");
        let detail: serde_json::Value = row.get("detail");
        events.push(
            decode(&event_type, node.as_deref(), &detail)
                .ok_or(ChainProgressError::CorruptProjection)?,
        );
    }
    RunState::replay(run_id, &events).map_err(|_| ChainProgressError::CorruptProjection)
}

async fn append_event(
    conn: &mut PgConnection,
    run_id: RunId,
    sequence: u64,
    event: &RunEvent,
    payload_digest: Option<Digest>,
    metadata: &JournalMetadata<'_>,
) -> Result<(), ChainProgressError> {
    let encoded = encode(event);
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, attempt, detail, payload_digest, actor, \
          trace_id, occurred_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(metadata.tenant_id.as_uuid())
    .bind(run_id.as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(encoded.event_type)
    .bind(encoded.node)
    .bind(encoded.attempt)
    .bind(encoded.detail)
    .bind(payload_digest.map(|value| value.as_bytes().to_vec()))
    .bind(metadata.actor)
    .bind(metadata.trace_id)
    .bind(metadata.occurred_at)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct JournalMetadata<'a> {
    tenant_id: TenantId,
    actor: &'a str,
    trace_id: Option<&'a str>,
    occurred_at: time::OffsetDateTime,
}

async fn mark_telemetry(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    run_id: RunId,
    head: u64,
) -> Result<(), ChainProgressError> {
    sqlx::query(
        "INSERT INTO telemetry_pending (tenant_id, run_id, head_seq, applied_seq, behind_since) \
         VALUES ($1,$2,$3,0,clock_timestamp()) ON CONFLICT (tenant_id, run_id) DO UPDATE SET \
           behind_since = CASE WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
                               THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
           head_seq = GREATEST(telemetry_pending.head_seq, EXCLUDED.head_seq)",
    )
    .bind(tenant_id.as_uuid())
    .bind(run_id.as_uuid())
    .bind(number(head, "telemetry head")?)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn validate_payload(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    value: Digest,
) -> Result<Vec<u8>, ChainProgressError> {
    let content: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant_id.as_uuid())
            .bind(value.as_bytes().as_slice())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    let content = content.ok_or(ChainProgressError::PayloadMissing)?;
    if Sha256::digest(&content).as_slice() != value.as_bytes() {
        return Err(ChainProgressError::CorruptProjection);
    }
    Ok(content)
}

async fn lock_chain(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<LockedChain, ChainProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id, current_run_id, definition_version_id, started_by, max_segments, \
                original_deadline_at, segment_count, revision, cancel_requested, final_outcome \
         FROM execution_chains WHERE tenant_id = $1 AND chain_id = $2 FOR UPDATE",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.chain_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Missing)?;
    if row.get::<uuid::Uuid, _>("namespace_id") != command.scope().namespace_id().as_uuid() {
        return Err(ChainProgressError::Missing);
    }
    Ok(LockedChain {
        namespace_id: NamespaceId::new(row.get("namespace_id")),
        current_run_id: RunId::new(row.get("current_run_id")),
        version_id: sagat_domain::definition::WorkflowVersionId::new(
            row.get("definition_version_id"),
        ),
        started_by: row.get("started_by"),
        max_segments: stored_u16(row.get("max_segments"), "chain max segments")?,
        original_deadline: row.get("original_deadline_at"),
        segment_count: stored_u16(row.get("segment_count"), "chain segment count")?,
        revision: stored_u64(row.get("revision"), "chain revision")?,
        cancel_requested: row.get("cancel_requested"),
        final_outcome: row.get("final_outcome"),
    })
}

async fn lock_run(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<LockedRun, ChainProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id, workflow_id, workflow_version_id, status, run_version, input_digest, \
                deadline_at, parent_run_id, stop_reason, paused \
         FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.expected_head().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Missing)?;
    if row.get::<uuid::Uuid, _>("namespace_id") != command.scope().namespace_id().as_uuid() {
        return Err(ChainProgressError::Missing);
    }
    let status: String = row.get("status");
    let paused: bool = row.get("paused");
    if !matches!(status.as_str(), "RUNNING" | "WAITING" | "SUCCEEDED")
        || row.get::<Option<String>, _>("stop_reason").is_some()
        || paused
    {
        return Err(ChainProgressError::RuntimeNotIntegrated);
    }
    Ok(LockedRun {
        workflow_id: sagat_domain::definition::WorkflowId::new(row.get("workflow_id")),
        version_id: sagat_domain::definition::WorkflowVersionId::new(
            row.get("workflow_version_id"),
        ),
        run_version: stored_u64(row.get("run_version"), "run version")?,
        input_digest: digest(row.get("input_digest"), "run input digest")?,
        parent_run_id: row.get("parent_run_id"),
    })
}

async fn lock_finalization_chain(
    conn: &mut PgConnection,
    command: &ChainFinalizationCommand,
) -> Result<LockedChain, ChainProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id,current_run_id,definition_version_id,started_by,max_segments, \
                original_deadline_at,segment_count,revision,cancel_requested,final_outcome \
         FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.chain_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Missing)?;
    if row.get::<uuid::Uuid, _>("namespace_id") != command.scope().namespace_id().as_uuid() {
        return Err(ChainProgressError::Missing);
    }
    Ok(LockedChain {
        namespace_id: NamespaceId::new(row.get("namespace_id")),
        current_run_id: RunId::new(row.get("current_run_id")),
        version_id: sagat_domain::definition::WorkflowVersionId::new(
            row.get("definition_version_id"),
        ),
        started_by: row.get("started_by"),
        max_segments: stored_u16(row.get("max_segments"), "chain max segments")?,
        original_deadline: row.get("original_deadline_at"),
        segment_count: stored_u16(row.get("segment_count"), "chain segment count")?,
        revision: stored_u64(row.get("revision"), "chain revision")?,
        cancel_requested: row.get("cancel_requested"),
        final_outcome: row.get("final_outcome"),
    })
}

async fn lock_finalization_run(
    conn: &mut PgConnection,
    command: &ChainFinalizationCommand,
) -> Result<FinalizationRun, ChainProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id,status,run_version,finished_at,trace_id FROM runs \
         WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.expected_head().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Missing)?;
    if row.get::<uuid::Uuid, _>("namespace_id") != command.scope().namespace_id().as_uuid() {
        return Err(ChainProgressError::Missing);
    }
    Ok(FinalizationRun {
        run_version: stored_u64(row.get("run_version"), "run version")?,
        status: row.get("status"),
        finished_at: row.get("finished_at"),
        trace_id: row.get("trace_id"),
    })
}

async fn check_scheduler_fence(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<(), ChainProgressError> {
    check_fence(conn, command.scheduler(), command.expected_head()).await
}

async fn check_fence(
    conn: &mut PgConnection,
    authority: &sagat_application::primitives::SchedulerAuthority,
    run_id: RunId,
) -> Result<(), ChainProgressError> {
    let lease = authority.lease();
    if lease.shard != Shard::of(run_id.as_uuid()) {
        return Err(ChainProgressError::SchedulerFenceLost);
    }
    let held: Option<i16> = sqlx::query_scalar(
        "SELECT shard FROM scheduler_shards WHERE shard = $1 AND generation = $2 AND owner = $3 \
         AND expires_at > clock_timestamp() FOR SHARE",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(number(lease.generation, "scheduler generation")?)
    .bind(authority.owner())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if held.is_none() {
        return Err(ChainProgressError::SchedulerFenceLost);
    }
    Ok(())
}

async fn child_link_cancelled(
    conn: &mut PgConnection,
    scope: ChainScope,
    chain_id: RunId,
) -> Result<bool, ChainProgressError> {
    let cancelled: Option<bool> = sqlx::query_scalar(
        "SELECT cancel_requested FROM child_chain_links \
         WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3 FOR UPDATE",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(chain_id.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(cancelled.unwrap_or(false))
}

async fn quota_lock(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    namespace_id: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<(), ChainProgressError> {
    let key = namespace_id.map_or_else(
        || format!("quota:{resource}:{tenant_id}"),
        |namespace| format!("quota:{resource}:{tenant_id}:{namespace}"),
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn effective_limit(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    namespace_id: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<u64, ChainProgressError> {
    let rows: Vec<(Option<uuid::Uuid>, i64, bool)> = sqlx::query_as(
        "SELECT namespace_id, max_value, false FROM quota_limits \
         WHERE tenant_id = $1 AND resource = $2 AND (namespace_id IS NULL OR namespace_id = $3) \
         UNION ALL SELECT NULL, max_value, true FROM quota_ceilings \
         WHERE tenant_id = $1 AND resource = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(resource.as_str())
    .bind(namespace_id.map(NamespaceId::as_uuid))
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let configured = |scoped: bool| {
        rows.iter()
            .find(|(namespace, _, ceiling)| !ceiling && namespace.is_some() == scoped)
            .map(|(_, value, _)| u64::try_from(*value).unwrap_or(0))
    };
    let granted = rows
        .iter()
        .find(|(_, _, ceiling)| *ceiling)
        .map(|(_, value, _)| u64::try_from(*value).unwrap_or(0));
    Ok(quota::resolve(
        resource,
        configured(true),
        configured(false),
        quota::ceiling(resource, granted),
    )
    .0)
}

async fn ensure_replacement_capacity(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<(), ChainProgressError> {
    let tenant = command.scope().tenant_id();
    let namespace = command.scope().namespace_id();
    quota_lock(conn, tenant, Some(namespace), QuotaResource::ActiveRuns).await?;
    let namespace_limit =
        effective_limit(conn, tenant, Some(namespace), QuotaResource::ActiveRuns).await?;
    quota_lock(conn, tenant, None, QuotaResource::TenantActiveRuns).await?;
    let tenant_limit = effective_limit(conn, tenant, None, QuotaResource::TenantActiveRuns).await?;
    let namespace_active: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts \
         WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let tenant_active: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let namespace_after_replacement = namespace_active.max(1) as u64;
    let tenant_after_replacement = tenant_active.max(1) as u64;
    if namespace_after_replacement > namespace_limit || tenant_after_replacement > tenant_limit {
        return Err(ChainProgressError::AtCapacity);
    }
    Ok(())
}

async fn segment_quiescent(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<bool, ChainProgressError> {
    sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 \
           AND (status <> 'SUCCEEDED' OR routed = false))",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.expected_head().as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)
    .map_err(ChainProgressError::from)
}

/// Adds chain metadata and its replay event inside the transaction that owns the initial run.
/// Dropping or rolling back `conn` therefore removes the run and its chain together.
pub async fn initialize_chain_in_transaction(
    conn: &mut PgConnection,
    initialization: &ChainInitialization,
) -> Result<(), ChainProgressError> {
    let scope = initialization.scope();
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, started_by, deadline_at, run_version, created_at \
         FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(initialization.initial_run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Missing)?;
    if row.get::<uuid::Uuid, _>("namespace_id") != scope.namespace_id().as_uuid() {
        return Err(ChainProgressError::Missing);
    }
    let deadline: Option<time::OffsetDateTime> = row.get("deadline_at");
    let deadline = deadline.ok_or(ChainProgressError::RuntimeNotIntegrated)?;
    let version = sagat_domain::definition::WorkflowVersionId::new(row.get("workflow_version_id"));
    let started_by: String = row.get("started_by");
    let created_at: time::OffsetDateTime = row.get("created_at");
    let run_version = stored_u64(row.get("run_version"), "initial run version")?;
    if run_version == 0 {
        return Err(ChainProgressError::RuntimeNotIntegrated);
    }
    sqlx::query(
        "INSERT INTO execution_chains \
         (tenant_id, namespace_id, chain_id, current_run_id, definition_version_id, started_by, \
          max_segments, original_deadline_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$3,$4,$5,$6,$7,$8,$8)",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(initialization.initial_run_id().as_uuid())
    .bind(version.as_uuid())
    .bind(&started_by)
    .bind(i32::from(initialization.policy().max_segments()))
    .bind(deadline)
    .bind(created_at)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    sqlx::query(
        "INSERT INTO chain_segments \
         (tenant_id, namespace_id, chain_id, segment_index, run_id, created_at) \
         VALUES ($1,$2,$3,0,$3,$4)",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(initialization.initial_run_id().as_uuid())
    .bind(created_at)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    let next_version = run_version
        .checked_add(1)
        .ok_or_else(|| corrupt("initial run version exhausted"))?;
    let metadata = JournalMetadata {
        tenant_id: scope.tenant_id(),
        actor: &started_by,
        trace_id: None,
        occurred_at: created_at,
    };
    append_event(
        conn,
        initialization.initial_run_id(),
        next_version,
        &RunEvent::ExecutionChainStarted {
            max_segments: initialization.policy().max_segments(),
            original_deadline: deadline,
        },
        None,
        &metadata,
    )
    .await?;
    let changed = sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = GREATEST(updated_at, $4) \
         WHERE tenant_id = $1 AND id = $2 AND run_version = $5",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(initialization.initial_run_id().as_uuid())
    .bind(number(next_version, "initial chain sequence")?)
    .bind(created_at)
    .bind(number(run_version, "initial run version")?)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?
    .rows_affected();
    if changed != 1 {
        return Err(ChainProgressError::CorruptProjection);
    }
    mark_telemetry(
        conn,
        scope.tenant_id(),
        initialization.initial_run_id(),
        next_version,
    )
    .await
}

async fn replay_handoff(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
    chain: &LockedChain,
) -> Result<ChainHandoffWrite, ChainProgressError> {
    let row = sqlx::query(
        "SELECT segment_index, successor_run_id, transfer_input_digest, transfer_state_digest \
         FROM chain_segments WHERE tenant_id = $1 AND namespace_id = $2 AND chain_id = $3 \
           AND run_id = $4 AND completion_kind = 'CONTINUED'",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.scope().namespace_id().as_uuid())
    .bind(command.chain_id().as_uuid())
    .bind(command.expected_head().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(ChainProgressError::Chain(
        sagat_domain::execution::ChainError::HeadChanged {
            current: chain.current_run_id,
        },
    ))?;
    let successor = RunId::new(row.get("successor_run_id"));
    let input = digest(row.get("transfer_input_digest"), "transfer input digest")?;
    let state = row
        .get::<Option<Vec<u8>>, _>("transfer_state_digest")
        .map(|value| digest(value, "transfer state digest"))
        .transpose()?;
    let outbox_matches: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM event_outbox WHERE tenant_id = $1 AND id = $2 \
           AND aggregate_id = $3 AND event_type = 'run.continued' \
           AND payload ->> 'successorRunId' = $4)",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.continuation_outbox_id())
    .bind(command.expected_head().as_uuid())
    .bind(command.successor().to_string())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if successor != command.successor()
        || input != command.transfer().input()
        || state != command.transfer().state()
        || !outbox_matches
    {
        return Err(ChainProgressError::Chain(
            sagat_domain::execution::ChainError::ContinuationConflict,
        ));
    }
    let successor_index = stored_u16(row.get("segment_index"), "segment index")?
        .checked_add(1)
        .ok_or_else(|| corrupt("successor segment index"))?;
    Ok(ChainHandoffWrite::new(
        command.expected_head(),
        successor,
        successor_index,
        true,
    ))
}

#[allow(clippy::too_many_lines)]
impl ChainProgressStore for PgChainProgressStore {
    async fn continue_as_new(
        &self,
        command: &ChainHandoffCommand,
    ) -> Result<ChainHandoffWrite, ChainProgressError> {
        validate_audit(command)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let chain = lock_chain(&mut tx, command).await?;
        let run = lock_run(&mut tx, command).await?;
        check_scheduler_fence(&mut tx, command).await?;
        if chain.current_run_id != command.expected_head() {
            let result = replay_handoff(&mut tx, command, &chain).await?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(result);
        }
        if child_link_cancelled(&mut tx, command.scope(), command.chain_id()).await? {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::ChildCancellationRequested,
            ));
        }
        if chain.namespace_id != command.scope().namespace_id()
            || chain.version_id != run.version_id
            || chain.revision != command.expected_chain_revision()
            || run.run_version != command.expected_run_version()
            || chain.cancel_requested
            || chain.final_outcome.is_some()
            || chain.segment_count >= chain.max_segments
            || chain.original_deadline <= command.occurred_at()
            || run.parent_run_id.is_some()
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        if !segment_quiescent(&mut tx, command).await? {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::NotQuiescent(
                    sagat_domain::execution::QuiescenceBlocker::SiblingWork,
                ),
            ));
        }
        ensure_replacement_capacity(&mut tx, command).await?;
        validate_payload(
            &mut tx,
            command.scope().tenant_id(),
            command.transfer().input(),
        )
        .await?;
        if command.transfer().input() != run.input_digest {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        let state_row = sqlx::query(
            "SELECT state_digest, state_schema_digest, state_size_bytes, state_revision \
             FROM workflow_operation_states WHERE tenant_id = $1 AND namespace_id = $2 \
               AND run_id = $3 FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        match (command.transfer().state(), state_row.as_ref()) {
            (None, None) => {}
            (Some(expected), Some(row)) => {
                validate_payload(&mut tx, command.scope().tenant_id(), expected).await?;
                if digest(row.get("state_digest"), "operation state digest")? != expected {
                    return Err(ChainProgressError::RuntimeNotIntegrated);
                }
            }
            _ => return Err(ChainProgressError::RuntimeNotIntegrated),
        }
        let definition = self
            .definition(&mut tx, command.scope().tenant_id(), run.version_id)
            .await?;
        let entrypoint = definition.entrypoint().clone();
        let activity = match definition.node(&entrypoint).map(|node| node.kind()) {
            Some(NodeKind::Activity { activity, .. }) => activity.clone(),
            _ => return Err(ChainProgressError::RuntimeNotIntegrated),
        };
        let successor_index = chain.segment_count;
        let old_sequence = run
            .run_version
            .checked_add(1)
            .ok_or_else(|| corrupt("predecessor journal exhausted"))?;
        let metadata = JournalMetadata {
            tenant_id: command.scope().tenant_id(),
            actor: command.actor().as_str(),
            trace_id: command.trace_id(),
            occurred_at: command.occurred_at(),
        };
        append_event(
            &mut tx,
            command.expected_head(),
            old_sequence,
            &RunEvent::RunContinued {
                successor: command.successor(),
                input_digest: command.transfer().input(),
                state_digest: command.transfer().state(),
            },
            Some(command.transfer().input()),
            &metadata,
        )
        .await?;
        let old_changed = sqlx::query(
            "UPDATE runs SET status = 'SUCCEEDED', run_version = $3, updated_at = $4, finished_at = $4 \
             WHERE tenant_id = $1 AND id = $2 AND run_version = $5 AND finished_at IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(number(old_sequence, "predecessor sequence")?)
        .bind(command.occurred_at())
        .bind(number(run.run_version, "predecessor run version")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if old_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        sqlx::query(
            "INSERT INTO runs \
             (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
              paused, input_digest, started_by, trace_id, deadline_at, created_at, updated_at, \
              root_run_id, depth) \
             VALUES ($1,$2,$3,$4,$5,'RUNNING',3,false,$6,$7,$8,$9,$10,$10,$1,0)",
        )
        .bind(command.successor().as_uuid())
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(run.workflow_id.as_uuid())
        .bind(run.version_id.as_uuid())
        .bind(command.transfer().input().as_bytes().as_slice())
        .bind(&chain.started_by)
        .bind(command.trace_id())
        .bind(chain.original_deadline)
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let entry_instance = ExecutionInstanceId::root(entrypoint.clone());
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
              last_hinted_at, hint_count, updated_at, instance_path_nodes, instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'READY',$6,$7,$7,1,$7,'{}','{}')",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.successor().as_uuid())
        .bind(entrypoint.as_str())
        .bind(instance_step_id(
            run.version_id,
            command.successor(),
            &entry_instance,
        ))
        .bind(command.scope().namespace_id().as_uuid())
        .bind(activity.as_str())
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        for (sequence, event) in [
            (
                1,
                RunEvent::RunStarted {
                    entrypoint: entrypoint.clone(),
                },
            ),
            (
                2,
                RunEvent::ExecutionChainSuccessorLinked {
                    chain_id: command.chain_id(),
                    predecessor: command.expected_head(),
                    segment_index: successor_index,
                    max_segments: chain.max_segments,
                    original_deadline: chain.original_deadline,
                    input_digest: command.transfer().input(),
                    state_digest: command.transfer().state(),
                },
            ),
            (
                3,
                RunEvent::StepReleased {
                    node: entrypoint.clone(),
                },
            ),
        ] {
            append_event(
                &mut tx,
                command.successor(),
                sequence,
                &event,
                None,
                &metadata,
            )
            .await?;
        }
        if let (Some(state_digest), Some(state_row)) = (command.transfer().state(), state_row) {
            sqlx::query(
                "INSERT INTO workflow_operation_states \
                 (tenant_id, namespace_id, run_id, state_digest, state_schema_digest, \
                  state_size_bytes, state_revision, last_changed_run_version, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,2,$8)",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.scope().namespace_id().as_uuid())
            .bind(command.successor().as_uuid())
            .bind(state_digest.as_bytes().as_slice())
            .bind(state_row.get::<Vec<u8>, _>("state_schema_digest"))
            .bind(state_row.get::<i32, _>("state_size_bytes"))
            .bind(state_row.get::<i64, _>("state_revision"))
            .bind(command.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let segment_changed = sqlx::query(
            "UPDATE chain_segments SET completion_kind = 'CONTINUED', successor_run_id = $5, \
                    transfer_input_digest = $6, transfer_state_digest = $7, completed_at = $8 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND chain_id = $3 AND run_id = $4 \
               AND completion_kind IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(command.successor().as_uuid())
        .bind(command.transfer().input().as_bytes().as_slice())
        .bind(
            command
                .transfer()
                .state()
                .map(|value| value.as_bytes().to_vec()),
        )
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if segment_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        let segment_inserted = sqlx::query(
            "INSERT INTO chain_segments \
             (tenant_id, namespace_id, chain_id, segment_index, run_id, predecessor_run_id, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .bind(i32::from(successor_index))
        .bind(command.successor().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if segment_inserted != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        let chain_changed = sqlx::query(
            "UPDATE execution_chains SET current_run_id = $4, segment_count = segment_count + 1, \
                    revision = revision + 1, updated_at = $5 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND chain_id = $3 \
               AND current_run_id = $6 AND revision = $7",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .bind(command.successor().as_uuid())
        .bind(command.occurred_at())
        .bind(command.expected_head().as_uuid())
        .bind(number(chain.revision, "chain revision")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if chain_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES ($1,$2,$3,'run.continued',$4)",
        )
        .bind(command.continuation_outbox_id())
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(json!({
            "tenantId": command.scope().tenant_id().to_string(),
            "namespaceId": command.scope().namespace_id().to_string(),
            "chainId": command.chain_id().to_string(),
            "predecessorRunId": command.expected_head().to_string(),
            "successorRunId": command.successor().to_string(),
            "traceId": command.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO event_outbox \
             (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1,$2,$3,'task.ready',$4,$5)",
        )
        .bind(command.successor_outbox_id())
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.successor().as_uuid())
        .bind(activity.as_str())
        .bind(json!({
            "tenantId": command.scope().tenant_id().to_string(),
            "namespaceId": command.scope().namespace_id().to_string(),
            "runId": command.successor().to_string(),
            "node": entrypoint.as_str(),
            "instancePath": [],
            "activity": activity.as_str(),
            "traceId": command.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, command.audit()).await?;
        mark_telemetry(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_head(),
            old_sequence,
        )
        .await?;
        mark_telemetry(&mut tx, command.scope().tenant_id(), command.successor(), 3).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(ChainHandoffWrite::new(
            command.expected_head(),
            command.successor(),
            successor_index,
            false,
        ))
    }

    async fn finalize_chain(
        &self,
        command: &ChainFinalizationCommand,
    ) -> Result<ChainFinalizationWrite, ChainProgressError> {
        validate_finalization_audit(command)?;
        let outcome = final_outcome(command.status())?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let chain = lock_finalization_chain(&mut tx, command).await?;
        if chain.current_run_id != command.expected_head() {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::HeadChanged {
                    current: chain.current_run_id,
                },
            ));
        }
        let run = lock_finalization_run(&mut tx, command).await?;
        check_fence(&mut tx, command.scheduler(), command.expected_head()).await?;
        if command.audit().trace_id() != run.trace_id.as_deref() {
            return Err(ChainProgressError::AuditMismatch);
        }
        if let Some(stored) = chain.final_outcome.as_deref() {
            let outboxes_match: bool = sqlx::query_scalar(
                "SELECT (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND \
                    ((id=$2 AND aggregate_id=$4 AND event_type='run.finished') OR \
                     (id=$3 AND aggregate_id=$5 AND event_type='chain.finished'))) = 2",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.run_outbox_id())
            .bind(command.chain_outbox_id())
            .bind(command.expected_head().as_uuid())
            .bind(command.chain_id().as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            let audit_matches: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM audit_records WHERE tenant_id=$1 AND id=$2 \
                   AND actor=$3 AND action='run.finished' AND resource_type='run' \
                   AND resource_id=$4 AND trace_id IS NOT DISTINCT FROM $5 AND created_at=$6)",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.audit().id().as_uuid())
            .bind(command.audit().actor().as_str())
            .bind(command.expected_head().to_string())
            .bind(command.audit().trace_id())
            .bind(command.occurred_at())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if stored == command.status().as_str()
                && run.status == command.status().as_str()
                && run.finished_at.is_some()
                && outboxes_match
                && audit_matches
            {
                tx.rollback().await.map_err(store_error)?;
                return Ok(ChainFinalizationWrite::new(
                    command.expected_head(),
                    outcome,
                    true,
                ));
            }
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::FinalOutcomeConflict,
            ));
        }
        if chain.revision != command.expected_chain_revision()
            || run.run_version != command.expected_run_version()
            || run.finished_at.is_some()
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        let child = sqlx::query(
            "SELECT final_outcome,settled FROM child_chain_links \
             WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3 FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if let Some(row) = child.as_ref() {
            let linked_outcome: Option<String> = row.get("final_outcome");
            if linked_outcome
                .as_deref()
                .is_some_and(|value| value != command.status().as_str())
                || row.get::<bool, _>("settled")
            {
                return Err(ChainProgressError::CorruptProjection);
            }
        }
        let mut replayed = replay_root_journal(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_head(),
        )
        .await?;
        if replayed.version() != run.run_version || replayed.status().as_str() != run.status {
            return Err(ChainProgressError::CorruptProjection);
        }
        replayed
            .apply(&RunEvent::RunFinished {
                status: command.status(),
            })
            .map_err(|_| ChainProgressError::RuntimeNotIntegrated)?;
        let sequence = run
            .run_version
            .checked_add(1)
            .ok_or_else(|| corrupt("final journal sequence"))?;
        let metadata = JournalMetadata {
            tenant_id: command.scope().tenant_id(),
            actor: command.scheduler().owner(),
            trace_id: run.trace_id.as_deref(),
            occurred_at: command.occurred_at(),
        };
        append_event(
            &mut tx,
            command.expected_head(),
            sequence,
            &RunEvent::RunFinished {
                status: command.status(),
            },
            None,
            &metadata,
        )
        .await?;
        let run_changed = sqlx::query(
            "UPDATE runs SET status=$3,run_version=$4,updated_at=$5,finished_at=$5 \
             WHERE tenant_id=$1 AND id=$2 AND run_version=$6 AND finished_at IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(command.status().as_str())
        .bind(number(sequence, "final run version")?)
        .bind(command.occurred_at())
        .bind(number(run.run_version, "expected final run version")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let segment_changed = sqlx::query(
            "UPDATE chain_segments SET completion_kind='FINAL',completed_at=$5 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND run_id=$4 \
               AND completion_kind IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let chain_changed = sqlx::query(
            "UPDATE execution_chains SET final_outcome=$4,revision=revision+1,updated_at=$5 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND current_run_id=$6 \
               AND revision=$7 AND final_outcome IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.chain_id().as_uuid())
        .bind(command.status().as_str())
        .bind(command.occurred_at())
        .bind(command.expected_head().as_uuid())
        .bind(number(chain.revision, "final chain revision")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if run_changed != 1 || segment_changed != 1 || chain_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        if child.is_some() {
            let linked = sqlx::query(
                "UPDATE child_chain_links SET final_outcome=$4,updated_at=$5 \
                 WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3 \
                   AND final_outcome IS NULL AND settled=false",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.scope().namespace_id().as_uuid())
            .bind(command.chain_id().as_uuid())
            .bind(command.status().as_str())
            .bind(command.occurred_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if linked != 1 {
                return Err(ChainProgressError::CorruptProjection);
            }
        }
        sqlx::query(
            "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
             VALUES ($1,$2,$3,'run.finished',$4),($5,$2,$6,'chain.finished',$7)",
        )
        .bind(command.run_outbox_id())
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(json!({
            "tenantId": command.scope().tenant_id().to_string(),
            "namespaceId": command.scope().namespace_id().to_string(),
            "runId": command.expected_head().to_string(),
            "status": command.status().as_str(),
            "traceId": run.trace_id,
        }))
        .bind(command.chain_outbox_id())
        .bind(command.chain_id().as_uuid())
        .bind(json!({
            "tenantId": command.scope().tenant_id().to_string(),
            "namespaceId": command.scope().namespace_id().to_string(),
            "chainId": command.chain_id().to_string(),
            "headRunId": command.expected_head().to_string(),
            "status": command.status().as_str(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, command.audit()).await?;
        mark_telemetry(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_head(),
            sequence,
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(ChainFinalizationWrite::new(
            command.expected_head(),
            outcome,
            false,
        ))
    }

    async fn settle_child_chain(
        &self,
        command: &ChildChainSettlementCommand,
    ) -> Result<ChildChainSettlementWrite, ChainProgressError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let parent_chain = sqlx::query(
            "SELECT namespace_id,current_run_id,revision,final_outcome FROM execution_chains \
             WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.parent_chain_id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        if parent_chain.get::<uuid::Uuid, _>("namespace_id")
            != command.scope().namespace_id().as_uuid()
        {
            return Err(ChainProgressError::Missing);
        }
        let current = RunId::new(parent_chain.get("current_run_id"));
        if current != command.expected_parent_head() {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::HeadChanged { current },
            ));
        }
        if parent_chain
            .get::<Option<String>, _>("final_outcome")
            .is_some()
            || stored_u64(parent_chain.get("revision"), "parent chain revision")?
                != command.expected_parent_chain_revision()
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        let parent = sqlx::query(
            "SELECT namespace_id,workflow_version_id,status,run_version,finished_at,trace_id \
             FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        if parent.get::<uuid::Uuid, _>("namespace_id") != command.scope().namespace_id().as_uuid() {
            return Err(ChainProgressError::Missing);
        }
        check_fence(&mut tx, command.scheduler(), command.expected_parent_head()).await?;
        let link = sqlx::query(
            "SELECT parent_instance_id,final_outcome,settled FROM child_chain_links \
             WHERE tenant_id=$1 AND namespace_id=$2 AND parent_run_id=$3 AND child_chain_id=$4 \
             FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(command.child_chain_id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        let parent_instance: uuid::Uuid = link.get("parent_instance_id");
        let outcome: Option<String> = link.get("final_outcome");
        if outcome.as_deref() != Some("SUCCEEDED") {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        if link.get::<bool, _>("settled") {
            let replayed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM event_outbox WHERE tenant_id=$1 AND id=$2 \
                   AND aggregate_id=$3 AND event_type='task.ready')",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.task_outbox_id())
            .bind(command.expected_parent_head().as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if replayed {
                tx.rollback().await.map_err(store_error)?;
                return Ok(ChildChainSettlementWrite::new(
                    command.expected_parent_head(),
                    command.child_chain_id(),
                    true,
                ));
            }
            return Err(ChainProgressError::CorruptProjection);
        }
        let parent_version = stored_u64(parent.get("run_version"), "parent run version")?;
        if parent_version != command.expected_parent_run_version()
            || parent
                .get::<Option<time::OffsetDateTime>, _>("finished_at")
                .is_some()
            || !matches!(
                parent.get::<String, _>("status").as_str(),
                "RUNNING" | "WAITING"
            )
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        let step = sqlx::query(
            "SELECT node,instance_path_nodes,instance_path_indices,child_run_id,status,waiting_kind \
             FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 FOR UPDATE",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(parent_instance)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(ChainProgressError::Missing)?;
        let path_nodes: Vec<String> = step.get("instance_path_nodes");
        let path_indices: Vec<i32> = step.get("instance_path_indices");
        if !path_nodes.is_empty()
            || !path_indices.is_empty()
            || step.get::<Option<uuid::Uuid>, _>("child_run_id")
                != Some(command.child_chain_id().as_uuid())
            || step.get::<String, _>("status") != "WAITING"
            || step.get::<Option<String>, _>("waiting_kind").as_deref() != Some("CHILD")
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        let child_node = NodeId::parse_stored(&step.get::<String, _>("node"))
            .map_err(|_| ChainProgressError::CorruptProjection)?;
        let version =
            sagat_domain::definition::WorkflowVersionId::new(parent.get("workflow_version_id"));
        let definition = self
            .definition(&mut tx, command.scope().tenant_id(), version)
            .await?;
        let mut state = replay_root_journal(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_parent_head(),
        )
        .await?;
        if state.version() != parent_version {
            return Err(ChainProgressError::CorruptProjection);
        }
        let progress = sagat_domain::execution::finish_child(
            &definition,
            &mut state,
            &child_node,
            command.child_chain_id(),
            ChildOutcome::Succeeded,
            command.occurred_at(),
        )
        .map_err(|_| ChainProgressError::RuntimeNotIntegrated)?;
        let [
            RunEvent::ChildFinished {
                node: finished,
                child,
                outcome: ChildOutcome::Succeeded,
            },
            RunEvent::StepRouted {
                node: routed,
                activated,
            },
            RunEvent::StepReleased { node: released },
        ] = progress.events.as_slice()
        else {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        };
        let [successor] = activated.as_slice() else {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        };
        if finished != &child_node
            || routed != &child_node
            || *child != command.child_chain_id()
            || released != successor
            || progress.pending.is_some()
        {
            return Err(ChainProgressError::CorruptProjection);
        }
        let activity = match definition.node(successor).map(|node| node.kind()) {
            Some(NodeKind::Activity { activity, .. }) => activity.clone(),
            _ => return Err(ChainProgressError::RuntimeNotIntegrated),
        };
        let trace_id: Option<String> = parent.get("trace_id");
        let metadata = JournalMetadata {
            tenant_id: command.scope().tenant_id(),
            actor: command.scheduler().owner(),
            trace_id: trace_id.as_deref(),
            occurred_at: command.occurred_at(),
        };
        for (offset, event) in progress.events.iter().enumerate() {
            append_event(
                &mut tx,
                command.expected_parent_head(),
                parent_version
                    .checked_add(u64::try_from(offset + 1).map_err(|_| corrupt("child offset"))?)
                    .ok_or_else(|| corrupt("parent journal sequence"))?,
                event,
                None,
                &metadata,
            )
            .await?;
        }
        let child_changed = sqlx::query(
            "UPDATE steps SET status='SUCCEEDED',waiting_kind=NULL,cancel_requested=false, \
                    routed=true,updated_at=$4 \
             WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(parent_instance)
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let successor_instance = ExecutionInstanceId::root(successor.clone());
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,activity,ready_at,last_hinted_at, \
              hint_count,updated_at,instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'READY',$6,$7,$7,1,$7,'{}','{}')",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(successor.as_str())
        .bind(instance_step_id(
            version,
            command.expected_parent_head(),
            &successor_instance,
        ))
        .bind(command.scope().namespace_id().as_uuid())
        .bind(activity.as_str())
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let final_version = parent_version
            .checked_add(u64::try_from(progress.events.len()).map_err(|_| corrupt("child events"))?)
            .ok_or_else(|| corrupt("parent run version"))?;
        let run_changed = sqlx::query(
            "UPDATE runs SET status='RUNNING',run_version=$3,updated_at=$4 \
             WHERE tenant_id=$1 AND id=$2 AND run_version=$5 AND finished_at IS NULL",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(number(final_version, "settled parent version")?)
        .bind(command.occurred_at())
        .bind(number(parent_version, "expected parent version")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let link_changed = sqlx::query(
            "UPDATE child_chain_links SET settled=true,updated_at=$5 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND parent_run_id=$3 AND child_chain_id=$4 \
               AND final_outcome='SUCCEEDED' AND settled=false",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.scope().namespace_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(command.child_chain_id().as_uuid())
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if child_changed != 1 || run_changed != 1 || link_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'task.ready',$4,$5)",
        )
        .bind(command.task_outbox_id())
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_parent_head().as_uuid())
        .bind(activity.as_str())
        .bind(json!({
            "tenantId": command.scope().tenant_id().to_string(),
            "namespaceId": command.scope().namespace_id().to_string(),
            "runId": command.expected_parent_head().to_string(),
            "node": successor.as_str(),
            "instancePath": [],
            "activity": activity.as_str(),
            "traceId": trace_id,
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        mark_telemetry(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_parent_head(),
            final_version,
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(ChildChainSettlementWrite::new(
            command.expected_parent_head(),
            command.child_chain_id(),
            false,
        ))
    }
}

impl ChainCommandStore for PgRunStore {
    async fn begin_chain_head(
        &self,
        scope: ChainScope,
        chain_id: RunId,
    ) -> Result<Option<PgRunTx>, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let row = sqlx::query(
            "SELECT current_run_id,revision,cancel_requested,final_outcome \
             FROM execution_chains \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(scope.namespace_id().as_uuid())
        .bind(chain_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let head = RunId::new(row.get("current_run_id"));
        let revision =
            u64::try_from(row.get::<i64, _>("revision")).map_err(|_| StoreError::Internal {
                detail: "stored chain revision is invalid".to_owned(),
            })?;
        let final_outcome =
            restore_final_outcome(row.get::<Option<String>, _>("final_outcome").as_deref())?;
        let locked = LockedChainHead::restore(
            scope,
            chain_id,
            head,
            revision,
            row.get("cancel_requested"),
            final_outcome,
        );
        self.begin_in_transaction(tx, scope.tenant_id(), head, Lock::Wait, Some(locked))
            .await
    }

    async fn inspect_chain(
        &self,
        scope: ChainScope,
        chain_id: RunId,
    ) -> Result<Option<ChainInspection>, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let row = sqlx::query(
            "SELECT current_run_id,revision,cancel_requested,final_outcome \
             FROM execution_chains \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR SHARE",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(scope.namespace_id().as_uuid())
        .bind(chain_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let segment_rows = sqlx::query(
            "SELECT segment_index,run_id,predecessor_run_id,successor_run_id \
             FROM chain_segments \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 \
             ORDER BY segment_index",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(scope.namespace_id().as_uuid())
        .bind(chain_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let segments = segment_rows
            .iter()
            .map(|segment| {
                Ok(ChainSegmentView {
                    index: u16::try_from(segment.get::<i32, _>("segment_index")).map_err(|_| {
                        StoreError::Internal {
                            detail: "stored chain segment index is invalid".to_owned(),
                        }
                    })?,
                    run_id: RunId::new(segment.get("run_id")),
                    predecessor: segment
                        .get::<Option<uuid::Uuid>, _>("predecessor_run_id")
                        .map(RunId::new),
                    successor: segment
                        .get::<Option<uuid::Uuid>, _>("successor_run_id")
                        .map(RunId::new),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let revision =
            u64::try_from(row.get::<i64, _>("revision")).map_err(|_| StoreError::Internal {
                detail: "stored chain revision is invalid".to_owned(),
            })?;
        let inspection = ChainInspection {
            chain_id,
            current_head: RunId::new(row.get("current_run_id")),
            revision,
            cancel_requested: row.get("cancel_requested"),
            final_outcome: restore_final_outcome(
                row.get::<Option<String>, _>("final_outcome").as_deref(),
            )?,
            segments,
        };
        tx.rollback().await.map_err(store_error)?;
        Ok(Some(inspection))
    }

    async fn chain_exists_outside(
        &self,
        tenant_id: TenantId,
        chain_id: RunId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM execution_chains \
             WHERE tenant_id<>$1 AND chain_id=$2)",
        )
        .bind(tenant_id.as_uuid())
        .bind(chain_id.as_uuid())
        .fetch_one(self.pool())
        .await
        .map_err(store_error)
    }
}
