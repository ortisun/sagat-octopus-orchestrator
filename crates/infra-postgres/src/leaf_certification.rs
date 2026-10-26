//! Dormant source certifier. Released runtime INSERT and production scheduling remain gated.

use sagat_application::{
    error::StoreError,
    execution::{DefinitionCache, instance_step_id},
    leaf_certification::{
        CertifyLeafCollectionSuccess as Command, LeafCollectionPathIdentity,
        LeafCollectionSuccessError as Error, LeafCollectionSuccessProfile,
        LeafCollectionSuccessRecord as Record, LeafCollectionSuccessResult,
        LeafCollectionSuccessStore,
    },
    primitives::CollectionScopeId,
};
use sagat_domain::{
    audit::{AuditAction, AuditId, AuditRecord, ResourceType},
    definition::WorkflowVersionId,
    execution::{CollectionOutcome, ExecutionInstanceId, RunEvent},
    workflow::{Digest, LeafCollectionSuccessShape, NodeId, NodeKind, WorkflowDefinition},
};
use sagat_telemetry::{LEAF_CERTIFICATION_SPAN, LeafCertificationOutcome};
use serde_json::{Value, json};
use sqlx::{Decode, PgPool, Postgres, Row, Transaction, Type, postgres::PgRow};
use time::OffsetDateTime;
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    event_codec,
    execution::{lock_member_chain_sql, locked_scheduler_time_sql},
    pool::BeginSafely,
};

#[derive(Clone)]
pub struct PgLeafCollectionSuccessStore {
    pool: PgPool,
    definitions: DefinitionCache,
}

struct LockedSourceRun {
    version: WorkflowVersionId,
    revision: u64,
    trace: Option<String>,
    now: OffsetDateTime,
}

impl PgLeafCollectionSuccessStore {
    #[must_use]
    pub const fn new(pool: PgPool, definitions: DefinitionCache) -> Self {
        Self { pool, definitions }
    }

