//! Tenant-bound storage evidence for the additive execution identity and collection admission.

use std::sync::Arc;

use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionEntry, CollectionLimits, CollectionScopeId, CollectionStore,
    CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::runs::RunStore;
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::{JsonIrCodec, JsonIrCodec as StructuredCodec};
use sagat_infra_postgres::PgCollectionStore;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::engine::{World, chain, ctx, world};
use crate::support::eventually;

#[derive(Debug)]
struct StructuredJsonIrCodec;

impl IrCodec for StructuredJsonIrCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        StructuredCodec.parse_structured(document)
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

fn collection_with_derived_obligations(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "limits": { "maxInstances": 16, "maxDepth": 2 },
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    {
                        "id": "work",
                        "type": "activity",
                        "activity": "line.process",
                        "compensateWith": "line.undo",
                        "reconcileWith": "line.lookup",
                        "next": "returned"
                    },
                    { "id": "returned", "type": "return", "result": "steps.work.output" }
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
                "maxItems": 2,
                "maxConcurrency": 2,
                "next": "done"
            },
            { "id": "done", "type": "end", "outputExpr": "steps.items.output" }
        ]
    })
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

async fn retain(pool: &sqlx::PgPool, tenant: Uuid, bytes: &[u8]) -> Digest {
    let digest = digest(bytes);
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
         ON CONFLICT (tenant_id,digest) DO NOTHING",
    )
    .bind(tenant)
    .bind(digest.as_bytes().as_slice())
    .bind(bytes)
    .execute(pool)
    .await
    .expect("retain collection payload");
    digest
}

