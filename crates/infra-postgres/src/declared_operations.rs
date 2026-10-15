//! PostgreSQL transactions for explicitly declared Query and Update operations.

use std::str::FromStr;

use sagat_application::declared_operations::{
    CompletedOperationResult, DeclaredOperationDocument, DeclaredOperationSnapshot,
    DeclaredOperationStore, DeclaredOperationStoreError, DeclaredQuery, DeclaredUpdate,
    DeclaredUpdateDecision, EvaluatedOperationWrite, OperationSnapshotPayload,
    declared_update_request_hash,
};
use sagat_application::error::StoreError;
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    DeclaredOperationsState, OperationAddress, OperationError, OperationScope, OperationValue,
    QuerySnapshot, RunEvent, RunId, RunStatus, UpdateEvaluation, UpdateReceipt,
    UpdateReceiptStatus, UpdateRejection, UpdateResolution,
};
use sagat_domain::identity::{NamespaceId, Permission, Role, Subject, TenantId};
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use serde_json::{Value, json};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Transaction};

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;

/// PostgreSQL-backed declared-operation store. Public routes remain gated until all release tests
/// and API security gates are available.
#[derive(Debug, Clone)]
pub struct PgDeclaredOperationStore {
    pool: PgPool,
}

impl PgDeclaredOperationStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct LockedRun {
    status: RunStatus,
    run_version: u64,
    workflow_version_id: uuid::Uuid,
    input_digest: Digest,
}

struct StoredState {
    value: OperationValue,
    content: Vec<u8>,
    declaration_digest: Digest,
    result_schema_digest: Digest,
    state_revision: u64,
    last_changed_run_version: u64,
}

fn stored_u64(row: &PgRow, column: &str) -> Result<u64, StoreError> {
    u64::try_from(col::<i64>(row, column)?).map_err(|_| corrupt(column))
}

fn stored_digest(row: &PgRow, column: &str) -> Result<Digest, StoreError> {
    let bytes: Vec<u8> = col(row, column)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| corrupt(column))?;
    Ok(Digest::new(bytes))
}

fn optional_digest(row: &PgRow, column: &str) -> Result<Option<Digest>, StoreError> {
    let bytes: Option<Vec<u8>> = col(row, column)?;
    bytes
        .map(|bytes| {
            bytes
                .try_into()
                .map(Digest::new)
                .map_err(|_| corrupt(column))
        })
        .transpose()
}

fn check_audit(
    audit: &AuditRecord,
    tenant_id: TenantId,
    actor: &Subject,
    run_id: RunId,
    action: AuditAction,
) -> Result<(), DeclaredOperationStoreError> {
    if audit.tenant_id() != tenant_id
        || audit.actor() != actor
        || audit.resource_type() != ResourceType::Run
        || audit.resource_id() != run_id.to_string()
        || audit.action() != action
        || audit.reason().is_some()
    {
        return Err(DeclaredOperationStoreError::AuditMismatch);
    }
    Ok(())
}

fn operation_audit(
    audit: &AuditRecord,
    operation: &ActivityName,
    outcome: Option<UpdateReceiptStatus>,
) -> AuditRecord {
    let reason = match outcome {
        None => format!("operation={}", operation.as_str()),
        Some(UpdateReceiptStatus::Applied) => {
            format!("operation={};outcome=applied", operation.as_str())
        }
        Some(UpdateReceiptStatus::Rejected) => {
            format!("operation={};outcome=rejected", operation.as_str())
        }
    };
    audit.clone().with_reason(reason)
}

fn stored_i64(value: u64, field: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| corrupt(field))
}

