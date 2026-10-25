//! Native PostgreSQL `RUN_DATA_LIMIT` collection drain evidence.
//!
//! The scheduler producer is intentionally outside this module. Tests drive public workflow and
//! worker prefixes, then invoke the checked storage command as a native command fixture.

use std::sync::Arc;

use sagat_application::execution::{
    DefinitionCache, MAX_PAYLOAD_BYTES, PathExecutionStore, PathExecutionTx, RunTx,
};
use sagat_application::primitives::{
    CollectionCheckpoint, CollectionDataLimitEvidence, CollectionDrain, CollectionScopeId,
    CollectionStore, CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    AncestorExecutionFence, CollectionPhase, ExecutionInstanceId, InstancePath, RunId, RunStatus,
    StopReason,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgCollectionStore, PgRunStore};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::byte_ceilings::{answer, collection};
use crate::collection_empty_quota::{
    PREFIX_COUNT, assert_prefix_outputs, complete_prefix_over_grpc, empty_collection_after_prefix,
};
use crate::limits::{
    allow_iterations, register_scoped, scheduler as scope_scheduler, scoped_workers,
};
use crate::support::blob_collections::{platform, scheduler, start_inline};
use crate::support::engine::{World, world};
use crate::support::now;

const OWNER: &str = "items";

fn cache() -> DefinitionCache {
    DefinitionCache::new(Arc::new(JsonIrCodec), 64)
}

fn native_store(pool: PgPool) -> PgCollectionStore {
    PgCollectionStore::with_definition_cache(pool, cache())
}

#[derive(Clone)]
struct NativeIdentity {
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    scope: CollectionScopeId,
    version: u64,
    scheduler_owner: String,
    scheduler_generation: u64,
    trace: Option<String>,
}

