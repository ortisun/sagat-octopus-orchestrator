//! Real PostgreSQL evidence for scoped worker leases and reports.

use std::{sync::Arc, time::Duration};

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_application::primitives::{
    CollectionAdmission, CollectionEntry, CollectionLimits, CollectionScopeId, CollectionStore,
    CollectionTransition, SchedulerAuthority,
};
use sagat_application::runs::RunStore;
use sagat_application::shards::Lease;
use sagat_application::telemetry::all_attempts;
use sagat_application::worker::{
    Capabilities, ClaimedTask, TaskRef, WorkerService, WorkerSettings,
};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
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

use crate::support::engine::{Session, World, chain, task_ref, world};
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
        let collection_store = PgCollectionStore::new(world.db.pool.clone());
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