async fn publish_structured(
    world: &World,
    name: &str,
) -> (WorkflowId, WorkflowVersionId, DefinitionCache) {
    let (workflow, _) = world.publish(chain(name)).await;
    let document = serde_json::to_vec(&collection_with_derived_obligations(name))
        .expect("structured definition");
    StructuredJsonIrCodec
        .parse(&document)
        .expect("valid structured definition");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id,tenant_id,workflow_id,version_number,digest,document,published_by,published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,clock_timestamp())",
    )
    .bind(version.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(world.admin.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("insert structured definition");
    (
        workflow,
        version,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
}

async fn collection_run(world: &World, workflow: WorkflowId, version: WorkflowVersionId) -> RunId {
    let run = RunId::generate();
    let tenant = world.admin.tenant_id().as_uuid();
    let input = retain(&world.db.pool, tenant, br#"{"items":[1,2]}"#).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let now = crate::support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version, \
          input_digest,started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,'collection-security', \
                 '0af7651916cd43dd8448eb211c80319c',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("insert collection run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,1,'run.started',jsonb_build_object('entrypoint','items'), \
                 'collection-security',$3)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("insert run start");
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
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("insert collection owner");
    run
}

async fn authority(world: &World, run: RunId) -> SchedulerAuthority {
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET owner='collection-security',generation=generation+1, \
                acquired_at=clock_timestamp(),expires_at=clock_timestamp()+interval '1 hour' \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("acquire collection shard");
    SchedulerAuthority::for_run(
        "collection-security",
        run,
        u64::try_from(generation).expect("generation"),
    )
}

fn transition(
    world: &World,
    run: RunId,
    version: u64,
    scheduler: SchedulerAuthority,
) -> CollectionTransition {
    CollectionTransition::new(
        world.admin.tenant_id(),
        world.namespace.id(),
        run,
        version,
        ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
        scheduler,
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
        None,
    )
}

async fn enter(
    world: &World,
    store: &PgCollectionStore,
    run: RunId,
    scheduler: SchedulerAuthority,
) -> (CollectionScopeId, u64) {
    let scope = CollectionScopeId::generate();
    let snapshot = retain(&world.db.pool, world.admin.tenant_id().as_uuid(), b"[1,2]").await;
    let written = store
        .enter_collection(
            &CollectionEntry::new(
                transition(world, run, 1, scheduler),
                scope,
                2,
                snapshot,
                CollectionLimits::new(2, 2, 16, 2).expect("limits"),
            )
            .expect("collection entry"),
        )
        .await
        .expect("enter collection");
    (scope, written.run_version)
}

#[tokio::test]
async fn scoped_identity_storage_rejects_foreign_tenant_keys_and_reads_look_missing() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("tenant-bound-instance")).await;
    let run = w.start(workflow, &json!({})).await;
    let owner = w.admin.tenant_id();
    let foreign = w.other_admin.tenant_id();

    let owner_graph = w
        .runs
        .graph(&w.admin, run, &ctx())
        .await
        .expect("owner graph");
    assert!(!owner_graph.steps.is_empty());
    let root_only: bool = sqlx::query_scalar(
        "SELECT bool_and(cardinality(instance_path_nodes) = 0 \
                         AND cardinality(instance_path_indices) = 0) \
         FROM steps WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(owner.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("root identity projection");
    assert!(
        root_only,
        "the current fixture is an old-compatible root run"
    );
    let foreign_read = w.runs.graph(&w.other_admin, run, &ctx()).await;
    let absent_read = w
        .runs
        .graph(
            &w.other_admin,
            sagat_domain::execution::RunId::generate(),
            &ctx(),
        )
        .await;
    assert_eq!(foreign_read.map(|_| ()), Err(AppError::NotFound));
    assert_eq!(absent_read.map(|_| ()), Err(AppError::NotFound));
    assert!(
        RunStore::graph(&w.store, foreign, run)
            .await
            .expect("tenant-filtered graph lookup")
            .is_none()
    );

    let foreign_step = sqlx::query(
        "INSERT INTO steps \
         SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
             'tenant_id', $3::uuid, 'step_id', $4::uuid, \
             'instance_path_nodes', jsonb_build_array('items'), \
             'instance_path_indices', jsonb_build_array(0) \
         ))).* \
         FROM steps s \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND cardinality(s.instance_path_nodes) = 0 \
         LIMIT 1",
    )
    .bind(owner.as_uuid())
    .bind(run.as_uuid())
    .bind(foreign.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect_err("a scoped step cannot point across the tenant/run boundary");
    assert_eq!(
        foreign_step
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    let foreign_event = sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, 999, 'step.released', 'a', '{}'::jsonb, 'foreign-fixture', \
                 clock_timestamp(), $3, ARRAY['items'], ARRAY[0])",
    )
    .bind(foreign.as_uuid())
    .bind(run.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect_err("a scoped journal event cannot point across the tenant/run boundary");
    assert_eq!(
        foreign_event
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    let foreign_rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = $2) + \
                (SELECT count(*) FROM run_events WHERE tenant_id = $1 AND run_id = $2)",
    )
    .bind(foreign.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("foreign storage count");
    assert_eq!(foreign_rows, 0);

    let audited = eventually("the cross-tenant run read is audited", || async {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records \
             WHERE tenant_id = $1 AND action = 'access.denied.cross_tenant' \
               AND resource_type = 'run' AND resource_id = $2",
        )
        .bind(foreign.as_uuid())
        .bind(run.to_string())
        .fetch_one(&w.db.pool)
        .await
        .expect("cross-tenant audit count");
        (count > 0).then_some(count)
    })
    .await;
    assert_eq!(
        audited, 1,
        "the denial is charged only to the caller tenant"
    );

    w.assert_consistent(run).await;
}