async fn identity(pool: &PgPool, tenant: TenantId, run: RunId) -> NativeIdentity {
    let row = sqlx::query(
        "SELECT run.namespace_id,run.run_version,run.trace_id,scope.scope_id,shard.owner,shard.generation \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN scheduler_shards shard ON shard.shard=$3 \
          WHERE run.tenant_id=$1 AND run.id=$2 AND scope.owner_path_nodes='{}' \
            AND scope.owner_path_indices='{}' AND scope.owner_node='items'",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(i16::from(sagat_domain::sharding::Shard::of(run.as_uuid()).index()))
    .fetch_one(pool)
    .await
    .expect("entered collection and scheduler fence");
    NativeIdentity {
        tenant,
        namespace: NamespaceId::new(row.get("namespace_id")),
        run,
        scope: CollectionScopeId::new(row.get("scope_id")),
        version: u64::try_from(row.get::<i64, _>("run_version")).expect("run version"),
        scheduler_owner: row.get("owner"),
        scheduler_generation: u64::try_from(row.get::<i64, _>("generation"))
            .expect("scheduler generation"),
        trace: row.get("trace_id"),
    }
}

fn data_limit_command(
    id: &NativeIdentity,
    owner: ExecutionInstanceId,
    evidence: CollectionDataLimitEvidence,
    outbox: Uuid,
) -> CollectionDrain {
    let at = now();
    let audit = AuditRecord::engine(
        id.tenant,
        AuditAction::RunDataLimitExceeded,
        ResourceType::Run,
        id.run.to_string(),
        evidence.required_audit_reason().as_str(),
        id.trace.clone(),
        at,
    );
    CollectionDrain::for_data_limit(
        CollectionTransition::new(
            id.tenant,
            id.namespace,
            id.run,
            id.version,
            owner,
            SchedulerAuthority::for_run(
                id.scheduler_owner.clone(),
                id.run,
                id.scheduler_generation,
            ),
            Subject::engine(),
            id.trace.clone(),
            at,
            Some(audit),
        ),
        id.scope,
        evidence,
        outbox,
    )
    .expect("checked native data-limit command")
}

async fn mark(pool: &PgPool, id: &NativeIdentity) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
                'runStatus',run.status,'stopReason',run.stop_reason,'stopCode',run.stop_code, \
                'runVersion',run.run_version,'scopeLifecycle',scope.lifecycle, \
                'drainKind',scope.drain_kind,'drainIteration',scope.drain_iteration, \
                'drainFailureCode',scope.drain_failure_code, \
                'drainSequence',scope.drain_sequence,'drainOutboxId',scope.drain_outbox_id, \
                'scopeResultDigest',encode(scope.result_digest,'hex'), \
                'scopeId',scope.scope_id,'ownerStepId',owner.step_id, \
                'ownerStatus',owner.status,'ownerWaitingKind',owner.waiting_kind, \
                'ownerOutputDigest',encode(owner.output_digest,'hex'), \
                'logicalOutputBytes',usage.logical_output_bytes, \
                'iterationRefs',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'index',member.iteration_index,'lifecycle',member.lifecycle, \
                    'returnId',member.return_instance_id, \
                    'resultDigest',encode(member.result_digest,'hex'), \
                    'checkpointSequence',member.checkpoint_sequence, \
                    'drainedSequence',member.drained_sequence, \
                    'drainOutboxId',member.drain_outbox_id) \
                  ORDER BY member.iteration_index),'[]'::jsonb) \
                  FROM collection_iterations member \
                  WHERE member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                    AND member.scope_id=scope.scope_id), \
                'checkpointRefs',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'index',checkpoint.iteration_index, \
                    'returnId',checkpoint.return_instance_id, \
                    'resultDigest',encode(checkpoint.result_digest,'hex'), \
                    'journalSequence',checkpoint.journal_sequence, \
                    'outboxId',checkpoint.outbox_id) \
                  ORDER BY checkpoint.iteration_index),'[]'::jsonb) \
                  FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=scope.tenant_id \
                    AND checkpoint.run_id=scope.run_id \
                    AND checkpoint.scope_id=scope.scope_id), \
                'eventCount',(SELECT count(*) FROM run_events e \
                  WHERE e.tenant_id=run.tenant_id AND e.run_id=run.id), \
                'outboxCount',(SELECT count(*) FROM event_outbox o \
                  WHERE o.tenant_id=run.tenant_id AND o.aggregate_id=run.id), \
                'auditCount',(SELECT count(*) FROM audit_records a \
                  WHERE a.tenant_id=run.tenant_id AND a.resource_type='run' \
                    AND a.resource_id=run.id::text), \
                'telemetryHead',coalesce((SELECT head_seq FROM telemetry_pending t \
                  WHERE t.tenant_id=run.tenant_id AND t.run_id=run.id),0), \
                'drainReceipt',(SELECT jsonb_build_object( \
                    'id',receipt.id,'tenantId',receipt.tenant_id, \
                    'aggregateId',receipt.aggregate_id,'eventType',receipt.event_type, \
                    'routingKey',receipt.routing_key,'payload',receipt.payload) \
                  FROM event_outbox receipt WHERE receipt.id=scope.drain_outbox_id)) \
           FROM runs run \
           JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id AND scope.scope_id=$3 \
           JOIN steps owner \
             ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
            AND owner.instance_path_nodes=scope.owner_path_nodes \
            AND owner.instance_path_indices=scope.owner_path_indices \
            AND owner.node=scope.owner_node \
           JOIN run_instance_usage usage \
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(id.tenant.as_uuid())
    .bind(id.run.as_uuid())
    .bind(id.scope.as_uuid())
    .fetch_one(pool)
    .await
    .expect("native writer fingerprint")
}

async fn public_empty_at_limit() -> (crate::support::worker::Platform, NativeIdentity, PgPool) {
    let p = platform().await;
    let name = format!("native-empty-data-limit-{}", Uuid::now_v7());
    let workflow = p
        .publish(&name, &empty_collection_after_prefix(&name))
        .await;
    let run_uuid = start_inline(&p, &workflow, &json!({})).await;
    let sched = scheduler(&p, None).await;
    let mut worker = p.worker("native-empty-data-limit-worker").await;
    worker
        .register_version(&["src.fetch", "src.line"], "1.5")
        .await;
    complete_prefix_over_grpc(&mut worker, &sched, &[MAX_PAYLOAD_BYTES; PREFIX_COUNT]).await;
    assert_prefix_outputs(&p, run_uuid, 16 * MAX_PAYLOAD_BYTES).await;
    let entered = sched.tick().await;
    assert_eq!(entered.failed, 0, "enter empty collection: {entered:?}");
    let runtime = p.api.db.runtime_pool().await;
    let run = RunId::new(run_uuid);
    let id = identity(&p.api.db.pool, p.tenant, run).await;
    (p, id, runtime)
}

