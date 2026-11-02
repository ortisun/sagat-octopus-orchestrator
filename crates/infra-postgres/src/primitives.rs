//! PostgreSQL collection progress adapter.
//!
//! The methods here are intentionally not wired into publication or scheduling. They persist a
//! collection transition together with its journal, projections, usage and outbox. Checkpoint and
//! finish additionally require an immutable definition cache. Runtime orchestration stays gated
//! until the planner owns scoped routing and supplies semantic audit evidence for every transition.

use sagat_application::collection_source::{SourceManifest, items_digest as source_items_digest};
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, EndOutputAnswer, MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES,
    RunTx, instance_step_id,
};
use sagat_application::primitives::{
    CollectionAdmission, CollectionCheckpoint, CollectionDataLimitEvidence, CollectionDrain,
    CollectionDrainFinish, CollectionEntry, CollectionEntryDispatch, CollectionFinish,
    CollectionIterationDrain, CollectionProgress, CollectionScopeId, CollectionScopeRef,
    CollectionStore, CollectionTransition, PrimitiveStoreError, PrimitiveWrite,
};
use sagat_application::runs::Lock;
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    CollectionDrainReason, CollectionOutcome, Evaluation, ExecutionInstanceId, FailureCode,
    InstancePath, Resolution, RunEvent,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::outbound::OutboundKind;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, Node, NodeId, NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::collection_outputs;
use crate::execution::{PgRunStore, RUN_SNAPSHOT, run_record};
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

#[derive(Debug)]
struct ReplayAddress {
    instance_id: Option<Uuid>,
    path_nodes: Option<Vec<String>>,
    path_indices: Option<Vec<i32>>,
}

impl ReplayAddress {
    fn matches(&self, instance: Option<&ExecutionInstanceId>, step_id: Option<Uuid>) -> bool {
        match instance {
            Some(instance) => {
                let (nodes, indices) = path_parts(instance);
                self.instance_id == step_id
                    && self.path_nodes.as_ref() == Some(&nodes)
                    && self.path_indices.as_ref() == Some(&indices)
            }
            None => {
                self.instance_id.is_none()
                    && self.path_nodes.is_none()
                    && self.path_indices.is_none()
            }
        }
    }
}