    async fn apply(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Command,
    ) -> Result<LeafCollectionSuccessResult, Error> {
        let i = &c.identity;
        lock_member_chain_sql(tx, i.tenant_id(), i.namespace_id(), i.run_id())
            .await
            .map_err(storage)?;
        let row = sqlx::query("SELECT workflow_version_id,run_version,trace_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE")
            .bind(i.tenant_id().as_uuid()).bind(i.namespace_id().as_uuid()).bind(i.run_id().as_uuid())
            .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
        let now =
            locked_scheduler_time_sql(tx, i.run_id(), c.scheduler.owner(), c.scheduler.lease())
                .await
                .map_err(storage)?
                .ok_or(Error::SchedulerFenceLost)?;
        let run = LockedSourceRun {
            version: WorkflowVersionId::new(col(&row, "workflow_version_id")?),
            revision: unsigned(col(&row, "run_version")?)?,
            trace: col(&row, "trace_id")?,
            now,
        };
        if let Some(record) =
            read_canonical_leaf_success(tx, &c.identity, run.version, run.revision).await?
        {
            return Ok(LeafCollectionSuccessResult {
                record,
                replayed: true,
            });
        }
        if run.revision != c.expected_run_revision {
            return Err(Error::StaleRunRevision {
                current: run.revision,
            });
        }
        let definition = if let Some(d) = self.definitions.get(run.version) {
            d
        } else {
            let document: Vec<u8> = sqlx::query_scalar(
                "SELECT document FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
            )
            .bind(i.tenant_id().as_uuid())
            .bind(run.version.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?
            .ok_or(Error::SourceEvidenceMissing)?;
            self.definitions
                .get_or_parse(run.version, &document)
                .map_err(|_| Error::ContradictorySource)?
        };
        let owner = i.path().scope_owner().ok_or(Error::UnsupportedPath)?;
        let shape = LeafCollectionSuccessShape::check(&definition, &owner)?;
        let segment = &i.path().segments()[0];
        let audit = AuditRecord::engine(
            i.tenant_id(),
            AuditAction::LeafCollectionSuccessCertified,
            ResourceType::LeafCollectionSuccess,
            format!(
                "{}/{}/{}/{}",
                i.namespace_id(),
                i.run_id(),
                segment.scope_node(),
                segment.iteration_index()
            ),
            "ordinary leaf collection path success certified",
            run.trace.clone(),
            run.now,
        );
        let record = prove(tx, c, &run, &definition, &shape, audit.id()).await?;
        insert_record(tx, c, &record).await?;
        // This fixed noncollapsing transition never invokes generic repeat-dedupe/advisory locks.
        // Classify diagnostics locally; raw SQL driver messages must not enter source logs.
        sqlx::query("INSERT INTO audit_records(id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(audit.id().as_uuid()).bind(audit.tenant_id().as_uuid()).bind(audit.actor().as_str())
            .bind(audit.action().as_str()).bind(audit.resource_type().as_str()).bind(audit.resource_id())
            .bind(audit.reason()).bind(audit.trace_id()).bind(audit.created_at())
            .execute(&mut **tx).await.map_err(storage)?;
        Ok(LeafCollectionSuccessResult {
            record,
            replayed: false,
        })
    }
}

impl LeafCollectionSuccessStore for PgLeafCollectionSuccessStore {
    async fn certify_leaf_collection_success(
        &self,
        c: &Command,
    ) -> Result<LeafCollectionSuccessResult, Error> {
        let i = &c.identity;
        let segment = &i.path().segments()[0]; // The application identity enforces depth1.
        // Operational attempt span inherits the caller trace. This is not a workflow journal
        // projection; direct untraced calls still emit metrics. No command/error/payload logging.
        let span = tracing::info_span!(
            LEAF_CERTIFICATION_SPAN,
            sagat.tenant.id = %i.tenant_id(),
            sagat.namespace.id = %i.namespace_id(),
            sagat.run.id = %i.run_id(),
            sagat.collection.owner_node = %segment.scope_node(),
            sagat.collection.index = segment.iteration_index(),
            sagat.outcome = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        let entered = span.clone();
        async {
            let started = std::time::Instant::now();
            let result = self.certify_transaction(c).await;
            let outcome = match &result {
                Ok(result) if result.replayed => LeafCertificationOutcome::Replayed,
                Ok(_) => LeafCertificationOutcome::Certified,
                Err(Error::Storage(_)) => LeafCertificationOutcome::StorageError,
                Err(Error::CommitOutcomeUnknown { .. }) => LeafCertificationOutcome::Unknown,
                Err(
                    Error::UnsupportedPath
                    | Error::Missing
                    | Error::UnsupportedShape(_)
                    | Error::UnsupportedStoredProfile
                    | Error::SchedulerFenceLost
                    | Error::StaleRunRevision { .. }
                    | Error::SourceNotClosed
                    | Error::SourceEvidenceMissing
                    | Error::ContradictorySource
                    | Error::InvalidRevision,
                ) => LeafCertificationOutcome::Refused,
            };
            // A disabled child must never record onto the caller's current span instead.
            span.record("sagat.outcome", outcome.as_str());
            if matches!(
                outcome,
                LeafCertificationOutcome::StorageError | LeafCertificationOutcome::Unknown
            ) {
                span.record("otel.status_code", "error");
            }
            outcome.record(started.elapsed());
            tracing::debug!(
                outcome = outcome.as_str(),
                "leaf certification attempt completed"
            );
            result
        }
        .instrument(entered)
        .await
    }
}

impl PgLeafCollectionSuccessStore {
    async fn certify_transaction(&self, c: &Command) -> Result<LeafCollectionSuccessResult, Error> {
        c.validate()?;
        let mut tx = self.pool.begin_safely().await.map_err(storage)?;
        let result = match self.apply(&mut tx, c).await {
            Ok(result) => result,
            Err(error) => {
                tx.rollback().await.map_err(storage)?;
                return Err(error);
            }
        };
        match tx.commit().await {
            Ok(()) => Ok(result),
            Err(error) if confirmed_abort(&error) => Err(storage(error)),
            Err(_) => Err(Error::CommitOutcomeUnknown {
                identity: c.identity.clone(),
            }),
        }
    }
}

fn storage(error: sqlx::Error) -> Error {
    // SQLSTATE is diagnostic metadata; never copy the driver message, detail, SQL or bind values.
    let code = error
        .as_database_error()
        .and_then(|e| e.code())
        .filter(|c| c.len() == 5 && c.bytes().all(|b| b.is_ascii_alphanumeric()));
    Error::Storage(StoreError::Internal {
        detail: code.map_or_else(
            || "leaf certification storage operation failed".into(),
            |code| format!("leaf certification storage failure (SQLSTATE {code})"),
        ),
    })
}
fn confirmed_abort(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|c| c.starts_with("23") || matches!(c.as_ref(), "40001" | "40P01"))
}
fn col<'r, T: Decode<'r, Postgres> + Type<Postgres>>(
    row: &'r PgRow,
    name: &str,
) -> Result<T, Error> {
    row.try_get(name).map_err(|_| Error::ContradictorySource)
}
fn unsigned(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::ContradictorySource)
}
fn sql_number(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::ContradictorySource)
}
fn digest(row: &PgRow, name: &str) -> Result<Digest, Error> {
    let bytes: Vec<u8> = col(row, name)?;
    Ok(Digest::new(
        bytes.try_into().map_err(|_| Error::ContradictorySource)?,
    ))
}
fn node(row: &PgRow, name: &str) -> Result<NodeId, Error> {
    NodeId::parse(&col::<String>(row, name)?).map_err(|_| Error::ContradictorySource)
}