#[tokio::test]
async fn collection_admission_derives_its_reservation_from_the_pinned_definition() {
    let world = world().await;
    let (workflow, version, definitions) =
        publish_structured(&world, "collection-derived-reserve").await;
    let run = collection_run(&world, workflow, version).await;
    let scheduler = authority(&world, run).await;
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,2,'collection-security')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("collection capacity");
    let store = PgCollectionStore::with_definition_cache(world.db.pool.clone(), definitions);
    let (scope, entered_version) = enter(&world, &store, run, scheduler.clone()).await;
    let path = InstancePath::root()
        .child(NodeId::parse("items").expect("owner"), 0)
        .expect("iteration path");
    let entry = ExecutionInstanceId::in_path(path, NodeId::parse("work").expect("entry"));
    let item = digest(b"1");
    let before: (i32, i32, i64) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances, \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2) \
         FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("usage before admission");
    let insufficient = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&world, run, entered_version, scheduler.clone()),
                scope,
                0,
                entry.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                Uuid::now_v7(),
            )
            .expect("under-reserved admission input"),
        )
        .await
        .expect_err("definition-derived obligations cannot be under-reserved");
    assert_eq!(insufficient, PrimitiveStoreError::InvalidTransition);
    let unchanged: (i32, i32, i64) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances, \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2) \
         FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("usage after rejected admission");
    assert_eq!(unchanged, before);

    let accepted = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&world, run, entered_version, scheduler),
                scope,
                0,
                entry,
                ActivityName::parse("line.process").expect("activity"),
                item,
                3,
                Uuid::now_v7(),
            )
            .expect("fully reserved admission"),
        )
        .await
        .expect("admit with definition-derived reserve");
    assert!(accepted.applied);
    let reserved: (i32, i32, i32) = sqlx::query_as(
        "SELECT usage.admitted_instances,usage.reserved_instances,iteration.remaining_reservations \
         FROM run_instance_usage usage JOIN collection_iterations iteration \
           ON iteration.tenant_id=usage.tenant_id AND iteration.run_id=usage.run_id \
         WHERE usage.tenant_id=$1 AND usage.run_id=$2 AND iteration.iteration_index=0",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("definition-derived reservation");
    assert_eq!(reserved, (before.0 + 1, before.1 + 3, 3));
}

#[tokio::test]
async fn tenant_collection_capacity_serializes_admission_across_runs() {
    let world = world().await;
    let (workflow, version, definitions) =
        publish_structured(&world, "collection-cross-run-capacity").await;
    let first_run = collection_run(&world, workflow, version).await;
    let second_run = collection_run(&world, workflow, version).await;
    let first_authority = authority(&world, first_run).await;
    let second_authority = authority(&world, second_run).await;
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,1,'collection-security')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("single tenant slot");
    let store = PgCollectionStore::with_definition_cache(world.db.pool.clone(), definitions);
    let (first_scope, first_version) =
        enter(&world, &store, first_run, first_authority.clone()).await;
    let (second_scope, second_version) =
        enter(&world, &store, second_run, second_authority.clone()).await;
    let owner = NodeId::parse("items").expect("owner");
    let entry_node = NodeId::parse("work").expect("entry");
    let admission = |run, version, scope, scheduler, index| {
        let path = InstancePath::root()
            .child(owner.clone(), index)
            .expect("iteration path");
        CollectionAdmission::new(
            transition(&world, run, version, scheduler),
            scope,
            index,
            ExecutionInstanceId::in_path(path, entry_node.clone()),
            ActivityName::parse("line.process").expect("activity"),
            digest(if index == 0 { b"1" } else { b"2" }),
            3,
            Uuid::now_v7(),
        )
        .expect("admission")
    };
    store
        .admit_collection_activity(&admission(
            first_run,
            first_version,
            first_scope,
            first_authority,
            0,
        ))
        .await
        .expect("first run takes tenant slot");
    let blocked = store
        .admit_collection_activity(&admission(
            second_run,
            second_version,
            second_scope,
            second_authority,
            0,
        ))
        .await
        .expect_err("second run cannot exceed tenant capacity");
    assert_eq!(blocked, PrimitiveStoreError::AtCapacity);
    let projection: (i32, i64, i64) = sqlx::query_as(
        "SELECT active_iterations, \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$3) \
         FROM tenant_iteration_usage WHERE tenant_id=$1",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(first_run.as_uuid())
    .bind(second_run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("cross-run capacity projection");
    assert_eq!(projection, (1, 1, 0));
}
