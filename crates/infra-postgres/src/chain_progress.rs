//! PostgreSQL transaction adapter for scheduler-owned execution-chain continuation.
//!
//! The adapter commits an already evaluated object input into an activity-entrypoint successor.
//! Schema-checked state transfer and non-activity successor planning remain closed at the port.

use sagat_application::chain_progress::{
    ChainCommandStore, ChainFinalizationCommand, ChainFinalizationWrite, ChainHandoffCommand,
    ChainHandoffWrite, ChainInitialization, ChainInspection, ChainParent, ChainProgressError,
    ChainProgressStore, ChainRunItem, ChainRunPage, ChainSegmentView, EvaluatedChainHandoffCommand,
    LockedChainHead,
};
use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::runs::Lock;
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::execution::{
    ChainFinalOutcome, ChainScope, ContinuationTransfer, ExecutionInstanceId, QuiescenceBlocker,
    RunEvent, RunId, RunState, RunStatus, SegmentCompletionKind,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::outbound::OutboundKind;
use sagat_domain::quota::{self, QuotaResource};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, NodeKind, WorkflowDefinition};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use time::OffsetDateTime;

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
    parent_node: Option<String>,
    cancel_policy: Option<String>,
    root_run_id: uuid::Uuid,
    depth: i16,
    trace_parent_span: Option<String>,
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

async fn retain_payload(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    value: Digest,
    content: &[u8],
) -> Result<(), ChainProgressError> {
    if Sha256::digest(content).as_slice() != value.as_bytes() {
        return Err(ChainProgressError::CorruptProjection);
    }
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
         ON CONFLICT (tenant_id,digest) DO NOTHING",
    )
    .bind(tenant_id.as_uuid())
    .bind(value.as_bytes().as_slice())
    .bind(content)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    let stored = validate_payload(conn, tenant_id, value).await?;
    if stored != content {
        return Err(ChainProgressError::CorruptProjection);
    }
    Ok(())
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
    hold_parent(conn, command.scope().tenant_id(), command.chain_id()).await?;
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