/// Read canonical metadata only. The caller retains the exact original Run and current fence in
/// this transaction and supplies its actual pin/revision; this reader grants no authority and
/// does not repeat the source proof or follow a chain head.
pub(crate) async fn read_canonical_leaf_success(
    tx: &mut Transaction<'_, Postgres>,
    i: &LeafCollectionPathIdentity,
    actual_definition_version: WorkflowVersionId,
    actual_run_revision: u64,
) -> Result<Option<Record>, Error> {
    let segment = &i.path().segments()[0];
    // PostgreSQL finite timestamps extend beyond time::OffsetDateTime's year 9999. SQLx's
    // binary decoder panics outside that range, so reject before transferring the timestamp.
    let row=sqlx::query("SELECT profile='ORDINARY_LEAF_V1' AS supported_profile,definition_version_id,scope_id,body_node,return_node,admitted_sequence,returned_sequence,checkpoint_sequence,checkpoint_result_digest,finished_sequence,aggregate_result_digest,owner_routed_sequence,certified_run_revision,CASE WHEN isfinite(certified_at) AND certified_at<TIMESTAMPTZ '10000-01-01 00:00:00+00' THEN certified_at END AS certified_at,audit_id FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 AND owner_node=$4 AND iteration_index=$5")
        .bind(i.tenant_id().as_uuid()).bind(i.namespace_id().as_uuid()).bind(i.run_id().as_uuid())
        .bind(segment.scope_node().as_str()).bind(i64::from(segment.iteration_index()))
        .fetch_optional(&mut **tx).await.map_err(storage)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if !col::<bool>(&row, "supported_profile")? {
        return Err(Error::UnsupportedStoredProfile);
    }
    let r = Record {
        identity: i.clone(),
        profile: LeafCollectionSuccessProfile::OrdinaryLeafV1,
        definition_version: WorkflowVersionId::new(col(&row, "definition_version_id")?),
        scope_id: CollectionScopeId::new(col(&row, "scope_id")?),
        body: node(&row, "body_node")?,
        return_instance: ExecutionInstanceId::in_path(i.path().clone(), node(&row, "return_node")?),
        admitted_sequence: unsigned(col(&row, "admitted_sequence")?)?,
        returned_sequence: unsigned(col(&row, "returned_sequence")?)?,
        checkpoint_sequence: unsigned(col(&row, "checkpoint_sequence")?)?,
        checkpoint_result_digest: digest(&row, "checkpoint_result_digest")?,
        finished_sequence: unsigned(col(&row, "finished_sequence")?)?,
        aggregate_result_digest: digest(&row, "aggregate_result_digest")?,
        owner_routed_sequence: unsigned(col(&row, "owner_routed_sequence")?)?,
        certified_run_revision: unsigned(col(&row, "certified_run_revision")?)?,
        certified_at: col(&row, "certified_at")?,
        audit_id: AuditId::new(col(&row, "audit_id")?),
    };
    r.validate_structure()?;
    if r.definition_version != actual_definition_version
        || r.certified_run_revision > actual_run_revision
    {
        return Err(Error::ContradictorySource);
    }
    Ok(Some(r))
}

