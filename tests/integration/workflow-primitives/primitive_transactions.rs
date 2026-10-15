//! Real PostgreSQL evidence for durable collection storage transactions.

use std::sync::Arc;

use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::execution::{DefinitionCache, RunTx, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionCheckpoint, CollectionEntry, CollectionFinish, CollectionLimits,
    CollectionScopeId, CollectionStore, CollectionTransition, PrimitiveStoreError,
    SchedulerAuthority,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{CollectionPhase, ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{MIGRATOR, PgCollectionStore, PgRunStore, migrate};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::support::engine::{chain, world};

const BEFORE: i64 = 20_261_028_000_000;

#[derive(Debug)]
struct StructuredJsonIrCodec;

impl IrCodec for StructuredJsonIrCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        JsonIrCodec.parse_structured(document)
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

fn collection_workflow(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    {
                        "id": "work",
                        "type": "activity",
                        "activity": "line.process",
                        "next": "returned"
                    },
                    {
                        "id": "returned",
                        "type": "return",
                        "result": "steps.work.output"
                    }
                ]
            }
        },
        "nodes": [
            {
                "id": "items",
                "type": "forEach",
                "items": "input.items",
                "body": "line",
                "result": "bodyResult",
                "maxItems": 1,
                "maxConcurrency": 1,
                "next": "done"
            },
            { "id": "done", "type": "end", "outputExpr": "steps.items.output" }
        ]
    })
}

async fn publish_collection_fixture(
    w: &crate::support::engine::World,
    name: &str,
) -> (WorkflowId, WorkflowVersionId, DefinitionCache) {
    let (workflow, _) = w.publish(chain(name)).await;
    let document = serde_json::to_vec(&collection_workflow(name)).expect("structured document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1, $2, $3, 2, $4, $5, $6, $7)",
    )
        .bind(version.as_uuid())
        .bind(w.admin.tenant_id().as_uuid())
        .bind(workflow.as_uuid())
        .bind(digest(&document).to_hex())
        .bind(document)
        .bind(w.admin.subject().as_str())
        .bind(crate::support::now())
        .execute(&w.db.pool)
        .await
        .expect("insert trusted structured fixture");
    (
        workflow,
        version,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
}

async fn retain_payload(pool: &sqlx::PgPool, tenant: Uuid, bytes: &[u8]) -> Digest {
    let value = digest(bytes);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(value.as_bytes().as_slice())
    .bind(bytes)
    .execute(pool)
    .await
    .expect("retain payload");
    value
}

async fn run_version(pool: &sqlx::PgPool, tenant: Uuid, run: Uuid) -> u64 {
    let value: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id = $1 AND id = $2")
            .bind(tenant)
            .bind(run)
            .fetch_one(pool)
            .await
            .expect("run version");
    u64::try_from(value).expect("nonnegative run version")
}

async fn scheduler_authority(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "primitive-test-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET \
           generation = CASE \
             WHEN owner = $2 AND expires_at > clock_timestamp() THEN generation \
             ELSE generation + 1 \
           END, \
           owner = $2, expires_at = clock_timestamp() + interval '1 hour', \
           acquired_at = clock_timestamp() \
         WHERE shard = run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("scheduler authority");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("nonnegative generation"),
    )
}

async fn set_tenant_collection_capacity(pool: &sqlx::PgPool, tenant: Uuid, limit: u32) {
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
           (tenant_id, max_active_iterations, updated_by) VALUES ($1, $2, 'test-operator') \
         ON CONFLICT (tenant_id) DO UPDATE SET \
           max_active_iterations = EXCLUDED.max_active_iterations, \
           updated_by = EXCLUDED.updated_by, updated_at = clock_timestamp()",
    )
    .bind(tenant)
    .bind(i32::try_from(limit).expect("capacity fits PostgreSQL integer"))
    .execute(pool)
    .await
    .expect("tenant collection capacity");
}

async fn primitive_run(
    w: &crate::support::engine::World,
    workflow: WorkflowId,
    workflow_version: WorkflowVersionId,
    owner: &ExecutionInstanceId,
) -> RunId {
    let run = RunId::generate();
    let tenant = w.admin.tenant_id().as_uuid();
    let input = retain_payload(&w.db.pool, tenant, b"{}").await;
    let now = crate::support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, trace_id, created_at, updated_at, root_run_id) \
         VALUES ($1, $2, $3, $4, $5, 'RUNNING', 0, $6, 'primitive-fixture', \
                 '0af7651916cd43dd8448eb211c80319c', $7, $7, $1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(workflow_version.as_uuid())
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("primitive run");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, '{}', '{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(instance_step_id(workflow_version, run, owner))
    .bind(w.namespace.id().as_uuid())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("pending collection owner");
    run
}

