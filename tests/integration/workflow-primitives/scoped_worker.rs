//! Real PostgreSQL evidence for scoped worker leases and reports.

use std::{sync::Arc, time::Duration};

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_application::primitives::{
    CollectionAdmission, CollectionDrain, CollectionDrainFinish, CollectionEntry,
    CollectionIterationDrain, CollectionLimits, CollectionScopeId, CollectionStore,
    CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::runs::{RunOperation, RunService, RunStore};
use sagat_application::shards::Lease;
use sagat_application::telemetry::all_attempts;
use sagat_application::worker::{
    Capabilities, ClaimedTask, TaskRef, WorkerService, WorkerSettings,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{CollectionDrainReason, ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::identity::Subject;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgCollectionStore, PgRunStore};
use sagat_scheduler::{
    CollectionAdvance, Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::support::engine::{Session, World, chain, ctx, task_ref, world};
use crate::support::now;

type ScopedWorkers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

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

fn structured_workflow(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "limits": { "maxInstances": 20, "maxDepth": 2 },
        "bodies": {
            "line": {
                "entrypoint": "a",
                "nodes": [
                    {
                        "id": "a", "type": "activity", "activity": "scoped.a", "next": "b",
                        "idempotencyTemplate": "scope:${item.sku}:${index}",
                        "secrets": ["scoped-key"],
                        "retry": {
                            "maxAttempts": 2, "initialDelayMs": 100, "maxDelayMs": 100,
                            "strategy": "fixed", "jitter": false
                        }
                    },
                    { "id": "b", "type": "activity", "activity": "scoped.b", "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.b.output" }
                ]
            }
        },
        "nodes": [
            {
                "id": "items", "type": "forEach", "items": "input.items", "body": "line",
                "result": "bodyResult", "maxItems": 2, "maxConcurrency": 2, "next": "a"
            },
            {
                "id": "a", "type": "activity", "activity": "act.a", "secrets": ["root-key"],
                "next": "done"
            },
            { "id": "done", "type": "end" }
        ]
    })
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

struct Fixture {
    world: World,
    store: PgRunStore,
    workers: ScopedWorkers,
    session: Session,
    run: RunId,
    placement: Placement,
}

async fn register_scoped(world: &World, workers: &ScopedWorkers) -> Session {
    let registered = workers
        .register(
            &world.worker,
            "payments",
            &format!("scoped-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec!["scoped.a".into(), "scoped.b".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 8,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register scoped worker");
    assert_eq!(registered.accepted_protocol_version, "1.5");
    Session {
        principal: world.worker.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    }
}

fn collection_transition(
    world: &World,
    run: RunId,
    version: u64,
    owner: ExecutionInstanceId,
    lease: Lease,
) -> CollectionTransition {
    CollectionTransition::new(
        world.admin.tenant_id(),
        world.namespace.id(),
        run,
        version,
        owner,
        SchedulerAuthority::new("scoped-worker-test", lease),
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        now(),
        None,
    )
}

async fn fixture_with_progress(name: &str, prepare_collection: bool) -> Fixture {
    let world = world().await;
    world.put_secret("root-key", "root-secret-value").await;
    world.put_secret("scoped-key", "scoped-secret-value").await;
    let (workflow, _) = world.publish(chain(name)).await;
    let tenant = world.admin.tenant_id().as_uuid();
    let document = serde_json::to_vec(&structured_workflow(name)).expect("structured document");
    StructuredJsonIrCodec
        .parse(&document)
        .expect("valid structured document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1, $2, $3, 2, $4, $5, $6, clock_timestamp())",
    )
    .bind(version.as_uuid())
    .bind(tenant)
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(world.admin.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("trusted structured definition");
    let run = RunId::generate();
    let input = serde_json::to_vec(&json!({
        "value": name,
        "items": [{"sku": "first"}, {"sku": "second"}]
    }))
    .expect("run input");
    let input_digest = digest(&input);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) \
         ON CONFLICT (tenant_id, digest) DO NOTHING",
    )
    .bind(tenant)
    .bind(input_digest.as_bytes().as_slice())
    .bind(&input)
    .execute(&world.db.pool)
    .await
    .expect("run input payload");
    let created_at = now();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version,input_digest, \
          started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,'scoped-worker-test', \
                 '0af7651916cd43dd8448eb211c80319c',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("structured run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,1,'run.started',jsonb_build_object('entrypoint','items'), \
                 'scoped-worker-test',$3)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("run start journal");
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("collection owner"));
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'items',$3,$4,'PENDING',$5,'{}','{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(world.namespace.id().as_uuid())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("collection owner step");
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,2,'scoped-worker-test')",
    )
    .bind(tenant)
    .execute(&world.db.pool)
    .await
    .expect("collection capacity");
    let placement = Placement::new("scoped-worker-test");
    Keeper::new(
        world.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    if prepare_collection {
        let lease = placement
            .snapshot()
            .lease_of(Shard::of(run.as_uuid()), std::time::Instant::now())
            .expect("scheduler shard");
        let items = [json!({"sku": "first"}), json!({"sku": "second"})];
        let snapshot = serde_json::to_vec(&items).expect("snapshot");
        let snapshot_digest = digest(&snapshot);
        sqlx::query(
            "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
             ON CONFLICT (tenant_id, digest) DO NOTHING",
        )
        .bind(tenant)
        .bind(snapshot_digest.as_bytes().as_slice())
        .bind(&snapshot)
        .execute(&world.db.pool)
        .await
        .expect("snapshot payload");
        let scope = CollectionScopeId::generate();
        let collection_store = PgCollectionStore::with_definition_cache(
            world.db.pool.clone(),
            DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        );
        let entered = collection_store
            .enter_collection(
                &CollectionEntry::new(
                    collection_transition(&world, run, 1, owner.clone(), lease),
                    scope,
                    2,
                    snapshot_digest,
                    CollectionLimits::new(2, 2, 20, 2).expect("collection limits"),
                )
                .expect("collection entry"),
            )
            .await
            .expect("enter collection");
        let mut run_version = entered.run_version;
        for (index, item) in items.iter().enumerate() {
            let item = serde_json::to_vec(item).expect("item");
            let item_digest = digest(&item);
            let path = InstancePath::root()
                .child(
                    NodeId::parse("items").expect("scope"),
                    u32::try_from(index).expect("index"),
                )
                .expect("path");
            let entry = ExecutionInstanceId::in_path(path, NodeId::parse("a").expect("node"));
            let admitted = collection_store
                .admit_collection_activity(
                    &CollectionAdmission::new(
                        collection_transition(&world, run, run_version, owner.clone(), lease),
                        scope,
                        u32::try_from(index).expect("index"),
                        entry,
                        sagat_domain::workflow::ActivityName::parse("scoped.a").expect("activity"),
                        item_digest,
                        2,
                        Uuid::now_v7(),
                    )
                    .expect("collection admission"),
                )
                .await
                .expect("admit collection iteration");
            run_version = admitted.run_version;
        }
    }

    let root_only = world
        .workers
        .register(
            &world.worker,
            "payments",
            &format!("root-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec!["act.a".into(), "act.b".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 8,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register compatibility worker");
    assert_eq!(root_only.accepted_protocol_version, "1.4");

    let cache = DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8);
    let store = PgRunStore::new(world.db.pool.clone(), cache).with_scoped_worker_protocol();
    let workers = WorkerService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(world.keys.cipher());
    let session = register_scoped(&world, &workers).await;
    sqlx::query(
        "DELETE FROM worker_bindings WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
    )
    .bind(tenant)
    .bind(world.namespace.id().as_uuid())
    .bind(world.worker.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("remove broad test binding");
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
         VALUES ($1,$2,$3,'scoped.*','scoped-worker-test')",
    )
    .bind(tenant)
    .bind(world.namespace.id().as_uuid())
    .bind(world.worker.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("narrow scoped binding");
    Fixture {
        world,
        store,
        workers,
        session,
        run,
        placement,
    }
}

async fn fixture(name: &str) -> Fixture {
    fixture_with_progress(name, true).await
}

async fn poll(fixture: &Fixture, max: u32) -> Vec<ClaimedTask> {
    fixture
        .workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            max,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll scoped work")
}

fn index(task: &ClaimedTask) -> u32 {
    task.instance.path().segments()[0].iteration_index()
}

async fn complete(
    fixture: &Fixture,
    task: &ClaimedTask,
    value: u32,
    key: &str,
) -> Result<sagat_application::worker::StepAck, AppError> {
    fixture
        .workers
        .complete(
            &fixture.session.principal,
            &task_ref(task),
            &serde_json::to_vec(&json!({"value": value})).expect("output"),
            key,
        )
        .await
}

async fn fail(fixture: &Fixture, task: &ClaimedTask, key: &str) -> Result<(), AppError> {
    fixture
        .workers
        .fail(
            &fixture.session.principal,
            &task_ref(task),
            "TRANSIENT_PROVIDER_FAILURE",
            true,
            key,
        )
        .await
        .map(|_| ())
}

fn scheduler(fixture: &Fixture) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        fixture.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(fixture.placement.clone())
}

async fn close_content_quota(fixture: &Fixture) {
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let used: i64 = sqlx::query_scalar(
        "SELECT content_bytes + content_reserved_bytes FROM tenant_retained_usage \
         WHERE tenant_id=$1",
    )
    .bind(tenant)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("retained usage");
    let mut tx = fixture
        .world
        .db
        .pool
        .begin()
        .await
        .expect("quota policy transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("quota-policy:tenant_content_bytes:{tenant}"))
        .execute(&mut *tx)
        .await
        .expect("quota policy fence");
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id=$1 AND namespace_id IS NULL \
         AND resource='tenant_content_bytes'",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("clear content limit");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) \
         VALUES ($1,NULL,'tenant_content_bytes',$2,'collection-quota-test')",
    )
    .bind(tenant)
    .bind(used)
    .execute(&mut *tx)
    .await
    .expect("close content limit");
    tx.commit().await.expect("commit content limit");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true")
        .execute(&fixture.world.db.pool)
        .await
        .expect("enable content enforcement");
}

async fn storage_quota_audits(fixture: &Fixture, reason: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action=$2 \
           AND resource_type='run' AND resource_id=$3 AND reason=$4",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(AuditAction::RunStorageQuotaExceeded.as_str())
    .bind(fixture.run.to_string())
    .bind(reason)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("storage quota audit count")
}

async fn cancel(fixture: &Fixture, key: &str) {
    let cache = DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8);
    RunService::new(
        PgRunStore::new(fixture.world.db.pool.clone(), cache.clone()).with_scoped_worker_protocol(),
        SystemClock,
        Engine::new(JsonEvaluator),
        cache,
    )
    .operate(
        &fixture.world.admin,
        fixture.run,
        RunOperation::Cancel,
        "cancel collection",
        None,
        key,
        &ctx(),
    )
    .await
    .expect("request collection cancellation");
}