#[tokio::test]
async fn native_empty_aggregate_stops_atomically_and_cold_replays_original_receipt() {
    let (p, id, runtime) = public_empty_at_limit().await;
    let outbox = Uuid::now_v7();
    let owner = ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner"));
    let command = data_limit_command(
        &id,
        owner.clone(),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty aggregate evidence"),
        outbox,
    );
    let before = mark(&p.api.db.pool, &id).await;
    let mut wrong_namespace = id.clone();
    wrong_namespace.namespace = NamespaceId::new(Uuid::now_v7());
    let wrong_namespace_command = data_limit_command(
        &wrong_namespace,
        owner.clone(),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        outbox,
    );
    assert!(matches!(
        native_store(runtime.clone())
            .start_collection_drain(&wrong_namespace_command)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, before);

    let mut stale_revision = id.clone();
    stale_revision.version = stale_revision
        .version
        .checked_add(1)
        .expect("bounded revision");
    let stale_revision_command = data_limit_command(
        &stale_revision,
        owner.clone(),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        outbox,
    );
    assert!(matches!(
        native_store(runtime.clone())
            .start_collection_drain(&stale_revision_command)
            .await,
        Err(PrimitiveStoreError::RevisionConflict { current }) if current == id.version
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, before);

    let mut stale_fence = id.clone();
    stale_fence.scheduler_generation = stale_fence
        .scheduler_generation
        .checked_add(1)
        .expect("bounded scheduler generation");
    let stale_fence_command = data_limit_command(
        &stale_fence,
        owner,
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        outbox,
    );
    assert!(matches!(
        native_store(runtime.clone())
            .start_collection_drain(&stale_fence_command)
            .await,
        Err(PrimitiveStoreError::SchedulerFenceLost)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, before);

    let written = native_store(runtime.clone())
        .start_collection_drain(&command)
        .await
        .expect("native aggregate refusal");
    assert!(written.applied);

    let row = sqlx::query(
        "SELECT run.status,run.stop_reason,run.stop_code,run.run_version, \
                scope.lifecycle,scope.drain_kind,scope.drain_failure_code,scope.drain_sequence, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id AND e.run_id=run.id \
                  AND e.event_type='run.stop_requested' AND e.detail->>'reason'='FAILED' \
                  AND e.detail->>'code'='RUN_DATA_LIMIT') AS stops, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id AND e.run_id=run.id \
                  AND e.event_type='collection.drain_started' AND e.detail->>'reason'='run_data_limit') AS drains, \
                (SELECT count(*) FROM event_outbox o WHERE o.id=$4 AND o.event_type='collection.drain.started' \
                  AND o.payload->>'reason'='FAILED') AS hints, \
                (SELECT count(*) FROM audit_records a WHERE a.tenant_id=run.tenant_id \
                  AND a.resource_type='run' AND a.resource_id=run.id::text \
                  AND a.action='run.data_limit_exceeded' AND a.reason='aggregate') AS audits, \
                (SELECT head_seq FROM telemetry_pending t WHERE t.tenant_id=run.tenant_id AND t.run_id=run.id) AS head \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id AND scope.scope_id=$3 \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(id.tenant.as_uuid())
    .bind(id.run.as_uuid())
    .bind(id.scope.as_uuid())
    .bind(outbox)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("native drain projections");
    assert_eq!(row.get::<String, _>("status"), "CANCELLING");
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("FAILED")
    );
    assert_eq!(
        row.get::<Option<String>, _>("stop_code").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(row.get::<String, _>("lifecycle"), "DRAINING");
    assert_eq!(
        row.get::<Option<String>, _>("drain_kind").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        row.get::<Option<String>, _>("drain_failure_code")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        (
            row.get::<i64, _>("stops"),
            row.get::<i64, _>("drains"),
            row.get::<i64, _>("hints"),
            row.get::<i64, _>("audits")
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(
        row.get::<i64, _>("run_version"),
        row.get::<i64, _>("drain_sequence")
    );
    assert_eq!(row.get::<i64, _>("head"), row.get::<i64, _>("run_version"));

    let run_store = PgRunStore::new(runtime.clone(), cache());
    let mut full = run_store
        .begin(id.tenant, id.run, Lock::Wait)
        .await
        .expect("full reader")
        .expect("run");
    let (_, state) = full.parts();
    assert_eq!(state.status(), RunStatus::Cancelling);
    assert_eq!(state.stop_reason(), Some(StopReason::Failed));
    assert_eq!(
        state.stop_code().map(|code| code.as_str()),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        state
            .collection(command.transition.owner())
            .map(|scope| scope.phase()),
        Some(CollectionPhase::Draining)
    );
    full.commit().await.expect("close full reader");

    let committed = mark(&p.api.db.pool, &id).await;
    assert!(matches!(
        native_store(runtime.clone())
            .start_collection_drain(&wrong_namespace_command)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, committed);
    assert!(matches!(
        native_store(runtime.clone())
            .start_collection_drain(&stale_fence_command)
            .await,
        Err(PrimitiveStoreError::SchedulerFenceLost)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, committed);
    let stale_receipt = native_store(runtime.clone())
        .start_collection_drain(&stale_revision_command)
        .await
        .expect("durable receipt precedes revision validation");
    assert!(!stale_receipt.applied);
    assert_eq!(stale_receipt.run_version, written.run_version);
    assert_eq!(mark(&p.api.db.pool, &id).await, committed);

    // Simulate a changed fresh budget after commit. Receipt replay must precede revalidation.
    sqlx::query(
        "UPDATE run_instance_usage SET logical_output_bytes=0 WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(id.tenant.as_uuid())
    .bind(id.run.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("isolated replay budget fixture");
    let changed_budget = mark(&p.api.db.pool, &id).await;
    let cold = native_store(runtime);
    let replay = cold
        .start_collection_drain(&command)
        .await
        .expect("cold replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, written.run_version);
    assert_eq!(mark(&p.api.db.pool, &id).await, changed_budget);
    let mismatch = data_limit_command(
        &id,
        ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner")),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        Uuid::now_v7(),
    );
    assert!(matches!(
        cold.start_collection_drain(&mismatch).await,
        Err(PrimitiveStoreError::DuplicateMismatch)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, changed_budget);
}

fn empty_first(name: &str) -> Value {
    json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
        "metadata":{"name":name}, "entrypoint":"items",
        "limits":{"maxInstances":4,"maxDepth":1},
        "bodies":{"line":{"entrypoint":"work","nodes":[
            {"id":"work","type":"activity","activity":"never","next":"returned"},
            {"id":"returned","type":"return","result":"steps.work.output"}
        ]}},
        "nodes":[
            {"id":"items","type":"forEach","items":"input.items","body":"line","result":"bodyResult","maxItems":1,"maxConcurrency":1,"next":"done"},
            {"id":"done","type":"end"}
        ]
    })
}

#[tokio::test]
async fn fitting_empty_aggregate_is_rejected_without_writes() {
    let p = platform().await;
    let name = format!("native-fitting-empty-{}", Uuid::now_v7());
    let workflow = p.publish(&name, &empty_first(&name)).await;
    let run = RunId::new(start_inline(&p, &workflow, &json!({"items":[]})).await);
    let sched = scheduler(&p, None).await;
    assert_eq!(sched.tick().await.failed, 0);
    let id = identity(&p.api.db.pool, p.tenant, run).await;
    let before = mark(&p.api.db.pool, &id).await;
    let command = data_limit_command(
        &id,
        ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner")),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        Uuid::now_v7(),
    );
    assert!(matches!(
        native_store(p.api.db.runtime_pool().await)
            .start_collection_drain(&command)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(mark(&p.api.db.pool, &id).await, before);
}

struct CheckpointFixture {
    world: World,
    id: NativeIdentity,
    runtime: PgPool,
    source: Vec<u8>,
    member_path: InstancePath,
}

async fn assert_native_checkpoint_source(
    pool: &PgPool,
    id: &NativeIdentity,
    source: &[u8],
    expected_sequence: u64,
) {
    let row = sqlx::query(
        "SELECT scope.lifecycle AS scope_lifecycle,scope.item_count, \
                scope.next_admission_index,scope.admitted_count,scope.completed_count, \
                scope.active_count,scope.drain_kind,scope.drain_iteration, \
                scope.drain_failure_code,scope.drain_sequence,scope.drain_outbox_id, \
                member.lifecycle AS member_lifecycle,member.slot_held, \
                member.return_path_nodes,member.return_path_indices,member.return_node, \
                member.return_instance_id AS member_return_id, \
                member.result_digest AS member_result_digest,member.checkpoint_sequence, \
                checkpoint.return_instance_id AS checkpoint_return_id, \
                checkpoint.result_digest AS checkpoint_result_digest, \
                checkpoint.journal_sequence,checkpoint.outbox_id AS checkpoint_outbox_id, \
                returned.step_id AS returned_id,returned.status AS returned_status, \
                returned.routed AS returned_routed,work.status AS work_status, \
                work.routed AS work_routed,work.output_digest AS work_output_digest, \
                payload.content AS result_content, \
                (SELECT count(*) FROM run_events event \
                  WHERE event.tenant_id=scope.tenant_id AND event.run_id=scope.run_id \
                    AND event.event_type='collection.drain_started') AS drain_events \
           FROM iteration_scopes scope \
           JOIN collection_iterations member \
             ON member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
            AND member.scope_id=scope.scope_id AND member.iteration_index=0 \
           JOIN iteration_checkpoints checkpoint \
             ON checkpoint.tenant_id=member.tenant_id AND checkpoint.run_id=member.run_id \
            AND checkpoint.scope_id=member.scope_id \
            AND checkpoint.iteration_index=member.iteration_index \
           JOIN steps returned \
             ON returned.tenant_id=member.tenant_id AND returned.run_id=member.run_id \
            AND returned.step_id=member.return_instance_id \
           JOIN steps work \
             ON work.tenant_id=member.tenant_id AND work.run_id=member.run_id \
            AND work.instance_path_nodes=member.entry_path_nodes \
            AND work.instance_path_indices=member.entry_path_indices \
            AND work.node=member.entry_node \
           JOIN payloads payload \
             ON payload.tenant_id=checkpoint.tenant_id \
            AND payload.digest=checkpoint.result_digest \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.scope_id=$3",
    )
    .bind(id.tenant.as_uuid())
    .bind(id.run.as_uuid())
    .bind(id.scope.as_uuid())
    .fetch_one(pool)
    .await
    .expect("typed native checkpoint projection");
    assert_eq!(row.get::<String, _>("scope_lifecycle"), "OPEN");
    assert_eq!(
        (
            row.get::<i32, _>("item_count"),
            row.get::<i32, _>("next_admission_index"),
            row.get::<i32, _>("admitted_count"),
            row.get::<i32, _>("completed_count"),
            row.get::<i32, _>("active_count"),
        ),
        (1, 1, 1, 1, 0)
    );
    assert!(row.get::<Option<String>, _>("drain_kind").is_none());
    assert!(row.get::<Option<i32>, _>("drain_iteration").is_none());
    assert!(row.get::<Option<String>, _>("drain_failure_code").is_none());
    assert!(row.get::<Option<i64>, _>("drain_sequence").is_none());
    assert!(row.get::<Option<Uuid>, _>("drain_outbox_id").is_none());
    assert_eq!(row.get::<i64, _>("drain_events"), 0);

    assert_eq!(row.get::<String, _>("member_lifecycle"), "CHECKPOINTED");
    assert!(!row.get::<bool, _>("slot_held"));
    assert_eq!(
        row.get::<Option<Vec<String>>, _>("return_path_nodes"),
        Some(vec![OWNER.to_owned()])
    );
    assert_eq!(
        row.get::<Option<Vec<i32>>, _>("return_path_indices"),
        Some(vec![0])
    );
    assert_eq!(
        row.get::<Option<String>, _>("return_node").as_deref(),
        Some("returned")
    );
    let member_return = row
        .get::<Option<Uuid>, _>("member_return_id")
        .expect("member return identity");
    assert_eq!(row.get::<Uuid, _>("checkpoint_return_id"), member_return);
    assert_eq!(row.get::<Uuid, _>("returned_id"), member_return);
    let expected_digest = Sha256::digest(source).to_vec();
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("member_result_digest"),
        Some(expected_digest.clone())
    );
    assert_eq!(
        row.get::<Vec<u8>, _>("checkpoint_result_digest"),
        expected_digest
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("work_output_digest"),
        row.get::<Option<Vec<u8>>, _>("member_result_digest")
    );
    let member_sequence = row
        .get::<Option<i64>, _>("checkpoint_sequence")
        .expect("member checkpoint sequence");
    assert_eq!(row.get::<i64, _>("journal_sequence"), member_sequence);
    assert_eq!(
        u64::try_from(member_sequence).expect("positive checkpoint sequence"),
        expected_sequence
    );
    assert_ne!(row.get::<Uuid, _>("checkpoint_outbox_id"), Uuid::nil());
    assert_eq!(row.get::<String, _>("work_status"), "SUCCEEDED");
    assert!(row.get::<bool, _>("work_routed"));
    assert_eq!(row.get::<String, _>("returned_status"), "SUCCEEDED");
    assert!(row.get::<bool, _>("returned_routed"));
    assert_eq!(row.get::<Vec<u8>, _>("result_content"), source);
}

async fn checkpointed_oversize() -> CheckpointFixture {
    let world = world().await;
    allow_iterations(&world, 1).await;
    let name = format!("native-checkpoint-limit-{}", Uuid::now_v7());
    let (workflow, _) = world
        .publish(collection(&name, "input.items", 1, 1, "bodyResult"))
        .await;
    let run = world.start(workflow, &json!({"items":[1]})).await;
    let sched = scope_scheduler(&world, &name).await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(sched.tick().await.collections_advanced, 1);
    assert_eq!(sched.tick().await.collections_advanced, 1);
    let source = format!("\"{}\"", "x".repeat(MAX_PAYLOAD_BYTES - 2)).into_bytes();
    let source_text = std::str::from_utf8(&source).expect("JSON text");
    assert_eq!(answer(&world, &workers, &session, source_text).await, 1);
    let runtime = world.db.runtime_pool_with(8).await;
    let before_checkpoint = identity(&world.db.pool, world.admin.tenant_id(), run).await;
    let member_path = InstancePath::root()
        .child(NodeId::parse(OWNER).expect("owner"), 0)
        .expect("member path");
    let return_instance = ExecutionInstanceId::in_path(
        member_path.clone(),
        NodeId::parse("returned").expect("return"),
    );
    // Test-only native storage fixture: public scheduling admitted the member and a public worker
    // reported the real output above. The current scheduler would reject the oversize aggregate
    // before checkpointing, so this typed command checkpoints that genuine pending Return path.
    // It does not claim that the current scheduler produced the checkpoint.
    let checkpoint = CollectionCheckpoint::from_result(
        CollectionTransition::new(
            before_checkpoint.tenant,
            before_checkpoint.namespace,
            before_checkpoint.run,
            before_checkpoint.version,
            ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner")),
            SchedulerAuthority::for_run(
                before_checkpoint.scheduler_owner.clone(),
                before_checkpoint.run,
                before_checkpoint.scheduler_generation,
            ),
            Subject::engine(),
            before_checkpoint.trace.clone(),
            now(),
            None,
        ),
        before_checkpoint.scope,
        0,
        return_instance,
        source.clone(),
        Uuid::now_v7(),
    )
    .expect("typed native checkpoint command");
    let checkpointed = native_store(runtime.clone())
        .checkpoint_collection_iteration(&checkpoint)
        .await
        .expect("checkpoint real worker result through native storage API");
    assert!(checkpointed.applied);
    let id = identity(&world.db.pool, world.admin.tenant_id(), run).await;
    assert_eq!(id.version, checkpointed.run_version);
    assert_native_checkpoint_source(&world.db.pool, &id, &source, checkpointed.run_version).await;
    CheckpointFixture {
        world,
        id,
        runtime,
        source,
        member_path,
    }
}

#[tokio::test]
async fn aggregate_requires_exact_source_owner_and_complete_checkpoint_tuple() {
    let fixture = checkpointed_oversize().await;
    let aggregate = {
        let mut value = Vec::with_capacity(fixture.source.len() + 2);
        value.push(b'[');
        value.extend_from_slice(&fixture.source);
        value.push(b']');
        value
    };
    let owner = ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner"));
    let before = mark(&fixture.world.db.pool, &fixture.id).await;

    let bad_source = data_limit_command(
        &fixture.id,
        owner.clone(),
        CollectionDataLimitEvidence::aggregate(&aggregate, &[(0, b"different".to_vec())])
            .expect("bounded mismatch evidence"),
        Uuid::now_v7(),
    );
    assert!(matches!(
        native_store(fixture.runtime.clone())
            .start_collection_drain(&bad_source)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(mark(&fixture.world.db.pool, &fixture.id).await, before);

    let wrong_owner = data_limit_command(
        &fixture.id,
        ExecutionInstanceId::root(NodeId::parse("done").expect("wrong owner")),
        CollectionDataLimitEvidence::aggregate(&aggregate, &[(0, fixture.source.clone())])
            .expect("aggregate evidence"),
        Uuid::now_v7(),
    );
    assert!(matches!(
        native_store(fixture.runtime.clone())
            .start_collection_drain(&wrong_owner)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(mark(&fixture.world.db.pool, &fixture.id).await, before);

    sqlx::query("UPDATE collection_iterations SET checkpoint_sequence=checkpoint_sequence+1 WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=0")
        .bind(fixture.id.tenant.as_uuid()).bind(fixture.id.run.as_uuid()).bind(fixture.id.scope.as_uuid())
        .execute(&fixture.world.db.pool).await.expect("isolated corrupt checkpoint tuple fixture");
    let corrupt_mark = mark(&fixture.world.db.pool, &fixture.id).await;
    let exact = data_limit_command(
        &fixture.id,
        owner.clone(),
        CollectionDataLimitEvidence::aggregate(&aggregate, &[(0, fixture.source.clone())])
            .expect("aggregate evidence"),
        Uuid::now_v7(),
    );
    assert!(matches!(
        native_store(fixture.runtime.clone())
            .start_collection_drain(&exact)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert_eq!(
        mark(&fixture.world.db.pool, &fixture.id).await,
        corrupt_mark
    );

    sqlx::query("UPDATE collection_iterations member SET checkpoint_sequence=checkpoint.journal_sequence FROM iteration_checkpoints checkpoint WHERE member.tenant_id=$1 AND member.run_id=$2 AND member.scope_id=$3 AND member.iteration_index=0 AND checkpoint.tenant_id=member.tenant_id AND checkpoint.run_id=member.run_id AND checkpoint.scope_id=member.scope_id AND checkpoint.iteration_index=member.iteration_index")
        .bind(fixture.id.tenant.as_uuid()).bind(fixture.id.run.as_uuid()).bind(fixture.id.scope.as_uuid())
        .execute(&fixture.world.db.pool).await.expect("restore genuine checkpoint tuple");
    let applied = native_store(fixture.runtime.clone())
        .start_collection_drain(&exact)
        .await
        .expect("oversize aggregate native drain");
    assert!(applied.applied);

    // A cold duplicate is identified by the durable receipt before the now-corrupt source tuple is
    // considered. This mutation is an isolated replay-order fixture, not certified execution.
    sqlx::query("UPDATE collection_iterations SET checkpoint_sequence=checkpoint_sequence+1 WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=0")
        .bind(fixture.id.tenant.as_uuid()).bind(fixture.id.run.as_uuid()).bind(fixture.id.scope.as_uuid())
        .execute(&fixture.world.db.pool).await.expect("post-commit replay source fault");
    let corrupt_replay_mark = mark(&fixture.world.db.pool, &fixture.id).await;
    let replay = native_store(fixture.runtime.clone())
        .start_collection_drain(&exact)
        .await
        .expect("cold source-independent replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, applied.run_version);
    assert_eq!(
        mark(&fixture.world.db.pool, &fixture.id).await,
        corrupt_replay_mark
    );
    sqlx::query("UPDATE collection_iterations member SET checkpoint_sequence=checkpoint.journal_sequence FROM iteration_checkpoints checkpoint WHERE member.tenant_id=$1 AND member.run_id=$2 AND member.scope_id=$3 AND member.iteration_index=0 AND checkpoint.tenant_id=member.tenant_id AND checkpoint.run_id=member.run_id AND checkpoint.scope_id=member.scope_id AND checkpoint.iteration_index=member.iteration_index")
        .bind(fixture.id.tenant.as_uuid()).bind(fixture.id.run.as_uuid()).bind(fixture.id.scope.as_uuid())
        .execute(&fixture.world.db.pool).await.expect("restore replay source tuple");

    let run_store = PgRunStore::new(fixture.runtime, cache());
    let mut path = run_store
        .begin_current_path(
            fixture.id.tenant,
            fixture.id.run,
            fixture.member_path,
            Lock::Wait,
        )
        .await
        .expect("path reader")
        .expect("member path");
    assert_eq!(
        path.snapshot()
            .state
            .global()
            .stop_code()
            .map(|code| code.as_str()),
        Some("RUN_DATA_LIMIT")
    );
    assert!(matches!(
        path.snapshot().state.ancestors(),
        [AncestorExecutionFence::Collection {
            phase: CollectionPhase::Draining,
            ..
        }]
    ));
    path.commit().await.expect("close path reader");
}

#[tokio::test]
async fn audit_outbox_and_telemetry_faults_roll_back_the_native_command() {
    let (p, id, runtime) = public_empty_at_limit().await;
    let command = data_limit_command(
        &id,
        ExecutionInstanceId::root(NodeId::parse(OWNER).expect("owner")),
        CollectionDataLimitEvidence::aggregate(b"[]", &[]).expect("empty evidence"),
        Uuid::now_v7(),
    );
    let before = mark(&p.api.db.pool, &id).await;
    for (suffix, table, predicate) in [
        (
            "audit",
            "audit_records",
            "NEW.action='run.data_limit_exceeded'",
        ),
        (
            "outbox",
            "event_outbox",
            "NEW.event_type='collection.drain.started'",
        ),
        ("telemetry", "telemetry_pending", "NEW.run_id IS NOT NULL"),
    ] {
        let function = format!(
            "reject_native_data_limit_{suffix}_{}",
            id.run.as_uuid().simple()
        );
        let trigger = format!("reject_native_data_limit_{suffix}");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF {predicate} THEN RAISE EXCEPTION 'injected native data-limit fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER {trigger} BEFORE INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION {function}()")))
            .execute(&p.api.db.pool).await.expect("install native fault");
        assert!(matches!(
            native_store(runtime.clone())
                .start_collection_drain(&command)
                .await,
            Err(PrimitiveStoreError::Storage(_))
        ));
        assert_eq!(mark(&p.api.db.pool, &id).await, before);
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER {trigger} ON {table}; DROP FUNCTION {function}()"
        )))
        .execute(&p.api.db.pool)
        .await
        .expect("remove native fault");
    }
    assert!(
        native_store(runtime)
            .start_collection_drain(&command)
            .await
            .expect("fresh retry")
            .applied
    );
}