async fn journaled_collection_run(
    w: &crate::support::engine::World,
    workflow: WorkflowId,
    workflow_version: WorkflowVersionId,
    owner: &ExecutionInstanceId,
) -> RunId {
    let run = primitive_run(w, workflow, workflow_version, owner).await;
    sqlx::query("UPDATE runs SET run_version = 1 WHERE tenant_id = $1 AND id = $2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("initialize journal version");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1, $2, 1, 'run.started', jsonb_build_object('entrypoint', $3::text), \
                 'primitive-fixture', $4)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(crate::support::now())
    .execute(&w.db.pool)
    .await
    .expect("run start journal");
    run
}

async fn route_collection_return(
    w: &crate::support::engine::World,
    run: RunId,
    version: WorkflowVersionId,
    entry: &ExecutionInstanceId,
    returned: &ExecutionInstanceId,
    current_version: u64,
) -> u64 {
    let tenant = w.admin.tenant_id().as_uuid();
    let (path_nodes, path_indices): (Vec<_>, Vec<_>) = entry
        .path()
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).expect("iteration index"),
            )
        })
        .unzip();
    assert_eq!(entry.path(), returned.path());
    let now = crate::support::now();
    let mut tx = w.db.pool.begin().await.expect("route transaction");
    sqlx::query(
        "UPDATE steps SET status = 'SUCCEEDED', attempt = 1, generation = 1, routed = true, \
                updated_at = $7 \
         WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = $3 \
           AND instance_path_indices = $4 AND node = $5 AND step_id = $6",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(&path_nodes)
    .bind(&path_indices)
    .bind(entry.node().as_str())
    .bind(instance_step_id(version, run, entry))
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("complete scoped activity");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, $7, $8)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(returned.node().as_str())
    .bind(instance_step_id(version, run, returned))
    .bind(w.namespace.id().as_uuid())
    .bind(now)
    .bind(&path_nodes)
    .bind(&path_indices)
    .execute(&mut *tx)
    .await
    .expect("pending scoped return");
    let entry_id = instance_step_id(version, run, entry);
    for (offset, event_type, detail) in [
        (
            1_u64,
            "step.leased",
            json!({ "attempt": 1, "generation": 1 }),
        ),
        (2, "step.started", json!({ "attempt": 1, "generation": 1 })),
        (
            3,
            "step.completed",
            json!({ "attempt": 1, "generation": 1 }),
        ),
        (
            4,
            "step.routed",
            json!({ "activated": [returned.node().as_str()] }),
        ),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id, run_id, seq, event_type, node, detail, actor, occurred_at, \
              instance_id, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, $4, $5, $6, 'primitive-fixture', $7, $8, $9, $10)",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(i64::try_from(current_version + offset).expect("journal sequence"))
        .bind(event_type)
        .bind(entry.node().as_str())
        .bind(detail)
        .bind(now)
        .bind(entry_id)
        .bind(&path_nodes)
        .bind(&path_indices)
        .execute(&mut *tx)
        .await
        .expect("scoped activity journal");
    }
    let routed_version = current_version + 4;
    sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = $4 WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(i64::try_from(routed_version).expect("run version"))
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("advance routed version");
    tx.commit().await.expect("commit route fixture");
    routed_version
}

fn transition(
    w: &crate::support::engine::World,
    run: RunId,
    version: u64,
    owner: ExecutionInstanceId,
    scheduler: SchedulerAuthority,
) -> CollectionTransition {
    CollectionTransition::new(
        w.admin.tenant_id(),
        w.namespace.id(),
        run,
        version,
        owner,
        scheduler,
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
        None,
    )
}

fn transition_with_audit(
    w: &crate::support::engine::World,
    run: RunId,
    version: u64,
    owner: ExecutionInstanceId,
    scheduler: SchedulerAuthority,
    audit: AuditRecord,
) -> CollectionTransition {
    CollectionTransition::new(
        w.admin.tenant_id(),
        w.namespace.id(),
        run,
        version,
        owner,
        scheduler,
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
        Some(audit),
    )
}