/// A child chain's successor is a run whose parent is the chain's parent, so inserting it takes a
/// key-share lock on the parent's row (the foreign key). Take that metadata guard before the head
/// run/link: otherwise a parent holding its own run and waiting for the link could block the
/// successor's late foreign-key check. Actual order here is chain, parent key-share, then head/link.
/// ADR-0037 permits this identity-only guard under immutable initial/head/link parent ancestry;
/// it grants no authority to read mutable parent state or mutate/upgrade the parent lock.
async fn hold_parent(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    chain_id: RunId,
) -> Result<(), ChainProgressError> {
    sqlx::query(
        "SELECT parent.id FROM runs first JOIN runs parent \
           ON parent.tenant_id = first.tenant_id AND parent.id = first.parent_run_id \
         WHERE first.tenant_id = $1 AND first.id = $2 FOR KEY SHARE OF parent",
    )
    .bind(tenant_id.as_uuid())
    .bind(chain_id.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn lock_run(
    conn: &mut PgConnection,
    command: &ChainHandoffCommand,
) -> Result<LockedRun, ChainProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id, workflow_id, workflow_version_id, status, run_version, input_digest, \
                deadline_at, parent_run_id, parent_node, cancel_policy, root_run_id, depth, \
                trace_parent_span, stop_reason, paused \
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
        parent_node: row.get("parent_node"),
        cancel_policy: row.get("cancel_policy"),
        root_run_id: row.get("root_run_id"),
        depth: row.get("depth"),
        trace_parent_span: row.get("trace_parent_span"),
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
    tx: &mut Transaction<'_, Postgres>,
    command: &ChainHandoffCommand,
) -> Result<(), ChainProgressError> {
    check_fence(tx, command.scheduler(), command.expected_head()).await
}

async fn check_fence(
    tx: &mut Transaction<'_, Postgres>,
    authority: &sagat_application::primitives::SchedulerAuthority,
    run_id: RunId,
) -> Result<(), ChainProgressError> {
    let lease = authority.lease();
    if lease.shard != Shard::of(run_id.as_uuid()) {
        return Err(ChainProgressError::SchedulerFenceLost);
    }
    // Preserve this port's existing out-of-range generation error; the common run helper
    // deliberately treats that input as an absent authority instead.
    number(lease.generation, "scheduler generation")?;
    if crate::execution::locked_scheduler_time(tx, run_id, authority.owner(), lease)
        .await?
        .is_none()
    {
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

/// The first obligation (most significant first) the run still holds besides its boundary step, as
/// a word of [`QuiescenceBlocker::label`], or `Ok(None)` when it is quiescent. A boundary that is no
/// longer a pending root step is not an obligation: the caller's snapshot is stale.
async fn typed_segment_blocker(
    conn: &mut PgConnection,
    command: &EvaluatedChainHandoffCommand,
) -> Result<Result<Option<QuiescenceBlocker>, BoundaryGone>, ChainProgressError> {
    let handoff = command.handoff();
    let (pending, blocker): (bool, Option<String>) = sqlx::query_as(
        "SELECT \
           EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
             AND instance_path_nodes='{}' AND instance_path_indices='{}' \
             AND status='PENDING' AND routed=false AND waiting_kind IS NULL \
             AND lease_token_hash IS NULL), \
           CASE \
             WHEN EXISTS (SELECT 1 FROM run_sagas WHERE tenant_id=$1 AND run_id=$2 \
               AND phase<>'DONE') THEN 'compensation' \
             WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND (node<>$3 OR instance_path_nodes<>'{}' OR instance_path_indices<>'{}') \
               AND status='WAITING' AND waiting_kind='RECONCILIATION') THEN 'unknown_effect' \
             WHEN EXISTS (SELECT 1 FROM approval_requests WHERE tenant_id=$1 AND run_id=$2 \
               AND status='PENDING') \
               OR EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status='WAITING' AND waiting_kind='APPROVAL') THEN 'approval' \
             WHEN EXISTS (SELECT 1 FROM runs WHERE tenant_id=$1 AND parent_run_id=$2 \
               AND parent_settled=false) \
               OR EXISTS (SELECT 1 FROM child_chain_links WHERE tenant_id=$1 AND parent_run_id=$2 \
               AND (final_outcome IS NULL OR settled=false)) \
               OR EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status='WAITING' AND waiting_kind='CHILD') THEN 'child' \
             WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND (node<>$3 OR instance_path_nodes<>'{}' OR instance_path_indices<>'{}') \
               AND status IN ('LEASED','RUNNING')) THEN 'lease' \
             WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status='WAITING' AND waiting_kind='TIMER') THEN 'timer' \
             WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status='WAITING' AND waiting_kind='EVENT') THEN 'subscription' \
             WHEN EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND (node<>$3 OR instance_path_nodes<>'{}' OR instance_path_indices<>'{}') \
               AND (status<>'SUCCEEDED' OR routed=false)) THEN 'sibling_work' \
           END",
    )
    .bind(handoff.scope().tenant_id().as_uuid())
    .bind(handoff.expected_head().as_uuid())
    .bind(command.boundary().as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if !pending {
        return Ok(Err(BoundaryGone));
    }
    let blocker = blocker
        .map(|label| {
            QuiescenceBlocker::ALL
                .into_iter()
                .find(|candidate| candidate.label() == label)
                .ok_or_else(|| corrupt("quiescence blocker"))
        })
        .transpose()?;
    Ok(Ok(blocker))
}

/// The boundary step is not a pending root step any more.
struct BoundaryGone;

/// How a successor begins, as the definition's entrypoint declares it.
enum SuccessorEntry {
    /// A task for a worker, announced by a `task.ready` hint.
    Activity(sagat_domain::workflow::ActivityName),
    /// A wait that is already open: its subscription commits with the hand-off.
    Wait {
        event_type: String,
        key: sagat_domain::execution::CorrelationKey,
        deadline: OffsetDateTime,
        since: OffsetDateTime,
    },
}

impl SuccessorEntry {
    const fn run_status(&self) -> &'static str {
        match self {
            Self::Activity(_) => "RUNNING",
            Self::Wait { .. } => "WAITING",
        }
    }

    /// The third event of the successor's journal: what the entry step does first.
    fn opening_event(&self, entrypoint: &sagat_domain::workflow::NodeId) -> RunEvent {
        match self {
            Self::Activity(_) => RunEvent::StepReleased {
                node: entrypoint.clone(),
            },
            Self::Wait { key, deadline, .. } => RunEvent::WaitStarted {
                node: entrypoint.clone(),
                correlation_key: key.clone(),
                deadline: *deadline,
            },
        }
    }
}