async fn validate_finish_replay(
    tx: &mut Transaction<'_, Postgres>,
    finish: &CollectionFinish,
    definition: &WorkflowDefinition,
    version: WorkflowVersionId,
    finish_sequence: u64,
) -> Result<(), PrimitiveStoreError> {
    let tenant = finish.transition.tenant_id().as_uuid();
    let run = finish.transition.run_id().as_uuid();
    if let Some(sources) = finish.source_digests() {
        let rows = sqlx::query(
            "SELECT iteration_index,result_digest FROM iteration_checkpoints \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 ORDER BY iteration_index",
        )
        .bind(tenant)
        .bind(run)
        .bind(finish.scope_id.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(store_error)?;
        if rows.len() != sources.len()
            || !rows.iter().zip(sources).all(|(row, (index, digest))| {
                u32::try_from(row.get::<i32, _>("iteration_index")) == Ok(*index)
                    && row.get::<Vec<u8>, _>("result_digest") == digest.as_bytes().as_slice()
            })
        {
            return Err(PrimitiveStoreError::DuplicateMismatch);
        }
    }
    let (owner_nodes, owner_indices) = path_parts(finish.transition.owner());
    let owner = sqlx::query(
        "SELECT step_id,output_digest FROM steps WHERE tenant_id=$1 AND run_id=$2 \
           AND instance_path_nodes=$3 AND instance_path_indices=$4 AND node=$5 \
           AND status='SUCCEEDED' AND routed",
    )
    .bind(tenant)
    .bind(run)
    .bind(&owner_nodes)
    .bind(&owner_indices)
    .bind(finish.transition.owner().node().as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::DuplicateMismatch)?;
    let owner_step_id = instance_step_id(
        version,
        finish.transition.run_id(),
        finish.transition.owner(),
    );
    if owner.get::<Uuid, _>("step_id") != owner_step_id {
        return Err(PrimitiveStoreError::DuplicateMismatch);
    }
    let owner_digest = owner.get::<Option<Vec<u8>>, _>("output_digest");
    if owner_digest
        .as_deref()
        .is_some_and(|digest| digest != finish.result_digest.as_bytes().as_slice())
    {
        return Err(PrimitiveStoreError::DuplicateMismatch);
    }
    let successor = definition
        .node_for_instance(finish.transition.owner())
        .and_then(Node::next)
        .cloned()
        .ok_or(PrimitiveStoreError::DuplicateMismatch)?;
    let successor =
        ExecutionInstanceId::in_path(finish.transition.owner().path().clone(), successor);
    let end_output = match definition.node_for_instance(&successor).map(Node::kind) {
        Some(NodeKind::End { output }) => Some(output),
        Some(_) => None,
        None => return Err(PrimitiveStoreError::DuplicateMismatch),
    };
    let inspect_end = owner_digest.is_some() && end_output.is_some();
    let prefix_last_sequence = finish_sequence
        .checked_add(if inspect_end { 2 } else { 1 })
        .ok_or_else(|| invalid_storage("finish replay prefix sequence"))?;
    let mut rows = sqlx::query(
        "SELECT seq,event_type,node,detail,payload_digest,instance_id, \
                instance_path_nodes,instance_path_indices \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND seq BETWEEN $3 AND $4 \
         ORDER BY seq",
    )
    .bind(tenant)
    .bind(run)
    .bind(number(finish_sequence, "finish sequence")?)
    .bind(number(
        prefix_last_sequence,
        "finish replay prefix sequence",
    )?)
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    if inspect_end {
        let tail_count = match rows.get(2).map(|row| row.get::<String, _>("event_type")) {
            Some(event_type) if event_type == "end.reached" => 1,
            Some(event_type)
                if matches!(event_type.as_str(), "step.failed" | "run.stop_requested") =>
            {
                2
            }
            _ => return Err(PrimitiveStoreError::DuplicateMismatch),
        };
        let tail_first_sequence = finish_sequence
            .checked_add(3)
            .ok_or_else(|| invalid_storage("finish replay tail sequence"))?;
        let tail_last_sequence = prefix_last_sequence
            .checked_add(tail_count)
            .ok_or_else(|| invalid_storage("finish replay tail sequence"))?;
        let tail = sqlx::query(
            "SELECT seq,event_type,node,detail,payload_digest,instance_id, \
                    instance_path_nodes,instance_path_indices \
             FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND seq BETWEEN $3 AND $4 \
             ORDER BY seq",
        )
        .bind(tenant)
        .bind(run)
        .bind(number(tail_first_sequence, "finish replay tail sequence")?)
        .bind(number(tail_last_sequence, "finish replay tail sequence")?)
        .fetch_all(&mut **tx)
        .await
        .map_err(store_error)?;
        rows.extend(tail);
    }
    let events = rows
        .iter()
        .map(|row| {
            let event_type = row.get::<String, _>("event_type");
            let node = row.get::<Option<String>, _>("node");
            crate::event_codec::decode(&event_type, node.as_deref(), &row.get::<Value, _>("detail"))
                .ok_or(PrimitiveStoreError::DuplicateMismatch)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let addresses = rows
        .iter()
        .map(|row| ReplayAddress {
            instance_id: row.get("instance_id"),
            path_nodes: row.get("instance_path_nodes"),
            path_indices: row.get("instance_path_indices"),
        })
        .collect::<Vec<_>>();
    let payloads = rows
        .iter()
        .map(|row| row.get::<Option<Vec<u8>>, _>("payload_digest"))
        .collect::<Vec<_>>();
    let consecutive = rows.iter().enumerate().all(|(offset, row)| {
        u64::try_from(row.get::<i64, _>("seq")).ok()
            == u64::try_from(offset)
                .ok()
                .and_then(|offset| finish_sequence.checked_add(offset))
    });
    let consecutive_prefix = rows.len() >= 2
        && consecutive
        && matches!(events.first(), Some(RunEvent::CollectionFinished {
            node, outcome: CollectionOutcome::Succeeded, result_digest: Some(digest),
        }) if node == finish.transition.owner().node() && digest == &finish.result_digest)
        && matches!(events.get(1), Some(RunEvent::StepRouted { node, activated })
            if node == finish.transition.owner().node()
                && activated == std::slice::from_ref(successor.node()))
        && addresses[0].matches(Some(finish.transition.owner()), Some(owner_step_id))
        && addresses[1].matches(Some(finish.transition.owner()), Some(owner_step_id));
    if !consecutive_prefix {
        return Err(PrimitiveStoreError::DuplicateMismatch);
    }
    // Historical writers have the same immutable source prefix but no owner output. Preserve their
    // original no-answer result without interpreting the later journal or repairing projections.
    if owner_digest.is_none() {
        return if finish.end_answer().is_none() {
            Ok(())
        } else {
            Err(PrimitiveStoreError::DuplicateMismatch)
        };
    }
    let Some(output) = end_output else {
        return if rows.len() == 2 && finish.end_answer().is_none() {
            Ok(())
        } else {
            Err(PrimitiveStoreError::DuplicateMismatch)
        };
    };
    validate_replayed_end(
        tx,
        finish,
        &successor,
        version,
        output,
        &events[2..],
        &addresses[2..],
        &payloads[2..],
    )
    .await
}

enum ReplayEnd {
    Success(Option<Vec<u8>>),
    Failure(FailureCode),
}

#[allow(clippy::too_many_arguments)] // Events, addresses and payloads describe the same replay suffix.
async fn validate_replayed_end(
    tx: &mut Transaction<'_, Postgres>,
    finish: &CollectionFinish,
    end: &ExecutionInstanceId,
    version: WorkflowVersionId,
    output_expression: &Option<sagat_domain::workflow::Expression>,
    events: &[RunEvent],
    addresses: &[ReplayAddress],
    payloads: &[Option<Vec<u8>>],
) -> Result<(), PrimitiveStoreError> {
    let (nodes, indices) = path_parts(end);
    let row = sqlx::query(
        "SELECT s.step_id,s.status,s.failure_code,s.output_digest,r.status AS run_status, \
                r.stop_code,r.result_digest \
         FROM steps s JOIN runs r ON r.tenant_id=s.tenant_id AND r.id=s.run_id \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.instance_path_nodes=$3 \
           AND s.instance_path_indices=$4 AND s.node=$5",
    )
    .bind(finish.transition.tenant_id().as_uuid())
    .bind(finish.transition.run_id().as_uuid())
    .bind(&nodes)
    .bind(&indices)
    .bind(end.node().as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::DuplicateMismatch)?;
    let step_id = instance_step_id(version, finish.transition.run_id(), end);
    if row.get::<Uuid, _>("step_id") != step_id {
        return Err(PrimitiveStoreError::DuplicateMismatch);
    }
    let status = row.get::<String, _>("status");
    let failure = row.get::<Option<String>, _>("failure_code");
    let output = row.get::<Option<Vec<u8>>, _>("output_digest");
    let run_status = row.get::<String, _>("run_status");
    let stop = row.get::<Option<String>, _>("stop_code");
    let result = row.get::<Option<Vec<u8>>, _>("result_digest");

    let pre_evaluation_limit = matches!(events, [
        RunEvent::RunStopRequested { code: Some(code), .. },
        RunEvent::StepCancelled { node },
        RunEvent::RunFinished { .. },
    ] if code == &FailureCode::instance_limit() && node == end.node())
        && addresses.len() == 3
        && addresses[0].matches(None, None)
        && addresses[1].matches(Some(end), Some(step_id))
        && addresses[2].matches(None, None)
        && payloads.iter().all(Option::is_none)
        && finish.end_answer().is_none()
        && status == "CANCELLED"
        && run_status == "FAILED"
        && stop.as_deref() == Some(FailureCode::instance_limit().as_str())
        && output.is_none()
        && result.is_none();
    if pre_evaluation_limit {
        return Ok(());
    }

    let expected = match (output_expression, finish.end_answer()) {
        (None, None) => ReplayEnd::Success(None),
        (Some(_), Some(EndOutputAnswer::Succeeded(bytes))) => {
            ReplayEnd::Success(Some(Sha256::digest(bytes).to_vec()))
        }
        (Some(_), Some(EndOutputAnswer::ExpressionError)) => {
            ReplayEnd::Failure(FailureCode::expression_error())
        }
        (Some(_), Some(EndOutputAnswer::RunDataLimit)) => {
            ReplayEnd::Failure(FailureCode::run_data_limit())
        }
        _ => return Err(PrimitiveStoreError::DuplicateMismatch),
    };
    let matches = match expected {
        ReplayEnd::Success(digest) => {
            matches!(events, [RunEvent::EndReached { node }, RunEvent::RunFinished { .. }]
                if node == end.node())
                && addresses.len() == 2
                && addresses[0].matches(Some(end), Some(step_id))
                && addresses[1].matches(None, None)
                && payloads.first().and_then(Option::as_ref) == digest.as_ref()
                && payloads.get(1).is_some_and(Option::is_none)
                && status == "SUCCEEDED"
                && run_status == "SUCCEEDED"
                && output.as_ref() == digest.as_ref()
                && result.as_ref() == digest.as_ref()
        }
        ReplayEnd::Failure(code) => {
            matches!(events, [
                RunEvent::StepFailed { node, code: failed, .. },
                RunEvent::RunStopRequested { code: Some(stopped), .. },
                RunEvent::RunFinished { .. },
            ] if node == end.node() && failed == &code && stopped == &code)
                && addresses.len() == 3
                && addresses[0].matches(Some(end), Some(step_id))
                && addresses[1].matches(None, None)
                && addresses[2].matches(None, None)
                && payloads.iter().all(Option::is_none)
                && status == "FAILED"
                && run_status == "FAILED"
                && failure.as_deref() == Some(code.as_str())
                && stop.as_deref() == Some(code.as_str())
                && output.is_none()
                && result.is_none()
        }
    };
    if matches {
        Ok(())
    } else {
        Err(PrimitiveStoreError::DuplicateMismatch)
    }
}

/// How the run ends when the compensating scope `owner` finishes: the stop of the run, worsened by
/// how the compensations of the scope ended. The scope's compensation must be over.
async fn run_status_after_compensation(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    transition: &CollectionTransition,
    owner_nodes: &[String],
    owner_indices: &[i32],
    locked: &LockedDrainRun,
) -> Result<String, PrimitiveStoreError> {
    use crate::scope_compensation::{DoneSagaError, read_done_saga, stopped_run_status};
    let failure = |error: DoneSagaError| match error {
        DoneSagaError::NotDone => PrimitiveStoreError::InvalidTransition,
        DoneSagaError::Corrupt(detail) => invalid_storage(detail),
        DoneSagaError::Database(error) => store_error(error).into(),
    };
    let saga = read_done_saga(
        tx,
        transition.tenant_id().as_uuid(),
        transition.run_id().as_uuid(),
        owner_nodes,
        owner_indices,
        transition.owner().node().as_str(),
    )
    .await
    .map_err(failure)?;
    Ok(stopped_run_status(
        locked.stop_reason.as_deref(),
        locked.stop_code.as_deref(),
        &saga,
    )
    .map_err(failure)?
    .as_str()
    .to_owned())
}

/// The reservations an iteration keeps when it is drained (D384): the compensations its body
/// declares, if its scope compensates, so that exhausting the forward budget cannot stop them.
/// Everything else it still held is released.
pub(crate) fn kept_reservations(
    definition: &sagat_domain::workflow::WorkflowDefinition,
    owner: &ExecutionInstanceId,
    remaining: i32,
) -> i32 {
    if !definition.scope_compensates(owner) {
        return 0;
    }
    let compensations = definition.scope_body(owner).map_or(0, |body| {
        body.all_nodes()
            .filter(|node| {
                matches!(
                    node.origin(),
                    sagat_domain::saga::NodeOrigin::Compensation(_)
                )
            })
            .count()
    });
    i32::try_from(compensations)
        .unwrap_or(i32::MAX)
        .min(remaining)
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
    tx: &mut Transaction<'_, Postgres>,
    transition: &CollectionTransition,
) -> Result<LockedRun, PrimitiveStoreError> {
    crate::execution::lock_member_chain(
        tx,
        transition.tenant_id(),
        transition.namespace_id(),
        transition.run_id(),
    )
    .await?;
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, run_version, status FROM runs \
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut **tx)
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
    paused: bool,
}

async fn lock_drain_run(
    tx: &mut Transaction<'_, Postgres>,
    transition: &CollectionTransition,
) -> Result<LockedDrainRun, PrimitiveStoreError> {
    crate::execution::lock_member_chain(
        tx,
        transition.tenant_id(),
        transition.namespace_id(),
        transition.run_id(),
    )
    .await?;
    let row = sqlx::query(
        "SELECT namespace_id,workflow_version_id,run_version,status,stop_reason,stop_code,paused \
         FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut **tx)
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
        paused: row.get("paused"),
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

fn checked_data_limit_evidence(
    drain: &CollectionDrain,
) -> Result<Option<&CollectionDataLimitEvidence>, PrimitiveStoreError> {
    let evidence = drain.data_limit_evidence();
    if !matches!(drain.reason, CollectionDrainReason::RunDataLimit) {
        return if evidence.is_none() {
            Ok(None)
        } else {
            Err(PrimitiveStoreError::InvalidTransition)
        };
    }
    let evidence = evidence.ok_or(PrimitiveStoreError::InvalidTransition)?;
    let valid_audit = drain.transition.audit().is_some_and(|audit| {
        audit.tenant_id() == drain.transition.tenant_id()
            && audit.actor() == drain.transition.actor()
            && audit.actor() == &Subject::engine()
            && audit.action() == AuditAction::RunDataLimitExceeded
            && audit.resource_type() == ResourceType::Run
            && audit.resource_id() == drain.transition.run_id().to_string()
            && audit.reason() == Some(evidence.required_audit_reason().as_str())
            && audit.trace_id() == drain.transition.trace_id()
            && audit.created_at() == drain.transition.occurred_at()
    });
    if !valid_audit {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    Ok(Some(evidence))
}

async fn data_limit_definition(
    store: &PgCollectionStore,
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    version: WorkflowVersionId,
) -> Result<std::sync::Arc<WorkflowDefinition>, PrimitiveStoreError> {
    let cache = store
        .definitions
        .as_ref()
        .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?;
    PgRunStore::new(store.pool.clone(), cache.clone())
        .definition(&mut *conn, transition.tenant_id(), version)
        .await
        .map_err(PrimitiveStoreError::from)
}

async fn effective_output_snapshot(
    store: &PgCollectionStore,
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    locked: LockedRun,
) -> Result<collection_outputs::EffectiveOutputBytes, PrimitiveStoreError> {
    let row = sqlx::query(RUN_SNAPSHOT)
        .bind(transition.tenant_id().as_uuid())
        .bind(transition.run_id().as_uuid())
        .fetch_one(&mut *conn)
        .await
        .map_err(store_error)?;
    let info = run_record(&row)?.info;
    if info.namespace_id != transition.namespace_id() || info.version_id != locked.version_id {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    let definition = data_limit_definition(store, conn, transition, locked.version_id).await?;
    collection_outputs::inspect(&mut *conn, &info, &definition)
        .await
        .map_err(PrimitiveStoreError::from)
}

async fn fresh_collection_aggregate_preflight(
    store: &PgCollectionStore,
    conn: &mut PgConnection,
    finish: &CollectionFinish,
    locked: LockedRun,
) -> Result<(usize, Option<collection_outputs::EffectiveOutputBytes>), PrimitiveStoreError> {
    let bytes = if let Some(content) = finish.result_content() {
        if !payload_matches(content, finish.result_digest) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        content.len()
    } else {
        let stored: Option<i64> = sqlx::query_scalar(
            "SELECT octet_length(content)::bigint FROM payloads WHERE tenant_id=$1 AND digest=$2",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.result_digest.as_bytes().as_slice())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
        usize::try_from(stored.ok_or(PrimitiveStoreError::InvalidTransition)?)
            .map_err(|_| invalid_storage("collection aggregate payload length"))?
    };
    if bytes > MAX_PAYLOAD_BYTES {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    if !finish.transition.owner().path().is_root() {
        return Ok((bytes, None));
    }
    let effective = effective_output_snapshot(store, conn, &finish.transition, locked).await?;
    let charge =
        u64::try_from(bytes).map_err(|_| invalid_storage("collection aggregate payload length"))?;
    let limit =
        u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| invalid_storage("run output limit"))?;
    let exceeds = effective
        .total()
        .checked_add(charge)
        .is_none_or(|total| total > limit);
    if exceeds {
        return Err(PrimitiveStoreError::Storage(StoreError::QuotaExceeded {
            what: "output retained by one run",
        }));
    }
    Ok((bytes, Some(effective)))
}

#[allow(clippy::too_many_lines)]
async fn validate_data_limit_evidence(
    store: &PgCollectionStore,
    conn: &mut PgConnection,
    drain: &CollectionDrain,
    locked: &LockedDrainRun,
    scope: &PgRow,
    evidence: &CollectionDataLimitEvidence,
) -> Result<bool, PrimitiveStoreError> {
    if !drain.transition.owner().path().is_root()
        || scope.get::<String, _>("kind") != "COLLECTION"
        || scope.get::<Option<Vec<u8>>, _>("result_digest").is_some()
    {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    let (owner_nodes, owner_indices) = path_parts(drain.transition.owner());
    let owner = sqlx::query(
        "SELECT kind,node_kind,status,waiting_kind,output_digest FROM steps \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
           AND instance_path_nodes=$4 AND instance_path_indices=$5 AND node=$6",
    )
    .bind(drain.transition.tenant_id().as_uuid())
    .bind(drain.transition.namespace_id().as_uuid())
    .bind(drain.transition.run_id().as_uuid())
    .bind(&owner_nodes)
    .bind(&owner_indices)
    .bind(drain.transition.owner().node().as_str())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::InvalidTransition)?;
    if owner.get::<String, _>("kind") != "FORWARD"
        || !matches!(
            owner.get::<String, _>("node_kind").as_str(),
            "forEach" | "legacyControl"
        )
        || owner.get::<String, _>("status") != "WAITING"
        || owner.get::<Option<String>, _>("waiting_kind").as_deref() != Some("COLLECTION")
        || owner.get::<Option<Vec<u8>>, _>("output_digest").is_some()
    {
        return Err(PrimitiveStoreError::InvalidTransition);
    }

    // Stored step metadata does not prove the kind of the immutable pinned definition.
    // This applies to every fresh reason; durable replay returns before this validation.
    let definition =
        data_limit_definition(store, conn, &drain.transition, locked.run.version_id).await?;
    if !definition
        .node(drain.transition.owner().node())
        .is_some_and(|owner| matches!(owner.kind(), NodeKind::ForEach { .. }))
    {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    let run_limit =
        u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| invalid_storage("run output limit"))?;
    match evidence {
        CollectionDataLimitEvidence::StoppedForRunDataLimit => {
            let run_data_limit = FailureCode::run_data_limit();
            if locked.status != "CANCELLING"
                || locked.stop_reason.as_deref() != Some("FAILED")
                || locked.stop_code.as_deref() != Some(run_data_limit.as_str())
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            Ok(false)
        }
        CollectionDataLimitEvidence::ExistingOutputs => {
            if !matches!(locked.status.as_str(), "RUNNING" | "WAITING")
                || locked.stop_reason.is_some()
                || locked.stop_code.is_some()
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            let effective =
                effective_output_snapshot(store, conn, &drain.transition, locked.run).await?;
            if effective.total() <= run_limit {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            Ok(true)
        }
        CollectionDataLimitEvidence::Aggregate(aggregate) => {
            if !matches!(locked.status.as_str(), "RUNNING" | "WAITING")
                || locked.stop_reason.is_some()
                || locked.stop_code.is_some()
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            let stored_item_count = scope.get::<i32, _>("item_count");
            let item_count = u32::try_from(stored_item_count)
                .map_err(|_| invalid_storage("collection item count"))?;
            if scope.get::<i32, _>("next_admission_index") != stored_item_count
                || scope.get::<i32, _>("admitted_count") != stored_item_count
                || scope.get::<i32, _>("completed_count") != stored_item_count
                || scope.get::<i32, _>("active_count") != 0
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            let rows = sqlx::query(
                "SELECT member.iteration_index,member.lifecycle,member.slot_held, \
                        checkpoint.result_digest, \
                        (member.return_path_nodes IS NOT NULL \
                         AND member.return_path_indices IS NOT NULL \
                         AND member.return_node IS NOT NULL \
                         AND member.return_instance_id IS NOT NULL \
                         AND member.result_digest IS NOT NULL \
                         AND member.checkpoint_sequence IS NOT NULL \
                         AND member.checkpoint_sequence > 0 \
                         AND member.return_instance_id=checkpoint.return_instance_id \
                         AND member.result_digest=checkpoint.result_digest \
                         AND member.checkpoint_sequence=checkpoint.journal_sequence \
                         AND member.return_instance_id=returned.step_id \
                         AND member.remaining_reservations >= 0 \
                         AND member.drained_sequence IS NULL \
                         AND member.drain_outbox_id IS NULL) IS TRUE AS complete_checkpoint \
                 FROM collection_iterations member \
                 LEFT JOIN iteration_checkpoints checkpoint \
                   ON checkpoint.tenant_id=member.tenant_id \
                  AND checkpoint.run_id=member.run_id \
                  AND checkpoint.scope_id=member.scope_id \
                  AND checkpoint.iteration_index=member.iteration_index \
                 LEFT JOIN steps returned \
                   ON returned.tenant_id=member.tenant_id AND returned.run_id=member.run_id \
                  AND returned.namespace_id=$4 \
                  AND returned.instance_path_nodes=member.return_path_nodes \
                  AND returned.instance_path_indices=member.return_path_indices \
                  AND returned.node=member.return_node \
                 WHERE member.tenant_id=$1 AND member.run_id=$2 AND member.scope_id=$3 \
                 ORDER BY member.iteration_index",
            )
            .bind(drain.transition.tenant_id().as_uuid())
            .bind(drain.transition.run_id().as_uuid())
            .bind(drain.scope_id.as_uuid())
            .bind(drain.transition.namespace_id().as_uuid())
            .fetch_all(&mut *conn)
            .await
            .map_err(store_error)?;
            if rows.len() != usize::try_from(item_count).unwrap_or(usize::MAX)
                || rows.len() != aggregate.source_digests().len()
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            for (position, (row, (expected_index, expected_digest))) in
                rows.iter().zip(aggregate.source_digests()).enumerate()
            {
                let index = u32::try_from(row.get::<i32, _>("iteration_index"))
                    .map_err(|_| invalid_storage("collection result index"))?;
                let stored = row
                    .get::<Option<Vec<u8>>, _>("result_digest")
                    .ok_or(PrimitiveStoreError::InvalidTransition)?;
                let stored = <[u8; 32]>::try_from(stored)
                    .map(Digest::new)
                    .map_err(|_| invalid_storage("collection result digest"))?;
                if usize::try_from(index).ok() != Some(position)
                    || row.get::<String, _>("lifecycle") != "CHECKPOINTED"
                    || row.get::<bool, _>("slot_held")
                    || !row.get::<bool, _>("complete_checkpoint")
                    || index != *expected_index
                    || stored != *expected_digest
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
            if item_count == 0 {
                let empty_digest = Digest::new(Sha256::digest(b"[]").into());
                if aggregate.byte_len() != 2
                    || !aggregate.source_digests().is_empty()
                    || aggregate.result_digest() != empty_digest
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
            let effective =
                effective_output_snapshot(store, conn, &drain.transition, locked.run).await?;
            let exceeds_run = match effective.total().checked_add(aggregate.byte_len()) {
                Some(total) => total > run_limit,
                None => true,
            };
            if aggregate.byte_len()
                <= u64::try_from(MAX_PAYLOAD_BYTES).map_err(|_| invalid_storage("payload limit"))?
                && !exceeds_run
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            Ok(true)
        }
    }
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
    status: &str,
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
    tx: &mut Transaction<'_, Postgres>,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    let authority = transition.scheduler();
    let lease = authority.lease();
    if lease.shard != Shard::of(transition.run_id().as_uuid()) {
        return Err(PrimitiveStoreError::SchedulerFenceLost);
    }
    // Keep this port's existing conversion error for an oversized generation.
    number(lease.generation, "scheduler generation")?;
    if crate::execution::locked_scheduler_time(tx, transition.run_id(), authority.owner(), lease)
        .await?
        .is_none()
    {
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
        if let Some(manifest) = SourceManifest::parse(&snapshot) {
            if usize::try_from(manifest.count()).ok() != Some(items.len())
                || source_items_digest(&items) != *manifest.items_digest()
            {
                return Err(invalid_storage("collection membership projection"));
            }
        } else {
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
        }
        Ok(Some(CollectionProgress {
            scope_id,
            snapshot,
            items,
            results,
        }))
    }

    async fn collection_scope(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<CollectionScopeRef>, PrimitiveStoreError> {
        let (owner_nodes, owner_indices) = path_parts(owner);
        let Some(row) = sqlx::query(
            "SELECT scope_id, item_count FROM iteration_scopes \
             WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes=$3 \
               AND owner_path_indices=$4 AND owner_node=$5",
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
        Ok(Some(CollectionScopeRef {
            scope_id: CollectionScopeId::new(row.get("scope_id")),
            item_count: u32::try_from(row.get::<i32, _>("item_count"))
                .map_err(|_| invalid_storage("collection item count"))?,
        }))
    }

    async fn collection_item(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        scope_id: CollectionScopeId,
        index: u32,
    ) -> Result<Option<Vec<u8>>, PrimitiveStoreError> {
        let Some(row) = sqlx::query(
            "SELECT i.item_digest, p.content \
             FROM collection_scope_items i JOIN payloads p \
               ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest \
             WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.scope_id=$3 AND i.iteration_index=$4",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(scope_id.as_uuid())
        .bind(i32::try_from(index).map_err(|_| invalid_storage("collection item index"))?)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let content: Vec<u8> = row.get("content");
        let digest: Vec<u8> = row.get("item_digest");
        if Sha256::digest(&content).to_vec() != digest {
            return Err(invalid_storage("collection item payload"));
        }
        Ok(Some(content))
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
        let item_payloads = if let Some(source) = entry.blob_source() {
            // The snapshot payload is the manifest of a blob read: it must describe exactly the
            // items the entry carries, so the digest the scope pins is the membership.
            let manifest = SourceManifest::parse(&snapshot_content)
                .ok_or(PrimitiveStoreError::InvalidTransition)?;
            if manifest.blob().id() != source.blob
                || usize::try_from(manifest.count()).ok() != Some(source.items.len())
                || source.items.len() != usize::try_from(entry.item_count).unwrap_or(usize::MAX)
                || source
                    .items
                    .iter()
                    .any(|item| item.len() > MAX_PAYLOAD_BYTES)
                || source_items_digest(&source.items) != *manifest.items_digest()
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            source
                .items
                .iter()
                .map(|item| (Sha256::digest(item).to_vec(), item.clone()))
                .collect::<Vec<_>>()
        } else {
            let snapshot: Value = serde_json::from_slice(&snapshot_content)
                .map_err(|_| PrimitiveStoreError::InvalidTransition)?;
            let Value::Array(items) = snapshot else {
                return Err(PrimitiveStoreError::InvalidTransition);
            };
            if items.len() != usize::try_from(entry.item_count).unwrap_or(usize::MAX) {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            items
                .iter()
                .map(|item| {
                    serde_json::to_vec(item)
                        .map(|bytes| (Sha256::digest(&bytes).to_vec(), bytes))
                        .map_err(|_| invalid_storage("collection snapshot item"))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
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
            "SELECT max_instances, admitted_instances, reserved_instances, retired_reservations FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let consumed = i64::from(usage.get::<i32, _>("admitted_instances"))
            + i64::from(usage.get::<i32, _>("reserved_instances"))
            + i64::from(usage.get::<i32, _>("retired_reservations"));
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
              created_at, updated_at, source_blob_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'COLLECTION', 'OPEN', $8, $9, $10, $11, $12, $12, $13)",
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
        .bind(entry.blob_source().map(|source| source.blob.as_uuid()))
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
                "source_blob": entry.blob_source().map(|source| source.blob.to_string()),
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
        if !matches!(admission.dispatch, CollectionEntryDispatch::Activity { .. }) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        self.admit_collection_member(admission).await
    }

    async fn admit_collection_member(
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
        let (dispatch, activity, outbox_id, node_kind) =
            match (&admission.dispatch, entry_definition.kind()) {
                (
                    CollectionEntryDispatch::Activity {
                        activity,
                        outbox_id,
                    },
                    NodeKind::Activity {
                        activity: declared, ..
                    },
                ) if activity == declared => (
                    "ACTIVITY",
                    Some(activity.as_str()),
                    Some(*outbox_id),
                    "activity",
                ),
                (CollectionEntryDispatch::Scheduler, NodeKind::Condition { .. }) => {
                    ("SCHEDULER", None, None, "condition")
                }
                (CollectionEntryDispatch::Scheduler, NodeKind::Switch { .. }) => {
                    ("SCHEDULER", None, None, "switch")
                }
                (CollectionEntryDispatch::Scheduler, NodeKind::Timer { .. }) => {
                    ("SCHEDULER", None, None, "timer")
                }
                (CollectionEntryDispatch::Scheduler, NodeKind::WaitEvent { .. }) => {
                    ("SCHEDULER", None, None, "waitEvent")
                }
                _ => return Err(PrimitiveStoreError::InvalidTransition),
            };
        if admission.entry.node() != body.entrypoint()
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
            "SELECT entry_instance_id, item_digest, activity, reserved_instances, outbox_id, entry_dispatch \
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
                && row.get::<Option<String>, _>("activity").as_deref() == activity
                && row.get::<String, _>("entry_dispatch") == dispatch
                && row.get::<i32, _>("reserved_instances")
                    == i32::from(admission.reserved_instances)
                && row.get::<Option<Uuid>, _>("outbox_id") == outbox_id;
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
            "SELECT max_instances, admitted_instances, reserved_instances, retired_reservations FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let charge = 1_i64 + i64::from(admission.reserved_instances);
        let consumed = i64::from(usage.get::<i32, _>("admitted_instances"))
            + i64::from(usage.get::<i32, _>("reserved_instances"))
            + i64::from(usage.get::<i32, _>("retired_reservations"));
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
        let final_sequence = admitted_sequence + u64::from(activity.is_some());
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
              last_hinted_at, hint_count, updated_at, instance_path_nodes, instance_path_indices, node_kind) \
             VALUES ($1, $2, $3, $4, $5, $10, $6, $11, $11, $12, $7, $8, $9, $13)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(run.namespace_id.as_uuid())
        .bind(activity)
        .bind(admission.transition.occurred_at())
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .bind(if activity.is_some() { "READY" } else { "PENDING" })
        .bind(activity.map(|_| admission.transition.occurred_at()))
        .bind(i32::from(activity.is_some()))
        .bind(node_kind)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO collection_iterations \
             (tenant_id, run_id, scope_id, iteration_index, entry_path_nodes, entry_path_indices, \
              entry_node, entry_instance_id, activity, item_digest, lifecycle, slot_held, \
              reserved_instances, remaining_reservations, admitted_sequence, outbox_id, \
              created_at, updated_at, entry_dispatch) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'ADMITTED', true, $11, $11, \
                     $12, $13, $14, $14, $15)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(activity)
        .bind(admission.item_digest.as_bytes().as_slice())
        .bind(i32::from(admission.reserved_instances))
        .bind(number(admitted_sequence, "journal sequence")?)
        .bind(outbox_id)
        .bind(admission.transition.occurred_at())
        .bind(dispatch)
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
        let mut admission_detail = json!({
            "index": admission.index,
            "entry": instance_json(&admission.entry),
            "item_digest": admission.item_digest.to_hex(),
        });
        if matches!(admission.dispatch, CollectionEntryDispatch::Scheduler) {
            admission_detail["entry_dispatch"] = json!("SCHEDULER");
        }
        append_event(
            &mut tx,
            &admission.transition,
            run,
            admitted_sequence,
            "collection.iteration_admitted",
            admission_detail,
        )
        .await?;
        if let CollectionEntryDispatch::Activity {
            activity,
            outbox_id,
        } = &admission.dispatch
        {
            append_instance_event(
                &mut tx,
                &admission.transition,
                run,
                &admission.entry,
                final_sequence,
                "step.released",
                json!({}),
            )
            .await?;
            sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4, $5)",
        )
        .bind(outbox_id)
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(activity.as_str())
        .bind(json!({
            "tenantId": admission.transition.tenant_id().to_string(),
            "namespaceId": admission.transition.namespace_id().to_string(),
            "runId": admission.transition.run_id().to_string(),
            "node": admission.entry.node().as_str(),
            "instancePath": instance_json(&admission.entry)["path"],
            "activity": activity.as_str(),
            "traceId": admission.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        }
        finish_version(&mut tx, &admission.transition, final_sequence).await?;
        mark_telemetry_pending(&mut tx, &admission.transition, final_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: final_sequence,
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
        let materialized: std::collections::BTreeSet<String> =
            member_steps.iter().map(|row| row.get("node")).collect();
        for member in &member_steps {
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
            } else {
                let instance = ExecutionInstanceId::in_path(
                    checkpoint.return_instance.path().clone(),
                    NodeId::parse_stored(&node)
                        .map_err(|_| invalid_storage("collection member node"))?,
                );
                let handled = definition
                    .node_for_instance(&instance)
                    .and_then(Node::on_error)
                    .is_some_and(|target| materialized.contains(target.as_str()));
                let status = member.get::<String, _>("status");
                if !member.get::<bool, _>("routed")
                    || !(matches!(status.as_str(), "SUCCEEDED" | "SKIPPED")
                        || (status == "FAILED" && handled))
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
        }
        if !found_return {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let Some(NodeKind::ForEach { body, .. }) = definition
            .node_for_instance(checkpoint.transition.owner())
            .map(Node::kind)
        else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let body = definition
            .body(body)
            .ok_or(PrimitiveStoreError::InvalidTransition)?;
        // Unselected branches can never activate after Return. Retire their forward and
        // derived reservations, preserving the lifetime charge and all accepted drain work.
        let retired = i32::try_from(
            body.all_nodes()
                .filter(|node| {
                    let forward = match node.origin() {
                        sagat_domain::saga::NodeOrigin::Declared => node.id(),
                        sagat_domain::saga::NodeOrigin::Compensation(forward)
                        | sagat_domain::saga::NodeOrigin::Reconciliation(forward) => forward,
                    };
                    !materialized.contains(forward.as_str())
                })
                .count(),
        )
        .map_err(|_| invalid_storage("collection branch reservations"))?;
        let remaining = iteration
            .get::<i32, _>("remaining_reservations")
            .checked_sub(1)
            .and_then(|value| value.checked_sub(retired))
            .filter(|value| *value >= 0)
            .ok_or_else(|| invalid_storage("collection branch reservations"))?;

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
                    remaining_reservations = $12, \
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
        .bind(remaining)
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
                    reserved_instances = reserved_instances - 1 - $3, \
                    retired_reservations = retired_reservations + $3 \
             WHERE tenant_id = $1 AND run_id = $2 AND reserved_instances >= 1 + $3",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(retired)
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
        let data_limit_evidence = checked_data_limit_evidence(drain)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let locked = lock_drain_run(&mut tx, &drain.transition).await?;
        check_scheduler_fence(&mut tx, &drain.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(drain.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id,owner_path_nodes,owner_path_indices,owner_node,kind,lifecycle, \
                    item_count,next_admission_index,admitted_count,completed_count,active_count, \
                    result_digest,drain_kind,drain_iteration,drain_failure_code,drain_sequence, \
                    drain_outbox_id \
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
        let run_data_limit_code = FailureCode::run_data_limit();
        let (kind, iteration, code) = match &drain.reason {
            CollectionDrainReason::Failed { iteration, code } => (
                "FAILED",
                Some(
                    i32::try_from(*iteration)
                        .map_err(|_| PrimitiveStoreError::InvalidTransition)?,
                ),
                Some(code.as_str()),
            ),
            CollectionDrainReason::RunDataLimit => {
                ("RUN_DATA_LIMIT", None, Some(run_data_limit_code.as_str()))
            }
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
            CollectionDrainReason::RunDataLimit => {
                let evidence = data_limit_evidence.ok_or(PrimitiveStoreError::InvalidTransition)?;
                if validate_data_limit_evidence(self, &mut tx, drain, &locked, &scope, evidence)
                    .await?
                {
                    sequence = sequence
                        .checked_add(1)
                        .ok_or_else(|| invalid_storage("journal sequence"))?;
                    append_run_event(
                        &mut tx,
                        &drain.transition,
                        sequence,
                        "run.stop_requested",
                        json!({"reason":"FAILED","code":run_data_limit_code.as_str()}),
                    )
                    .await?;
                    let updated = sqlx::query(
                        "UPDATE runs SET status='CANCELLING',stop_reason='FAILED',stop_code=$3 \
                         WHERE tenant_id=$1 AND id=$2 AND status IN ('RUNNING','WAITING') \
                           AND stop_reason IS NULL AND stop_code IS NULL",
                    )
                    .bind(drain.transition.tenant_id().as_uuid())
                    .bind(drain.transition.run_id().as_uuid())
                    .bind(run_data_limit_code.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                    if updated.rows_affected() != 1 {
                        return Err(PrimitiveStoreError::InvalidTransition);
                    }
                }
            }
            CollectionDrainReason::Cancelled => {
                if locked.status != "CANCELLING"
                    || locked.stop_reason.as_deref() != Some("CANCELLED")
                    || locked.stop_code.is_some()
                {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
            }
            CollectionDrainReason::Failed { iteration, code } => {
                // Either a member failed with this code, or the iteration's body is done and its
                // result boundary (the pending `return`) cannot be evaluated or retained: an
                // expression error or a value over a ceiling, which no worker report can show.
                let definition = self
                    .definition(
                        &mut tx,
                        drain.transition.tenant_id().as_uuid(),
                        locked.run.version_id,
                    )
                    .await?;
                let members = sqlx::query(
                    "SELECT s.node,s.status,s.routed,s.failure_code,s.attempt,s.generation, \
                            s.instance_path_nodes,s.instance_path_indices \
                     FROM collection_iterations i JOIN steps s \
                       ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id \
                      AND s.instance_path_nodes=i.entry_path_nodes \
                      AND s.instance_path_indices=i.entry_path_indices \
                     WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.scope_id=$3 \
                       AND i.iteration_index=$4 AND i.lifecycle='ADMITTED' ORDER BY s.node",
                )
                .bind(drain.transition.tenant_id().as_uuid())
                .bind(drain.transition.run_id().as_uuid())
                .bind(drain.scope_id.as_uuid())
                .bind(i32::try_from(*iteration).unwrap_or(i32::MAX))
                .fetch_all(&mut *tx)
                .await
                .map_err(store_error)?;
                let materialized: std::collections::BTreeSet<String> =
                    members.iter().map(|row| row.get("node")).collect();
                let mut failed = false;
                let mut pending_return = false;
                let mut closed_body = !members.is_empty();
                for member in &members {
                    let instance = stored_instance(
                        member.get("instance_path_nodes"),
                        member.get("instance_path_indices"),
                        member.get("node"),
                    )?;
                    let declared = definition
                        .node_for_instance(&instance)
                        .ok_or_else(|| invalid_storage("collection member declaration"))?;
                    let status: String = member.get("status");
                    let routed: bool = member.get("routed");
                    let handled = routed
                        && declared
                            .on_error()
                            .is_some_and(|target| materialized.contains(target.as_str()));
                    failed |= status == "FAILED"
                        && !handled
                        && member.get::<Option<String>, _>("failure_code").as_deref()
                            == Some(code.as_str());
                    let is_return = matches!(declared.kind(), NodeKind::Return { .. })
                        && status == "PENDING"
                        && !routed
                        && member.get::<i32, _>("attempt") == 0
                        && member.get::<i64, _>("generation") == 0;
                    pending_return |= is_return;
                    closed_body &= is_return
                        || (routed
                            && (matches!(status.as_str(), "SUCCEEDED" | "SKIPPED")
                                || (status == "FAILED" && handled)));
                }
                failed |= matches!(code.as_str(), "EXPRESSION_ERROR" | "RUN_DATA_LIMIT")
                    && pending_return
                    && closed_body;
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
            CollectionDrainReason::RunDataLimit => json!({"reason":"run_data_limit"}),
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
            "reason":if matches!(drain.reason, CollectionDrainReason::RunDataLimit) {
                "FAILED"
            } else {
                kind
            },
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
        let definition = self
            .definition(
                &mut tx,
                drain.transition.tenant_id().as_uuid(),
                locked.run.version_id,
            )
            .await?;
        let member_steps = sqlx::query(concat!(
            "SELECT s.node,s.status,s.attempt,s.generation,s.cancel_requested,s.routed,s.condition,s.switch_case_id, \
                s.timed_out,s.failure_code,s.next_attempt_at,s.waiting_kind,s.due_at,s.correlation_key, \
                s.child_run_id,s.approval_request_id,s.namespace_id,s.step_id,s.event_type,s.event_since,s.node_kind AS stored_node_kind, (",
            crate::execution::scoped_timer_metadata_valid!(),") AS timer_metadata_valid, (",
            crate::execution::scoped_event_metadata_valid!(),") AS event_metadata_valid \
             FROM steps s WHERE tenant_id=$1 AND run_id=$2 \
               AND instance_path_nodes=$3 AND instance_path_indices=$4 ORDER BY node FOR UPDATE",
        ))
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
        let member_path = crate::execution::instance_path(
            path_nodes.clone(),
            path_indices.clone(),
            "collection drain path",
        )
        .map_err(PrimitiveStoreError::Storage)?;
        let mut event_waits = Vec::new();
        for step in &member_steps {
            let node = NodeId::parse_stored(&step.get::<String, _>("node"))
                .map_err(|_| invalid_storage("collection member node"))?;
            let status = step.get::<String, _>("status");
            let terminal = matches!(
                status.as_str(),
                "SUCCEEDED" | "FAILED" | "CANCELLED" | "SKIPPED"
            );
            // Nothing is in flight: a step that has not started, or that waits to retry or is
            // ready to retry in a run that is stopping (it can no longer be leased). One whose last
            // attempt was lost keeps saying so when it is cancelled, and is compensated.
            let safe_unstarted = (matches!(status.as_str(), "PENDING" | "READY")
                && step.get::<i32, _>("attempt") == 0)
                || matches!(status.as_str(), "READY" | "RETRY_WAIT");
            let instance = ExecutionInstanceId::in_path(member_path.clone(), node.clone());
            let waiting_suspension = if matches!(
                definition.node_for_instance(&instance).map(Node::kind),
                Some(NodeKind::Timer { .. } | NodeKind::WaitEvent { .. })
            ) {
                let state =
                    crate::execution::step_state(step).map_err(PrimitiveStoreError::Storage)?;
                crate::execution::validate_scoped_control_row(
                    definition.node_for_instance(&instance),
                    &state,
                    step,
                )
                .map_err(PrimitiveStoreError::Storage)?;
                crate::execution::validate_scoped_event_identity(
                    definition.node_for_instance(&instance),
                    locked.run.namespace_id,
                    instance_step_id(locked.run.version_id, drain.transition.run_id(), &instance),
                    step,
                )
                .map_err(PrimitiveStoreError::Storage)?;
                if status == "WAITING"
                    && matches!(
                        definition.node_for_instance(&instance).map(Node::kind),
                        Some(NodeKind::WaitEvent { .. })
                    )
                {
                    event_waits.push((node.clone(), step));
                }
                status == "WAITING"
            } else {
                false
            };
            if (node.is_derived() && !matches!(status.as_str(), "SUCCEEDED" | "SKIPPED"))
                || (!terminal && !safe_unstarted && !waiting_suspension)
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }
        let failure_entry = scope.get::<Option<String>, _>("drain_kind").as_deref()
            == Some("FAILED")
            && scope.get::<Option<i32>, _>("drain_iteration")
                == Some(i32::try_from(drain.index).unwrap_or(i32::MAX));
        let failure_code = scope.get::<Option<String>, _>("drain_failure_code");
        if failure_entry
            && event_waits
                .iter()
                .any(|(node, _)| node.as_str() == entry_node)
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        for (node, step) in event_waits {
            let updated = sqlx::query(concat!(
                "UPDATE steps s SET status='CANCELLED',routed=true,failure_code=NULL, \
                    waiting_kind=NULL,due_at=NULL,event_type=NULL,correlation_key=NULL,event_since=NULL,event_scanned_at=NULL, \
                    next_attempt_at=NULL,cancel_requested=false,updated_at=$12 \
                 WHERE tenant_id=$1 AND run_id=$2 AND namespace_id=$3 AND instance_path_nodes=$4 AND instance_path_indices=$5 \
                   AND node=$6 AND step_id=$7 AND node_kind='waitEvent' AND status='WAITING' AND NOT routed \
                   AND event_type=$8 AND correlation_key=$9 AND due_at=$10 AND event_since=$11 AND (",
                crate::execution::scoped_event_metadata_valid!(),")"))
                .bind(drain.transition.tenant_id().as_uuid())
                .bind(drain.transition.run_id().as_uuid())
                .bind(locked.run.namespace_id.as_uuid())
                .bind(&path_nodes)
                .bind(&path_indices)
                .bind(node.as_str())
                .bind(step.get::<Uuid, _>("step_id"))
                .bind(step.get::<String, _>("event_type"))
                .bind(step.get::<String, _>("correlation_key"))
                .bind(step.get::<OffsetDateTime, _>("due_at"))
                .bind(step.get::<OffsetDateTime, _>("event_since"))
                .bind(drain.transition.occurred_at())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?
                .rows_affected();
            if updated != 1 {
                return Err(invalid_storage("collection event wait drain binding"));
            }
        }
        sqlx::query(
            "UPDATE steps SET status=CASE WHEN $5 AND node=$6 THEN 'FAILED' ELSE 'CANCELLED' END, \
                    failure_code=CASE WHEN $5 AND node=$6 THEN $7 ELSE failure_code END, \
                    routed=true,waiting_kind=NULL,due_at=NULL,event_type=NULL,correlation_key=NULL,event_since=NULL,event_scanned_at=NULL, \
                    next_attempt_at=NULL,cancel_requested=false,updated_at=$8 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 \
               AND ((status IN ('PENDING','READY') AND attempt=0) OR status IN ('READY','RETRY_WAIT') \
                 OR (status='WAITING' AND node_kind='timer' AND waiting_kind='TIMER' AND attempt=0 AND generation=0))",
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
        // Decide from the actual post-drain states, including a lost attempt that became
        // CANCELLED with its unknown-outcome code. This is the same predicate as the planner.
        let mut kept = 0_i32;
        if definition.scope_compensates(drain.transition.owner()) {
            let drained_steps = sqlx::query(
                "SELECT node,status,failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 \
                   AND instance_path_nodes=$3 AND instance_path_indices=$4 ORDER BY node",
            )
            .bind(drain.transition.tenant_id().as_uuid())
            .bind(drain.transition.run_id().as_uuid())
            .bind(&path_nodes)
            .bind(&path_indices)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
            let materialized: std::collections::BTreeSet<String> =
                drained_steps.iter().map(|row| row.get("node")).collect();
            let body = definition
                .scope_body(drain.transition.owner())
                .ok_or(PrimitiveStoreError::InvalidTransition)?;
            for step in &drained_steps {
                let node = NodeId::parse_stored(&step.get::<String, _>("node"))
                    .map_err(|_| invalid_storage("collection compensation source"))?;
                if node.is_derived()
                    || body.node(&node.compensation()).is_none()
                    || materialized.contains(node.compensation().as_str())
                {
                    continue;
                }
                let status = step
                    .get::<String, _>("status")
                    .parse()
                    .map_err(|_| invalid_storage("collection compensation source status"))?;
                let failure = step
                    .get::<Option<String>, _>("failure_code")
                    .as_deref()
                    .map(FailureCode::parse)
                    .transpose()
                    .map_err(|_| invalid_storage("collection compensation source failure"))?;
                if sagat_domain::execution::scope_compensation_candidate(status, failure.as_ref()) {
                    kept += 1;
                }
            }
        }
        if remaining < 0 || kept > remaining {
            return Err(invalid_storage("collection compensation reservations"));
        }
        let sequence = locked.run.run_version + 1;
        sqlx::query(
            "UPDATE collection_iterations SET lifecycle='DRAINED',slot_held=false, \
                    remaining_reservations=$8,drained_sequence=$5,drain_outbox_id=$6,updated_at=$7 \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=$4",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(drain.scope_id.as_uuid())
        .bind(i32::try_from(drain.index).unwrap_or(i32::MAX))
        .bind(number(sequence, "journal sequence")?)
        .bind(drain.outbox_id)
        .bind(drain.transition.occurred_at())
        .bind(kept)
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
            "UPDATE run_instance_usage SET reserved_instances=reserved_instances-$3, \
                    retired_reservations=retired_reservations+$3 \
             WHERE tenant_id=$1 AND run_id=$2 AND reserved_instances >= $3",
        )
        .bind(drain.transition.tenant_id().as_uuid())
        .bind(drain.transition.run_id().as_uuid())
        .bind(remaining - kept)
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
                    active_count,drain_kind,drain_iteration,drain_failure_code,finish_sequence, \
                    finish_outbox_id \
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
        let iteration = scope.get::<Option<i32>, _>("drain_iteration");
        let code = scope.get::<Option<String>, _>("drain_failure_code");
        let (outcome, terminal_status, expected_reason) = match (kind.as_str(), code.as_deref()) {
            ("FAILED", Some(code)) => (json!({"outcome":"failed","code":code}), "FAILED", "FAILED"),
            ("RUN_DATA_LIMIT", Some(code))
                if iteration.is_none() && code == FailureCode::run_data_limit().as_str() =>
            {
                (json!({"outcome":"failed","code":code}), "FAILED", "FAILED")
            }
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
        // A scope that compensates ends the run with the worse of the stop and how its
        // compensations ended (D385); it cannot end while one is still open.
        let definition = self
            .definition(
                &mut tx,
                finish.transition.tenant_id().as_uuid(),
                locked.run.version_id,
            )
            .await?;
        let run_status = if definition.scope_compensates(finish.transition.owner()) {
            run_status_after_compensation(
                &mut tx,
                &finish.transition,
                &owner_nodes,
                &owner_indices,
                &locked,
            )
            .await?
        } else {
            terminal_status.to_owned()
        };
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
            json!({"status":run_status}),
        )
        .await?;
        let run_finished = sqlx::query(
            "UPDATE runs SET status=$3,run_version=$4,updated_at=$5,finished_at=$5 \
             WHERE tenant_id=$1 AND id=$2 AND status='CANCELLING'",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(&run_status)
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
        outbox_run_finished(&mut tx, &finish.transition, &definition, &run_status).await?;
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
            let finish_sequence = scope.get::<Option<i64>, _>("finish_sequence");
            let matches = scope.get::<Option<Vec<u8>>, _>("result_digest")
                == Some(finish.result_digest.as_bytes().to_vec())
                && scope.get::<Option<Uuid>, _>("finish_outbox_id") == Some(finish.outbox_id)
                && finish_sequence.is_some();
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            validate_finish_replay(
                &mut tx,
                finish,
                &definition,
                run.version_id,
                u64::try_from(finish_sequence.unwrap_or_default())
                    .map_err(|_| invalid_storage("finish sequence"))?,
            )
            .await?;
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        if locked_run.paused || !matches!(locked_run.status.as_str(), "RUNNING" | "WAITING") {
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
        let Some(owner_definition) = definition.node_for_instance(finish.transition.owner()) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        let NodeKind::ForEach { result, .. } = owner_definition.kind() else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
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
        if !successor_is_end && finish.end_answer().is_some() {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let (aggregate_bytes, effective_output) =
            fresh_collection_aggregate_preflight(self, &mut tx, finish, run).await?;
        let aggregate_content = if successor_is_end
            && finish.transition.owner().path().is_root()
            && let Some(content) = finish.result_content()
        {
            content.to_vec()
        } else {
            retain_payload_in_transaction(
                &mut tx,
                finish.transition.tenant_id(),
                finish.result_digest,
                finish.result_content(),
            )
            .await?
        };
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
        if successor_is_end && finish.transition.owner().path().is_root() {
            let run_store = PgRunStore::new(
                self.pool.clone(),
                self.definitions
                    .clone()
                    .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?,
            );
            let mut run_tx = run_store
                .begin_in_transaction(
                    tx,
                    finish.transition.tenant_id(),
                    finish.transition.run_id(),
                    Lock::Wait,
                    None,
                )
                .await?
                .ok_or(PrimitiveStoreError::Missing)?;
            let finish_sequence = run_tx.parts().1.version() + 1;
            let finished = RunEvent::CollectionFinished {
                node: finish.transition.owner().node().clone(),
                outcome: CollectionOutcome::Succeeded,
                result_digest: Some(finish.result_digest),
            };
            run_tx
                .parts()
                .1
                .apply(&finished)
                .map_err(|_| PrimitiveStoreError::InvalidTransition)?;
            let now = run_tx.now();
            let mut progress = {
                let (definition, state) = run_tx.parts();
                sagat_domain::execution::release_pending(definition, state, now)
                    .map_err(|_| PrimitiveStoreError::InvalidTransition)?
            };
            if !matches!(
                progress.events.first(),
                Some(RunEvent::StepRouted { node, activated })
                    if node == finish.transition.owner().node()
                        && activated == std::slice::from_ref(successor.node())
            ) {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
            let mut events = vec![finished];
            events.append(&mut progress.events);
            let mut effects = Effects::by(
                Actor::System("engine"),
                finish.transition.trace_id().map(str::to_owned),
            );
            effects.outputs.push((
                finish.transition.owner().node().clone(),
                aggregate_content.clone(),
            ));
            effects.audit = finish.transition.audit().cloned();
            match progress.pending.take() {
                Some(Evaluation::Output { node, .. }) if node == *successor.node() => {
                    let (outcome, output) = match finish.end_answer() {
                        Some(EndOutputAnswer::Succeeded(bytes)) => (Ok(()), Some(bytes.clone())),
                        Some(EndOutputAnswer::ExpressionError) => {
                            (Err(FailureCode::expression_error()), None)
                        }
                        Some(EndOutputAnswer::RunDataLimit) => {
                            (Err(FailureCode::run_data_limit()), None)
                        }
                        None => return Err(PrimitiveStoreError::InvalidTransition),
                    };
                    if let Some(output) = output {
                        effects.outputs.push((node.clone(), output));
                    }
                    let now = run_tx.now();
                    progress = {
                        let (definition, state) = run_tx.parts();
                        sagat_domain::execution::resolve(
                            definition,
                            state,
                            Resolution::Output { node, outcome },
                            now,
                        )
                        .map_err(|_| PrimitiveStoreError::InvalidTransition)?
                    };
                    events.append(&mut progress.events);
                    if progress.pending.is_some() {
                        return Err(PrimitiveStoreError::InvalidTransition);
                    }
                }
                None if finish.end_answer().is_none() => {}
                Some(_) | None => return Err(PrimitiveStoreError::InvalidTransition),
            }
            run_tx.persist(&events, effects).await?;
            run_tx
                .project_collection_finish(finish, finish_sequence)
                .await?;
            let final_version = run_tx.parts().1.version();
            run_tx.commit().await?;
            return Ok(PrimitiveWrite {
                applied: true,
                run_version: final_version,
            });
        }
        let owner_output = finish
            .transition
            .owner()
            .path()
            .is_root()
            .then_some(finish.result_digest.as_bytes().as_slice());
        if owner_output.is_some() {
            crate::execution::charge_collection_owner_output(
                &mut tx,
                finish.transition.tenant_id(),
                finish.transition.run_id(),
                finish.transition.owner(),
                effective_output
                    .as_ref()
                    .ok_or_else(|| invalid_storage("root collection output snapshot"))?,
                aggregate_bytes,
            )
            .await?;
        }
        let owner_finished = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', waiting_kind = NULL, routed = true, \
                    updated_at = $6, output_digest = COALESCE($7, output_digest) \
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
        .bind(owner_output)
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

    async fn collection_scope(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<CollectionScopeRef>, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .collection_scope(tenant_id, run_id, owner)
            .await
    }

    async fn collection_item(
        &self,
        tenant_id: TenantId,
        run_id: sagat_domain::execution::RunId,
        scope_id: CollectionScopeId,
        index: u32,
    ) -> Result<Option<Vec<u8>>, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .collection_item(tenant_id, run_id, scope_id, index)
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

    async fn admit_collection_member(
        &self,
        admission: &CollectionAdmission,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        PgCollectionStore::with_definition_cache(self.pool.clone(), self.cache.clone())
            .admit_collection_member(admission)
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