#[tokio::test]
async fn populated_schema_upgrades_and_backfills_root_usage() {
    let w = world().await;
    let (workflow, workflow_version) = w.publish(chain("collection-migration")).await;
    let run = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("remove collection migration from populated schema");
    let absent: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'iteration_scopes')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("table lookup");
    assert!(absent);

    migrate(&w.db.pool).await.expect("upgrade populated schema");
    let admitted: i32 = sqlx::query_scalar(
        "SELECT admitted_instances FROM run_instance_usage \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("backfilled usage");
    let historically_chargeable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = $2 \
         AND status IN ('READY', 'LEASED', 'RUNNING', 'WAITING', 'RETRY_WAIT', 'SUCCEEDED', 'FAILED')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("historic activated projection count");
    assert_eq!(i64::from(admitted), historically_chargeable);

    let pending_node = NodeId::parse("future").expect("pending node");
    let pending_instance = ExecutionInstanceId::root(pending_node.clone());
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', clock_timestamp(), '{}', '{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(pending_node.as_str())
    .bind(instance_step_id(workflow_version, run, &pending_instance))
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("pending root instance");
    sqlx::query(
        "UPDATE steps SET status = 'READY', ready_at = clock_timestamp(), \
           updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(pending_node.as_str())
    .execute(&w.db.pool)
    .await
    .expect("activate root instance");
    let charged: i32 = sqlx::query_scalar(
        "SELECT admitted_instances FROM run_instance_usage \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("charged root instance");
    assert_eq!(charged, admitted + 1);
    sqlx::query(
        "UPDATE steps SET status = 'READY', updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(pending_node.as_str())
    .execute(&w.db.pool)
    .await
    .expect("idempotent root update");
    let unchanged: i32 = sqlx::query_scalar(
        "SELECT admitted_instances FROM run_instance_usage \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("unchanged root charge");
    assert_eq!(unchanged, charged);

    let blocked_node = NodeId::parse("blocked").expect("blocked node");
    let blocked_instance = ExecutionInstanceId::root(blocked_node.clone());
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', clock_timestamp(), '{}', '{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(blocked_node.as_str())
    .bind(instance_step_id(workflow_version, run, &blocked_instance))
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("budget-blocked root instance");
    sqlx::query(
        "UPDATE run_instance_usage SET max_instances = admitted_instances + reserved_instances \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("exhaust root budget");
    sqlx::query(
        "UPDATE steps SET status = 'READY', ready_at = clock_timestamp(), \
           updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(blocked_node.as_str())
    .execute(&w.db.pool)
    .await
    .expect_err("exhausted lifetime budget must reject a root activation");
    let blocked_status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(blocked_node.as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("blocked root status");
    assert_eq!(blocked_status, "PENDING");

    let cancelled_node = NodeId::parse("cancelled").expect("cancelled node");
    let cancelled_instance = ExecutionInstanceId::root(cancelled_node.clone());
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', clock_timestamp(), '{}', '{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(cancelled_node.as_str())
    .bind(instance_step_id(workflow_version, run, &cancelled_instance))
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("pending cancellation fixture");
    sqlx::query(
        "UPDATE steps SET status = 'CANCELLED', updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(cancelled_node.as_str())
    .execute(&w.db.pool)
    .await
    .expect("pending cancellation bypasses forward admission budget");
    let after_cancel: i32 = sqlx::query_scalar(
        "SELECT admitted_instances FROM run_instance_usage \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("usage after cancellation");
    assert_eq!(after_cancel, charged);
}

#[tokio::test]
async fn populated_scope_prevents_destructive_migration_rollback() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let (workflow, workflow_version) = w.publish(chain("collection-rollback-guard")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    PgCollectionStore::new(w.db.pool.clone())
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner, authority),
                CollectionScopeId::generate(),
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");

    let error = MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect_err("retained scope must prevent destructive rollback");
    assert!(
        error
            .to_string()
            .contains("collection progress is retained")
    );
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM iteration_scopes")
        .fetch_one(&w.db.pool)
        .await
        .expect("retained scopes");
    assert_eq!(retained, 1);
}

#[tokio::test]
async fn collection_payloads_require_same_tenant_and_matching_content_digests() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let (workflow, workflow_version) = w.publish(chain("collection-snapshot-tenant")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let foreign_snapshot = retain_payload(
        &w.db.pool,
        w.other_admin.tenant_id().as_uuid(),
        b"foreign collection snapshot",
    )
    .await;
    let error = PgCollectionStore::new(w.db.pool.clone())
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                CollectionScopeId::generate(),
                1,
                foreign_snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect_err("foreign payload provenance must be rejected");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let state: (String, i64) = sqlx::query_as(
        "SELECT s.status, \
           (SELECT count(*) FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2) \
         FROM steps s WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'b'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rolled-back owner and scope");
    assert_eq!(state, ("PENDING".into(), 0));

    let wrong_count_snapshot = retain_payload(&w.db.pool, tenant, b"[1,2]").await;
    let error = PgCollectionStore::new(w.db.pool.clone())
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                CollectionScopeId::generate(),
                1,
                wrong_count_snapshot,
                CollectionLimits::new(2, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect_err("snapshot cardinality must equal the pinned item count");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);

    let corrupted_snapshot = digest(b"[9]");
    sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, '[8]')")
        .bind(tenant)
        .bind(corrupted_snapshot.as_bytes().as_slice())
        .execute(&w.db.pool)
        .await
        .expect("corrupt snapshot payload fixture");
    let error = PgCollectionStore::new(w.db.pool.clone())
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                CollectionScopeId::generate(),
                1,
                corrupted_snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect_err("snapshot content must match its digest");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);

    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let local_snapshot = retain_payload(&w.db.pool, tenant, b"[2]").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                scope,
                1,
                local_snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner.node().clone(), 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let corrupted_item = digest(b"2");
    sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, '3')")
        .bind(tenant)
        .bind(corrupted_item.as_bytes().as_slice())
        .execute(&w.db.pool)
        .await
        .expect("corrupt item payload fixture");
    let error = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(
                    &w,
                    run,
                    entered.run_version,
                    owner.clone(),
                    authority.clone(),
                ),
                scope,
                0,
                member.clone(),
                ActivityName::parse("line.process").expect("activity"),
                corrupted_item,
                1,
                Uuid::now_v7(),
            )
            .expect("admission"),
        )
        .await
        .expect_err("item content must match its captured digest");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let error = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&w, run, entered.run_version, owner, authority),
                scope,
                0,
                member,
                ActivityName::parse("line.process").expect("activity"),
                foreign_snapshot,
                1,
                Uuid::now_v7(),
            )
            .expect("admission"),
        )
        .await
        .expect_err("foreign item provenance must be rejected");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let counts: (i64, i32) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2), \
           (SELECT active_count FROM iteration_scopes \
            WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rolled-back item admission");
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn collection_admission_writes_expected_journal_projection_and_usage_atomically() {
    let w = world().await;
    let owner_node = NodeId::parse("b").expect("owner node");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, workflow_version) = w.publish(chain("collection-transaction")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let member_path = InstancePath::root()
        .child(owner_node, 0)
        .expect("member path");
    let activity_instance = ExecutionInstanceId::in_path(
        member_path.clone(),
        NodeId::parse("work").expect("activity node"),
    );
    let admission_outbox = Uuid::now_v7();
    let admitted = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(
                    &w,
                    run,
                    entered.run_version,
                    owner.clone(),
                    authority.clone(),
                ),
                scope,
                0,
                activity_instance.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                admission_outbox,
            )
            .expect("admission"),
        )
        .await
        .expect("admit activity");

    let events: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, detail FROM run_events WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("collection journal");
    assert_eq!(
        events
            .iter()
            .map(|(kind, _)| kind.as_str())
            .collect::<Vec<_>>(),
        [
            "collection.entered",
            "collection.iteration_admitted",
            "step.released",
        ]
    );
    assert_eq!(events[0].1["snapshot_digest"], snapshot.to_hex());
    assert_eq!(events[1].1["entry"]["node"], "work");
    assert_eq!(events[1].1["entry"]["path"][0]["scope"], "b");
    assert_eq!(events[1].1["entry"]["path"][0]["index"], 0);
    assert_eq!(events[2].1, json!({}));

    let scope_row = sqlx::query(
        "SELECT lifecycle, admitted_count, completed_count, active_count \
         FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("scope projection");
    assert_eq!(scope_row.get::<String, _>("lifecycle"), "OPEN");
    assert_eq!(scope_row.get::<i32, _>("admitted_count"), 1);
    assert_eq!(scope_row.get::<i32, _>("completed_count"), 0);
    assert_eq!(scope_row.get::<i32, _>("active_count"), 1);
    let active: i32 = sqlx::query_scalar(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(&w.db.pool)
    .await
    .expect("tenant usage");
    assert_eq!(active, 1);
    let projected_status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id = $1 AND run_id = $2 \
         AND instance_path_nodes = ARRAY['b'] AND instance_path_indices = ARRAY[0] \
         AND node = 'work'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("scoped activity projection");
    assert_eq!(projected_status, "READY");
    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        admitted.run_version
    );

    let unsupported_return =
        ExecutionInstanceId::in_path(member_path, NodeId::parse("return").expect("return node"));
    let checkpoint_error = store
        .checkpoint_collection_iteration(
            &CollectionCheckpoint::new(
                transition(&w, run, admitted.run_version, owner, authority),
                scope,
                0,
                unsupported_return,
                digest(b"2"),
                Uuid::now_v7(),
            )
            .expect("checkpoint request"),
        )
        .await
        .expect_err("checkpoint stays disabled until body routing replays");
    assert_eq!(checkpoint_error, PrimitiveStoreError::RuntimeNotIntegrated);
}

#[tokio::test]
async fn checkpoint_and_finish_are_atomic_replayable_and_restart_safe() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner node"));
    let (workflow, workflow_version, definitions) =
        publish_collection_fixture(&w, "collection-checkpoint").await;
    let run = journaled_collection_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let verifier = PgRunStore::new(w.db.pool.clone(), definitions.clone());
    let store = PgCollectionStore::with_definition_cache(w.db.pool.clone(), definitions);
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, br#"[1]"#).await;
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 1, owner.clone(), authority.clone()),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let member_path = InstancePath::root()
        .child(owner.node().clone(), 0)
        .expect("member path");
    let entry = ExecutionInstanceId::in_path(
        member_path.clone(),
        NodeId::parse("work").expect("activity node"),
    );
    let returned =
        ExecutionInstanceId::in_path(member_path, NodeId::parse("returned").expect("return node"));
    let wrong_item = retain_payload(&w.db.pool, tenant, b"2").await;
    let error = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(
                    &w,
                    run,
                    entered.run_version,
                    owner.clone(),
                    authority.clone(),
                ),
                scope,
                0,
                entry.clone(),
                ActivityName::parse("line.process").expect("activity"),
                wrong_item,
                1,
                Uuid::now_v7(),
            )
            .expect("mismatched admission"),
        )
        .await
        .expect_err("admission item must equal the captured snapshot member");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        entered.run_version
    );
    let admission_outbox = Uuid::now_v7();
    let admitted = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(
                    &w,
                    run,
                    entered.run_version,
                    owner.clone(),
                    authority.clone(),
                ),
                scope,
                0,
                entry.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                admission_outbox,
            )
            .expect("admission"),
        )
        .await
        .expect("admit iteration");
    let routed_version = route_collection_return(
        &w,
        run,
        workflow_version,
        &entry,
        &returned,
        admitted.run_version,
    )
    .await;

    let missing_result = digest(b"missing-result");
    let error = store
        .checkpoint_collection_iteration(
            &CollectionCheckpoint::new(
                transition(&w, run, routed_version, owner.clone(), authority.clone()),
                scope,
                0,
                returned.clone(),
                missing_result,
                Uuid::now_v7(),
            )
            .expect("missing-payload checkpoint"),
        )
        .await
        .expect_err("unretained result must fail closed");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        routed_version
    );

    let result = retain_payload(&w.db.pool, tenant, br#""member-result""#).await;
    let stale =
        SchedulerAuthority::for_run(authority.owner(), run, authority.lease().generation + 1);
    let error = store
        .checkpoint_collection_iteration(
            &CollectionCheckpoint::new(
                transition(&w, run, routed_version, owner.clone(), stale),
                scope,
                0,
                returned.clone(),
                result,
                Uuid::now_v7(),
            )
            .expect("stale checkpoint"),
        )
        .await
        .expect_err("stale checkpoint fence");
    assert_eq!(error, PrimitiveStoreError::SchedulerFenceLost);

    let conflicting_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1, $2, $3, 'run.finished', '{}'::jsonb)",
    )
    .bind(conflicting_outbox)
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("conflicting checkpoint outbox");
    let error = store
        .checkpoint_collection_iteration(
            &CollectionCheckpoint::new(
                transition(&w, run, routed_version, owner.clone(), authority.clone()),
                scope,
                0,
                returned.clone(),
                result,
                conflicting_outbox,
            )
            .expect("checkpoint with duplicate outbox"),
        )
        .await
        .expect_err("outbox failure must roll back checkpoint");
    assert!(matches!(error, PrimitiveStoreError::Storage(_)));
    let before_checkpoint: (String, bool, i32, i32, i32, String) = sqlx::query_as(
        "SELECT ci.lifecycle, ci.slot_held, s.active_count, t.active_iterations, \
                u.reserved_instances, r.status \
         FROM collection_iterations ci \
         JOIN iteration_scopes s USING (tenant_id, run_id, scope_id) \
         JOIN tenant_iteration_usage t USING (tenant_id) \
         JOIN run_instance_usage u USING (tenant_id, run_id) \
         JOIN steps r ON r.tenant_id = ci.tenant_id AND r.run_id = ci.run_id \
           AND r.instance_path_nodes = ci.entry_path_nodes \
           AND r.instance_path_indices = ci.entry_path_indices AND r.node = 'returned' \
         WHERE ci.tenant_id = $1 AND ci.run_id = $2 AND ci.scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("checkpoint rollback state");
    assert_eq!(
        before_checkpoint,
        ("ADMITTED".into(), true, 1, 1, 1, "PENDING".into())
    );
    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        routed_version
    );

    let duplicate_audit = AuditRecord::by(
        &w.admin,
        AuditAction::RunStarted,
        ResourceType::Run,
        run.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
    );
    sqlx::query(
        "INSERT INTO audit_records \
         (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(duplicate_audit.id().as_uuid())
    .bind(duplicate_audit.tenant_id().as_uuid())
    .bind(duplicate_audit.actor().as_str())
    .bind(duplicate_audit.action().as_str())
    .bind(duplicate_audit.resource_type().as_str())
    .bind(duplicate_audit.resource_id())
    .bind(duplicate_audit.reason())
    .bind(duplicate_audit.trace_id())
    .bind(duplicate_audit.created_at())
    .execute(&w.db.pool)
    .await
    .expect("existing audit fixture");
    let error = store
        .checkpoint_collection_iteration(
            &CollectionCheckpoint::new(
                transition_with_audit(
                    &w,
                    run,
                    routed_version,
                    owner.clone(),
                    authority.clone(),
                    duplicate_audit,
                ),
                scope,
                0,
                returned.clone(),
                result,
                Uuid::now_v7(),
            )
            .expect("checkpoint with duplicate audit"),
        )
        .await
        .expect_err("audit failure must roll back checkpoint");
    assert!(matches!(error, PrimitiveStoreError::Storage(_)));
    let after_audit_failure: (String, bool, i32, i32, i64) = sqlx::query_as(
        "SELECT ci.lifecycle, ci.slot_held, s.active_count, t.active_iterations, \
                (SELECT count(*) FROM iteration_checkpoints c \
                 WHERE c.tenant_id = ci.tenant_id AND c.run_id = ci.run_id \
                   AND c.scope_id = ci.scope_id) \
         FROM collection_iterations ci \
         JOIN iteration_scopes s USING (tenant_id, run_id, scope_id) \
         JOIN tenant_iteration_usage t USING (tenant_id) \
         WHERE ci.tenant_id = $1 AND ci.run_id = $2 AND ci.scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit rollback state");
    assert_eq!(after_audit_failure, ("ADMITTED".into(), true, 1, 1, 0));
    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        routed_version
    );

    let checkpoint_outbox = Uuid::now_v7();
    let checkpoint_request = CollectionCheckpoint::new(
        transition(&w, run, routed_version, owner.clone(), authority.clone()),
        scope,
        0,
        returned,
        result,
        checkpoint_outbox,
    )
    .expect("checkpoint");
    let checkpointed = store
        .checkpoint_collection_iteration(&checkpoint_request)
        .await
        .expect("checkpoint iteration");
    assert!(checkpointed.applied);
    assert_eq!(checkpointed.run_version, routed_version + 2);
    let stale_checkpoint_replay = CollectionCheckpoint::new(
        transition(
            &w,
            run,
            routed_version,
            owner.clone(),
            SchedulerAuthority::for_run(authority.owner(), run, authority.lease().generation + 1),
        ),
        scope,
        0,
        checkpoint_request.return_instance.clone(),
        result,
        checkpoint_outbox,
    )
    .expect("stale checkpoint replay");
    assert_eq!(
        store
            .checkpoint_collection_iteration(&stale_checkpoint_replay)
            .await
            .expect_err("replay still requires the scheduler fence"),
        PrimitiveStoreError::SchedulerFenceLost
    );
    let replayed = store
        .checkpoint_collection_iteration(&checkpoint_request)
        .await
        .expect("replay checkpoint");
    assert!(!replayed.applied);
    assert_eq!(replayed.run_version, checkpointed.run_version);
    let checkpoint_state: (String, bool, i32, i32, i32, String, bool) = sqlx::query_as(
        "SELECT ci.lifecycle, ci.slot_held, s.completed_count, t.active_iterations, \
                u.reserved_instances, r.status, r.routed \
         FROM collection_iterations ci \
         JOIN iteration_scopes s USING (tenant_id, run_id, scope_id) \
         JOIN tenant_iteration_usage t USING (tenant_id) \
         JOIN run_instance_usage u USING (tenant_id, run_id) \
         JOIN steps r ON r.tenant_id = ci.tenant_id AND r.run_id = ci.run_id \
           AND r.instance_path_nodes = ci.return_path_nodes \
           AND r.instance_path_indices = ci.return_path_indices AND r.node = ci.return_node \
         WHERE ci.tenant_id = $1 AND ci.run_id = $2 AND ci.scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("checkpoint state");
    assert_eq!(
        checkpoint_state,
        (
            "CHECKPOINTED".into(),
            false,
            1,
            0,
            0,
            "SUCCEEDED".into(),
            true
        )
    );
    let replayed_admission = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(
                    &w,
                    run,
                    entered.run_version,
                    owner.clone(),
                    authority.clone(),
                ),
                scope,
                0,
                entry.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                admission_outbox,
            )
            .expect("replayed admission"),
        )
        .await
        .expect("admission replay after checkpoint");
    assert!(!replayed_admission.applied);
    assert_eq!(replayed_admission.run_version, checkpointed.run_version);
    let mut reopened = verifier
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("reopen checkpointed run")
        .expect("checkpointed run exists");
    let state = reopened.parts().1;
    let reopened_scope = state.collection(&owner).expect("reopened collection");
    assert_eq!(reopened_scope.phase(), CollectionPhase::Open);
    assert_eq!(reopened_scope.active_count(), 0);
    assert_eq!(
        reopened_scope
            .ordered_results()
            .map(|(index, value)| (index, value.digest()))
            .collect::<Vec<_>>(),
        [(0, result)]
    );
    drop(reopened);
    verifier
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("checkpoint journal and projection agree after restart");

    let missing_aggregate = digest(b"missing-aggregate");
    let error = store
        .finish_collection(&CollectionFinish::new(
            transition(
                &w,
                run,
                checkpointed.run_version,
                owner.clone(),
                authority.clone(),
            ),
            scope,
            missing_aggregate,
            Uuid::now_v7(),
        ))
        .await
        .expect_err("unretained aggregate must fail closed");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let wrong_aggregate = retain_payload(&w.db.pool, tenant, br#"["wrong"]"#).await;
    let error = store
        .finish_collection(&CollectionFinish::new(
            transition(
                &w,
                run,
                checkpointed.run_version,
                owner.clone(),
                authority.clone(),
            ),
            scope,
            wrong_aggregate,
            Uuid::now_v7(),
        ))
        .await
        .expect_err("aggregate order and values must match checkpoints");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let aggregate = retain_payload(&w.db.pool, tenant, br#"["member-result"]"#).await;
    let finish_outbox = Uuid::now_v7();
    let finish_request = CollectionFinish::new(
        transition(
            &w,
            run,
            checkpointed.run_version,
            owner.clone(),
            authority.clone(),
        ),
        scope,
        aggregate,
        finish_outbox,
    );
    let finished = store
        .finish_collection(&finish_request)
        .await
        .expect("finish collection");
    assert!(finished.applied);
    let stale_finish_replay = CollectionFinish::new(
        transition(
            &w,
            run,
            checkpointed.run_version,
            owner.clone(),
            SchedulerAuthority::for_run(authority.owner(), run, authority.lease().generation + 1),
        ),
        scope,
        aggregate,
        finish_outbox,
    );
    assert_eq!(
        store
            .finish_collection(&stale_finish_replay)
            .await
            .expect_err("finish replay still requires the scheduler fence"),
        PrimitiveStoreError::SchedulerFenceLost
    );
    let replayed = store
        .finish_collection(&finish_request)
        .await
        .expect("replay finish");
    assert!(!replayed.applied);
    assert_eq!(replayed.run_version, finished.run_version);

    let final_scope: (String, Vec<u8>, Option<i64>, Option<Uuid>) = sqlx::query_as(
        "SELECT lifecycle, result_digest, finish_sequence, finish_outbox_id \
         FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("finished collection scope");
    assert_eq!(final_scope.0, "SUCCEEDED");
    assert_eq!(final_scope.1, aggregate.as_bytes());
    assert_eq!(
        final_scope.2,
        Some(i64::try_from(finished.run_version - 1).expect("version"))
    );
    assert_eq!(final_scope.3, Some(finish_outbox));
    let owner_state: (String, bool, Option<String>) = sqlx::query_as(
        "SELECT status, routed, waiting_kind FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 \
           AND instance_path_nodes = '{}' AND instance_path_indices = '{}' AND node = 'items'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("finished collection owner");
    assert_eq!(owner_state, ("SUCCEEDED".into(), true, None));
    let successor: (String, bool) = sqlx::query_as(
        "SELECT status, routed FROM steps WHERE tenant_id = $1 AND run_id = $2 \
           AND instance_path_nodes = '{}' AND instance_path_indices = '{}' AND node = 'done'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("collection successor");
    assert_eq!(successor, ("PENDING".into(), false));
    let hints: (i64, i64) = sqlx::query_as(
        "SELECT \
           count(*) FILTER (WHERE event_type = 'task.ready'), \
           count(*) FILTER (WHERE event_type = 'collection.finished') \
         FROM event_outbox WHERE tenant_id = $1 AND aggregate_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("collection outbox");
    assert_eq!(hints, (1, 1));
    let mut reopened = verifier
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("reopen finished run")
        .expect("finished run exists");
    let state = reopened.parts().1;
    let reopened_scope = state
        .collection(&owner)
        .expect("reopened terminal collection");
    assert_eq!(reopened_scope.phase(), CollectionPhase::Succeeded);
    assert_eq!(reopened_scope.result_digest(), Some(aggregate));
    drop(reopened);
    let terminal_events: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM run_events WHERE tenant_id = $1 AND run_id = $2 \
           AND event_type LIKE 'collection.%' ORDER BY seq",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("collection journal");
    assert_eq!(
        terminal_events,
        [
            "collection.entered",
            "collection.iteration_admitted",
            "collection.iteration_returned",
            "collection.iteration_checkpointed",
            "collection.finished",
        ]
    );
    verifier
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("terminal journal and projection agree after restart");
    sqlx::query(
        "UPDATE iteration_scopes SET result_digest = $4 \
         WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .bind(wrong_aggregate.as_bytes().as_slice())
    .execute(&w.db.pool)
    .await
    .expect("corrupt aggregate projection");
    let error = verifier
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect_err("aggregate projection corruption must be visible after restart");
    assert!(
        error.contains("projection differs from the journal"),
        "{error}"
    );
}

#[tokio::test]
async fn empty_collection_finishes_without_admission_or_capacity_charge() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner node"));
    let (workflow, workflow_version, definitions) =
        publish_collection_fixture(&w, "empty-collection").await;
    let run = journaled_collection_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let verifier = PgRunStore::new(w.db.pool.clone(), definitions.clone());
    let store = PgCollectionStore::with_definition_cache(w.db.pool.clone(), definitions);
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[]").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 1, owner.clone(), authority.clone()),
                scope,
                0,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("empty entry"),
        )
        .await
        .expect("enter empty collection");
    let aggregate = retain_payload(&w.db.pool, tenant, b"[]").await;
    let finished = store
        .finish_collection(&CollectionFinish::new(
            transition(&w, run, entered.run_version, owner, authority),
            scope,
            aggregate,
            Uuid::now_v7(),
        ))
        .await
        .expect("finish empty collection");
    assert!(finished.applied);
    let active: i32 = sqlx::query_scalar(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(&w.db.pool)
    .await
    .expect("tenant iteration usage");
    assert_eq!(active, 0);
    verifier
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("empty terminal journal and projection agree after restart");
}

#[tokio::test]
async fn failed_outbox_insert_rolls_back_admission_and_every_counter() {
    let w = world().await;
    let owner_node = NodeId::parse("b").expect("owner node");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, workflow_version) = w.publish(chain("collection-rollback")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let duplicate_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1, $2, $3, 'run.finished', '{}'::jsonb)",
    )
    .bind(duplicate_outbox)
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("conflicting outbox fixture");
    let entry_instance = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner_node, 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let error = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&w, run, entered.run_version, owner, authority),
                scope,
                0,
                entry_instance,
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                duplicate_outbox,
            )
            .expect("admission"),
        )
        .await
        .expect_err("duplicate outbox must abort the transaction");
    assert!(matches!(error, PrimitiveStoreError::Storage(_)));

    assert_eq!(
        run_version(&w.db.pool, tenant, run.as_uuid()).await,
        entered.run_version
    );
    let counts: (i64, i32, i32) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2), \
           (SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1), \
           (SELECT active_count FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(scope.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rolled-back projections");
    assert_eq!(counts, (0, 0, 0));
}

#[tokio::test]
async fn admission_owner_must_match_the_stored_scope_owner() {
    let w = world().await;
    let stored_owner_node = NodeId::parse("b").expect("owner node");
    let stored_owner = ExecutionInstanceId::root(stored_owner_node);
    let (workflow, workflow_version) = w.publish(chain("collection-owner-match")).await;
    let run = primitive_run(&w, workflow, workflow_version, &stored_owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, stored_owner, authority.clone()),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");

    let supplied_owner_node = NodeId::parse("other").expect("different owner node");
    let supplied_owner = ExecutionInstanceId::root(supplied_owner_node.clone());
    let supplied_entry = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(supplied_owner_node, 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let error = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&w, run, entered.run_version, supplied_owner, authority),
                scope,
                0,
                supplied_entry,
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                Uuid::now_v7(),
            )
            .expect("admission"),
        )
        .await
        .expect_err("a different owner cannot admit into the scope");
    assert_eq!(error, PrimitiveStoreError::InvalidTransition);
    let iterations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("collection iterations");
    assert_eq!(iterations, 0);
}

