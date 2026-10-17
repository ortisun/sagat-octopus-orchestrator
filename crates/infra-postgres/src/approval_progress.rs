//! PostgreSQL transaction boundary for workflow approval progress.

use std::str::FromStr;

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalExpiryAttempt, ApprovalListItem,
    ApprovalOpening, ApprovalPageSize, ApprovalProgressError, ApprovalProgressStore,
    ApprovalRoutes,
};
use sagat_application::error::StoreError;
use sagat_application::execution::instance_step_id;
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionReceipt, ApprovalDecisionResolution, ApprovalError,
    ApprovalPolicy, ApprovalReasonCode, ApprovalReceiptId, ApprovalRequest, ApprovalRequestId,
    ApprovalStatus, ApprovalTransition, DecisionKey, ExecutionInstanceId, InstancePath, RunEvent,
    RunId,
};
use sagat_domain::identity::{Principal, Role, Subject, TenantId};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, NodeId};
use sagat_telemetry::{WorkflowPrimitiveMetric, WorkflowPrimitiveOutcome};
use serde_json::json;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::event_codec::{decode, encode};
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;

#[derive(Debug, Clone)]
pub struct PgApprovalProgressStore {
    pool: PgPool,
}

impl PgApprovalProgressStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct LockedRun {
    version: u64,
    workflow_version: WorkflowVersionId,
    /// The run is terminal. Only a caller that can still answer from a settled request may proceed.
    ended: bool,
}

struct ApprovalCommitContext<'a> {
    actor: &'a str,
    trace_id: Option<&'a str>,
    occurred_at: time::OffsetDateTime,
    run_version: u64,
    workflow_version: WorkflowVersionId,
}

struct StoredApproval {
    request: ApprovalRequest,
    routes: ApprovalRoutes,
    opened_at: time::OffsetDateTime,
}

fn record_approval_wait(
    opened_at: time::OffsetDateTime,
    settled_at: time::OffsetDateTime,
    outcome: WorkflowPrimitiveOutcome,
) {
    metrics::histogram!(
        WorkflowPrimitiveMetric::ApprovalWait.as_str(),
        "outcome" => outcome.as_str()
    )
    .record((settled_at - opened_at).as_seconds_f64().max(0.0));
}

fn stored_u64(row: &PgRow, field: &str) -> Result<u64, StoreError> {
    u64::try_from(col::<i64>(row, field)?).map_err(|_| corrupt(field))
}

fn stored_i64(value: u64, field: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| corrupt(field))
}

fn path_parts(path: &InstancePath) -> (Vec<String>, Vec<i32>) {
    path.segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
            )
        })
        .unzip()
}

fn stored_path(row: &PgRow) -> Result<InstancePath, StoreError> {
    let nodes: Vec<String> = col(row, "owner_path_nodes")?;
    let indices: Vec<i32> = col(row, "owner_path_indices")?;
    if nodes.len() != indices.len() {
        return Err(corrupt("approval owner path"));
    }
    let mut path = InstancePath::root();
    for (node, index) in nodes.into_iter().zip(indices) {
        path = path
            .child(
                NodeId::parse_stored(&node).map_err(|_| corrupt("approval owner path"))?,
                u32::try_from(index).map_err(|_| corrupt("approval owner path"))?,
            )
            .map_err(|_| corrupt("approval owner path"))?;
    }
    Ok(path)
}

fn status(value: &str) -> Result<ApprovalStatus, StoreError> {
    match value {
        "PENDING" => Ok(ApprovalStatus::Pending),
        "APPROVED" => Ok(ApprovalStatus::Approved),
        "REJECTED" => Ok(ApprovalStatus::Rejected),
        "EXPIRED" => Ok(ApprovalStatus::Expired),
        "CANCELLED" => Ok(ApprovalStatus::Cancelled),
        _ => Err(corrupt("approval status")),
    }
}

fn decision(value: &str) -> Result<ApprovalDecision, StoreError> {
    match value {
        "APPROVE" => Ok(ApprovalDecision::Approve),
        "REJECT" => Ok(ApprovalDecision::Reject),
        _ => Err(corrupt("approval decision")),
    }
}

fn digest(bytes: Vec<u8>, field: &str) -> Result<Digest, StoreError> {
    bytes
        .try_into()
        .map(Digest::new)
        .map_err(|_| corrupt(field))
}

fn check_audit(
    audit: &AuditRecord,
    tenant: TenantId,
    request: ApprovalRequestId,
    action: AuditAction,
    actor: Option<&Subject>,
) -> Result<(), ApprovalProgressError> {
    if audit.tenant_id() != tenant
        || audit.action() != action
        || audit.resource_type() != ResourceType::ApprovalRequest
        || audit.resource_id() != request.to_string()
        || (action == AuditAction::ApprovalDecided && audit.reason().is_some())
        || actor.is_some_and(|actor| audit.actor() != actor)
    {
        return Err(ApprovalProgressError::AuditMismatch);
    }
    Ok(())
}