async fn prove(
    tx: &mut Transaction<'_, Postgres>,
    c: &Command,
    run: &LockedSourceRun,
    definition: &WorkflowDefinition,
    shape: &LeafCollectionSuccessShape,
    audit_id: AuditId,
) -> Result<Record, Error> {
    let i = &c.identity;
    let segment = &i.path().segments()[0];
    let index = segment.iteration_index();
    let scope=sqlx::query("SELECT scope_id,namespace_id,kind='COLLECTION' AS collection_kind,CASE WHEN octet_length(lifecycle)<=9 THEN lifecycle END AS lifecycle,item_count,next_admission_index,admitted_count,completed_count,active_count,result_digest,finish_sequence,(drain_kind IS NULL AND drain_iteration IS NULL AND drain_failure_code IS NULL AND drain_sequence IS NULL AND drain_outbox_id IS NULL) AS no_drain FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes='{}' AND owner_path_indices='{}' AND owner_node=$3")
        .bind(i.tenant_id().as_uuid()).bind(i.run_id().as_uuid()).bind(segment.scope_node().as_str())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::SourceEvidenceMissing)?;
    if col::<Uuid>(&scope, "namespace_id")? != i.namespace_id().as_uuid()
        || !col::<bool>(&scope, "collection_kind")?
    {
        return Err(Error::ContradictorySource);
    }
    match col::<Option<String>>(&scope, "lifecycle")?.as_deref() {
        Some("SUCCEEDED") => {}
        Some("OPEN" | "DRAINING" | "FAILED" | "CANCELLED") => return Err(Error::SourceNotClosed),
        _ => return Err(Error::ContradictorySource),
    }
    let count: i32 = col(&scope, "item_count")?;
    if i64::from(index) >= i64::from(count) {
        return Err(Error::SourceEvidenceMissing);
    }
    if !col::<bool>(&scope, "no_drain")?
        || col::<i32>(&scope, "next_admission_index")? != count
        || col::<i32>(&scope, "admitted_count")? != count
        || col::<i32>(&scope, "completed_count")? != count
        || col::<i32>(&scope, "active_count")? != 0
    {
        return Err(Error::ContradictorySource);
    }
    let scope_id: Uuid = col(&scope, "scope_id")?;
    let iteration=sqlx::query("SELECT entry_path_nodes,entry_path_indices,entry_node,entry_instance_id,activity,item_digest,lifecycle='CHECKPOINTED' AS checkpointed,slot_held,remaining_reservations,return_path_nodes,return_path_indices,return_node,return_instance_id,result_digest,admitted_sequence,checkpoint_sequence,(drained_sequence IS NULL AND drain_outbox_id IS NULL) AS no_drain FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=$4")
        .bind(i.tenant_id().as_uuid()).bind(i.run_id().as_uuid()).bind(scope_id).bind(i64::from(index))
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::SourceEvidenceMissing)?;
    let entry = ExecutionInstanceId::in_path(i.path().clone(), shape.activities()[0].clone());
    let returned = ExecutionInstanceId::in_path(i.path().clone(), shape.return_node().clone());
    let nodes = vec![segment.scope_node().as_str().to_owned()];
    let indices = vec![i32::try_from(index).map_err(|_| Error::ContradictorySource)?];
    let body = definition
        .body(shape.body())
        .ok_or(Error::ContradictorySource)?;
    let entry_activity = match body.node(entry.node()).map(|n| n.kind()) {
        Some(NodeKind::Activity { activity, .. }) => activity.as_str(),
        _ => return Err(Error::ContradictorySource),
    };
    if !col::<bool>(&iteration, "checkpointed")?
        || col::<bool>(&iteration, "slot_held")?
        || col::<i32>(&iteration, "remaining_reservations")? != 0
        || !col::<bool>(&iteration, "no_drain")?
        || col::<Vec<String>>(&iteration, "entry_path_nodes")? != nodes
        || col::<Vec<i32>>(&iteration, "entry_path_indices")? != indices
        || node(&iteration, "entry_node")? != *entry.node()
        || col::<Uuid>(&iteration, "entry_instance_id")?
            != instance_step_id(run.version, i.run_id(), &entry)
        || col::<String>(&iteration, "activity")? != entry_activity
        || col::<Option<Vec<String>>>(&iteration, "return_path_nodes")?.as_ref() != Some(&nodes)
        || col::<Option<Vec<i32>>>(&iteration, "return_path_indices")?.as_ref() != Some(&indices)
        || node(&iteration, "return_node")? != *returned.node()
        || col::<Uuid>(&iteration, "return_instance_id")?
            != instance_step_id(run.version, i.run_id(), &returned)
    {
        return Err(Error::ContradictorySource);
    }
    let checkpoint=sqlx::query("SELECT return_instance_id,result_digest,journal_sequence FROM iteration_checkpoints WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=$4")
        .bind(i.tenant_id().as_uuid()).bind(i.run_id().as_uuid()).bind(scope_id).bind(i64::from(index))
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::SourceEvidenceMissing)?;
    let checkpoint_sequence = unsigned(col(&checkpoint, "journal_sequence")?)?;
    if col::<Uuid>(&checkpoint, "return_instance_id")?
        != instance_step_id(run.version, i.run_id(), &returned)
        || digest(&checkpoint, "result_digest")? != digest(&iteration, "result_digest")?
        || col::<i64>(&checkpoint, "journal_sequence")?
            != col::<i64>(&iteration, "checkpoint_sequence")?
    {
        return Err(Error::ContradictorySource);
    }
    let finished_sequence = unsigned(col(&scope, "finish_sequence")?)?;
    let r = Record {
        identity: i.clone(),
        profile: LeafCollectionSuccessProfile::OrdinaryLeafV1,
        definition_version: run.version,
        scope_id: CollectionScopeId::new(scope_id),
        body: shape.body().clone(),
        return_instance: returned.clone(),
        admitted_sequence: unsigned(col(&iteration, "admitted_sequence")?)?,
        returned_sequence: checkpoint_sequence
            .checked_sub(1)
            .ok_or(Error::ContradictorySource)?,
        checkpoint_sequence,
        checkpoint_result_digest: digest(&checkpoint, "result_digest")?,
        finished_sequence,
        aggregate_result_digest: digest(&scope, "result_digest")?,
        owner_routed_sequence: finished_sequence
            .checked_add(1)
            .ok_or(Error::ContradictorySource)?,
        certified_run_revision: run.revision,
        certified_at: run.now,
        audit_id,
    };
    r.validate_structure()?;
    let owner_rows = steps(tx, c, &[], &[], Some(shape.owner().node().as_str())).await?;
    if owner_rows.is_empty() {
        return Err(Error::SourceEvidenceMissing);
    }
    if owner_rows.len() != 1 {
        return Err(Error::ContradictorySource);
    }
    check_step(&owner_rows[0], c, run, shape.owner(), None)?;
    let members = steps(tx, c, &nodes, &indices, None).await?;
    let expected = shape
        .activities()
        .iter()
        .chain(std::iter::once(shape.return_node()))
        .collect::<Vec<_>>();
    if members.len() > expected.len() {
        return Err(Error::ContradictorySource);
    }
    for row in &members {
        let name =
            NodeId::parse(&col::<Option<String>>(row, "node")?.ok_or(Error::ContradictorySource)?)
                .map_err(|_| Error::ContradictorySource)?;
        if !expected.contains(&&name) {
            return Err(Error::ContradictorySource);
        }
        let activity = match body.node(&name).map(|n| n.kind()) {
            Some(NodeKind::Activity { activity, .. }) => Some(activity.as_str()),
            Some(NodeKind::Return { .. }) => None,
            _ => return Err(Error::ContradictorySource),
        };
        check_step(
            row,
            c,
            run,
            &ExecutionInstanceId::in_path(i.path().clone(), name),
            activity,
        )?;
    }
    if members.len() != expected.len() {
        return Err(Error::SourceEvidenceMissing);
    }
    let saga:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes='{}' AND owner_path_indices='{}' AND owner_node=$3) OR EXISTS(SELECT 1 FROM scope_saga_entries WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes='{}' AND owner_path_indices='{}' AND owner_node=$3 AND iteration_index=$4)")
        .bind(i.tenant_id().as_uuid()).bind(i.run_id().as_uuid()).bind(segment.scope_node().as_str()).bind(i64::from(index))
        .fetch_one(&mut **tx).await.map_err(storage)?;
    if saga {
        return Err(Error::ContradictorySource);
    }
    let successor = definition
        .node(shape.owner().node())
        .and_then(|n| n.next())
        .ok_or(Error::ContradictorySource)?;
    verify_journal(
        tx,
        c,
        run,
        &r,
        &entry,
        digest(&iteration, "item_digest")?,
        successor,
    )
    .await?;
    Ok(r)
}