#[tokio::test]
async fn stale_scheduler_fence_rejects_entry_and_admission() {
    let w = world().await;
    let owner_node = NodeId::parse("b").expect("owner node");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, workflow_version) = w.publish(chain("collection-scheduler-fence")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    let stale = SchedulerAuthority::for_run(
        authority.owner(),
        run,
        authority.lease().generation.saturating_add(1),
    );
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let entry = CollectionEntry::new(
        transition(&w, run, 0, owner.clone(), stale),
        scope,
        1,
        snapshot,
        CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
    )
    .expect("entry");
    assert_eq!(
        store
            .enter_collection(&entry)
            .await
            .expect_err("stale entry fence"),
        PrimitiveStoreError::SchedulerFenceLost
    );
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 0, owner.clone(), authority.clone()),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter with current fence");
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replacement-scheduler', \
           generation = generation + 1, expires_at = clock_timestamp() + interval '1 hour', \
           acquired_at = clock_timestamp() WHERE shard = run_shard($1)",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("scheduler takeover");
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner_node, 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let admission = CollectionAdmission::new(
        transition(&w, run, entered.run_version, owner, authority),
        scope,
        0,
        member,
        ActivityName::parse("line.process").expect("activity"),
        item,
        1,
        Uuid::now_v7(),
    )
    .expect("admission");
    assert_eq!(
        store
            .admit_collection_activity(&admission)
            .await
            .expect_err("takeover fences stale admission"),
        PrimitiveStoreError::SchedulerFenceLost
    );
    let iterations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("no stale admission");
    assert_eq!(iterations, 0);
}