/// Locks the run for a transition that needs it to be alive.
async fn lock_run(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: sagat_domain::identity::NamespaceId,
    run: RunId,
) -> Result<LockedRun, ApprovalProgressError> {
    let locked = lock_run_whatever_its_state(tx, tenant, namespace, run).await?;
    if locked.ended {
        return Err(ApprovalProgressError::RuntimeUnavailable);
    }
    Ok(locked)
}

/// Locks the run and reports whether it ended, for callers that may still read a settled outcome.
async fn lock_run_whatever_its_state(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: sagat_domain::identity::NamespaceId,
    run: RunId,
) -> Result<LockedRun, ApprovalProgressError> {
    let row = sqlx::query(
        "SELECT namespace_id, status, run_version, workflow_version_id FROM runs \
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(ApprovalProgressError::NotFound)?;
    if col::<Uuid>(&row, "namespace_id")? != namespace.as_uuid() {
        return Err(ApprovalProgressError::NotFound);
    }
    let run_status: String = col(&row, "status")?;
    let ended = matches!(
        run_status.as_str(),
        "SUCCEEDED"
            | "FAILED"
            | "CANCELLED"
            | "TIMED_OUT"
            | "COMPENSATED"
            | "PARTIAL"
            | "COMPENSATION_FAILED"
    );
    Ok(LockedRun {
        version: stored_u64(&row, "run_version")?,
        workflow_version: WorkflowVersionId::new(col(&row, "workflow_version_id")?),
        ended,
    })
}

async fn check_scheduler_fence(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    authority: &SchedulerAuthority,
) -> Result<(), ApprovalProgressError> {
    let lease = authority.lease();
    if lease.shard != Shard::of(run.as_uuid()) {
        return Err(ApprovalProgressError::SchedulerFenceLost);
    }
    let held: Option<i16> = sqlx::query_scalar(
        "SELECT shard FROM scheduler_shards \
         WHERE shard = $1 AND generation = $2 AND owner = $3 \
           AND expires_at > clock_timestamp() FOR SHARE",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(stored_i64(lease.generation, "scheduler generation")?)
    .bind(authority.owner())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    if held.is_none() {
        return Err(ApprovalProgressError::SchedulerFenceLost);
    }
    Ok(())
}

async fn request_run(
    tx: &mut Transaction<'_, Postgres>,
    address: ApprovalAddress,
) -> Result<RunId, ApprovalProgressError> {
    sqlx::query_scalar(
        "SELECT run_id FROM approval_requests \
         WHERE tenant_id = $1 AND namespace_id = $2 AND request_id = $3",
    )
    .bind(address.tenant_id().as_uuid())
    .bind(address.namespace_id().as_uuid())
    .bind(address.request_id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .map(RunId::new)
    .ok_or(ApprovalProgressError::NotFound)
}

async fn current_principal(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    actor: &Subject,
) -> Result<Option<Principal>, ApprovalProgressError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2 FOR SHARE",
    )
    .bind(tenant.as_uuid())
    .bind(actor.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    let Some(role) = role else {
        return Ok(None);
    };
    let role = Role::from_str(&role).map_err(|_| ApprovalProgressError::CorruptProjection)?;
    Ok(Some(Principal::new(tenant, actor.clone(), role)))
}

async fn load_receipt(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    request_id: ApprovalRequestId,
    receipt_id: ApprovalReceiptId,
) -> Result<ApprovalDecisionReceipt, ApprovalProgressError> {
    let row = sqlx::query(
        "SELECT receipt_id, request_id, decision_key, request_hash, actor, decision, reason_code, \
                decided_at, outcome_sequence \
         FROM approval_decision_receipts \
         WHERE tenant_id = $1 AND request_id = $2 AND receipt_id = $3",
    )
    .bind(tenant.as_uuid())
    .bind(request_id.as_uuid())
    .bind(receipt_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(ApprovalProgressError::CorruptProjection)?;
    Ok(ApprovalDecisionReceipt::restore(
        ApprovalReceiptId::new(col(&row, "receipt_id")?),
        ApprovalRequestId::new(col(&row, "request_id")?),
        DecisionKey::parse(&col::<String>(&row, "decision_key")?)?,
        digest(col(&row, "request_hash")?, "approval request hash")?,
        Subject::parse(&col::<String>(&row, "actor")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
        decision(&col::<String>(&row, "decision")?)?,
        col::<Option<String>>(&row, "reason_code")?
            .map(|code| ApprovalReasonCode::parse(&code))
            .transpose()?,
        col(&row, "decided_at")?,
        stored_u64(&row, "outcome_sequence")?,
    ))
}

async fn load_request(
    tx: &mut Transaction<'_, Postgres>,
    address: ApprovalAddress,
    lock: bool,
) -> Result<StoredApproval, ApprovalProgressError> {
    let sql = if lock {
        "SELECT * FROM approval_requests WHERE tenant_id = $1 AND namespace_id = $2 \
         AND request_id = $3 FOR UPDATE"
    } else {
        "SELECT * FROM approval_requests WHERE tenant_id = $1 AND namespace_id = $2 \
         AND request_id = $3"
    };
    let row = sqlx::query(sql)
        .bind(address.tenant_id().as_uuid())
        .bind(address.namespace_id().as_uuid())
        .bind(address.request_id().as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
        .ok_or(ApprovalProgressError::NotFound)?;
    let request_id = ApprovalRequestId::new(col(&row, "request_id")?);
    let subjects = col::<Vec<String>>(&row, "policy_subjects")?
        .into_iter()
        .map(|subject| Subject::parse(&subject))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ApprovalProgressError::CorruptProjection)?;
    let roles = col::<Vec<String>>(&row, "policy_roles")?
        .into_iter()
        .map(|role| Role::from_str(&role))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ApprovalProgressError::CorruptProjection)?;
    let policy =
        ApprovalPolicy::with_initiator_exclusion(subjects, roles, col(&row, "exclude_initiator")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?;
    let winning_receipt =
        col::<Option<Uuid>>(&row, "winning_receipt_id")?.map(ApprovalReceiptId::new);
    let receipt = match winning_receipt {
        Some(receipt) => Some(load_receipt(tx, address.tenant_id(), request_id, receipt).await?),
        None => None,
    };
    let owner = ExecutionInstanceId::in_path(
        stored_path(&row)?,
        NodeId::parse_stored(&col::<String>(&row, "owner_node")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
    );
    let request = ApprovalRequest::restore(
        request_id,
        address.tenant_id(),
        address.namespace_id(),
        RunId::new(col(&row, "run_id")?),
        owner,
        policy,
        Subject::parse(&col::<String>(&row, "initiator")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
        stored_u64(&row, "opened_sequence")?,
        col(&row, "deadline")?,
        status(&col::<String>(&row, "status")?)?,
        receipt,
        col::<Option<i64>>(&row, "outcome_sequence")?
            .map(u64::try_from)
            .transpose()
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
        col(&row, "settled_at")?,
    )
    .map_err(|_| ApprovalProgressError::CorruptProjection)?;
    let routes = ApprovalRoutes::new(
        NodeId::parse_stored(&col::<String>(&row, "approved_node")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
        NodeId::parse_stored(&col::<String>(&row, "rejected_node")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
        NodeId::parse_stored(&col::<String>(&row, "expired_node")?)
            .map_err(|_| ApprovalProgressError::CorruptProjection)?,
    );
    verify_projection(tx, &request, &routes).await?;
    Ok(StoredApproval {
        request,
        routes,
        opened_at: col(&row, "created_at")?,
    })
}

async fn journal_event(
    tx: &mut Transaction<'_, Postgres>,
    request: &ApprovalRequest,
    sequence: u64,
) -> Result<(RunEvent, ExecutionInstanceId), ApprovalProgressError> {
    let row = sqlx::query(
        "SELECT event_type, node, detail, instance_path_nodes, instance_path_indices \
         FROM run_events WHERE tenant_id = $1 AND run_id = $2 AND seq = $3",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(stored_i64(sequence, "approval journal sequence")?)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(ApprovalProgressError::CorruptProjection)?;
    let event_type: String = col(&row, "event_type")?;
    let node: Option<String> = col(&row, "node")?;
    let detail: serde_json::Value = col(&row, "detail")?;
    let event = decode(&event_type, node.as_deref(), &detail)
        .ok_or(ApprovalProgressError::CorruptProjection)?;
    let path = {
        let nodes: Vec<String> = col(&row, "instance_path_nodes")?;
        let indices: Vec<i32> = col(&row, "instance_path_indices")?;
        if nodes.len() != indices.len() {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let mut path = InstancePath::root();
        for (node, index) in nodes.into_iter().zip(indices) {
            path = path
                .child(
                    NodeId::parse_stored(&node)
                        .map_err(|_| ApprovalProgressError::CorruptProjection)?,
                    u32::try_from(index).map_err(|_| ApprovalProgressError::CorruptProjection)?,
                )
                .map_err(|_| ApprovalProgressError::CorruptProjection)?;
        }
        path
    };
    let node = event
        .node()
        .cloned()
        .ok_or(ApprovalProgressError::CorruptProjection)?;
    Ok((event, ExecutionInstanceId::in_path(path, node)))
}

async fn verify_projection(
    tx: &mut Transaction<'_, Postgres>,
    request: &ApprovalRequest,
    routes: &ApprovalRoutes,
) -> Result<(), ApprovalProgressError> {
    let (opened, opened_owner) = journal_event(tx, request, request.opened_sequence()).await?;
    if opened_owner != *request.owner()
        || opened
            != (RunEvent::ApprovalOpened {
                node: request.owner().node().clone(),
                request_id: request.id(),
                deadline: request.deadline(),
            })
    {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    let (path_nodes, path_indices) = path_parts(request.owner().path());
    let step = sqlx::query(
        "SELECT status, routed, waiting_kind, approval_request_id FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND instance_path_nodes = $4 AND instance_path_indices = $5",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(request.owner().node().as_str())
    .bind(&path_nodes)
    .bind(&path_indices)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(ApprovalProgressError::CorruptProjection)?;
    if request.status() == ApprovalStatus::Pending {
        if col::<String>(&step, "status")? != "WAITING"
            || col::<bool>(&step, "routed")?
            || col::<Option<String>>(&step, "waiting_kind")?.as_deref() != Some("APPROVAL")
            || col::<Option<Uuid>>(&step, "approval_request_id")? != Some(request.id().as_uuid())
        {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        return Ok(());
    }
    if request.status() == ApprovalStatus::Cancelled {
        if col::<String>(&step, "status")? != "CANCELLED"
            || !col::<bool>(&step, "routed")?
            || col::<Option<String>>(&step, "waiting_kind")?.is_some()
            || col::<Option<Uuid>>(&step, "approval_request_id")?.is_some()
        {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let sequence = request
            .outcome_sequence()
            .ok_or(ApprovalProgressError::CorruptProjection)?;
        let (outcome, outcome_owner) = journal_event(tx, request, sequence).await?;
        if outcome_owner != *request.owner()
            || outcome
                != (RunEvent::StepCancelled {
                    node: request.owner().node().clone(),
                })
        {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        return Ok(());
    }
    if col::<String>(&step, "status")? != "SUCCEEDED"
        || !col::<bool>(&step, "routed")?
        || col::<Option<String>>(&step, "waiting_kind")?.is_some()
        || col::<Option<Uuid>>(&step, "approval_request_id")?.is_some()
    {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    let sequence = request
        .outcome_sequence()
        .ok_or(ApprovalProgressError::CorruptProjection)?;
    let (outcome, outcome_owner) = journal_event(tx, request, sequence).await?;
    if outcome_owner != *request.owner() {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    let route = match request.status() {
        ApprovalStatus::Approved => routes.approved(),
        ApprovalStatus::Rejected => routes.rejected(),
        ApprovalStatus::Expired => routes.expired(),
        ApprovalStatus::Pending | ApprovalStatus::Cancelled => {
            return Err(ApprovalProgressError::CorruptProjection);
        }
    };
    let expected = match request.status() {
        ApprovalStatus::Approved | ApprovalStatus::Rejected => {
            let receipt = request
                .winning_receipt()
                .ok_or(ApprovalProgressError::CorruptProjection)?;
            RunEvent::ApprovalDecided {
                node: request.owner().node().clone(),
                request_id: request.id(),
                receipt_id: receipt.id(),
                decision: receipt.decision(),
                actor: receipt.actor().clone(),
                reason_code: receipt.reason_code().cloned(),
            }
        }
        ApprovalStatus::Expired => RunEvent::ApprovalExpired {
            node: request.owner().node().clone(),
            request_id: request.id(),
        },
        ApprovalStatus::Pending | ApprovalStatus::Cancelled => unreachable!(),
    };
    if outcome != expected {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    let (routed, routed_owner) = journal_event(tx, request, sequence + 1).await?;
    if routed_owner != *request.owner()
        || routed
            != (RunEvent::StepRouted {
                node: request.owner().node().clone(),
                activated: vec![route.clone()],
            })
    {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    Ok(())
}

async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    request: &ApprovalRequest,
    sequence: u64,
    event: &RunEvent,
    context: &ApprovalCommitContext<'_>,
) -> Result<(), ApprovalProgressError> {
    let encoded = encode(event);
    let (path_nodes, path_indices) = path_parts(request.owner().path());
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(stored_i64(sequence, "approval journal sequence")?)
    .bind(encoded.event_type)
    .bind(encoded.node)
    .bind(encoded.attempt)
    .bind(encoded.detail)
    .bind(context.actor)
    .bind(context.trace_id)
    .bind(context.occurred_at)
    .bind(instance_step_id(
        context.workflow_version,
        request.run_id(),
        request.owner(),
    ))
    .bind(path_nodes)
    .bind(path_indices)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn mark_telemetry(
    tx: &mut Transaction<'_, Postgres>,
    request: &ApprovalRequest,
    head: u64,
) -> Result<(), ApprovalProgressError> {
    sqlx::query(
        "INSERT INTO telemetry_pending (tenant_id, run_id, head_seq, applied_seq, behind_since) \
         VALUES ($1,$2,$3,0,clock_timestamp()) \
         ON CONFLICT (tenant_id, run_id) DO UPDATE \
           SET behind_since = CASE WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
                                   THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
               head_seq = GREATEST(telemetry_pending.head_seq, EXCLUDED.head_seq)",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(stored_i64(head, "approval telemetry head")?)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn route(
    tx: &mut Transaction<'_, Postgres>,
    request: &ApprovalRequest,
    route: &NodeId,
    sequence: u64,
    outbox_id: Uuid,
    context: &ApprovalCommitContext<'_>,
) -> Result<(), ApprovalProgressError> {
    let routed = RunEvent::StepRouted {
        node: request.owner().node().clone(),
        activated: vec![route.clone()],
    };
    insert_event(tx, request, sequence + 1, &routed, context).await?;
    let (path_nodes, path_indices) = path_parts(request.owner().path());
    let updated = sqlx::query(
        "UPDATE steps SET status = 'SUCCEEDED', routed = true, waiting_kind = NULL, due_at = NULL, \
                approval_request_id = NULL, updated_at = $6 \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND instance_path_nodes = $4 AND instance_path_indices = $5 \
           AND status = 'WAITING' AND waiting_kind = 'APPROVAL' AND approval_request_id = $7",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(request.owner().node().as_str())
    .bind(&path_nodes)
    .bind(&path_indices)
    .bind(context.occurred_at)
    .bind(request.id().as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if updated != 1 {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    let target = ExecutionInstanceId::in_path(request.owner().path().clone(), route.clone());
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, kind, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1,$2,$3,$4,$5,'PENDING',$6,'FORWARD',$7,$8)",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(route.as_str())
    .bind(instance_step_id(
        context.workflow_version,
        request.run_id(),
        &target,
    ))
    .bind(request.namespace_id().as_uuid())
    .bind(context.occurred_at)
    .bind(&path_nodes)
    .bind(&path_indices)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1,$2,$3,'approval.settled',$4)",
    )
    .bind(outbox_id)
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(json!({
        "tenantId": request.tenant_id().to_string(),
        "namespaceId": request.namespace_id().to_string(),
        "runId": request.run_id().to_string(),
        "requestId": request.id().to_string(),
        "status": request.status().as_str(),
        "route": route.as_str(),
        "outcomeSequence": sequence,
        "traceId": context.trace_id,
    }))
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    let run_updated = sqlx::query(
        "UPDATE runs SET status = 'RUNNING', run_version = $3, updated_at = $4 \
         WHERE tenant_id = $1 AND id = $2 AND run_version = $5",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.run_id().as_uuid())
    .bind(stored_i64(sequence + 1, "approval routed sequence")?)
    .bind(context.occurred_at)
    .bind(stored_i64(context.run_version, "run version")?)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if run_updated != 1 {
        return Err(ApprovalProgressError::CorruptProjection);
    }
    mark_telemetry(tx, request, sequence + 1).await?;
    Ok(())
}

async fn audit_denial(
    tx: &mut Transaction<'_, Postgres>,
    audit: &AuditRecord,
    reason: &'static str,
) -> Result<(), ApprovalProgressError> {
    let denied = audit.follow_up(
        AuditAction::ApprovalDenied,
        ResourceType::ApprovalRequest,
        audit.resource_id(),
        reason,
    );
    insert_audit(tx, &denied).await?;
    Ok(())
}

impl ApprovalProgressStore for PgApprovalProgressStore {
    async fn open(
        &self,
        opening: &ApprovalOpening,
    ) -> Result<ApprovalRequest, ApprovalProgressError> {
        let request = opening.request();
        check_audit(
            opening.audit(),
            request.tenant_id(),
            request.id(),
            AuditAction::ApprovalOpened,
            None,
        )?;
        if opening.audit().created_at() != opening.occurred_at() {
            return Err(ApprovalProgressError::AuditMismatch);
        }
        if opening.audit().actor() != &Subject::engine() {
            return Err(ApprovalProgressError::AuditMismatch);
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(
            &mut tx,
            request.tenant_id(),
            request.namespace_id(),
            request.run_id(),
        )
        .await?;
        check_scheduler_fence(&mut tx, request.run_id(), opening.scheduler()).await?;
        if request.status() != ApprovalStatus::Pending
            || run.version.checked_add(1) != Some(request.opened_sequence())
            || request.deadline() <= opening.occurred_at()
        {
            return Err(ApprovalProgressError::RuntimeUnavailable);
        }
        let (path_nodes, path_indices) = path_parts(request.owner().path());
        let owner_id = instance_step_id(run.workflow_version, request.run_id(), request.owner());
        let owner_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 \
               AND node = $3 AND step_id = $4 AND namespace_id = $5 \
               AND instance_path_nodes = $6 AND instance_path_indices = $7 \
               AND status = 'PENDING' AND routed = false)",
        )
        .bind(request.tenant_id().as_uuid())
        .bind(request.run_id().as_uuid())
        .bind(request.owner().node().as_str())
        .bind(owner_id)
        .bind(request.namespace_id().as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !owner_exists {
            return Err(ApprovalProgressError::RuntimeUnavailable);
        }
        let subjects = request
            .policy()
            .subjects()
            .map(|subject| subject.as_str().to_owned())
            .collect::<Vec<_>>();
        let roles = request
            .policy()
            .roles()
            .map(|role| role.as_str().to_owned())
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO approval_requests \
             (tenant_id, namespace_id, request_id, run_id, owner_instance_id, owner_node, \
              owner_path_nodes, owner_path_indices, policy_subjects, policy_roles, exclude_initiator, \
              initiator, opened_sequence, deadline, status, approved_node, rejected_node, expired_node, \
              open_outbox_id, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'PENDING',$15,$16,$17,$18,$19,$19)",
        )
        .bind(request.tenant_id().as_uuid())
        .bind(request.namespace_id().as_uuid())
        .bind(request.id().as_uuid())
        .bind(request.run_id().as_uuid())
        .bind(owner_id)
        .bind(request.owner().node().as_str())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(subjects)
        .bind(roles)
        .bind(request.policy().excludes_initiator())
        .bind(request.initiator().as_str())
        .bind(stored_i64(request.opened_sequence(), "approval opened sequence")?)
        .bind(request.deadline())
        .bind(opening.routes().approved().as_str())
        .bind(opening.routes().rejected().as_str())
        .bind(opening.routes().expired().as_str())
        .bind(opening.outbox_id())
        .bind(opening.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO approval_timers (tenant_id, request_id, due_at, state, updated_at) \
             VALUES ($1,$2,$3,'PENDING',$4)",
        )
        .bind(request.tenant_id().as_uuid())
        .bind(request.id().as_uuid())
        .bind(request.deadline())
        .bind(opening.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let event = RunEvent::ApprovalOpened {
            node: request.owner().node().clone(),
            request_id: request.id(),
            deadline: request.deadline(),
        };
        let context = ApprovalCommitContext {
            actor: opening.audit().actor().as_str(),
            trace_id: opening.audit().trace_id(),
            occurred_at: opening.occurred_at(),
            run_version: run.version,
            workflow_version: run.workflow_version,
        };
        insert_event(
            &mut tx,
            request,
            request.opened_sequence(),
            &event,
            &context,
        )
        .await?;
        let owner_updated = sqlx::query(
            "UPDATE steps SET status = 'WAITING', waiting_kind = 'APPROVAL', due_at = $6, \
                    approval_request_id = $7, updated_at = $8 \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5 \
               AND status = 'PENDING' AND routed = false",
        )
        .bind(request.tenant_id().as_uuid())
        .bind(request.run_id().as_uuid())
        .bind(request.owner().node().as_str())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(request.deadline())
        .bind(request.id().as_uuid())
        .bind(opening.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if owner_updated != 1 {
            return Err(ApprovalProgressError::RuntimeUnavailable);
        }
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES ($1,$2,$3,'approval.opened',$4)",
        )
        .bind(opening.outbox_id())
        .bind(request.tenant_id().as_uuid())
        .bind(request.run_id().as_uuid())
        .bind(json!({
            "tenantId": request.tenant_id().to_string(),
            "namespaceId": request.namespace_id().to_string(),
            "runId": request.run_id().to_string(),
            "requestId": request.id().to_string(),
            "deadline": request.deadline().unix_timestamp(),
            "traceId": opening.audit().trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let run_updated = sqlx::query(
            "UPDATE runs SET status = 'WAITING', run_version = $3, updated_at = $4 \
             WHERE tenant_id = $1 AND id = $2 AND run_version = $5",
        )
        .bind(request.tenant_id().as_uuid())
        .bind(request.run_id().as_uuid())
        .bind(stored_i64(
            request.opened_sequence(),
            "approval opened sequence",
        )?)
        .bind(opening.occurred_at())
        .bind(stored_i64(run.version, "run version")?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if run_updated != 1 {
            return Err(ApprovalProgressError::RuntimeUnavailable);
        }
        mark_telemetry(&mut tx, request, request.opened_sequence()).await?;
        insert_audit(&mut tx, opening.audit()).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(request.clone())
    }

    async fn decide(
        &self,
        attempt: &ApprovalDecisionAttempt,
    ) -> Result<ApprovalDecisionResolution, ApprovalProgressError> {
        let address = attempt.address();
        check_audit(
            attempt.audit(),
            address.tenant_id(),
            address.request_id(),
            AuditAction::ApprovalDecided,
            Some(attempt.actor()),
        )?;
        if attempt.audit().created_at() != attempt.occurred_at() {
            return Err(ApprovalProgressError::AuditMismatch);
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run_id = match request_run(&mut tx, address).await {
            Ok(run) => run,
            Err(ApprovalProgressError::NotFound) => {
                audit_denial(&mut tx, attempt.audit(), "not_found_or_outside_tenant").await?;
                tx.commit().await.map_err(store_error)?;
                return Err(ApprovalProgressError::NotFound);
            }
            Err(error) => return Err(error),
        };
        let run = lock_run_whatever_its_state(
            &mut tx,
            address.tenant_id(),
            address.namespace_id(),
            run_id,
        )
        .await?;
        let mut stored = load_request(&mut tx, address, true).await?;
        // A request that already has its outcome answers from it (the receipt again, or the
        // conflict) whether or not the run went on to end; only a request still open on an ended
        // run has nothing left to decide.
        if run.ended && stored.request.status() == ApprovalStatus::Pending {
            return Err(ApprovalProgressError::RuntimeUnavailable);
        }
        let principal =
            match current_principal(&mut tx, address.tenant_id(), attempt.actor()).await? {
                Some(principal) => principal,
                None => {
                    let error = ApprovalError::Forbidden;
                    audit_denial(&mut tx, attempt.audit(), error.reason()).await?;
                    tx.commit().await.map_err(store_error)?;
                    return Err(error.into());
                }
            };
        let sequence = run
            .version
            .checked_add(1)
            .ok_or(ApprovalProgressError::CorruptProjection)?;
        let resolution = match stored.request.decide(
            &principal,
            attempt.receipt_id(),
            attempt.command(),
            attempt.occurred_at(),
            sequence,
        ) {
            Ok(resolution) => resolution,
            Err(error @ (ApprovalError::Forbidden | ApprovalError::InitiatorCannotDecide)) => {
                audit_denial(&mut tx, attempt.audit(), error.reason()).await?;
                tx.commit().await.map_err(store_error)?;
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        if resolution.replayed() {
            tx.commit().await.map_err(store_error)?;
            return Ok(resolution);
        }
        let receipt = resolution.receipt();
        sqlx::query(
            "INSERT INTO approval_decision_receipts \
             (tenant_id, namespace_id, receipt_id, request_id, decision_key, request_hash, actor, \
              decision, reason_code, decided_at, outcome_sequence) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(address.tenant_id().as_uuid())
        .bind(address.namespace_id().as_uuid())
        .bind(receipt.id().as_uuid())
        .bind(receipt.request_id().as_uuid())
        .bind(receipt.decision_key().as_str())
        .bind(receipt.request_hash().as_bytes().as_slice())
        .bind(receipt.actor().as_str())
        .bind(receipt.decision().as_str())
        .bind(receipt.reason_code().map(ApprovalReasonCode::as_str))
        .bind(receipt.decided_at())
        .bind(stored_i64(
            receipt.outcome_sequence(),
            "approval outcome sequence",
        )?)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let route_node = match receipt.decision() {
            ApprovalDecision::Approve => stored.routes.approved(),
            ApprovalDecision::Reject => stored.routes.rejected(),
        };
        let projected = sqlx::query(
            "UPDATE approval_requests SET status = $4, winning_receipt_id = $5, \
                    outcome_sequence = $6, settled_at = $7, routed_node = $8, \
                    outcome_outbox_id = $9, updated_at = $7 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND request_id = $3 AND status = 'PENDING'",
        )
        .bind(address.tenant_id().as_uuid())
        .bind(address.namespace_id().as_uuid())
        .bind(address.request_id().as_uuid())
        .bind(stored.request.status().as_str())
        .bind(receipt.id().as_uuid())
        .bind(stored_i64(sequence, "approval outcome sequence")?)
        .bind(attempt.occurred_at())
        .bind(route_node.as_str())
        .bind(attempt.outbox_id())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if projected != 1 {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let timer_updated = sqlx::query(
            "UPDATE approval_timers SET state = 'CANCELLED', updated_at = $3 \
             WHERE tenant_id = $1 AND request_id = $2 AND state = 'PENDING'",
        )
        .bind(address.tenant_id().as_uuid())
        .bind(address.request_id().as_uuid())
        .bind(attempt.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if timer_updated != 1 {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let event = RunEvent::ApprovalDecided {
            node: stored.request.owner().node().clone(),
            request_id: stored.request.id(),
            receipt_id: receipt.id(),
            decision: receipt.decision(),
            actor: receipt.actor().clone(),
            reason_code: receipt.reason_code().cloned(),
        };
        let journal_actor = format!("user:{}", attempt.actor().as_str());
        let context = ApprovalCommitContext {
            actor: &journal_actor,
            trace_id: attempt.audit().trace_id(),
            occurred_at: attempt.occurred_at(),
            run_version: run.version,
            workflow_version: run.workflow_version,
        };
        insert_event(&mut tx, &stored.request, sequence, &event, &context).await?;
        route(
            &mut tx,
            &stored.request,
            route_node,
            sequence,
            attempt.outbox_id(),
            &context,
        )
        .await?;
        insert_audit(&mut tx, attempt.audit()).await?;
        tx.commit().await.map_err(store_error)?;
        let outcome = match receipt.decision() {
            ApprovalDecision::Approve => WorkflowPrimitiveOutcome::Approved,
            ApprovalDecision::Reject => WorkflowPrimitiveOutcome::Rejected,
        };
        record_approval_wait(stored.opened_at, attempt.occurred_at(), outcome);
        Ok(resolution)
    }

    async fn expire(
        &self,
        attempt: &ApprovalExpiryAttempt,
    ) -> Result<ApprovalTransition, ApprovalProgressError> {
        let address = attempt.address();
        check_audit(
            attempt.audit(),
            address.tenant_id(),
            address.request_id(),
            AuditAction::ApprovalExpired,
            None,
        )?;
        if attempt.audit().created_at() != attempt.occurred_at() {
            return Err(ApprovalProgressError::AuditMismatch);
        }
        if attempt.audit().actor() != &Subject::engine() {
            return Err(ApprovalProgressError::AuditMismatch);
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run_id = request_run(&mut tx, address).await?;
        let run = lock_run(&mut tx, address.tenant_id(), address.namespace_id(), run_id).await?;
        check_scheduler_fence(&mut tx, run_id, attempt.scheduler()).await?;
        let mut stored = load_request(&mut tx, address, true).await?;
        let sequence = run
            .version
            .checked_add(1)
            .ok_or(ApprovalProgressError::CorruptProjection)?;
        let transition = stored.request.expire(attempt.occurred_at(), sequence)?;
        if !transition.changed() {
            let losing = attempt.audit().clone().with_reason(format!(
                "outcome=lost;status={}",
                transition.status().as_str()
            ));
            insert_audit(&mut tx, &losing).await?;
            tx.commit().await.map_err(store_error)?;
            return Ok(transition);
        }
        let projected = sqlx::query(
            "UPDATE approval_requests SET status = 'EXPIRED', outcome_sequence = $4, \
                    settled_at = $5, routed_node = expired_node, outcome_outbox_id = $6, \
                    updated_at = $5 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND request_id = $3 AND status = 'PENDING'",
        )
        .bind(address.tenant_id().as_uuid())
        .bind(address.namespace_id().as_uuid())
        .bind(address.request_id().as_uuid())
        .bind(stored_i64(sequence, "approval outcome sequence")?)
        .bind(attempt.occurred_at())
        .bind(attempt.outbox_id())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if projected != 1 {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let timer_updated = sqlx::query(
            "UPDATE approval_timers SET state = 'FIRED', updated_at = $3 \
             WHERE tenant_id = $1 AND request_id = $2 AND state = 'PENDING'",
        )
        .bind(address.tenant_id().as_uuid())
        .bind(address.request_id().as_uuid())
        .bind(attempt.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if timer_updated != 1 {
            return Err(ApprovalProgressError::CorruptProjection);
        }
        let event = RunEvent::ApprovalExpired {
            node: stored.request.owner().node().clone(),
            request_id: stored.request.id(),
        };
        let context = ApprovalCommitContext {
            actor: attempt.audit().actor().as_str(),
            trace_id: attempt.audit().trace_id(),
            occurred_at: attempt.occurred_at(),
            run_version: run.version,
            workflow_version: run.workflow_version,
        };
        insert_event(&mut tx, &stored.request, sequence, &event, &context).await?;
        let route_node = stored.routes.expired();
        route(
            &mut tx,
            &stored.request,
            route_node,
            sequence,
            attempt.outbox_id(),
            &context,
        )
        .await?;
        insert_audit(&mut tx, attempt.audit()).await?;
        tx.commit().await.map_err(store_error)?;
        record_approval_wait(
            stored.opened_at,
            attempt.occurred_at(),
            WorkflowPrimitiveOutcome::Expired,
        );
        Ok(transition)
    }

    async fn load(
        &self,
        address: ApprovalAddress,
    ) -> Result<ApprovalRequest, ApprovalProgressError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let stored = load_request(&mut tx, address, false).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(stored.request)
    }

    async fn list_for_run(
        &self,
        tenant_id: TenantId,
        namespace_id: sagat_domain::identity::NamespaceId,
        run_id: RunId,
        after_opened_sequence: Option<u64>,
        limit: ApprovalPageSize,
    ) -> Result<Vec<ApprovalListItem>, ApprovalProgressError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let after = after_opened_sequence
            .map(|sequence| stored_i64(sequence, "approval list cursor"))
            .transpose()?
            .unwrap_or(0);
        let request_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT request_id FROM approval_requests \
             WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $3 \
               AND opened_sequence > $4 \
             ORDER BY opened_sequence, request_id LIMIT $5",
        )
        .bind(tenant_id.as_uuid())
        .bind(namespace_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(after)
        .bind(i64::from(limit.get()))
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut items = Vec::with_capacity(request_ids.len());
        for request_id in request_ids {
            let stored = load_request(
                &mut tx,
                ApprovalAddress::new(tenant_id, namespace_id, ApprovalRequestId::new(request_id)),
                false,
            )
            .await?;
            if stored.request.run_id() != run_id {
                return Err(ApprovalProgressError::CorruptProjection);
            }
            items.push(ApprovalListItem::from_request(&stored.request));
        }
        tx.commit().await.map_err(store_error)?;
        Ok(items)
    }
}