async fn subscribe_to_run_finished(fixture: &Fixture, label: &str) -> Uuid {
    let subscription = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO webhook_subscriptions \
         (id,tenant_id,namespace_id,name,destination_kind,destination_settings,url,secret_name, \
          kinds,rate_per_second,timeout_ms,status,created_by) \
         VALUES ($1,$2,$3,$4,'WEBHOOK','{}','https://example.invalid/drain','drain-key', \
                 ARRAY['run.finished'],100,1000,'ACTIVE','test')",
    )
    .bind(subscription)
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(format!("drain-{label}-{}", Uuid::now_v7()))
    .execute(&fixture.world.db.pool)
    .await
    .expect("run finished subscription");
    subscription
}

#[tokio::test]
async fn cancellation_drains_members_atomically_and_finishes_after_restart() {
    let fixture = fixture("scoped-collection-cancel").await;
    let tenant = fixture.world.admin.tenant_id();
    let subscription = subscribe_to_run_finished(&fixture, "cancel").await;
    cancel(&fixture, "cancel-scoped-collection").await;

    let (scope, version): (Uuid, i64) = sqlx::query_as(
        "SELECT scope.scope_id,run.run_version FROM iteration_scopes scope JOIN runs run \
           ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("open drain identity");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("scheduler shard");
    let conflicting_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished','{}')",
    )
    .bind(conflicting_outbox)
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("conflicting drain outbox");
    let error = fixture
        .store
        .start_collection_drain(&CollectionDrain::new(
            collection_transition(
                &fixture.world,
                fixture.run,
                u64::try_from(version).expect("run version"),
                ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
                lease,
            ),
            CollectionScopeId::new(scope),
            CollectionDrainReason::Cancelled,
            conflicting_outbox,
        ))
        .await
        .expect_err("outbox conflict must roll back drain start");
    assert!(matches!(error, PrimitiveStoreError::Storage(_)));
    let rolled_back_start: (String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,run.run_version, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='collection.drain_started'), \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
                  AND message.aggregate_id=run.id AND message.event_type='collection.drain.started') \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back drain start");
    assert_eq!(
        rolled_back_start,
        ("CANCELLING".into(), "OPEN".into(), version, 0, 0)
    );
    let started = scheduler(&fixture).tick().await;
    assert_eq!(started.collections_advanced, 1, "{started:?}");
    let phase: String = sqlx::query_scalar(
        "SELECT lifecycle FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("draining collection");
    assert_eq!(phase, "DRAINING");

    sqlx::query(
        "CREATE FUNCTION reject_collection_drain() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.event_type='collection.iteration_drained' THEN \
             RAISE EXCEPTION 'injected drain failure'; \
           END IF; \
           RETURN NEW; \
         END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault function");
    sqlx::query(
        "CREATE TRIGGER reject_collection_drain BEFORE INSERT ON run_events \
         FOR EACH ROW EXECUTE FUNCTION reject_collection_drain()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault trigger");
    let rolled_back = scheduler(&fixture).tick().await;
    assert_eq!(rolled_back.collections_advanced, 0, "{rolled_back:?}");
    assert_eq!(rolled_back.failed, 1, "{rolled_back:?}");
    let before_retry: (i64, i32, i32) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE lifecycle='DRAINED'), \
                min(remaining_reservations), \
                (SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1) \
         FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back drain");
    assert_eq!(before_retry, (0, 2, 2));
    sqlx::query("DROP TRIGGER reject_collection_drain ON run_events")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault trigger");
    sqlx::query("DROP FUNCTION reject_collection_drain()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault function");

    for expected in 1..=2 {
        let drained = scheduler(&fixture).tick().await;
        assert_eq!(drained.collections_advanced, 1, "{drained:?}");
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM collection_iterations \
             WHERE tenant_id=$1 AND run_id=$2 AND lifecycle='DRAINED'",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("drained count");
        assert_eq!(count, expected);
    }
    let before_terminal: (String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,run.run_version, \
                coalesce((SELECT head_seq FROM telemetry_pending \
                          WHERE tenant_id=run.tenant_id AND run_id=run.id),0), \
                (SELECT count(*) FROM webhook_deliveries \
                  WHERE tenant_id=run.tenant_id AND subscription_id=$3) \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("pre-terminal drain evidence");
    assert_eq!(before_terminal.2, before_terminal.3);
    sqlx::query(
        "CREATE FUNCTION reject_collection_terminal_delivery() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected terminal delivery failure'; END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("terminal delivery fault function");
    sqlx::query(
        "CREATE TRIGGER reject_collection_terminal_delivery BEFORE INSERT ON webhook_deliveries \
         FOR EACH ROW EXECUTE FUNCTION reject_collection_terminal_delivery()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("terminal delivery fault trigger");
    let rejected = scheduler(&fixture).tick().await;
    assert_eq!(rejected.collections_advanced, 0, "{rejected:?}");
    assert_eq!(rejected.failed, 1, "{rejected:?}");
    let after_rejected: (String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,run.run_version, \
                coalesce((SELECT head_seq FROM telemetry_pending \
                          WHERE tenant_id=run.tenant_id AND run_id=run.id),0), \
                (SELECT count(*) FROM webhook_deliveries \
                  WHERE tenant_id=run.tenant_id AND subscription_id=$3) \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back terminal evidence");
    assert_eq!(after_rejected, before_terminal);
    sqlx::query("DROP TRIGGER reject_collection_terminal_delivery ON webhook_deliveries")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop terminal delivery fault trigger");
    sqlx::query("DROP FUNCTION reject_collection_terminal_delivery()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop terminal delivery fault function");
    let finished = scheduler(&fixture).tick().await;
    assert_eq!(finished.collections_advanced, 1, "{finished:?}");
    let terminal: (String, String, i32, i32, i32, i64, i64, i64, Vec<u8>) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,scope.active_count,usage.active_iterations, \
                instances.reserved_instances, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
                  AND e.run_id=run.id AND e.event_type='collection.iteration_drained'), \
                telemetry.head_seq,run.run_version,delivery.payload \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=run.tenant_id \
         JOIN run_instance_usage instances \
           ON instances.tenant_id=run.tenant_id AND instances.run_id=run.id \
         JOIN telemetry_pending telemetry \
           ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
         JOIN webhook_deliveries delivery \
           ON delivery.tenant_id=run.tenant_id AND delivery.subscription_id=$3 \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal drain projection");
    assert_eq!(terminal.0, "CANCELLED");
    assert_eq!(terminal.1, "CANCELLED");
    assert_eq!(
        (terminal.2, terminal.3, terminal.4, terminal.5),
        (0, 0, 0, 2)
    );
    assert_eq!(terminal.6, terminal.7);
    let outbound: serde_json::Value =
        serde_json::from_slice(&terminal.8).expect("terminal outbound payload");
    assert_eq!(outbound["data"]["status"], "CANCELLED");
    let (scope, drain_outbox, finish_outbox, version): (Uuid, Uuid, Uuid, i64) = sqlx::query_as(
        "SELECT scope.scope_id,scope.drain_outbox_id,scope.finish_outbox_id,run.run_version \
             FROM iteration_scopes scope JOIN runs run \
               ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
             WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal receipts");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("scheduler shard");
    let transition = || {
        collection_transition(
            &fixture.world,
            fixture.run,
            u64::try_from(version).expect("run version"),
            ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
            lease,
        )
    };
    assert!(
        !fixture
            .store
            .start_collection_drain(&CollectionDrain::new(
                transition(),
                CollectionScopeId::new(scope),
                CollectionDrainReason::Cancelled,
                drain_outbox,
            ))
            .await
            .expect("replay drain start")
            .applied
    );
    for index in 0..2 {
        let outbox: Uuid = sqlx::query_scalar(
            "SELECT drain_outbox_id FROM collection_iterations \
             WHERE tenant_id=$1 AND run_id=$2 AND iteration_index=$3",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .bind(index)
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("member drain receipt");
        assert!(
            !fixture
                .store
                .drain_collection_iteration(&CollectionIterationDrain::new(
                    transition(),
                    CollectionScopeId::new(scope),
                    u32::try_from(index).expect("index"),
                    outbox,
                ))
                .await
                .expect("replay member drain")
                .applied
        );
    }
    assert!(
        !fixture
            .store
            .finish_collection_drain(&CollectionDrainFinish::new(
                transition(),
                CollectionScopeId::new(scope),
                finish_outbox,
            ))
            .await
            .expect("replay drain finish")
            .applied
    );
    let replay_effects: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM webhook_deliveries \
             WHERE tenant_id=$1 AND subscription_id=$3), \
           (SELECT head_seq FROM telemetry_pending WHERE tenant_id=$1 AND run_id=$2)",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("replayed terminal effects");
    assert_eq!(replay_effects, (1, terminal.6));
    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("drained run replay");
    // The conflicting row above was seeded by this test, not published by the platform.
    let published = crate::hint_contract::assert_outbox_matches_the_contract_except(
        &fixture.world.db.pool,
        &[conflicting_outbox],
    )
    .await;
    crate::hint_contract::assert_published(
        &published,
        &[
            "task.ready",
            "collection.drain.started",
            "collection.finished",
            "run.finished",
        ],
    );
}

#[tokio::test]
async fn exhausted_member_failure_starts_fail_fast_drain_and_survives_restart() {
    let fixture = fixture("scoped-collection-failure-drain").await;
    let tenant = fixture.world.admin.tenant_id();
    let subscription = subscribe_to_run_finished(&fixture, "failed").await;
    let first = poll(&fixture, 1).await.pop().expect("first attempt");
    fail(&fixture, &first, "drain-first-failure")
        .await
        .expect("schedule retry");
    age_instance_column(&fixture, &first, "next_attempt_at").await;
    let released = scheduler(&fixture).tick().await;
    assert_eq!(released.retries_released, 1, "{released:?}");
    let mut claimed = poll(&fixture, 8).await;
    let position = claimed
        .iter()
        .position(|task| task.instance == first.instance)
        .expect("replacement attempt");
    let replacement = claimed.remove(position);
    for task in &claimed {
        complete(&fixture, task, 7, "drain-other-member")
            .await
            .expect("settle other accepted member");
    }
    fail(&fixture, &replacement, "drain-final-failure")
        .await
        .expect("exhaust retry policy");
    let failed_status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(first.step_id)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("failed member status");
    assert_eq!(failed_status, "FAILED");

    let started = scheduler(&fixture).tick().await;
    assert_eq!(started.collections_advanced, 1, "{started:?}");
    let drain: (String, String, i32, String) = sqlx::query_as(
        "SELECT run.status,scope.drain_kind,scope.drain_iteration,scope.drain_failure_code \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("fail-fast drain");
    assert_eq!(
        drain,
        (
            "CANCELLING".into(),
            "FAILED".into(),
            i32::try_from(index(&first)).expect("index"),
            "TRANSIENT_PROVIDER_FAILURE".into(),
        )
    );

    for _ in 0..2 {
        let drained = scheduler(&fixture).tick().await;
        let evidence: Vec<(Vec<i32>, String, i32)> = sqlx::query_as(
            "SELECT instance_path_indices,status,attempt FROM steps \
             WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)>0 \
             ORDER BY instance_path_indices,node",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_all(&fixture.world.db.pool)
        .await
        .expect("drain evidence");
        assert_eq!(drained.collections_advanced, 1, "{drained:?} {evidence:?}");
    }
    let finished = scheduler(&fixture).tick().await;
    assert_eq!(finished.collections_advanced, 1, "{finished:?}");
    let terminal: (String, String, i64, i64, i64, i64, Vec<u8>) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
                  AND e.run_id=run.id AND e.event_type='run.stop_requested'), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=run.tenant_id \
                  AND e.run_id=run.id AND e.event_type='run.finished'), \
                telemetry.head_seq,run.run_version,delivery.payload \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         JOIN telemetry_pending telemetry \
           ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
         JOIN webhook_deliveries delivery \
           ON delivery.tenant_id=run.tenant_id AND delivery.subscription_id=$3 \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("failed terminal projection");
    assert_eq!(terminal.0, "FAILED");
    assert_eq!(terminal.1, "FAILED");
    assert_eq!((terminal.2, terminal.3), (1, 1));
    assert_eq!(terminal.4, terminal.5);
    let outbound: serde_json::Value =
        serde_json::from_slice(&terminal.6).expect("failed outbound payload");
    assert_eq!(outbound["data"]["status"], "FAILED");
    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("failed drain replay");
}

#[tokio::test]
async fn failed_derived_obligations_keep_member_capacity_reserved() {
    for (label, node, kind, failure_code) in [
        ("compensation", "a~c", "COMPENSATION", "COMPENSATION_FAILED"),
        (
            "reconciliation",
            "a~r",
            "RECONCILIATION",
            "RECONCILIATION_FAILED",
        ),
    ] {
        let fixture = fixture(&format!("scoped-collection-{label}-drain")).await;
        let tenant = fixture.world.admin.tenant_id();
        cancel(&fixture, &format!("cancel-before-failed-{label}")).await;
        let started = scheduler(&fixture).tick().await;
        assert_eq!(started.collections_advanced, 1, "{label}: {started:?}");
        let changed = sqlx::query(
            "UPDATE steps SET node=$3,kind=$4,status='FAILED',failure_code=$5 \
             WHERE tenant_id=$1 AND run_id=$2 AND node='a' AND instance_path_indices=ARRAY[0]",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .bind(node)
        .bind(kind)
        .bind(failure_code)
        .execute(&fixture.world.db.pool)
        .await
        .expect("failed derived projection");
        assert_eq!(changed.rows_affected(), 1, "{label}");
        let (scope, version): (Uuid, i64) = sqlx::query_as(
            "SELECT scope.scope_id,run.run_version FROM iteration_scopes scope JOIN runs run \
               ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
             WHERE scope.tenant_id=$1 AND scope.run_id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("drain identity");
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("scheduler shard");
        let error = fixture
            .store
            .drain_collection_iteration(&CollectionIterationDrain::new(
                collection_transition(
                    &fixture.world,
                    fixture.run,
                    u64::try_from(version).expect("run version"),
                    ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
                    lease,
                ),
                CollectionScopeId::new(scope),
                0,
                Uuid::now_v7(),
            ))
            .await
            .expect_err("failed derived obligation must stay visible");
        assert_eq!(error, PrimitiveStoreError::InvalidTransition, "{label}");
        let retained: (String, bool, i32, i32, i32, i32, i64, i64) = sqlx::query_as(
            "SELECT member.lifecycle,member.slot_held,member.remaining_reservations, \
                    usage.active_iterations,instances.admitted_instances, \
                    instances.reserved_instances, \
                    (SELECT count(*) FROM run_events WHERE tenant_id=member.tenant_id \
                      AND run_id=member.run_id AND event_type='collection.iteration_drained'), \
                    (SELECT count(*) FROM event_outbox WHERE tenant_id=member.tenant_id \
                      AND aggregate_id=member.run_id \
                      AND event_type='collection.iteration.drained') \
             FROM collection_iterations member JOIN tenant_iteration_usage usage \
               ON usage.tenant_id=member.tenant_id \
             JOIN run_instance_usage instances \
               ON instances.tenant_id=member.tenant_id AND instances.run_id=member.run_id \
             WHERE member.tenant_id=$1 AND member.run_id=$2 AND member.iteration_index=0",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("retained drain obligation");
        assert_eq!(
            retained,
            ("ADMITTED".into(), true, 2, 2, 3, 4, 0, 0),
            "{label}"
        );
    }
}

#[tokio::test]
async fn collection_discovery_filters_unrelated_pending_roots_before_pagination() {
    let fixture = fixture_with_progress("collection-discovery-page", false).await;
    for index in 0..16 {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'PENDING',$6 - interval '1 day','{}','{}')",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .bind(format!("noise-{index}"))
        .bind(Uuid::now_v7())
        .bind(fixture.world.namespace.id().as_uuid())
        .bind(now())
        .execute(&fixture.world.db.pool)
        .await
        .expect("unrelated pending root");
    }

    let candidates = fixture
        .store
        .collection_candidates(&Window::everything(1))
        .await
        .expect("collection candidates");
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].run_id, fixture.run);
    assert_eq!(candidates[0].node.as_str(), "items");
}

#[tokio::test]
async fn scheduler_resumes_collection_through_checkpoint_and_finish() {
    let fixture = fixture_with_progress("scoped-collection-cycle", false).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let tenant = fixture.world.admin.tenant_id();
    let entered = scheduler(&fixture).tick().await;
    assert_eq!(
        entered.collections_advanced, 1,
        "automatic enter: {entered:?}"
    );
    assert_eq!(entered.failed, 0, "automatic enter: {entered:?}");
    let discovered = fixture
        .store
        .collection_candidates(&Window::everything(8))
        .await
        .expect("discover collection admission");
    assert_eq!(discovered.len(), 1, "{discovered:?}");
    for transition in ["first admission", "second admission"] {
        let report = scheduler(&fixture).tick().await;
        let progress: (String, i32, i32, i32, i32) = sqlx::query_as(
            "SELECT lifecycle,next_admission_index,item_count,active_count,max_concurrency \
             FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_node='items'",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("collection progress");
        assert_eq!(
            report.collections_advanced, 1,
            "automatic {transition}: {report:?}, progress={progress:?}"
        );
        assert_eq!(report.failed, 0, "automatic {transition}: {report:?}");
    }
    let waiting = scheduler(&fixture).tick().await;
    assert_eq!(waiting.collections_advanced, 0, "{waiting:?}");
    assert_eq!(waiting.failed, 0, "{waiting:?}");

    let mut entries = poll(&fixture, 8).await;
    entries.sort_by_key(index);
    assert_eq!(entries.len(), 2);
    for (position, task) in entries.iter().enumerate() {
        complete(
            &fixture,
            task,
            u32::try_from(position + 1).expect("result"),
            &format!("collection-entry-{position}"),
        )
        .await
        .expect("complete body entry");
    }
    let mut bodies = poll(&fixture, 8).await;
    bodies.sort_by_key(index);
    assert_eq!(bodies.len(), 2);
    for (position, task) in bodies.iter().enumerate() {
        complete(
            &fixture,
            task,
            u32::try_from((position + 1) * 10).expect("result"),
            &format!("collection-body-{position}"),
        )
        .await
        .expect("complete body result");
    }

    sqlx::query(
        "CREATE FUNCTION reject_collection_checkpoint() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.event_type='collection.iteration_checkpointed' THEN \
             RAISE EXCEPTION 'injected checkpoint failure'; \
           END IF; \
           RETURN NEW; \
         END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault function");
    sqlx::query(
        "CREATE TRIGGER reject_collection_checkpoint BEFORE INSERT ON run_events \
         FOR EACH ROW EXECUTE FUNCTION reject_collection_checkpoint()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault trigger");
    let failed_checkpoint = scheduler(&fixture).tick().await;
    assert_eq!(
        failed_checkpoint.collections_advanced, 0,
        "{failed_checkpoint:?}"
    );
    assert_eq!(failed_checkpoint.failed, 1, "{failed_checkpoint:?}");
    let partial: (i64, i64) = sqlx::query_as(
        "SELECT \
           count(*) FILTER (WHERE event_type='collection.iteration_returned'), \
           count(*) FILTER (WHERE event_type='collection.iteration_checkpointed') \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rollback journal evidence");
    assert_eq!(partial, (0, 0));
    sqlx::query("DROP TRIGGER reject_collection_checkpoint ON run_events")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault trigger");
    sqlx::query("DROP FUNCTION reject_collection_checkpoint()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault function");

    let first_checkpoint = scheduler(&fixture).tick().await;
    assert_eq!(
        first_checkpoint.collections_advanced, 1,
        "{first_checkpoint:?}"
    );
    assert_eq!(first_checkpoint.failed, 0, "{first_checkpoint:?}");
    let (first_digest, first_result): (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT c.result_digest,p.content FROM iteration_checkpoints c JOIN payloads p \
           ON p.tenant_id=c.tenant_id AND p.digest=c.result_digest \
         WHERE c.tenant_id=$1 AND c.run_id=$2 AND c.iteration_index=0",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("first checkpoint payload");
    sqlx::query("ALTER TABLE payloads DISABLE TRIGGER payloads_append_only")
        .execute(&fixture.world.db.pool)
        .await
        .expect("disable append-only guard for corruption test");
    sqlx::query("UPDATE payloads SET content='null' WHERE tenant_id=$1 AND digest=$2")
        .bind(tenant.as_uuid())
        .bind(&first_digest)
        .execute(&fixture.world.db.pool)
        .await
        .expect("corrupt checkpoint payload");
    let corrupt_checkpoint = scheduler(&fixture).tick().await;
    assert_eq!(
        corrupt_checkpoint.collections_advanced, 0,
        "{corrupt_checkpoint:?}"
    );
    assert_eq!(corrupt_checkpoint.failed, 1, "{corrupt_checkpoint:?}");
    sqlx::query("UPDATE payloads SET content=$3 WHERE tenant_id=$1 AND digest=$2")
        .bind(tenant.as_uuid())
        .bind(first_digest)
        .bind(first_result)
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore checkpoint payload");
    sqlx::query("ALTER TABLE payloads ENABLE TRIGGER payloads_append_only")
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore append-only guard");
    let second_checkpoint = scheduler(&fixture).tick().await;
    assert_eq!(
        second_checkpoint.collections_advanced, 1,
        "{second_checkpoint:?}"
    );
    assert_eq!(second_checkpoint.failed, 0, "{second_checkpoint:?}");

    sqlx::raw_sql(
        "CREATE FUNCTION reject_collection_successor_hint() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected successor hint failure'; END $$; \
         CREATE TRIGGER reject_collection_successor_hint BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'task.ready' AND NEW.routing_key = 'act.a') \
         EXECUTE FUNCTION reject_collection_successor_hint()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("install successor fault");
    let failed_finish = scheduler(&fixture).tick().await;
    assert_eq!(failed_finish.collections_advanced, 0, "{failed_finish:?}");
    assert_eq!(failed_finish.failed, 1, "{failed_finish:?}");
    let rolled_back_finish: (String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,o.status, \
           (SELECT count(*) FROM steps next_step \
             WHERE next_step.tenant_id=s.tenant_id AND next_step.run_id=s.run_id \
               AND next_step.node='a' AND cardinality(next_step.instance_path_nodes)=0), \
           (SELECT count(*) FROM run_events event \
             WHERE event.tenant_id=s.tenant_id AND event.run_id=s.run_id \
               AND event.event_type='collection.finished'), \
           (SELECT count(*) FROM event_outbox message \
             WHERE message.tenant_id=s.tenant_id AND message.aggregate_id=s.run_id \
               AND message.event_type='task.ready' AND message.routing_key='act.a') \
         FROM iteration_scopes s JOIN steps o \
           ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node \
          AND o.instance_path_nodes=s.owner_path_nodes \
          AND o.instance_path_indices=s.owner_path_indices \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='items'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("finish rollback evidence");
    assert_eq!(
        rolled_back_finish,
        ("OPEN".into(), "WAITING".into(), 0, 0, 0)
    );
    sqlx::raw_sql(
        "DROP TRIGGER reject_collection_successor_hint ON event_outbox; \
         DROP FUNCTION reject_collection_successor_hint()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("remove successor fault");

    let finished = scheduler(&fixture).tick().await;
    assert_eq!(finished.collections_advanced, 1, "{finished:?}");
    assert_eq!(finished.failed, 0, "{finished:?}");
    assert_eq!(
        scheduler(&fixture)
            .advance_collection(tenant, fixture.run, &owner)
            .await
            .expect("reopen finished collection"),
        CollectionAdvance::AlreadyFinished
    );

    let aggregate: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM iteration_scopes s JOIN payloads p \
           ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='items'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("aggregate result");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&aggregate).expect("aggregate json"),
        json!([{"value": 10}, {"value": 20}])
    );
    let lifecycle: (String, String, Option<String>, i64) = sqlx::query_as(
        "SELECT s.lifecycle,o.status,o.waiting_kind, \
           (SELECT count(*) FROM iteration_checkpoints c \
             WHERE c.tenant_id=s.tenant_id AND c.run_id=s.run_id AND c.scope_id=s.scope_id) \
         FROM iteration_scopes s JOIN steps o \
           ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node \
          AND o.instance_path_nodes=s.owner_path_nodes \
          AND o.instance_path_indices=s.owner_path_indices \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='items'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("finished projection");
    assert_eq!(lifecycle, ("SUCCEEDED".into(), "SUCCEEDED".into(), None, 2));

    let successor: (String, String, bool, i32, i64, i64) = sqlx::query_as(
        "SELECT status,activity,routed,hint_count, \
           (SELECT count(*) FROM run_events event \
             WHERE event.tenant_id=step.tenant_id AND event.run_id=step.run_id \
               AND event.node=step.node AND cardinality(event.instance_path_nodes)=0 \
               AND event.event_type='step.released'), \
           (SELECT count(*) FROM event_outbox message \
             WHERE message.tenant_id=step.tenant_id AND message.aggregate_id=step.run_id \
               AND message.event_type='task.ready' AND message.routing_key=step.activity) \
         FROM steps step WHERE tenant_id=$1 AND run_id=$2 AND node='a' \
           AND cardinality(instance_path_nodes)=0",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("successor projection");
    assert_eq!(successor, ("READY".into(), "act.a".into(), false, 1, 1, 1));

    sqlx::query(
        "UPDATE event_outbox SET delivered_at=clock_timestamp() \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready' \
           AND routing_key='act.a'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("lose successor notification");
    sqlx::query(
        "UPDATE steps SET last_hinted_at=clock_timestamp() - interval '1 minute' \
         WHERE tenant_id=$1 AND run_id=$2 AND node='a' \
           AND cardinality(instance_path_nodes)=0 AND status='READY'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("age successor notification");
    let recovered = scheduler(&fixture).tick().await;
    assert_eq!(recovered.hints_renewed, 1, "{recovered:?}");
    let replacement: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready' \
           AND routing_key='act.a' AND delivered_at IS NULL",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("replacement successor notification");
    assert_eq!(replacement, 1);
    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("journal and projections remain consistent");
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.world.db.pool).await;
    crate::hint_contract::assert_published(&published, &["collection.iteration.checkpointed"]);
}

#[tokio::test]
async fn automatic_collection_progress_rechecks_the_tenant_capacity() {
    let fixture = fixture_with_progress("scoped-collection-capacity", false).await;
    let tenant = fixture.world.admin.tenant_id();
    sqlx::query(
        "UPDATE tenant_collection_policies SET max_active_iterations=1 \
         WHERE tenant_id=$1",
    )
    .bind(tenant.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("restrict tenant capacity");

    let entered = scheduler(&fixture).tick().await;
    assert_eq!(entered.collections_advanced, 1, "{entered:?}");
    let admitted = scheduler(&fixture).tick().await;
    assert_eq!(admitted.collections_advanced, 1, "{admitted:?}");
    let capacity = scheduler(&fixture).tick().await;
    assert_eq!(capacity.collections_advanced, 0, "{capacity:?}");
    assert_eq!(capacity.skipped, 1, "{capacity:?}");
    let usage: (i32, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1), \
           (SELECT count(*) FROM collection_iterations \
             WHERE tenant_id=$1 AND run_id=$2 AND lifecycle='ADMITTED'), \
           (SELECT count(*) FROM steps \
             WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)>0 \
               AND status='READY')",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("capacity projection");
    assert_eq!(usage, (1, 1, 1));

    let first = poll(&fixture, 1).await.pop().expect("first entry task");
    complete(&fixture, &first, 1, "capacity-entry")
        .await
        .expect("complete first entry");
    let body = poll(&fixture, 1).await.pop().expect("first body task");
    complete(&fixture, &body, 10, "capacity-body")
        .await
        .expect("complete first body");
    let checkpointed = scheduler(&fixture).tick().await;
    assert_eq!(checkpointed.collections_advanced, 1, "{checkpointed:?}");
    let next = scheduler(&fixture).tick().await;
    assert_eq!(next.collections_advanced, 1, "{next:?}");
    let progress: (i32, i32, i32, i32) = sqlx::query_as(
        "SELECT admitted_count,completed_count,active_count, \
           (SELECT active_iterations FROM tenant_iteration_usage \
             WHERE tenant_id=scope.tenant_id) \
         FROM iteration_scopes scope \
         WHERE tenant_id=$1 AND run_id=$2 AND owner_node='items'",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("progress after released capacity");
    assert_eq!(progress, (2, 1, 1, 1));
}

async fn age_instance_column(fixture: &Fixture, task: &ClaimedTask, column: &str) {
    assert!(matches!(
        column,
        "next_attempt_at" | "lease_expires_at" | "last_hinted_at"
    ));
    let path = task.instance.path().segments();
    let sql = format!(
        "UPDATE steps SET {column}=clock_timestamp() - interval '1 second' \
         WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
           AND instance_path_nodes=$4 AND instance_path_indices=$5"
    );
    let changed = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .bind(task.instance.node().as_str())
        .bind(
            path.iter()
                .map(|segment| segment.scope_node().as_str().to_owned())
                .collect::<Vec<_>>(),
        )
        .bind(
            path.iter()
                .map(|segment| i32::try_from(segment.iteration_index()).expect("index"))
                .collect::<Vec<_>>(),
        )
        .execute(&fixture.world.db.pool)
        .await
        .expect("age scoped instance");
    assert_eq!(changed.rows_affected(), 1);
}

#[tokio::test]
async fn stale_hints_are_renewed_for_each_scoped_instance() {
    let fixture = fixture("scoped-worker-hints").await;
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let changed = sqlx::query(
        "UPDATE steps SET last_hinted_at=clock_timestamp() - interval '1 minute' \
         WHERE tenant_id=$1 AND run_id=$2 AND status='READY' \
           AND cardinality(instance_path_nodes)>0",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("age scoped hints");
    assert_eq!(changed.rows_affected(), 2);
    let initial_hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox \
         WHERE aggregate_id=$1 AND event_type='task.ready' AND payload ? 'instancePath'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("admission hints");

    let report = scheduler(&fixture).tick().await;
    assert_eq!(report.hints_renewed, 2, "{report:?}");
    let rows = sqlx::query(
        "SELECT hint_count, instance_path_indices FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND status='READY' \
           AND cardinality(instance_path_nodes)>0 ORDER BY instance_path_indices",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .fetch_all(&fixture.world.db.pool)
    .await
    .expect("renewed projections");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<i32, _>("hint_count"), 2);
    assert_eq!(rows[1].get::<i32, _>("hint_count"), 2);
    assert_eq!(rows[0].get::<Vec<i32>, _>("instance_path_indices"), [0]);
    assert_eq!(rows[1].get::<Vec<i32>, _>("instance_path_indices"), [1]);
    let hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox \
         WHERE aggregate_id=$1 AND event_type='task.ready' \
           AND payload ? 'instancePath'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("scoped hint outbox");
    assert_eq!(hints, initial_hints + 2);
}

async fn instance_retry_evidence(fixture: &Fixture, task: &ClaimedTask) -> (String, i32, i64, i64) {
    let path = task.instance.path().segments();
    sqlx::query_as(
        "SELECT s.status, s.attempt, \
          (SELECT count(*) FROM run_events e \
             WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
               AND e.event_type='step.retry_scheduled' AND e.node=s.node \
               AND e.instance_path_nodes=s.instance_path_nodes \
               AND e.instance_path_indices=s.instance_path_indices), \
          (SELECT count(*) FROM run_events e \
             WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
               AND e.event_type='step.retry_due' AND e.node=s.node \
               AND e.instance_path_nodes=s.instance_path_nodes \
               AND e.instance_path_indices=s.instance_path_indices) \
         FROM steps s \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node=$3 \
           AND s.instance_path_nodes=$4 AND s.instance_path_indices=$5",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(task.instance.node().as_str())
    .bind(
        path.iter()
            .map(|segment| segment.scope_node().as_str().to_owned())
            .collect::<Vec<_>>(),
    )
    .bind(
        path.iter()
            .map(|segment| i32::try_from(segment.iteration_index()).expect("index"))
            .collect::<Vec<_>>(),
    )
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("scoped retry evidence")
}

#[tokio::test]
async fn two_equal_nodes_keep_distinct_leases_outputs_routes_and_replays() {
    let fixture = fixture("scoped-worker-identities").await;
    let mut tasks = poll(&fixture, 8).await;
    tasks.sort_by_key(index);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks.iter().map(index).collect::<Vec<_>>(), [0, 1]);
    assert!(
        tasks
            .iter()
            .all(|task| task.instance.node().as_str() == "a")
    );
    assert_eq!(
        tasks
            .iter()
            .map(|task| task.activity.as_str())
            .collect::<Vec<_>>(),
        ["scoped.a", "scoped.a"]
    );
    let contexts = tasks
        .iter()
        .map(|task| serde_json::from_slice::<serde_json::Value>(&task.input).expect("task input"))
        .collect::<Vec<_>>();
    assert_eq!(contexts[0]["item"], json!({"sku": "first"}));
    assert_eq!(contexts[0]["index"], 0);
    assert_eq!(contexts[1]["item"], json!({"sku": "second"}));
    assert_eq!(contexts[1]["index"], 1);
    assert_eq!(tasks[0].idempotency_key, "scope:first:0");
    assert_eq!(tasks[1].idempotency_key, "scope:second:1");
    assert_ne!(tasks[0].step_id, tasks[1].step_id);

    let crossed = TaskRef {
        run_id: fixture.run,
        instance: &tasks[1].instance,
        step_id: Some(tasks[1].step_id),
        fence: tasks[0].fence,
        lease_token: tasks[0].lease_token.expose(),
    };
    assert!(matches!(
        fixture
            .workers
            .complete(&fixture.session.principal, &crossed, b"{}", "crossed-fence")
            .await,
        Err(AppError::StaleAttempt)
    ));

    fixture
        .workers
        .heartbeat(&fixture.session.principal, &task_ref(&tasks[0]))
        .await
        .expect("scoped heartbeat");
    let secrets = fixture
        .workers
        .resolve_secrets(&fixture.session.principal, &task_ref(&tasks[0]), None)
        .await
        .expect("scoped secrets");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].name.as_str(), "scoped-key");
    assert_eq!(secrets[0].value.expose(), b"scoped-secret-value");
    let first = complete(&fixture, &tasks[0], 10, "scoped-done-0")
        .await
        .expect("complete first instance");
    assert!(!first.already_applied);
    let replay = complete(&fixture, &tasks[0], 10, "scoped-done-0")
        .await
        .expect("replay first completion");
    assert!(replay.already_applied);
    complete(&fixture, &tasks[1], 20, "scoped-done-1")
        .await
        .expect("complete second instance");

    let graph = fixture
        .store
        .graph(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("graph query")
        .expect("run graph");
    let scoped_nodes = graph
        .steps
        .iter()
        .filter(|step| !step.instance.path().is_root())
        .map(|step| {
            (
                step.node.as_str(),
                step.instance.path().segments()[0].iteration_index(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(scoped_nodes, [("a", 0), ("b", 0), ("a", 1), ("b", 1)]);

    let projected = all_attempts(
        &fixture.store,
        fixture.world.admin.tenant_id(),
        fixture.run,
        now(),
    )
    .await
    .expect("attempt projection")
    .expect("traced run");
    let scoped_attempts = projected
        .iter()
        .filter(|attempt| {
            !attempt.instance.path().is_root() && attempt.instance.node().as_str() == "a"
        })
        .collect::<Vec<_>>();
    assert_eq!(scoped_attempts.len(), 2);
    assert_ne!(scoped_attempts[0].span, scoped_attempts[1].span);

    let rows = sqlx::query(
        "SELECT s.node, s.status, s.routed, s.instance_path_indices, p.content \
         FROM steps s LEFT JOIN payloads p \
           ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND cardinality(s.instance_path_nodes)>0 \
         ORDER BY s.instance_path_indices, s.node",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_all(&fixture.world.db.pool)
    .await
    .expect("scoped projections");
    assert_eq!(rows.len(), 4);
    for (position, row) in rows.iter().enumerate() {
        let node: String = row.get("node");
        let status: String = row.get("status");
        if position % 2 == 0 {
            assert_eq!(node, "a");
            assert_eq!(status, "SUCCEEDED");
            assert!(row.get::<bool, _>("routed"));
            assert!(row.get::<Option<Vec<u8>>, _>("content").is_some());
        } else {
            assert_eq!(node, "b");
            assert_eq!(status, "READY");
        }
    }

    let mut successors = poll(&fixture, 8).await;
    successors.sort_by_key(index);
    assert_eq!(successors.iter().map(index).collect::<Vec<_>>(), [0, 1]);
    assert!(
        successors
            .iter()
            .all(|task| task.instance.node().as_str() == "b")
    );
    for task in &successors {
        assert_eq!(task.activity.as_str(), "scoped.b");
        let context: serde_json::Value =
            serde_json::from_slice(&task.input).expect("successor input");
        let expected = if index(task) == 0 { 10 } else { 20 };
        assert_eq!(context["steps"]["a"]["output"], json!({"value": expected}));
        assert_eq!(context["index"], index(task));
    }
}

#[tokio::test]
async fn a_late_journal_failure_rolls_back_the_whole_scoped_report() {
    let fixture = fixture("scoped-worker-rollback").await;
    let mut tasks = poll(&fixture, 1).await;
    let task = tasks.pop().expect("one scoped task");
    sqlx::query(
        "CREATE FUNCTION reject_scoped_route() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.event_type='step.routed' AND cardinality(NEW.instance_path_nodes)>0 THEN \
             RAISE EXCEPTION 'injected scoped route failure'; \
           END IF; \
           RETURN NEW; \
         END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault function");
    sqlx::query(
        "CREATE TRIGGER reject_scoped_route BEFORE INSERT ON run_events \
         FOR EACH ROW EXECUTE FUNCTION reject_scoped_route()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault trigger");
    let failure = complete(&fixture, &task, 99, "scoped-rollback")
        .await
        .expect_err("injected journal failure");
    assert!(
        matches!(failure, AppError::Internal | AppError::Unavailable),
        "unexpected failure: {failure:?}"
    );
    sqlx::query("DROP TRIGGER reject_scoped_route ON run_events")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault trigger");
    sqlx::query("DROP FUNCTION reject_scoped_route()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault function");

    let path = task.instance.path().segments();
    let evidence: (String, bool, Option<Vec<u8>>, i64, i64) = sqlx::query_as(
        "SELECT status, routed, output_digest, \
          (SELECT count(*) FROM run_events e WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
             AND e.event_type='step.completed' AND e.instance_path_nodes=$3 \
             AND e.instance_path_indices=$4), \
          (SELECT count(*) FROM steps n WHERE n.tenant_id=s.tenant_id AND n.run_id=s.run_id \
             AND n.node='b' AND n.instance_path_nodes=$3 AND n.instance_path_indices=$4) \
         FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='a' \
           AND s.instance_path_nodes=$3 AND s.instance_path_indices=$4",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(vec![path[0].scope_node().as_str().to_owned()])
    .bind(vec![
        i32::try_from(path[0].iteration_index()).expect("stored index"),
    ])
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rollback evidence");
    assert_eq!(evidence, ("LEASED".into(), false, None, 0, 0));

    complete(&fixture, &task, 99, "scoped-rollback")
        .await
        .expect("retry after rollback");
}

#[tokio::test]
async fn a_scoped_failure_retries_after_restart_and_rejects_the_old_fence() {
    let fixture = fixture("scoped-worker-retry").await;
    let original = poll(&fixture, 1).await.pop().expect("one scoped task");

    fail(&fixture, &original, "scoped-transient-failure")
        .await
        .expect("schedule scoped retry");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("RETRY_WAIT".into(), 1, 1, 0)
    );
    age_instance_column(&fixture, &original, "next_attempt_at").await;

    let compatibility = fixture.world.scheduler.tick().await;
    assert_eq!(compatibility.retries_released, 0, "{compatibility:?}");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("RETRY_WAIT".into(), 1, 1, 0)
    );

    let report = scheduler(&fixture).tick().await;
    assert_eq!(report.retries_released, 1, "{report:?}");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("READY".into(), 1, 1, 1)
    );

    let replacement = poll(&fixture, 8)
        .await
        .into_iter()
        .find(|candidate| candidate.instance == original.instance)
        .expect("retried scoped task");
    assert_eq!(replacement.step_id, original.step_id);
    assert_eq!(replacement.fence.attempt, original.fence.attempt + 1);
    assert_eq!(replacement.fence.generation, original.fence.generation + 1);
    assert_ne!(
        replacement.lease_token.expose(),
        original.lease_token.expose()
    );
    assert_eq!(
        complete(&fixture, &original, 1, "scoped-stale-after-retry")
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    complete(&fixture, &replacement, 2, "scoped-retry-complete")
        .await
        .expect("complete replacement");
}

#[tokio::test]
async fn a_scoped_lost_lease_rolls_back_then_recovers_after_restart() {
    let fixture = fixture("scoped-worker-lost-lease").await;
    let original = poll(&fixture, 1).await.pop().expect("one scoped task");
    age_instance_column(&fixture, &original, "lease_expires_at").await;

    let compatibility = fixture.world.scheduler.tick().await;
    assert_eq!(compatibility.leases_expired, 0, "{compatibility:?}");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("LEASED".into(), 1, 0, 0)
    );

    sqlx::query(
        "CREATE FUNCTION reject_scoped_retry() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.event_type='step.retry_scheduled' \
              AND cardinality(NEW.instance_path_nodes)>0 THEN \
             RAISE EXCEPTION 'injected scoped retry failure'; \
           END IF; \
           RETURN NEW; \
         END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault function");
    sqlx::query(
        "CREATE TRIGGER reject_scoped_retry BEFORE INSERT ON run_events \
         FOR EACH ROW EXECUTE FUNCTION reject_scoped_retry()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("fault trigger");

    let failed = scheduler(&fixture).tick().await;
    assert_eq!(failed.leases_expired, 0, "{failed:?}");
    assert_eq!(failed.failed, 1, "{failed:?}");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("LEASED".into(), 1, 0, 0)
    );

    sqlx::query("DROP TRIGGER reject_scoped_retry ON run_events")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault trigger");
    sqlx::query("DROP FUNCTION reject_scoped_retry()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop fault function");

    let recovered = scheduler(&fixture).tick().await;
    assert_eq!(recovered.leases_expired, 1, "{recovered:?}");
    assert_eq!(
        instance_retry_evidence(&fixture, &original).await,
        ("RETRY_WAIT".into(), 1, 1, 0)
    );
    assert_eq!(
        complete(&fixture, &original, 1, "scoped-late-after-loss")
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );

    age_instance_column(&fixture, &original, "next_attempt_at").await;
    let released = scheduler(&fixture).tick().await;
    assert_eq!(released.retries_released, 1, "{released:?}");
    let replacement = poll(&fixture, 8)
        .await
        .into_iter()
        .find(|candidate| candidate.instance == original.instance)
        .expect("recovered scoped task");
    assert_eq!(replacement.fence.attempt, 2);
    assert_eq!(replacement.fence.generation, 2);
    complete(&fixture, &replacement, 2, "scoped-loss-recovered")
        .await
        .expect("complete recovered task");
}

#[tokio::test]
async fn scoped_lease_capabilities_reject_a_foreign_tenant_without_leaking_the_attempt() {
    let fixture = fixture("scoped-worker-tenant-boundary").await;
    let task = poll(&fixture, 1).await.pop().expect("one scoped task");
    let outsider = sagat_domain::identity::Principal::new(
        fixture.world.other_admin.tenant_id(),
        crate::support::engine::subject_of("foreign-scoped-worker"),
        sagat_domain::identity::Role::Worker,
    );
    let reference = task_ref(&task);

    assert_eq!(
        fixture
            .workers
            .heartbeat(&outsider, &reference)
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        fixture
            .workers
            .complete(&outsider, &reference, b"{}", "foreign-scoped-complete")
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        fixture
            .workers
            .resolve_secrets(&outsider, &reference, None)
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        fixture
            .workers
            .check_blob_upload(&outsider, &reference)
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );

    let secrets = fixture
        .workers
        .resolve_secrets(&fixture.session.principal, &reference, None)
        .await
        .expect("owner still resolves scoped secrets");
    assert_eq!(secrets[0].name.as_str(), "scoped-key");
    complete(&fixture, &task, 1, "owner-after-foreign-denials")
        .await
        .expect("foreign attempts did not consume the lease");
}

#[tokio::test]
async fn a_scoped_lease_observes_cancellation_and_acknowledges_it_before_restart() {
    let fixture = fixture("scoped-worker-cancel-lease").await;
    let task = poll(&fixture, 1).await.pop().expect("one scoped task");
    cancel(&fixture, "cancel-with-scoped-lease").await;
    let draining = scheduler(&fixture).tick().await;
    assert_eq!(draining.collections_advanced, 1, "{draining:?}");

    let beat = fixture
        .workers
        .heartbeat(&fixture.session.principal, &task_ref(&task))
        .await
        .expect("leased member observes cancellation");
    assert!(beat.cancel_requested);
    fixture
        .workers
        .fail(
            &fixture.session.principal,
            &task_ref(&task),
            "CANCELLED",
            false,
            "scoped-cancel-ack",
        )
        .await
        .expect("worker acknowledges cancellation");

    let status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
           AND instance_path_nodes=$4 AND instance_path_indices=$5",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(task.instance.node().as_str())
    .bind(vec!["items".to_owned()])
    .bind(vec![i32::try_from(index(&task)).expect("index")])
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("cancelled scoped projection");
    assert_eq!(status, "CANCELLED");

    let cancellation_events: Vec<(String, Vec<String>, Vec<i32>)> = sqlx::query_as(
        "SELECT event_type,instance_path_nodes,instance_path_indices FROM run_events \
         WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type IN ('step.cancel_requested','step.cancelled') ORDER BY seq",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_all(&fixture.world.db.pool)
    .await
    .expect("canonical scoped cancellation journal");
    assert_eq!(
        cancellation_events,
        vec![
            (
                "step.cancel_requested".to_owned(),
                vec!["items".to_owned()],
                vec![i32::try_from(index(&task)).expect("index")],
            ),
            (
                "step.cancelled".to_owned(),
                vec!["items".to_owned()],
                vec![i32::try_from(index(&task)).expect("index")],
            ),
        ]
    );

    let restarted = scheduler(&fixture).tick().await;
    assert_eq!(restarted.collections_advanced, 1, "{restarted:?}");
    fixture
        .store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cancelled scoped run replays after restart");
}

#[tokio::test]
async fn snapshot_retention_quota_stops_the_pending_owner_without_creating_a_scope() {
    let fixture = fixture_with_progress("collection-snapshot-storage-quota", false).await;
    let tenant = fixture.world.admin.tenant_id();
    close_content_quota(&fixture).await;

    let stopped = scheduler(&fixture).tick().await;
    assert_eq!(stopped.quota_stops, 1, "{stopped:?}");
    assert_eq!(stopped.collections_advanced, 0, "{stopped:?}");
    assert_eq!(stopped.failed, 0, "{stopped:?}");

    let projection: (String, Option<String>, Option<String>, i64, i64) = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code, \
                (SELECT count(*) FROM iteration_scopes scope \
                  WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id), \
                (SELECT count(*) FROM run_events event \
                  WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                    AND event.event_type='run.stop_requested') \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("quota-stopped pending collection");
    assert_eq!(
        projection,
        (
            "FAILED".to_owned(),
            Some("FAILED".to_owned()),
            Some("STORAGE_QUOTA_EXCEEDED".to_owned()),
            0,
            1,
        )
    );
    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("pending quota stop replays");
    assert_eq!(
        storage_quota_audits(&fixture, "storage quota exceeded").await,
        1
    );

    let replay = scheduler(&fixture).tick().await;
    assert_eq!(replay.quota_stops, 0, "{replay:?}");
    assert_eq!(replay.failed, 0, "{replay:?}");
    assert_eq!(
        storage_quota_audits(&fixture, "storage quota exceeded").await,
        1
    );
}

#[tokio::test]
async fn aggregate_retention_quota_atomically_drains_and_replays_the_open_scope() {
    let fixture = fixture_with_progress("collection-aggregate-storage-quota", false).await;
    let tenant = fixture.world.admin.tenant_id();
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));

    for _ in 0..3 {
        let report = scheduler(&fixture).tick().await;
        assert_eq!(report.collections_advanced, 1, "{report:?}");
        assert_eq!(report.failed, 0, "{report:?}");
    }
    let mut entries = poll(&fixture, 8).await;
    entries.sort_by_key(index);
    assert_eq!(entries.len(), 2);
    for (position, task) in entries.iter().enumerate() {
        complete(
            &fixture,
            task,
            u32::try_from(position + 1).expect("entry output"),
            &format!("quota-entry-{position}"),
        )
        .await
        .expect("complete entry");
    }
    let mut bodies = poll(&fixture, 8).await;
    bodies.sort_by_key(index);
    assert_eq!(bodies.len(), 2);
    for (position, task) in bodies.iter().enumerate() {
        complete(
            &fixture,
            task,
            u32::try_from((position + 1) * 10).expect("body output"),
            &format!("quota-body-{position}"),
        )
        .await
        .expect("complete body");
    }
    for _ in 0..2 {
        let checkpoint = scheduler(&fixture).tick().await;
        assert_eq!(checkpoint.collections_advanced, 1, "{checkpoint:?}");
        assert_eq!(checkpoint.failed, 0, "{checkpoint:?}");
    }

    let before: (i64, i32, i32, i32) = sqlx::query_as(
        "SELECT run.run_version,scope.active_count,usage.active_iterations,instances.reserved_instances \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=run.tenant_id \
         JOIN run_instance_usage instances \
           ON instances.tenant_id=run.tenant_id AND instances.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("pre-quota projection");
    assert_eq!((before.1, before.2, before.3), (0, 0, 0));
    let scope_id: Uuid = sqlx::query_scalar(
        "SELECT scope_id FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("open collection scope");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("scheduler shard");
    let transition = collection_transition(
        &fixture.world,
        fixture.run,
        u64::try_from(before.0).expect("run version"),
        owner.clone(),
        lease,
    );
    for unaudited in [
        transition.clone(),
        transition.with_audit(AuditRecord::engine(
            tenant,
            AuditAction::RunCancelRequested,
            ResourceType::Run,
            fixture.run.to_string(),
            "wrong action",
            None,
            now(),
        )),
        collection_transition(
            &fixture.world,
            fixture.run,
            u64::try_from(before.0).expect("run version"),
            owner.clone(),
            lease,
        )
        .with_audit(AuditRecord::engine(
            tenant,
            AuditAction::RunStorageQuotaExceeded,
            ResourceType::Run,
            RunId::generate().to_string(),
            "wrong resource",
            None,
            now(),
        )),
        collection_transition(
            &fixture.world,
            fixture.run,
            u64::try_from(before.0).expect("run version"),
            owner.clone(),
            lease,
        )
        .with_audit(AuditRecord::operator(
            tenant,
            AuditAction::RunStorageQuotaExceeded,
            ResourceType::Run,
            fixture.run.to_string(),
            "wrong actor",
            now(),
        )),
    ] {
        assert!(matches!(
            fixture
                .store
                .start_collection_drain(&CollectionDrain::new(
                    unaudited,
                    CollectionScopeId::new(scope_id),
                    CollectionDrainReason::StorageQuotaExceeded,
                    Uuid::now_v7(),
                ))
                .await,
            Err(PrimitiveStoreError::InvalidTransition)
        ));
    }
    let after_rejections: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid())
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("run version after rejected audits");
    assert_eq!(after_rejections, before.0);
    close_content_quota(&fixture).await;

    sqlx::query(
        "CREATE FUNCTION reject_quota_drain_telemetry() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF EXISTS (SELECT 1 FROM runs WHERE tenant_id=NEW.tenant_id AND id=NEW.run_id \
                      AND status='CANCELLING') THEN \
             RAISE EXCEPTION 'injected quota drain telemetry failure'; \
           END IF; \
           RETURN NEW; \
         END $$",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("quota drain telemetry fault function");
    sqlx::query(
        "CREATE TRIGGER reject_quota_drain_telemetry BEFORE INSERT OR UPDATE ON telemetry_pending \
         FOR EACH ROW EXECUTE FUNCTION reject_quota_drain_telemetry()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("quota drain telemetry fault trigger");
    let rejected = scheduler(&fixture).tick().await;
    assert_eq!(rejected.quota_stops, 0, "{rejected:?}");
    assert_eq!(rejected.collections_advanced, 0, "{rejected:?}");
    assert_eq!(rejected.failed, 1, "{rejected:?}");
    let rolled_back: (String, String, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle, \
                (SELECT count(*) FROM run_events event \
                 WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                   AND event.event_type IN ('run.stop_requested','collection.drain_started')), \
                (SELECT count(*) FROM audit_records audit \
                 WHERE audit.tenant_id=run.tenant_id AND audit.resource_type='run' \
                   AND audit.resource_id=run.id::text AND audit.action=$3) \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(AuditAction::RunStorageQuotaExceeded.as_str())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back quota drain evidence");
    assert_eq!(rolled_back, ("WAITING".to_owned(), "OPEN".to_owned(), 0, 0));
    sqlx::query("DROP TRIGGER reject_quota_drain_telemetry ON telemetry_pending")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop quota drain telemetry fault trigger");
    sqlx::query("DROP FUNCTION reject_quota_drain_telemetry()")
        .execute(&fixture.world.db.pool)
        .await
        .expect("drop quota drain telemetry fault function");

    let stopped = scheduler(&fixture).tick().await;
    assert_eq!(stopped.quota_stops, 1, "{stopped:?}");
    assert_eq!(stopped.collections_advanced, 0, "{stopped:?}");
    assert_eq!(stopped.failed, 0, "{stopped:?}");
    let draining: (
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                    run.run_version, \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.event_type='run.stop_requested'), \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.event_type='collection.drain_started'), \
                    coalesce((SELECT head_seq FROM telemetry_pending telemetry \
                      WHERE telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id),0) \
             FROM runs run JOIN iteration_scopes scope \
               ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
             WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("atomic quota drain");
    assert_eq!(draining.0, "CANCELLING");
    assert_eq!(draining.1, "DRAINING");
    assert_eq!(draining.2.as_deref(), Some("STORAGE_QUOTA"));
    assert_eq!(draining.3.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));
    assert_eq!(draining.4, before.0 + 2);
    assert_eq!((draining.5, draining.6), (1, 1));
    assert_eq!(draining.7, draining.4);
    assert_eq!(
        storage_quota_audits(&fixture, "collection storage quota exceeded").await,
        1
    );

    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("draining quota projection replays after restart");
    let finished = scheduler(&fixture).tick().await;
    assert_eq!(finished.collections_advanced, 1, "{finished:?}");
    assert_eq!(finished.failed, 0, "{finished:?}");

    let receipts: (Uuid, Uuid, Uuid, i64) = sqlx::query_as(
        "SELECT scope.scope_id,scope.drain_outbox_id,scope.finish_outbox_id,run.run_version \
             FROM runs run JOIN iteration_scopes scope \
               ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
             WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal quota receipts");
    let terminal: (String, String, Option<String>, i32, i32, i32, i64, i64) =
        sqlx::query_as(
            "SELECT run.status,scope.lifecycle,owner.failure_code,scope.active_count, \
                    usage.active_iterations,instances.reserved_instances, \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.event_type='collection.finished'), \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.event_type='run.finished') \
             FROM runs run JOIN iteration_scopes scope \
               ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
             JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
               AND owner.node=scope.owner_node AND owner.instance_path_nodes=scope.owner_path_nodes \
               AND owner.instance_path_indices=scope.owner_path_indices \
             JOIN tenant_iteration_usage usage ON usage.tenant_id=run.tenant_id \
             JOIN run_instance_usage instances \
               ON instances.tenant_id=run.tenant_id AND instances.run_id=run.id \
             WHERE run.tenant_id=$1 AND run.id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("terminal quota drain");
    assert_eq!(
        (terminal.0.as_str(), terminal.1.as_str()),
        ("FAILED", "FAILED")
    );
    assert_eq!(terminal.2.as_deref(), Some("STORAGE_QUOTA_EXCEEDED"));
    assert_eq!((terminal.3, terminal.4, terminal.5), (0, 0, 0));
    assert_eq!((terminal.6, terminal.7), (1, 1));

    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("scheduler shard");
    let transition = || {
        collection_transition(
            &fixture.world,
            fixture.run,
            u64::try_from(receipts.3).expect("run version"),
            owner.clone(),
            lease,
        )
    };
    assert!(
        !fixture
            .store
            .start_collection_drain(&CollectionDrain::new(
                transition(),
                CollectionScopeId::new(receipts.0),
                CollectionDrainReason::StorageQuotaExceeded,
                receipts.1,
            ))
            .await
            .expect("replay quota drain start")
            .applied
    );
    assert!(
        !fixture
            .store
            .finish_collection_drain(&CollectionDrainFinish::new(
                transition(),
                CollectionScopeId::new(receipts.0),
                receipts.2,
            ))
            .await
            .expect("replay quota drain finish")
            .applied
    );
    fixture
        .store
        .verify_run(tenant, fixture.run)
        .await
        .expect("terminal quota drain replays");
    let replay = scheduler(&fixture).tick().await;
    assert_eq!(replay.quota_stops, 0, "{replay:?}");
    assert_eq!(replay.failed, 0, "{replay:?}");
    assert_eq!(
        storage_quota_audits(&fixture, "collection storage quota exceeded").await,
        1
    );
}