#[tokio::test]
async fn missing_tenant_policy_fails_closed() {
    let w = world().await;
    let owner_node = NodeId::parse("b").expect("owner node");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, workflow_version) = w.publish(chain("collection-policy-required")).await;
    let run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let authority = scheduler_authority(&w.db.pool, run).await;
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    let entry = CollectionEntry::new(
        transition(&w, run, 0, owner.clone(), authority.clone()),
        scope,
        1,
        snapshot,
        CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
    )
    .expect("entry");
    assert_eq!(
        store
            .enter_collection(&entry)
            .await
            .expect_err("missing policy closes entry"),
        PrimitiveStoreError::PolicyMissing
    );
    let unchanged: (String, i64, i64) = sqlx::query_as(
        "SELECT s.status, r.run_version, \
           (SELECT count(*) FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2) \
         FROM steps s JOIN runs r ON r.tenant_id = s.tenant_id AND r.id = s.run_id \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'b'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("entry rollback state");
    assert_eq!(unchanged, ("PENDING".into(), 0, 0));

    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let entered = store
        .enter_collection(&entry)
        .await
        .expect("enter collection");
    sqlx::query("DELETE FROM tenant_collection_policies WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&w.db.pool)
        .await
        .expect("remove tenant policy");
    let replayed = store
        .enter_collection(&entry)
        .await
        .expect("accepted entry replay does not reacquire capacity");
    assert!(!replayed.applied);
    assert_eq!(replayed.run_version, entered.run_version);
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner_node, 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let admission = CollectionAdmission::new(
        transition(&w, run, entered.run_version, owner, authority),
        scope,
        0,
        member,
        ActivityName::parse("line.process").expect("activity"),
        item,
        1,
        Uuid::now_v7(),
    )
    .expect("admission");
    assert_eq!(
        store
            .admit_collection_activity(&admission)
            .await
            .expect_err("missing policy closes admission"),
        PrimitiveStoreError::PolicyMissing
    );
}

#[tokio::test]
async fn persisted_tenant_capacity_serializes_admission_across_runs() {
    let w = world().await;
    let owner_node = NodeId::parse("b").expect("owner node");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, workflow_version) = w.publish(chain("collection-tenant-contention")).await;
    let first_run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let second_run = primitive_run(&w, workflow, workflow_version, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let first_authority = scheduler_authority(&w.db.pool, first_run).await;
    let second_authority = scheduler_authority(&w.db.pool, second_run).await;
    set_tenant_collection_capacity(&w.db.pool, tenant, 1).await;
    let store = PgCollectionStore::new(w.db.pool.clone());
    let first_scope = CollectionScopeId::generate();
    let second_scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let first_entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, first_run, 0, owner.clone(), first_authority.clone()),
                first_scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("first entry"),
        )
        .await
        .expect("enter first collection");
    let second_entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, second_run, 0, owner.clone(), second_authority.clone()),
                second_scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("second entry"),
        )
        .await
        .expect("enter second collection");
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let first_member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner_node.clone(), 0)
            .expect("first member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let second_member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner_node, 0)
            .expect("second member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let first = CollectionAdmission::new(
        transition(
            &w,
            first_run,
            first_entered.run_version,
            owner.clone(),
            first_authority,
        ),
        first_scope,
        0,
        first_member,
        ActivityName::parse("line.process").expect("activity"),
        item,
        1,
        Uuid::now_v7(),
    )
    .expect("first admission");
    let second = CollectionAdmission::new(
        transition(
            &w,
            second_run,
            second_entered.run_version,
            owner,
            second_authority,
        ),
        second_scope,
        0,
        second_member,
        ActivityName::parse("line.process").expect("activity"),
        item,
        1,
        Uuid::now_v7(),
    )
    .expect("second admission");

    let (first_result, second_result) = tokio::join!(
        store.admit_collection_activity(&first),
        store.admit_collection_activity(&second)
    );
    let results = [first_result, second_result];
    let applied = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(applied, 1);
    let refused = results
        .iter()
        .filter(|result| matches!(result, Err(PrimitiveStoreError::AtCapacity)))
        .count();
    assert_eq!(refused, 1);
    let persisted: (i32, i64) = sqlx::query_as(
        "SELECT \
           (SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1), \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id = $1)",
    )
    .bind(tenant)
    .fetch_one(&w.db.pool)
    .await
    .expect("tenant contention result");
    assert_eq!(persisted, (1, 1));
}