async fn steps(
    tx: &mut Transaction<'_, Postgres>,
    c: &Command,
    nodes: &[String],
    indices: &[i32],
    root_node: Option<&str>,
) -> Result<Vec<PgRow>, Error> {
    sqlx::query("SELECT CASE WHEN octet_length(node)<=64 THEN node END AS node,octet_length(node)<=64 AS node_valid,step_id,namespace_id,kind='FORWARD' AS forward,status='SUCCEEDED' AS succeeded,routed,CASE WHEN octet_length(activity)<=128 THEN activity END AS activity,(activity IS NULL OR octet_length(activity)<=128) AS activity_valid,(lease_token_hash IS NULL AND lease_expires_at IS NULL AND attempt_deadline_at IS NULL AND leased_by IS NULL AND next_attempt_at IS NULL AND waiting_kind IS NULL) AS no_work FROM steps WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 AND instance_path_indices=$4 AND ($5::text IS NULL OR node=$5) ORDER BY steps.node LIMIT 10")
        .bind(c.identity.tenant_id().as_uuid()).bind(c.identity.run_id().as_uuid()).bind(nodes).bind(indices).bind(root_node)
        .fetch_all(&mut **tx).await.map_err(storage)
}
fn check_step(
    row: &PgRow,
    c: &Command,
    run: &LockedSourceRun,
    instance: &ExecutionInstanceId,
    activity: Option<&str>,
) -> Result<(), Error> {
    if !col::<bool>(row, "node_valid")?
        || !col::<bool>(row, "activity_valid")?
        || !col::<bool>(row, "forward")?
        || !col::<bool>(row, "succeeded")?
        || !col::<bool>(row, "routed")?
        || !col::<bool>(row, "no_work")?
        || col::<Option<String>>(row, "node")?.as_deref() != Some(instance.node().as_str())
        || col::<Option<String>>(row, "activity")?.as_deref() != activity
        || col::<Uuid>(row, "namespace_id")? != c.identity.namespace_id().as_uuid()
        || col::<Uuid>(row, "step_id")?
            != instance_step_id(run.version, c.identity.run_id(), instance)
    {
        return Err(Error::ContradictorySource);
    }
    Ok(())
}