async fn ensure_tree_capacity_for_successor(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    run: &LockedRun,
) -> Result<(), ChainProgressError> {
    if run.parent_run_id.is_none() {
        return Ok(());
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("run-tree:{tenant_id}:{}", run.root_run_id))
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND root_run_id=$2")
            .bind(tenant_id.as_uuid())
            .bind(run.root_run_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(store_error)?;
    if usize::try_from(members).unwrap_or(usize::MAX) >= sagat_domain::execution::MAX_TREE_RUNS {
        return Err(ChainProgressError::Chain(
            sagat_domain::execution::ChainError::TreeBudgetExhausted,
        ));
    }
    Ok(())
}

/// Adds chain metadata and its replay event inside the transaction that owns the initial run.
/// Dropping or rolling back `conn` therefore removes the run and its chain together.
pub async fn initialize_chain_in_transaction(
    conn: &mut PgConnection,
    initialization: &ChainInitialization,
) -> Result<(), ChainProgressError> {
    let scope = initialization.scope();
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, started_by, deadline_at, run_version, created_at, \
                trace_id \
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
    let trace_id: Option<String> = row.get("trace_id");
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
    if let Some(parent) = initialization.parent() {
        link_child_chain(
            conn,
            scope,
            initialization.initial_run_id(),
            parent,
            created_at,
        )
        .await?;
    }
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
        trace_id: trace_id.as_deref(),
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

/// Records that `chain` answers to the parent's `child` step, and that its runs are not told to the
/// parent one by one: the link carries the chain's final outcome (`ended_children`).
async fn link_child_chain(
    conn: &mut PgConnection,
    scope: ChainScope,
    chain: RunId,
    parent: &ChainParent,
    created_at: time::OffsetDateTime,
) -> Result<(), ChainProgressError> {
    let step: Option<(uuid::Uuid, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT step_id, child_run_id FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
           AND instance_path_nodes='{}' AND instance_path_indices='{}'",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(parent.run().as_uuid())
    .bind(parent.node().as_str())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some((parent_instance, Some(reserved))) = step else {
        return Err(ChainProgressError::Missing);
    };
    if reserved != chain.as_uuid() {
        return Err(ChainProgressError::CorruptProjection);
    }
    sqlx::query(
        "INSERT INTO child_chain_links \
         (tenant_id, namespace_id, parent_run_id, parent_instance_id, child_chain_id, \
          created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$6)",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(parent.run().as_uuid())
    .bind(parent_instance)
    .bind(chain.as_uuid())
    .bind(created_at)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    sqlx::query("UPDATE runs SET parent_settled=true WHERE tenant_id=$1 AND id=$2")
        .bind(scope.tenant_id().as_uuid())
        .bind(chain.as_uuid())
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    Ok(())
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
    let audit_matches: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM audit_records WHERE tenant_id=$1 AND id=$2 \
           AND actor=$3 AND action='run.continued' AND resource_type='run' \
           AND resource_id=$4 AND trace_id IS NOT DISTINCT FROM $5 AND created_at=$6)",
    )
    .bind(command.scope().tenant_id().as_uuid())
    .bind(command.audit().id().as_uuid())
    .bind(command.audit().actor().as_str())
    .bind(command.expected_head().to_string())
    .bind(command.audit().trace_id())
    .bind(command.occurred_at())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if successor != command.successor()
        || input != command.transfer().input()
        || state != command.transfer().state()
        || !outbox_matches
        || !audit_matches
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
        crate::execution::capture_run_limits(
            &mut tx,
            command.scope().tenant_id().as_uuid(),
            command.successor().as_uuid(),
            definition.limits(),
        )
        .await?;
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

    async fn continue_evaluated(
        &self,
        prepared: &EvaluatedChainHandoffCommand,
    ) -> Result<ChainHandoffWrite, ChainProgressError> {
        let command = prepared.handoff();
        validate_audit(command)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let chain = lock_chain(&mut tx, command).await?;
        let run = lock_run(&mut tx, command).await?;
        check_scheduler_fence(&mut tx, command).await?;
        if chain.current_run_id != command.expected_head() {
            let result = replay_handoff(&mut tx, command, &chain).await?;
            let selected = sqlx::query(
                "SELECT node,detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
                   AND seq=$3 AND event_type='continuation.selected' AND node=$4",
            )
            .bind(command.scope().tenant_id().as_uuid())
            .bind(command.expected_head().as_uuid())
            .bind(number(
                command
                    .expected_run_version()
                    .checked_add(1)
                    .ok_or_else(|| corrupt("predecessor journal exhausted"))?,
                "continuation replay sequence",
            )?)
            .bind(prepared.boundary().as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let selected = selected.and_then(|row| {
                let node: Option<String> = row.get("node");
                let detail: serde_json::Value = row.get("detail");
                decode("continuation.selected", node.as_deref(), &detail)
            });
            if selected
                != Some(RunEvent::ContinuationSelected {
                    node: prepared.boundary().clone(),
                    successor: command.successor(),
                    input_digest: command.transfer().input(),
                    state_digest: command.transfer().state(),
                })
            {
                return Err(ChainProgressError::Chain(
                    sagat_domain::execution::ChainError::ContinuationConflict,
                ));
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(result);
        }
        if child_link_cancelled(&mut tx, command.scope(), command.chain_id()).await? {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::ChildCancellationRequested,
            ));
        }
        let transaction_now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        if chain.namespace_id != command.scope().namespace_id()
            || chain.version_id != run.version_id
            || chain.revision != command.expected_chain_revision()
            || run.run_version != command.expected_run_version()
            || chain.cancel_requested
            || chain.final_outcome.is_some()
        {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        // A boundary that reaches its hand-off after the original deadline is not a broken
        // projection: the deadline pass ends the run `TIMED_OUT` and the chain with it.
        if chain.original_deadline <= transaction_now {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::DeadlineElapsed,
            ));
        }
        if chain.segment_count >= chain.max_segments {
            return Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::SegmentLimitExceeded,
            ));
        }
        ensure_tree_capacity_for_successor(&mut tx, command.scope().tenant_id(), &run).await?;
        ensure_replacement_capacity(&mut tx, command).await?;
        if !matches!(
            serde_json::from_slice::<serde_json::Value>(prepared.input()),
            Ok(serde_json::Value::Object(_))
        ) {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        retain_payload(
            &mut tx,
            command.scope().tenant_id(),
            command.transfer().input(),
            prepared.input(),
        )
        .await?;
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
        match (
            command.transfer().state(),
            prepared.state(),
            prepared.expected_state_revision(),
            state_row.as_ref(),
        ) {
            (None, None, None, None) => {}
            (Some(expected), Some(content), Some(revision), Some(row)) => {
                if digest(row.get("state_digest"), "operation state digest")? != expected
                    || row.get::<i64, _>("state_revision")
                        != number(revision, "operation state revision")?
                    || row.get::<i32, _>("state_size_bytes")
                        != i32::try_from(content.len())
                            .map_err(|_| corrupt("operation state size exceeds storage"))?
                {
                    return Err(ChainProgressError::RuntimeNotIntegrated);
                }
                retain_payload(&mut tx, command.scope().tenant_id(), expected, content).await?;
            }
            _ => return Err(ChainProgressError::RuntimeNotIntegrated),
        }
        let definition = self
            .definition(&mut tx, command.scope().tenant_id(), run.version_id)
            .await?;
        if !matches!(
            definition.node(prepared.boundary()).map(|node| node.kind()),
            Some(NodeKind::ContinueAsNew { .. })
        ) {
            return Err(ChainProgressError::RuntimeNotIntegrated);
        }
        match typed_segment_blocker(&mut tx, prepared).await? {
            Err(BoundaryGone) => return Err(ChainProgressError::RuntimeNotIntegrated),
            Ok(Some(blocker)) => {
                return Err(ChainProgressError::Chain(
                    sagat_domain::execution::ChainError::NotQuiescent(blocker),
                ));
            }
            Ok(None) => {}
        }
        let entrypoint = definition.entrypoint().clone();
        // What the successor begins with. The selection must match the definition exactly: a key for
        // an activity entry, or none for a wait, is a caller that did not read the same snapshot.
        let entry = match (
            definition.node(&entrypoint).map(|node| node.kind()),
            prepared.correlation_key(),
        ) {
            (Some(NodeKind::Activity { activity, .. }), None) => {
                SuccessorEntry::Activity(activity.clone())
            }
            (
                Some(NodeKind::WaitEvent {
                    event_type,
                    timeout,
                    lookback,
                    ..
                }),
                Some(key),
            ) if lookback.is_zero() => {
                // The wait is measured from this transaction's clock, not from the scheduler's
                // earlier read: an event admitted in between is later than the wait it finds.
                let timeout = time::Duration::try_from(*timeout)
                    .map_err(|_| corrupt("wait timeout exceeds the calendar"))?;
                let deadline = transaction_now
                    .checked_add(timeout)
                    .ok_or_else(|| corrupt("wait deadline exceeds the calendar"))?;
                SuccessorEntry::Wait {
                    event_type: event_type.as_str().to_owned(),
                    key: key.clone(),
                    deadline,
                    since: transaction_now,
                }
            }
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
        let selected = RunEvent::ContinuationSelected {
            node: prepared.boundary().clone(),
            successor: command.successor(),
            input_digest: command.transfer().input(),
            state_digest: command.transfer().state(),
        };
        let mut replayed = replay_root_journal(
            &mut tx,
            command.scope().tenant_id(),
            command.expected_head(),
        )
        .await?;
        if replayed.version() != run.run_version || replayed.apply(&selected).is_err() {
            return Err(ChainProgressError::CorruptProjection);
        }
        append_event(
            &mut tx,
            command.expected_head(),
            old_sequence,
            &selected,
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
        let boundary_changed = sqlx::query(
            "UPDATE steps SET status='SUCCEEDED',routed=true,updated_at=$5 \
             WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
               AND instance_path_nodes='{}' AND instance_path_indices='{}' \
               AND step_id=$4 AND status='PENDING' AND routed=false",
        )
        .bind(command.scope().tenant_id().as_uuid())
        .bind(command.expected_head().as_uuid())
        .bind(prepared.boundary().as_str())
        .bind(instance_step_id(
            run.version_id,
            command.expected_head(),
            &ExecutionInstanceId::root(prepared.boundary().clone()),
        ))
        .bind(command.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if boundary_changed != 1 {
            return Err(ChainProgressError::CorruptProjection);
        }
        let successor_root = run
            .parent_run_id
            .map_or(command.successor().as_uuid(), |_| run.root_run_id);
        sqlx::query(
            "INSERT INTO runs \
             (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
              paused, input_digest, started_by, trace_id, deadline_at, created_at, updated_at, \
              root_run_id, depth, parent_run_id, parent_node, cancel_policy, trace_parent_span, \
              parent_settled) \
             VALUES ($1,$2,$3,$4,$5,$17,3,false,$6,$7,$8,$9,$10,$10,$11,$12,$13,$14,$15,$16, \
                     $13 IS NOT NULL)",
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
        .bind(successor_root)
        .bind(run.depth)
        .bind(run.parent_run_id)
        .bind(run.parent_node.as_deref())
        .bind(run.cancel_policy.as_deref())
        .bind(run.trace_parent_span.as_deref())
        .bind(entry.run_status())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        crate::execution::capture_run_limits(
            &mut tx,
            command.scope().tenant_id().as_uuid(),
            command.successor().as_uuid(),
            definition.limits(),
        )
        .await?;
        let entry_instance = ExecutionInstanceId::root(entrypoint.clone());
        let entry_step = instance_step_id(run.version_id, command.successor(), &entry_instance);
        match &entry {
            SuccessorEntry::Activity(activity) => {
                sqlx::query(
                    "INSERT INTO steps \
                     (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
                      last_hinted_at, hint_count, updated_at, instance_path_nodes, \
                      instance_path_indices) \
                     VALUES ($1,$2,$3,$4,$5,'READY',$6,$7,$7,1,$7,'{}','{}')",
                )
                .bind(command.scope().tenant_id().as_uuid())
                .bind(command.successor().as_uuid())
                .bind(entrypoint.as_str())
                .bind(entry_step)
                .bind(command.scope().namespace_id().as_uuid())
                .bind(activity.as_str())
                .bind(command.occurred_at())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
            SuccessorEntry::Wait {
                event_type,
                key,
                deadline,
                since,
            } => {
                // The subscription: the dispatcher and the chain signal find the wait through this
                // row, so it commits with the hand-off and with nothing after it.
                sqlx::query(
                    "INSERT INTO steps \
                     (tenant_id, run_id, node, step_id, namespace_id, status, waiting_kind, \
                      due_at, event_type, correlation_key, event_since, updated_at, \
                      instance_path_nodes, instance_path_indices) \
                     VALUES ($1,$2,$3,$4,$5,'WAITING','EVENT',$6,$7,$8,$9,$9,'{}','{}')",
                )
                .bind(command.scope().tenant_id().as_uuid())
                .bind(command.successor().as_uuid())
                .bind(entrypoint.as_str())
                .bind(entry_step)
                .bind(command.scope().namespace_id().as_uuid())
                .bind(deadline)
                .bind(event_type)
                .bind(key.as_str())
                .bind(since)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
        }
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
            (3, entry.opening_event(&entrypoint)),
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
        // Only an activity is a task for a worker; a wait has nothing to announce.
        if let SuccessorEntry::Activity(activity) = &entry {
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
        }
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
            "traceId": run.trace_id,
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        // The final head is a finished run for its subscribers too, as on every other terminal path.
        let definition = self
            .definition(&mut tx, command.scope().tenant_id(), chain.version_id)
            .await?;
        let body = json!({
            "id": command.run_outbox_id().to_string(),
            "type": "run.finished",
            "time": command
                .occurred_at()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
            "data": {
                "tenantId": command.scope().tenant_id().to_string(),
                "namespaceId": command.scope().namespace_id().to_string(),
                "runId": command.expected_head().to_string(),
                "workflow": definition.name().as_str(),
                "status": command.status().as_str(),
                "traceId": run.trace_id,
            },
        });
        crate::outbound::fan_out(
            &mut tx,
            command.scope().tenant_id(),
            command.scope().namespace_id(),
            OutboundKind::RunFinished,
            &command.run_outbox_id().to_string(),
            &serde_json::to_vec(&body).unwrap_or_default(),
            command.occurred_at(),
        )
        .await
        .map_err(ChainProgressError::Storage)?;
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
}

impl ChainProgressStore for PgRunStore {
    async fn continue_as_new(
        &self,
        command: &ChainHandoffCommand,
    ) -> Result<ChainHandoffWrite, ChainProgressError> {
        PgChainProgressStore::new(self.pool.clone(), self.cache.clone())
            .continue_as_new(command)
            .await
    }

    async fn continue_evaluated(
        &self,
        command: &EvaluatedChainHandoffCommand,
    ) -> Result<ChainHandoffWrite, ChainProgressError> {
        PgChainProgressStore::new(self.pool.clone(), self.cache.clone())
            .continue_evaluated(command)
            .await
    }

    async fn finalize_chain(
        &self,
        command: &ChainFinalizationCommand,
    ) -> Result<ChainFinalizationWrite, ChainProgressError> {
        PgChainProgressStore::new(self.pool.clone(), self.cache.clone())
            .finalize_chain(command)
            .await
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

    async fn list_chain_runs(
        &self,
        scope: ChainScope,
        chain_id: RunId,
        after: Option<u16>,
        limit: u32,
    ) -> Result<Option<ChainRunPage>, StoreError> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM execution_chains \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3)",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(scope.namespace_id().as_uuid())
        .bind(chain_id.as_uuid())
        .fetch_one(self.pool())
        .await
        .map_err(store_error)?;
        if !exists {
            return Ok(None);
        }
        // One more than the page: its presence says that another page exists.
        let rows = sqlx::query(
            "SELECT segment.segment_index,segment.run_id,segment.predecessor_run_id, \
                    segment.successor_run_id,segment.completion_kind,run.status, \
                    run.created_at,run.finished_at \
             FROM chain_segments segment \
             JOIN runs run ON run.tenant_id=segment.tenant_id AND run.id=segment.run_id \
             WHERE segment.tenant_id=$1 AND segment.namespace_id=$2 AND segment.chain_id=$3 \
               AND ($4::int IS NULL OR segment.segment_index > $4) \
             ORDER BY segment.segment_index LIMIT $5",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(scope.namespace_id().as_uuid())
        .bind(chain_id.as_uuid())
        .bind(after.map(i32::from))
        .bind(i64::from(limit) + 1)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        let invalid = |what: &str| StoreError::Internal {
            detail: format!("stored chain segment {what} is invalid"),
        };
        let mut items = rows
            .iter()
            .map(|row| {
                Ok(ChainRunItem {
                    index: u16::try_from(row.get::<i32, _>("segment_index"))
                        .map_err(|_| invalid("index"))?,
                    run_id: RunId::new(row.get("run_id")),
                    status: row
                        .get::<String, _>("status")
                        .parse()
                        .map_err(|_| invalid("run status"))?,
                    completion_kind: match row
                        .get::<Option<String>, _>("completion_kind")
                        .as_deref()
                    {
                        None => None,
                        Some("CONTINUED") => Some(SegmentCompletionKind::Continued),
                        Some("FINAL") => Some(SegmentCompletionKind::Final),
                        Some(_) => return Err(invalid("completion kind")),
                    },
                    predecessor: row
                        .get::<Option<uuid::Uuid>, _>("predecessor_run_id")
                        .map(RunId::new),
                    successor: row
                        .get::<Option<uuid::Uuid>, _>("successor_run_id")
                        .map(RunId::new),
                    created_at: row.get("created_at"),
                    finished_at: row.get("finished_at"),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let next_after = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|item| item.index)
        } else {
            None
        };
        Ok(Some(ChainRunPage { items, next_after }))
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