fn check_run_address(
    scope: &OperationScope,
    resolved_run_id: RunId,
) -> Result<(), DeclaredOperationStoreError> {
    match scope.address() {
        OperationAddress::Run(run_id) if *run_id == resolved_run_id => Ok(()),
        OperationAddress::Run(_) | OperationAddress::Chain(_) => {
            Err(DeclaredOperationStoreError::AddressUnavailable)
        }
    }
}

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    actor: &Subject,
    permission: Permission,
) -> Result<(), DeclaredOperationStoreError> {
    let namespace_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM namespaces WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tenant_id.as_uuid())
    .bind(namespace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(store_error)?;
    if !namespace_exists {
        return Err(DeclaredOperationStoreError::NotFound);
    }
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2 FOR KEY SHARE",
    )
    .bind(tenant_id.as_uuid())
    .bind(actor.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    let Some(role) = role else {
        return Err(DeclaredOperationStoreError::Forbidden);
    };
    let role = Role::from_str(&role).map_err(|_| StoreError::Internal {
        detail: "stored membership role is invalid".to_owned(),
    })?;
    if !role.permissions().contains(&permission) {
        return Err(DeclaredOperationStoreError::Forbidden);
    }
    Ok(())
}

async fn lock_run(
    tx: &mut Transaction<'_, Postgres>,
    scope: &OperationScope,
    resolved_run_id: RunId,
    exclusive: bool,
) -> Result<LockedRun, DeclaredOperationStoreError> {
    let row = if exclusive {
        sqlx::query(
            "SELECT namespace_id, status, run_version, workflow_version_id, input_digest FROM runs \
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(resolved_run_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
    } else {
        sqlx::query(
            "SELECT namespace_id, status, run_version, workflow_version_id, input_digest FROM runs \
             WHERE tenant_id = $1 AND id = $2 FOR SHARE",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(resolved_run_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
    }
    .ok_or(DeclaredOperationStoreError::NotFound)?;
    let namespace_id: uuid::Uuid = col(&row, "namespace_id")?;
    if namespace_id != scope.namespace_id().as_uuid() {
        return Err(DeclaredOperationStoreError::NotFound);
    }
    let status_text: String = col(&row, "status")?;
    let status = RunStatus::from_str(&status_text).map_err(|_| StoreError::Internal {
        detail: "stored run status is invalid".to_owned(),
    })?;
    Ok(LockedRun {
        status,
        run_version: stored_u64(&row, "run_version")?,
        workflow_version_id: col(&row, "workflow_version_id")?,
        input_digest: stored_digest(&row, "input_digest")?,
    })
}

async fn declared_state(
    tx: &mut Transaction<'_, Postgres>,
    scope: &OperationScope,
    run: &LockedRun,
    resolved_run_id: RunId,
    operation: &ActivityName,
    kind: &str,
) -> Result<StoredState, DeclaredOperationStoreError> {
    let definition = sqlx::query(
        "SELECT declaration_digest, state_schema_digest, result_schema_digest \
         FROM declared_operation_definitions \
         WHERE tenant_id = $1 AND namespace_id = $2 AND workflow_version_id = $3 \
           AND operation_kind = $4 AND operation_name = $5 \
           AND disclosure = 'declared-business-result-v1'",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(run.workflow_version_id)
    .bind(kind)
    .bind(operation.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
    let declared_schema = stored_digest(&definition, "state_schema_digest")?;
    let declaration_digest = stored_digest(&definition, "declaration_digest")?;
    let result_schema_digest = stored_digest(&definition, "result_schema_digest")?;
    let row = sqlx::query(
        "SELECT s.state_digest, s.state_schema_digest, s.state_size_bytes, s.state_revision, \
                s.last_changed_run_version, p.content AS state_content, \
                octet_length(p.content) AS retained_state_size \
         FROM workflow_operation_states s \
         JOIN payloads p ON p.tenant_id = s.tenant_id AND p.digest = s.state_digest \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.run_id = $3",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(scope.namespace_id().as_uuid())
    .bind(resolved_run_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
    let state_schema = stored_digest(&row, "state_schema_digest")?;
    if state_schema != declared_schema {
        return Err(DeclaredOperationStoreError::DefinitionUnavailable);
    }
    let size_bytes = u32::try_from(col::<i32>(&row, "state_size_bytes")?)
        .map_err(|_| corrupt("state_size_bytes"))?;
    let retained_size = u32::try_from(col::<i32>(&row, "retained_state_size")?)
        .map_err(|_| corrupt("retained_state_size"))?;
    if retained_size != size_bytes {
        return Err(DeclaredOperationStoreError::DefinitionUnavailable);
    }
    let state_digest = stored_digest(&row, "state_digest")?;
    let state_revision = stored_u64(&row, "state_revision")?;
    let last_changed_run_version = stored_u64(&row, "last_changed_run_version")?;
    if state_revision > 0 {
        let journal = sqlx::query(
            "SELECT event_type, detail, payload_digest FROM run_events \
             WHERE tenant_id = $1 AND run_id = $2 AND seq = $3",
        )
        .bind(scope.tenant_id().as_uuid())
        .bind(resolved_run_id.as_uuid())
        .bind(stored_i64(
            last_changed_run_version,
            "last changed run version",
        )?)
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
        .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
        let event_type: String = col(&journal, "event_type")?;
        let detail: Value = col(&journal, "detail")?;
        let event = crate::event_codec::decode(&event_type, None, &detail)
            .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
        let payload_digest = optional_digest(&journal, "payload_digest")?;
        if !matches!(
            event,
            RunEvent::StateUpdated {
                after_revision,
                state_digest: event_digest,
                ..
            } if after_revision == state_revision
                && event_digest == state_digest
                && payload_digest == Some(state_digest)
        ) {
            return Err(DeclaredOperationStoreError::DefinitionUnavailable);
        }
    }
    Ok(StoredState {
        value: OperationValue::new(state_digest, state_schema, size_bytes)?,
        content: col(&row, "state_content")?,
        declaration_digest,
        result_schema_digest,
        state_revision,
        last_changed_run_version,
    })
}

async fn operation_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    scope: &OperationScope,
    run: &LockedRun,
    resolved_run_id: RunId,
    snapshot: QuerySnapshot,
    declaration_digest: Digest,
    state_content: Vec<u8>,
) -> Result<DeclaredOperationSnapshot, DeclaredOperationStoreError> {
    let version = sqlx::query(
        "SELECT digest, document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(run.workflow_version_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?
    .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
    let definition_digest = Digest::from_hex(&col::<String>(&version, "digest")?)
        .ok_or_else(|| corrupt("workflow version digest"))?;
    let definition = DeclaredOperationDocument::restore(
        definition_digest,
        col::<Vec<u8>>(&version, "document")?,
    )?;
    let input_content: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(scope.tenant_id().as_uuid())
            .bind(run.input_digest.as_bytes().as_slice())
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?
            .ok_or(DeclaredOperationStoreError::DefinitionUnavailable)?;
    let workflow_input = OperationSnapshotPayload::restore(run.input_digest, input_content)?;
    let rows = sqlx::query(
        "SELECT s.node, s.output_digest, p.content \
         FROM steps s \
         JOIN payloads p ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest \
         WHERE s.tenant_id = $1 AND s.run_id = $2 \
           AND cardinality(s.instance_path_nodes) = 0 AND s.output_digest IS NOT NULL \
         ORDER BY s.node",
    )
    .bind(scope.tenant_id().as_uuid())
    .bind(resolved_run_id.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let mut completed_results = Vec::with_capacity(rows.len());
    for row in rows {
        let node = NodeId::parse_stored(&col::<String>(&row, "node")?)
            .map_err(|_| corrupt("completed result node"))?;
        let digest = stored_digest(&row, "output_digest")?;
        let payload = OperationSnapshotPayload::restore(digest, col(&row, "content")?)?;
        completed_results.push(CompletedOperationResult::new(node, payload));
    }
    DeclaredOperationSnapshot::restore(
        snapshot,
        declaration_digest,
        definition,
        workflow_input,
        state_content,
        completed_results,
    )
    .map_err(Into::into)
}

fn address_parts(scope: &OperationScope) -> (&'static str, uuid::Uuid) {
    match scope.address() {
        OperationAddress::Run(run_id) => ("RUN", run_id.as_uuid()),
        OperationAddress::Chain(chain_id) => ("CHAIN", chain_id.as_uuid()),
    }
}

fn restored_receipt(row: &PgRow, scope: OperationScope) -> Result<UpdateReceipt, StoreError> {
    let status_text: String = col(row, "status")?;
    let status = match status_text.as_str() {
        "APPLIED" => UpdateReceiptStatus::Applied,
        "REJECTED" => UpdateReceiptStatus::Rejected,
        _ => return Err(corrupt("receipt status")),
    };
    let rejection_code: Option<String> = col(row, "rejection_code")?;
    let rejection = match rejection_code.as_deref() {
        None => None,
        Some("UPDATE_REJECTED") => Some(UpdateRejection::ValidatorRejected),
        Some("REVISION_CONFLICT") => Some(UpdateRejection::RevisionConflict {
            expected: stored_u64(row, "rejection_expected")?,
            actual: stored_u64(row, "rejection_actual")?,
        }),
        Some(_) => return Err(corrupt("receipt rejection")),
    };
    let result = match optional_digest(row, "result_digest")? {
        Some(digest) => {
            let schema_digest = optional_digest(row, "result_schema_digest")?
                .ok_or_else(|| corrupt("receipt result schema"))?;
            let size = col::<Option<i32>>(row, "result_size_bytes")?
                .ok_or_else(|| corrupt("receipt result size"))?;
            Some(
                OperationValue::new(
                    digest,
                    schema_digest,
                    u32::try_from(size).map_err(|_| corrupt("receipt result size"))?,
                )
                .map_err(|_| corrupt("receipt result"))?,
            )
        }
        None => None,
    };
    UpdateReceipt::restore(
        scope,
        RunId::new(col(row, "resolved_run_id")?),
        ActivityName::parse(&col::<String>(row, "operation_name")?)
            .map_err(|_| corrupt("receipt operation"))?,
        sagat_domain::execution::UpdateKey::parse(&col::<String>(row, "update_key")?)
            .map_err(|_| corrupt("receipt update key"))?,
        stored_digest(row, "request_hash")?,
        status,
        rejection,
        result,
        stored_u64(row, "before_state_revision")?,
        stored_u64(row, "after_state_revision")?,
        stored_u64(row, "run_version")?,
        Subject::parse(&col::<String>(row, "actor")?).map_err(|_| corrupt("receipt actor"))?,
    )
    .map_err(|_| corrupt("receipt invariants"))
}

async fn find_receipt(
    tx: &mut Transaction<'_, Postgres>,
    update: &DeclaredUpdate,
) -> Result<Option<UpdateReceipt>, DeclaredOperationStoreError> {
    let request = update.request();
    let (address_scope, target_id) = address_parts(request.scope());
    let row = sqlx::query(
        "SELECT resolved_run_id, operation_name, update_key, request_hash, status, rejection_code, \
                rejection_expected, rejection_actual, result_digest, result_schema_digest, \
                result_size_bytes, before_state_revision, after_state_revision, run_version, actor \
         FROM operation_update_receipts \
         WHERE tenant_id = $1 AND namespace_id = $2 AND address_scope = $3 \
           AND target_id = $4 AND update_key = $5",
    )
    .bind(request.scope().tenant_id().as_uuid())
    .bind(request.scope().namespace_id().as_uuid())
    .bind(address_scope)
    .bind(target_id)
    .bind(request.update_key().as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    row.map(|row| restored_receipt(&row, request.scope().clone()))
        .transpose()
        .map_err(Into::into)
}

async fn insert_rejected_receipt(
    tx: &mut Transaction<'_, Postgres>,
    update: &DeclaredUpdate,
    receipt: &UpdateReceipt,
) -> Result<(), DeclaredOperationStoreError> {
    let (address_scope, target_id) = address_parts(receipt.scope());
    let (code, expected, actual) = match receipt.rejection() {
        Some(UpdateRejection::RevisionConflict { expected, actual }) => (
            Some("REVISION_CONFLICT"),
            Some(i64::try_from(*expected).map_err(|_| corrupt("receipt expected revision"))?),
            Some(i64::try_from(*actual).map_err(|_| corrupt("receipt actual revision"))?),
        ),
        Some(UpdateRejection::ValidatorRejected) => (Some("UPDATE_REJECTED"), None, None),
        None => return Err(OperationError::InvalidStoredOperation.into()),
    };
    sqlx::query(
        "INSERT INTO operation_update_receipts \
         (tenant_id, namespace_id, address_scope, target_id, update_key, resolved_run_id, \
          operation_name, request_hash, status, rejection_code, rejection_expected, \
          rejection_actual, before_state_revision, after_state_revision, run_version, actor, \
          committed_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'REJECTED',$9,$10,$11,$12,$13,$14,$15,$16)",
    )
    .bind(receipt.scope().tenant_id().as_uuid())
    .bind(receipt.scope().namespace_id().as_uuid())
    .bind(address_scope)
    .bind(target_id)
    .bind(receipt.update_key().as_str())
    .bind(receipt.resolved_run_id().as_uuid())
    .bind(receipt.operation().as_str())
    .bind(receipt.request_hash().as_bytes().as_slice())
    .bind(code)
    .bind(expected)
    .bind(actual)
    .bind(i64::try_from(receipt.before_state_revision()).map_err(|_| corrupt("receipt revision"))?)
    .bind(i64::try_from(receipt.after_state_revision()).map_err(|_| corrupt("receipt revision"))?)
    .bind(i64::try_from(receipt.run_version()).map_err(|_| corrupt("receipt run version"))?)
    .bind(receipt.actor().as_str())
    .bind(update.occurred_at())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn ensure_retained_value(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
    value: &OperationValue,
) -> Result<(), DeclaredOperationStoreError> {
    let size: Option<i32> = sqlx::query_scalar(
        "SELECT octet_length(content) FROM payloads WHERE tenant_id = $1 AND digest = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(value.digest().as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    if size.and_then(|size| u32::try_from(size).ok()) != Some(value.size_bytes()) {
        return Err(OperationError::InvalidStoredOperation.into());
    }
    Ok(())
}

async fn retain_evaluated_write(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
    write: &EvaluatedOperationWrite,
) -> Result<(), DeclaredOperationStoreError> {
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) \
         ON CONFLICT (tenant_id, digest) DO NOTHING",
    )
    .bind(tenant_id.as_uuid())
    .bind(write.value().digest().as_bytes().as_slice())
    .bind(write.content())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    let retained: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant_id.as_uuid())
            .bind(write.value().digest().as_bytes().as_slice())
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
    if retained.as_deref() != Some(write.content()) {
        return Err(OperationError::InvalidStoredOperation.into());
    }
    Ok(())
}

async fn commit_applied(
    tx: &mut Transaction<'_, Postgres>,
    update: &DeclaredUpdate,
    run: &LockedRun,
    previous: &StoredState,
    state: &DeclaredOperationsState,
    receipt: &UpdateReceipt,
) -> Result<(), DeclaredOperationStoreError> {
    let request = update.request();
    let new_state = state.state();
    let result = receipt
        .result()
        .ok_or(OperationError::InvalidStoredOperation)?;
    ensure_retained_value(tx, request.scope().tenant_id(), new_state).await?;
    ensure_retained_value(tx, request.scope().tenant_id(), result).await?;

    let event = RunEvent::StateUpdated {
        operation: request.operation().clone(),
        before_revision: receipt.before_state_revision(),
        after_revision: receipt.after_state_revision(),
        state_digest: new_state.digest(),
    };
    let encoded = crate::event_codec::encode(&event);
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, attempt, detail, payload_digest, actor, \
          trace_id, occurred_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(request.scope().tenant_id().as_uuid())
    .bind(request.resolved_run_id().as_uuid())
    .bind(stored_i64(receipt.run_version(), "receipt run version")?)
    .bind(encoded.event_type)
    .bind(encoded.node)
    .bind(encoded.attempt)
    .bind(encoded.detail)
    .bind(new_state.digest().as_bytes().as_slice())
    .bind(request.actor().as_str())
    .bind(update.audit().trace_id())
    .bind(update.occurred_at())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;

    let state_changed = sqlx::query(
        "UPDATE workflow_operation_states \
         SET state_digest = $4, state_schema_digest = $5, state_size_bytes = $6, \
             state_revision = $7, last_changed_run_version = $8, updated_at = $9 \
         WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $3 \
           AND state_digest = $10 AND state_revision = $11 AND last_changed_run_version = $12",
    )
    .bind(request.scope().tenant_id().as_uuid())
    .bind(request.scope().namespace_id().as_uuid())
    .bind(request.resolved_run_id().as_uuid())
    .bind(new_state.digest().as_bytes().as_slice())
    .bind(new_state.schema_digest().as_bytes().as_slice())
    .bind(i32::try_from(new_state.size_bytes()).map_err(|_| corrupt("state size"))?)
    .bind(stored_i64(state.state_revision(), "state revision")?)
    .bind(stored_i64(
        state.last_changed_run_version(),
        "last changed run version",
    )?)
    .bind(update.occurred_at())
    .bind(previous.value.digest().as_bytes().as_slice())
    .bind(stored_i64(
        previous.state_revision,
        "previous state revision",
    )?)
    .bind(stored_i64(
        previous.last_changed_run_version,
        "previous changed run version",
    )?)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if state_changed != 1 {
        return Err(OperationError::InvalidStoredOperation.into());
    }

    sqlx::query(
        "INSERT INTO event_outbox \
         (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
         VALUES ($1,$2,$3,'run.state_updated',$4,$5)",
    )
    .bind(update.outbox_id())
    .bind(request.scope().tenant_id().as_uuid())
    .bind(request.resolved_run_id().as_uuid())
    .bind(request.operation().as_str())
    .bind(json!({
        "tenantId": request.scope().tenant_id().to_string(),
        "namespaceId": request.scope().namespace_id().to_string(),
        "runId": request.resolved_run_id().to_string(),
        "operation": request.operation().as_str(),
        "runVersion": receipt.run_version(),
        "stateRevision": receipt.after_state_revision(),
        "traceId": update.audit().trace_id(),
    }))
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;

    let (address_scope, target_id) = address_parts(receipt.scope());
    sqlx::query(
        "INSERT INTO operation_update_receipts \
         (tenant_id, namespace_id, address_scope, target_id, update_key, resolved_run_id, \
          operation_name, request_hash, status, result_digest, result_schema_digest, \
          result_size_bytes, before_state_revision, after_state_revision, run_version, actor, \
          outbox_id, committed_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'APPLIED',$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(receipt.scope().tenant_id().as_uuid())
    .bind(receipt.scope().namespace_id().as_uuid())
    .bind(address_scope)
    .bind(target_id)
    .bind(receipt.update_key().as_str())
    .bind(receipt.resolved_run_id().as_uuid())
    .bind(receipt.operation().as_str())
    .bind(receipt.request_hash().as_bytes().as_slice())
    .bind(result.digest().as_bytes().as_slice())
    .bind(result.schema_digest().as_bytes().as_slice())
    .bind(i32::try_from(result.size_bytes()).map_err(|_| corrupt("result size"))?)
    .bind(stored_i64(
        receipt.before_state_revision(),
        "receipt revision",
    )?)
    .bind(stored_i64(
        receipt.after_state_revision(),
        "receipt revision",
    )?)
    .bind(stored_i64(receipt.run_version(), "receipt run version")?)
    .bind(receipt.actor().as_str())
    .bind(update.outbox_id())
    .bind(update.occurred_at())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;

    let run_changed = sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = $4 \
         WHERE tenant_id = $1 AND id = $2 AND run_version = $5",
    )
    .bind(request.scope().tenant_id().as_uuid())
    .bind(request.resolved_run_id().as_uuid())
    .bind(stored_i64(receipt.run_version(), "receipt run version")?)
    .bind(update.occurred_at())
    .bind(stored_i64(run.run_version, "previous run version")?)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if run_changed != 1 {
        return Err(OperationError::InvalidStoredOperation.into());
    }
    Ok(())
}

impl DeclaredOperationStore for PgDeclaredOperationStore {
    async fn query_snapshot(
        &self,
        query: &DeclaredQuery,
        audit: &AuditRecord,
    ) -> Result<DeclaredOperationSnapshot, DeclaredOperationStoreError> {
        check_run_address(query.scope(), query.resolved_run_id())?;
        check_audit(
            audit,
            query.scope().tenant_id(),
            query.actor(),
            query.resolved_run_id(),
            AuditAction::RunQueried,
        )?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        authorize(
            &mut tx,
            query.scope().tenant_id(),
            query.scope().namespace_id(),
            query.actor(),
            Permission::RunQuery,
        )
        .await?;
        authorize(
            &mut tx,
            query.scope().tenant_id(),
            query.scope().namespace_id(),
            query.actor(),
            Permission::RunRead,
        )
        .await?;
        let run = lock_run(&mut tx, query.scope(), query.resolved_run_id(), false).await?;
        let stored = declared_state(
            &mut tx,
            query.scope(),
            &run,
            query.resolved_run_id(),
            query.operation(),
            "QUERY",
        )
        .await?;
        let state = DeclaredOperationsState::restore(
            stored.value.clone(),
            stored.state_revision,
            stored.last_changed_run_version,
            [],
        )?;
        let snapshot = state.query_snapshot(query.resolved_run_id(), run.run_version);
        let snapshot = operation_snapshot(
            &mut tx,
            query.scope(),
            &run,
            query.resolved_run_id(),
            snapshot,
            stored.declaration_digest,
            stored.content,
        )
        .await?;
        let audit = operation_audit(audit, query.operation(), None);
        insert_audit(&mut tx, &audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(snapshot)
    }

    async fn apply_update<V, F>(
        &self,
        update: &DeclaredUpdate,
        validate_arguments: V,
        evaluate: F,
    ) -> Result<UpdateResolution, DeclaredOperationStoreError>
    where
        V: FnOnce(
                &DeclaredOperationSnapshot,
                &DeclaredOperationArguments,
            ) -> Result<(), OperationError>
            + Send,
        F: FnOnce(&DeclaredOperationSnapshot) -> Result<DeclaredUpdateDecision, OperationError>
            + Send,
    {
        let request = update.request();
        if request.request_hash()
            != declared_update_request_hash(
                request.operation(),
                update.arguments(),
                request.expected_state_revision(),
                request.resolved_run_id(),
            )
        {
            return Err(OperationError::InvalidStoredOperation.into());
        }
        check_run_address(request.scope(), request.resolved_run_id())?;
        check_audit(
            update.audit(),
            request.scope().tenant_id(),
            request.actor(),
            request.resolved_run_id(),
            AuditAction::RunUpdated,
        )?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        authorize(
            &mut tx,
            request.scope().tenant_id(),
            request.scope().namespace_id(),
            request.actor(),
            Permission::RunUpdate,
        )
        .await?;
        let run = lock_run(&mut tx, request.scope(), request.resolved_run_id(), true).await?;
        let prior = find_receipt(&mut tx, update).await?;
        if prior.as_ref().is_some_and(|receipt| {
            receipt.operation() != request.operation()
                || receipt.request_hash() != request.request_hash()
        }) {
            return Err(OperationError::UpdateKeyReused.into());
        }
        let stored = declared_state(
            &mut tx,
            request.scope(),
            &run,
            request.resolved_run_id(),
            request.operation(),
            "UPDATE",
        )
        .await?;
        let mut state = DeclaredOperationsState::restore(
            stored.value.clone(),
            stored.state_revision,
            stored.last_changed_run_version,
            prior.clone(),
        )?;
        if prior.is_some() {
            let resolution = state.resolve_update_advancing(
                run.status,
                request,
                run.run_version,
                |_| -> Result<UpdateEvaluation, OperationError> {
                    Err(OperationError::InvalidStoredOperation)
                },
            )?;
            let audit = operation_audit(
                update.audit(),
                request.operation(),
                Some(resolution.receipt().status()),
            );
            insert_audit(&mut tx, &audit).await?;
            tx.commit().await.map_err(store_error)?;
            return Ok(resolution);
        }
        if !matches!(run.status, RunStatus::Running | RunStatus::Waiting) {
            return Err(OperationError::RunNotMutable(run.status).into());
        }
        let query_snapshot = state.query_snapshot(request.resolved_run_id(), run.run_version);
        let snapshot = operation_snapshot(
            &mut tx,
            request.scope(),
            &run,
            request.resolved_run_id(),
            query_snapshot,
            stored.declaration_digest,
            stored.content.clone(),
        )
        .await?;
        validate_arguments(&snapshot, update.arguments())?;
        let mut decision = None;
        let resolution =
            state.resolve_update_advancing(run.status, request, run.run_version, |_| {
                let evaluated = evaluate(&snapshot)?;
                let domain = evaluated.domain_evaluation();
                decision = Some(evaluated);
                Ok(domain)
            })?;
        if resolution.replayed() {
            return Err(OperationError::InvalidStoredOperation.into());
        }
        match resolution.receipt().status() {
            UpdateReceiptStatus::Rejected => {
                insert_rejected_receipt(&mut tx, update, resolution.receipt()).await?;
                let audit = operation_audit(
                    update.audit(),
                    request.operation(),
                    Some(UpdateReceiptStatus::Rejected),
                );
                insert_audit(&mut tx, &audit).await?;
                tx.commit().await.map_err(store_error)?;
                Ok(resolution)
            }
            UpdateReceiptStatus::Applied => {
                if let Some((new_state, result)) =
                    decision.as_ref().and_then(DeclaredUpdateDecision::writes)
                {
                    if resolution.receipt().result() != Some(result.value())
                        || state.state() != new_state.value()
                    {
                        return Err(OperationError::InvalidStoredOperation.into());
                    }
                    retain_evaluated_write(&mut tx, request.scope().tenant_id(), new_state).await?;
                    retain_evaluated_write(&mut tx, request.scope().tenant_id(), result).await?;
                }
                if resolution
                    .receipt()
                    .result()
                    .is_none_or(|result| result.schema_digest() != stored.result_schema_digest)
                {
                    return Err(OperationError::InvalidStoredOperation.into());
                }
                commit_applied(&mut tx, update, &run, &stored, &state, resolution.receipt())
                    .await?;
                let audit = operation_audit(
                    update.audit(),
                    request.operation(),
                    Some(UpdateReceiptStatus::Applied),
                );
                insert_audit(&mut tx, &audit).await?;
                tx.commit().await.map_err(store_error)?;
                Ok(resolution)
            }
        }
    }

    async fn update_result(
        &self,
        update: &DeclaredUpdate,
        resolution: &UpdateResolution,
    ) -> Result<Option<EvaluatedOperationWrite>, DeclaredOperationStoreError> {
        let request = update.request();
        check_run_address(request.scope(), request.resolved_run_id())?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        authorize(
            &mut tx,
            request.scope().tenant_id(),
            request.scope().namespace_id(),
            request.actor(),
            Permission::RunUpdate,
        )
        .await?;
        let run = lock_run(&mut tx, request.scope(), request.resolved_run_id(), false).await?;
        let stored = declared_state(
            &mut tx,
            request.scope(),
            &run,
            request.resolved_run_id(),
            request.operation(),
            "UPDATE",
        )
        .await?;
        let receipt = find_receipt(&mut tx, update)
            .await?
            .ok_or(OperationError::InvalidStoredOperation)?;
        if &receipt != resolution.receipt() {
            return Err(OperationError::InvalidStoredOperation.into());
        }
        let result = match receipt.result() {
            None => None,
            Some(value) => {
                if value.schema_digest() != stored.result_schema_digest {
                    return Err(OperationError::InvalidStoredOperation.into());
                }
                let content: Vec<u8> = sqlx::query_scalar(
                    "SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2",
                )
                .bind(request.scope().tenant_id().as_uuid())
                .bind(value.digest().as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?
                .ok_or(OperationError::InvalidStoredOperation)?;
                Some(EvaluatedOperationWrite::restore(value.clone(), content)?)
            }
        };
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}