const JOURNAL_METADATA: &str = r#"
WITH source AS (
 SELECT seq,event_type,node,instance_id,instance_path_nodes,instance_path_indices,
 CASE WHEN event_type IN ('collection.iteration_admitted','collection.iteration_returned','collection.iteration_checkpointed','collection.finished','step.routed') THEN detail ELSE '{}'::jsonb END AS d
 FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND seq=ANY($3::bigint[])
)
SELECT seq,CASE WHEN octet_length(event_type)<=64 THEN event_type END AS event_type,
 CASE WHEN octet_length(node)<=64 THEN node END AS node,instance_id,instance_path_nodes,instance_path_indices,
 CASE WHEN jsonb_typeof(d->'index')='number' AND (d->>'index')~'^[0-9]{1,4}$' THEN (d->>'index')::bigint END AS detail_index,
 CASE WHEN jsonb_typeof(d#>'{entry,path}')='array' THEN jsonb_array_length(d#>'{entry,path}') END AS entry_depth,
 CASE WHEN jsonb_typeof(d#>'{entry,node}')='string' AND octet_length(d#>>'{entry,node}')<=64 THEN d#>>'{entry,node}' END AS entry_node,
 CASE WHEN jsonb_typeof(d#>'{entry,path,0,scope}')='string' AND octet_length(d#>>'{entry,path,0,scope}')<=64 THEN d#>>'{entry,path,0,scope}' END AS entry_scope,
 CASE WHEN jsonb_typeof(d#>'{entry,path,0,index}')='number' AND (d#>>'{entry,path,0,index}')~'^[0-9]{1,4}$' THEN (d#>>'{entry,path,0,index}')::bigint END AS entry_index,
 CASE WHEN jsonb_typeof(d#>'{return_instance,path}')='array' THEN jsonb_array_length(d#>'{return_instance,path}') END AS return_depth,
 CASE WHEN jsonb_typeof(d#>'{return_instance,node}')='string' AND octet_length(d#>>'{return_instance,node}')<=64 THEN d#>>'{return_instance,node}' END AS return_node,
 CASE WHEN jsonb_typeof(d#>'{return_instance,path,0,scope}')='string' AND octet_length(d#>>'{return_instance,path,0,scope}')<=64 THEN d#>>'{return_instance,path,0,scope}' END AS return_scope,
 CASE WHEN jsonb_typeof(d#>'{return_instance,path,0,index}')='number' AND (d#>>'{return_instance,path,0,index}')~'^[0-9]{1,4}$' THEN (d#>>'{return_instance,path,0,index}')::bigint END AS return_index,
 CASE WHEN jsonb_typeof(d->'item_digest')='string' AND octet_length(d->>'item_digest')<=64 THEN d->>'item_digest' END AS item_digest,
 CASE WHEN jsonb_typeof(d->'result_digest')='string' AND octet_length(d->>'result_digest')<=64 THEN d->>'result_digest' END AS result_digest,
 CASE WHEN jsonb_typeof(d->'outcome')='string' AND octet_length(d->>'outcome')<=16 THEN d->>'outcome' END AS outcome,
 CASE WHEN jsonb_typeof(d->'activated')='array' THEN jsonb_array_length(d->'activated') END AS activated_count,
 CASE WHEN jsonb_typeof(d#>'{activated,0}')='string' AND octet_length(d#>>'{activated,0}')<=64 THEN d#>>'{activated,0}' END AS activated_node
FROM source ORDER BY seq
"#;

fn projected_instance(row: &PgRow, prefix: &str) -> Result<Value, Error> {
    let depth: Option<i32> = col(row, &format!("{prefix}_depth"))?;
    let node: Option<String> = col(row, &format!("{prefix}_node"))?;
    let scope: Option<String> = col(row, &format!("{prefix}_scope"))?;
    let index: Option<i64> = col(row, &format!("{prefix}_index"))?;
    Ok(match (depth, node, scope, index) {
        (Some(1), Some(node), Some(scope), Some(index)) => {
            json!({"path":[{"scope":scope,"index":index}],"node":node})
        }
        _ => Value::Null,
    })
}
async fn verify_journal(
    tx: &mut Transaction<'_, Postgres>,
    c: &Command,
    run: &LockedSourceRun,
    r: &Record,
    entry: &ExecutionInstanceId,
    item_digest: Digest,
    successor: &NodeId,
) -> Result<(), Error> {
    let owner = c
        .identity
        .path()
        .scope_owner()
        .ok_or(Error::UnsupportedPath)?;
    let node = owner.node();
    let index = c.identity.path().segments()[0].iteration_index();
    let expected = [
        (
            r.admitted_sequence,
            RunEvent::CollectionIterationAdmitted {
                node: node.clone(),
                index,
                entry: entry.clone(),
                item_digest,
            },
        ),
        (
            r.returned_sequence,
            RunEvent::CollectionIterationReturned {
                node: node.clone(),
                index,
                return_instance: r.return_instance.clone(),
            },
        ),
        (
            r.checkpoint_sequence,
            RunEvent::CollectionIterationCheckpointed {
                node: node.clone(),
                index,
                return_instance: r.return_instance.clone(),
                result_digest: r.checkpoint_result_digest,
            },
        ),
        (
            r.finished_sequence,
            RunEvent::CollectionFinished {
                node: node.clone(),
                outcome: CollectionOutcome::Succeeded,
                result_digest: Some(r.aggregate_result_digest),
            },
        ),
        (
            r.owner_routed_sequence,
            RunEvent::StepRouted {
                node: node.clone(),
                activated: vec![successor.clone()],
            },
        ),
    ];
    let keys = expected
        .iter()
        .map(|(seq, _)| sql_number(*seq))
        .collect::<Result<Vec<_>, _>>()?;
    let rows = sqlx::query(JOURNAL_METADATA)
        .bind(c.identity.tenant_id().as_uuid())
        .bind(c.identity.run_id().as_uuid())
        .bind(keys)
        .fetch_all(&mut **tx)
        .await
        .map_err(storage)?;
    if rows.len() != 5 {
        return Err(Error::SourceEvidenceMissing);
    }
    for (row, (seq, event)) in rows.iter().zip(expected) {
        let event_type: Option<String> = col(row, "event_type")?;
        let stored_node: Option<String> = col(row, "node")?;
        if unsigned(col(row, "seq")?)? != seq
            || event_type.as_deref() != Some(event.name())
            || stored_node.as_deref() != Some(node.as_str())
            || col::<Option<Uuid>>(row, "instance_id")?
                != Some(instance_step_id(run.version, c.identity.run_id(), &owner))
            || !col::<Option<Vec<String>>>(row, "instance_path_nodes")?
                .is_some_and(|p| p.is_empty())
            || !col::<Option<Vec<i32>>>(row, "instance_path_indices")?.is_some_and(|p| p.is_empty())
        {
            return Err(Error::ContradictorySource);
        }
        let activated = if col::<Option<i32>>(row, "activated_count")? == Some(1) {
            json!([col::<Option<String>>(row, "activated_node")?])
        } else {
            Value::Null
        };
        let detail = json!({"index":col::<Option<i64>>(row,"detail_index")?,"entry":projected_instance(row,"entry")?,"return_instance":projected_instance(row,"return")?,"item_digest":col::<Option<String>>(row,"item_digest")?,"result_digest":col::<Option<String>>(row,"result_digest")?,"outcome":col::<Option<String>>(row,"outcome")?,"activated":activated});
        if event_codec::decode(event.name(), stored_node.as_deref(), &detail).as_ref()
            != Some(&event)
        {
            return Err(Error::ContradictorySource);
        }
    }
    Ok(())
}

async fn insert_record(
    tx: &mut Transaction<'_, Postgres>,
    c: &Command,
    r: &Record,
) -> Result<(), Error> {
    let i = &c.identity;
    let segment = &i.path().segments()[0];
    sqlx::query("INSERT INTO leaf_collection_success_certifications(tenant_id,namespace_id,run_id,owner_node,iteration_index,profile,definition_version_id,scope_id,body_node,return_node,admitted_sequence,returned_sequence,checkpoint_sequence,checkpoint_result_digest,finished_sequence,aggregate_result_digest,owner_routed_sequence,certified_run_revision,certified_at,audit_id) VALUES($1,$2,$3,$4,$5,'ORDINARY_LEAF_V1',$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)")
        .bind(i.tenant_id().as_uuid()).bind(i.namespace_id().as_uuid()).bind(i.run_id().as_uuid()).bind(segment.scope_node().as_str()).bind(i64::from(segment.iteration_index()))
        .bind(r.definition_version.as_uuid()).bind(r.scope_id.as_uuid()).bind(r.body.as_str()).bind(r.return_instance.node().as_str())
        .bind(sql_number(r.admitted_sequence)?).bind(sql_number(r.returned_sequence)?).bind(sql_number(r.checkpoint_sequence)?).bind(r.checkpoint_result_digest.as_bytes().as_slice())
        .bind(sql_number(r.finished_sequence)?).bind(r.aggregate_result_digest.as_bytes().as_slice()).bind(sql_number(r.owner_routed_sequence)?).bind(sql_number(r.certified_run_revision)?).bind(r.certified_at).bind(r.audit_id.as_uuid())
        .execute(&mut **tx).await.map_err(storage)?;
    Ok(())
}
