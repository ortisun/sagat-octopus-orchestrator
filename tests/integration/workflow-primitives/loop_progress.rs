//! Real PostgreSQL evidence for sequential-loop checkpoint and delay transactions.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, Engine, RunTx, instance_step_id};
use sagat_application::loop_progress::{
    LoopCheckpointCommand, LoopCheckpointEvaluation, LoopDelayElapsedCommand, LoopDrainCommand,
    LoopDrainFinishCommand, LoopEntryCommand, LoopIterationDrainCommand, LoopNextActivity,
    LoopProgressError, LoopProgressStore, LoopTransition,
};
use sagat_application::maintenance::Window;
use sagat_application::primitives::SchedulerAuthority;
use sagat_application::runs::{Lock, RunStore};
use sagat_application::worker::{
    Capabilities, ClaimedTask, Registered, WorkerService, WorkerSettings,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ExecutionInstanceId, FailureCode, InstancePath, LoopContinuation, LoopDrainReason, LoopKind,
    LoopPhase, RunId, RunStatus, StepStatus, WaitingOn,
};
use sagat_domain::identity::{Principal, Subject};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{MIGRATOR, PgLoopProgressStore, PgRunStore, migrate};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{chain, task_ref, world};

const BEFORE_LOOP_PROGRESS: i64 = 20_261_030_000_000;
const BEFORE_LOOP_ENTRY_RECEIPTS: i64 = 20_261_103_000_000;
const BEFORE_LOOP_DRAIN: i64 = 20_261_106_000_000;

type IterationLimitProjection = (
    String,
    Option<String>,
    Option<String>,
    String,
    bool,
    Option<String>,
    String,
    i32,
    i64,
);

type LoopEntryProjection = (
    String,
    Option<String>,
    String,
    String,
    i32,
    i32,
    i32,
    i64,
    i64,
    i64,
    i64,
);

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

fn micros(value: OffsetDateTime) -> i64 {
    i64::try_from(value.unix_timestamp_nanos() / 1_000).expect("timestamp fits journal encoding")
}

fn instance_json(instance: &ExecutionInstanceId) -> serde_json::Value {
    json!({
        "path": instance.path().segments().iter().map(|segment| json!({
            "scope": segment.scope_node().as_str(),
            "index": segment.iteration_index(),
        })).collect::<Vec<_>>(),
        "node": instance.node().as_str(),
    })
}

fn path_parts(instance: &ExecutionInstanceId) -> (Vec<String>, Vec<i32>) {
    instance
        .path()
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).expect("bounded iteration"),
            )
        })
        .unzip()
}

fn loop_workflow_with(name: &str, loop_type: &str, max_iterations: u32) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "poll",
        "bodies": {
            "iteration": {
                "entrypoint": "work",
                "nodes": [
                    {
                        "id": "work",
                        "type": "activity",
                        "activity": "poll.check",
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
                "id": "poll",
                "type": loop_type,
                "condition": "carry < 3",
                "body": "iteration",
                "initial": "0",
                "carry": "bodyResult",
                "maxIterations": max_iterations,
                "timeoutMs": 60000,
                "delayMs": 1,
                "next": "done"
            },
            { "id": "done", "type": "end", "outputExpr": "steps.poll.output" }
        ]
    })
}

async fn retain(pool: &sqlx::PgPool, tenant: Uuid, content: &[u8]) -> Digest {
    let digest = digest(content);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(digest.as_bytes().as_slice())
    .bind(content)
    .execute(pool)
    .await
    .expect("retain payload");
    digest
}

async fn scheduler(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "loop-progress-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation = generation + 1, owner=$2, \
                expires_at=clock_timestamp() + interval '1 hour', acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("scheduler fence");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("nonnegative generation"),
    )
}

struct LoopFixture {
    db: crate::support::TestDb,
    tenant: Uuid,
    namespace: sagat_domain::identity::NamespaceId,
    run: RunId,
    owner: ExecutionInstanceId,
    entry: ExecutionInstanceId,
    returned: ExecutionInstanceId,
    definitions: DefinitionCache,
    authority: SchedulerAuthority,
    initial_carry: Digest,
    worker: Principal,
}

type LoopWorkers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

async fn lease_loop_member(fixture: &LoopFixture) -> (LoopWorkers, Registered, ClaimedTask) {
    let workers = WorkerService::new(
        PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
            .with_scoped_worker_protocol(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let registered = workers
        .register(
            &fixture.worker,
            "payments",
            &format!("loop-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec!["poll.check".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 1,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register scoped loop worker");
    assert_eq!(registered.accepted_protocol_version, "1.5");
    let mut tasks = workers
        .poll(
            &fixture.worker,
            registered.worker_id,
            registered.session_token.expose(),
            1,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("lease loop member");
    assert_eq!(tasks.len(), 1);
    let task = tasks.pop().expect("one loop member");
    assert_eq!(task.instance, fixture.entry);
    (workers, registered, task)
}

async fn seed_running_loop(name: &str) -> LoopFixture {
    seed_running_loop_with_limit(name, 3).await
}

async fn seed_leasable_loop(name: &str) -> LoopFixture {
    seed_loop_fixture(name, 3, true).await
}

async fn seed_running_loop_with_limit(name: &str, max_iterations: u32) -> LoopFixture {
    seed_loop_fixture(name, max_iterations, false).await
}

async fn seed_loop_fixture(name: &str, max_iterations: u32, member_ready: bool) -> LoopFixture {
    let w = world().await;
    let worker = w.worker.clone();
    let tenant = w.admin.tenant_id().as_uuid();
    let (workflow, _) = w.publish(chain(name)).await;
    let document = serde_json::to_vec(&loop_workflow_with(name, "while", max_iterations))
        .expect("loop document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(tenant)
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(w.admin.subject().as_str())
    .bind(crate::support::now())
    .execute(&w.db.pool)
    .await
    .expect("structured loop version");
    let run = RunId::generate();
    let owner = ExecutionInstanceId::root(NodeId::parse("poll").expect("owner"));
    let member = InstancePath::root()
        .child(owner.node().clone(), 0)
        .expect("first iteration");
    let entry =
        ExecutionInstanceId::in_path(member.clone(), NodeId::parse("work").expect("entry node"));
    let returned =
        ExecutionInstanceId::in_path(member, NodeId::parse("returned").expect("return node"));
    let input = retain(&w.db.pool, tenant, b"{}").await;
    let initial_carry = retain(&w.db.pool, tenant, b"0").await;
    let now = crate::support::now();
    let deadline = now + Duration::minutes(1);
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version,input_digest, \
          started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',$6,$7,'loop-fixture', \
                 '0af7651916cd43dd8448eb211c80319c',$8,$8,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(if member_ready { 4_i64 } else { 8_i64 })
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("loop run");
    let (entry_nodes, entry_indices) = path_parts(&entry);
    if member_ready {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,waiting_kind,updated_at, \
              instance_path_nodes,instance_path_indices,activity) \
             VALUES ($1,$2,'poll',$3,$4,'WAITING','LOOP',$5,'{}','{}',NULL), \
                    ($1,$2,'work',$6,$4,'READY',NULL,$5,$7,$8,'poll.check')",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(instance_step_id(version, run, &owner))
        .bind(w.namespace.id().as_uuid())
        .bind(now)
        .bind(instance_step_id(version, run, &entry))
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .execute(&w.db.pool)
        .await
        .expect("leasable loop steps");
    } else {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,waiting_kind,updated_at, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,'poll',$3,$4,'WAITING','LOOP',$5,'{}','{}'), \
                    ($1,$2,'work',$6,$4,'SUCCEEDED',NULL,$5,$7,$8), \
                    ($1,$2,'returned',$9,$4,'PENDING',NULL,$5,$7,$8)",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(instance_step_id(version, run, &owner))
        .bind(w.namespace.id().as_uuid())
        .bind(now)
        .bind(instance_step_id(version, run, &entry))
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .bind(instance_step_id(version, run, &returned))
        .execute(&w.db.pool)
        .await
        .expect("completed loop steps");
        sqlx::query(
            "UPDATE steps SET attempt=1,generation=1,routed=true \
             WHERE tenant_id=$1 AND run_id=$2 AND node='work' AND instance_path_nodes=$3 \
               AND instance_path_indices=$4",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .execute(&w.db.pool)
        .await
        .expect("completed body activity");
    }
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,2,'loop-test') \
         ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations=2, updated_by='loop-test'",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("loop capacity policy");
    sqlx::query("UPDATE tenant_iteration_usage SET active_iterations=1 WHERE tenant_id=$1")
        .bind(tenant)
        .execute(&w.db.pool)
        .await
        .expect("active loop usage");
    sqlx::query(
        "UPDATE run_instance_usage SET admitted_instances=2,reserved_instances=1 \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("loop run usage");
    sqlx::query(
        "INSERT INTO loop_scopes \
         (tenant_id,namespace_id,run_id,owner_instance_id,owner_path_nodes,owner_path_indices, \
          owner_node,kind,lifecycle,max_iterations,deadline_at,initial_carry_digest, \
          current_carry_digest,initial_condition,next_iteration,active_iteration,entered_sequence, \
          created_at,updated_at) \
         VALUES ($1,$2,$3,$4,'{}','{}','poll','WHILE','RUNNING',$5,$6,$7,$7,true,0,0,2,$8,$8)",
    )
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(i32::try_from(max_iterations).expect("bounded loop limit"))
    .bind(deadline)
    .bind(initial_carry.as_bytes().as_slice())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("loop scope");
    sqlx::query(
        "INSERT INTO loop_iterations \
         (tenant_id,run_id,owner_instance_id,iteration_index,entry_instance_id,entry_path_nodes, \
          entry_path_indices,entry_node,activity,lifecycle,slot_held,reserved_instances, \
          remaining_reservations,admitted_sequence,created_at,updated_at) \
         VALUES ($1,$2,$3,0,$4,$5,$6,'work','poll.check','ADMITTED',true,1,1,3,$7,$7)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(instance_step_id(version, run, &entry))
    .bind(&entry_nodes)
    .bind(&entry_indices)
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("active loop iteration");

    let owner_id = instance_step_id(version, run, &owner);
    let entry_id = instance_step_id(version, run, &entry);
    let event_count = if member_ready { 4 } else { 8 };
    for (sequence, event_type, node, detail, instance, nodes, indices) in [
        (
            1_i64,
            "run.started",
            None,
            json!({"entrypoint": "poll"}),
            None,
            None,
            None,
        ),
        (
            2,
            "loop.entered",
            Some("poll"),
            json!({
                "kind": "while",
                "max_iterations": max_iterations,
                "entered_at_us": micros(now),
                "deadline_us": micros(deadline),
                "initial_carry_digest": initial_carry.to_hex(),
                "initial_condition": true,
            }),
            Some(owner_id),
            Some(Vec::<String>::new()),
            Some(Vec::<i32>::new()),
        ),
        (
            3,
            "loop.iteration_admitted",
            Some("poll"),
            json!({"index": 0, "entry": instance_json(&entry)}),
            Some(owner_id),
            Some(Vec::<String>::new()),
            Some(Vec::<i32>::new()),
        ),
        (
            4,
            "step.released",
            Some("work"),
            json!({}),
            Some(entry_id),
            Some(entry_nodes.clone()),
            Some(entry_indices.clone()),
        ),
        (
            5,
            "step.leased",
            Some("work"),
            json!({"attempt": 1, "generation": 1}),
            Some(entry_id),
            Some(entry_nodes.clone()),
            Some(entry_indices.clone()),
        ),
        (
            6,
            "step.started",
            Some("work"),
            json!({"attempt": 1, "generation": 1}),
            Some(entry_id),
            Some(entry_nodes.clone()),
            Some(entry_indices.clone()),
        ),
        (
            7,
            "step.completed",
            Some("work"),
            json!({"attempt": 1, "generation": 1}),
            Some(entry_id),
            Some(entry_nodes.clone()),
            Some(entry_indices.clone()),
        ),
        (
            8,
            "step.routed",
            Some("work"),
            json!({"activated": ["returned"]}),
            Some(entry_id),
            Some(entry_nodes),
            Some(entry_indices),
        ),
    ]
    .into_iter()
    .take(event_count)
    {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,detail,actor,occurred_at,instance_id, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,$6,'loop-fixture',$7,$8,$9,$10)",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind(node)
        .bind(detail)
        .bind(now)
        .bind(instance)
        .bind(nodes)
        .bind(indices)
        .execute(&w.db.pool)
        .await
        .expect("loop journal fixture");
    }
    let authority = scheduler(&w.db.pool, run).await;
    LoopFixture {
        db: w.db,
        tenant,
        namespace: w.namespace.id(),
        run,
        owner,
        entry,
        returned,
        definitions: DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        authority,
        initial_carry,
        worker,
    }
}

async fn seed_pending_loop(name: &str) -> LoopFixture {
    seed_pending_loop_with_kind(name, "while").await
}

async fn seed_pending_loop_with_kind(name: &str, loop_type: &str) -> LoopFixture {
    let w = world().await;
    let worker = w.worker.clone();
    let tenant = w.admin.tenant_id().as_uuid();
    let (workflow, _) = w.publish(chain(name)).await;
    let document =
        serde_json::to_vec(&loop_workflow_with(name, loop_type, 3)).expect("loop document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(tenant)
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(w.admin.subject().as_str())
    .bind(crate::support::now())
    .execute(&w.db.pool)
    .await
    .expect("structured loop version");
    let run = RunId::generate();
    let owner = ExecutionInstanceId::root(NodeId::parse("poll").expect("owner"));
    let member = InstancePath::root()
        .child(owner.node().clone(), 0)
        .expect("first iteration");
    let entry =
        ExecutionInstanceId::in_path(member.clone(), NodeId::parse("work").expect("entry node"));
    let returned =
        ExecutionInstanceId::in_path(member, NodeId::parse("returned").expect("return node"));
    let input = retain(&w.db.pool, tenant, b"{}").await;
    let initial_carry = retain(&w.db.pool, tenant, b"0").await;
    let now = crate::support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version,input_digest, \
          started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,'loop-entry-fixture', \
                 '0af7651916cd43dd8448eb211c80319c',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("pending loop run");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'poll',$3,$4,'PENDING',$5,'{}','{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(w.namespace.id().as_uuid())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("pending loop owner");
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,2,'loop-test') \
         ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations=2, updated_by='loop-test'",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("loop capacity policy");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,1,'run.started',$3,'loop-entry-fixture',$4)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(json!({"entrypoint": "poll"}))
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("run start journal");
    let authority = scheduler(&w.db.pool, run).await;
    LoopFixture {
        db: w.db,
        tenant,
        namespace: w.namespace.id(),
        run,
        owner,
        entry,
        returned,
        definitions: DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        authority,
        initial_carry,
        worker,
    }
}

fn audit(fixture: &LoopFixture) -> AuditRecord {
    AuditRecord::by(
        &sagat_domain::identity::Principal::new(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            Subject::engine(),
            sagat_domain::identity::Role::Admin,
        ),
        AuditAction::RunUpdated,
        ResourceType::Run,
        fixture.run.to_string(),
        None,
        crate::support::now(),
    )
}

fn transition(
    fixture: &LoopFixture,
    version: u64,
    authority: SchedulerAuthority,
    audit: AuditRecord,
    occurred_at: OffsetDateTime,
) -> LoopTransition {
    LoopTransition::new(
        sagat_domain::identity::TenantId::new(fixture.tenant),
        fixture.namespace,
        fixture.run,
        version,
        fixture.owner.clone(),
        authority,
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        occurred_at,
        audit,
    )
}

fn next_activity(fixture: &LoopFixture, index: u32) -> LoopNextActivity {
    LoopNextActivity::new(
        ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(fixture.owner.node().clone(), index)
                .expect("next iteration"),
            NodeId::parse("work").expect("activity node"),
        ),
        ActivityName::parse("poll.check").expect("activity"),
        1,
    )
    .expect("next activity")
}

fn entry_command(
    fixture: &LoopFixture,
    authority: SchedulerAuthority,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
) -> LoopEntryCommand {
    LoopEntryCommand::from_carry(
        transition(fixture, 1, authority, audit(fixture), occurred_at),
        LoopKind::While,
        3,
        occurred_at + Duration::minutes(1),
        b"0".to_vec(),
        Some(true),
        Some(next_activity(fixture, 0)),
        outbox_id,
    )
    .expect("loop entry command")
}

async fn run_version(fixture: &LoopFixture) -> u64 {
    let value: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.tenant)
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("run version");
    u64::try_from(value).expect("nonnegative run version")
}

fn delay_checkpoint(
    fixture: &LoopFixture,
    outbox_id: Uuid,
    timer_id: Uuid,
    audit: AuditRecord,
) -> LoopCheckpointCommand {
    let checked_at = crate::support::now() - Duration::seconds(2);
    LoopCheckpointCommand::new(
        transition(
            fixture,
            8,
            fixture.authority.clone(),
            audit,
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        true,
        checked_at,
        LoopContinuation::DelayUntil(checked_at + Duration::seconds(1)),
        None,
        Some(timer_id),
        outbox_id,
    )
    .expect("delay checkpoint")
}

#[tokio::test]
async fn loop_entry_is_atomic_restart_safe_and_exactly_idempotent() {
    let fixture = seed_pending_loop("loop-entry-atomic").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let carry = b"1".to_vec();
    let carry_digest = digest(&carry);
    let occurred_at = crate::support::now();
    let command = LoopEntryCommand::from_carry(
        transition(
            &fixture,
            1,
            fixture.authority.clone(),
            audit(&fixture),
            occurred_at,
        ),
        LoopKind::While,
        3,
        occurred_at + Duration::minutes(1),
        carry,
        Some(true),
        Some(next_activity(&fixture, 0)),
        Uuid::now_v7(),
    )
    .expect("loop entry command");

    let applied = store.enter_loop(&command).await.expect("enter loop");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 4);
    let retained: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id=$1 AND digest=$2")
            .bind(fixture.tenant)
            .bind(carry_digest.as_bytes().as_slice())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("initial carry retained with entry");
    assert_eq!(retained, b"1");
    let replay = store.enter_loop(&command).await.expect("entry replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 4);
    let mut mismatch = command.clone();
    mismatch.outbox_id = Uuid::now_v7();
    assert_eq!(
        store.enter_loop(&mismatch).await,
        Err(LoopProgressError::DuplicateMismatch)
    );

    let projection: LoopEntryProjection = sqlx::query_as(
        "SELECT owner.status,owner.waiting_kind,entry.status,scope.lifecycle, \
                    tenant_usage.active_iterations,run_usage.admitted_instances, \
                    run_usage.reserved_instances, \
                    (SELECT count(*) FROM run_events WHERE tenant_id=scope.tenant_id \
                       AND run_id=scope.run_id), \
                    (SELECT count(*) FROM event_outbox WHERE tenant_id=scope.tenant_id \
                       AND aggregate_id=scope.run_id AND event_type='task.ready'), \
                    (SELECT count(*) FROM audit_records WHERE tenant_id=scope.tenant_id \
                       AND resource_id=scope.run_id::text), \
                    (SELECT head_seq FROM telemetry_pending WHERE tenant_id=scope.tenant_id \
                       AND run_id=scope.run_id) \
             FROM loop_scopes scope \
             JOIN loop_iterations iteration USING (tenant_id,run_id,owner_instance_id) \
             JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
               AND owner.step_id=scope.owner_instance_id \
             JOIN steps entry ON entry.tenant_id=iteration.tenant_id \
               AND entry.run_id=iteration.run_id AND entry.step_id=iteration.entry_instance_id \
             JOIN tenant_iteration_usage tenant_usage \
               ON tenant_usage.tenant_id=scope.tenant_id \
             JOIN run_instance_usage run_usage \
               ON run_usage.tenant_id=scope.tenant_id AND run_usage.run_id=scope.run_id \
             WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND iteration.iteration_index=0",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("loop entry projection");
    assert_eq!(
        projection,
        (
            "WAITING".into(),
            Some("LOOP".into()),
            "READY".into(),
            "RUNNING".into(),
            1,
            2,
            1,
            4,
            1,
            1,
            4,
        )
    );

    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let mut reopened = verifier
        .begin(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
            Lock::Wait,
        )
        .await
        .expect("reopen loop run")
        .expect("stored loop run");
    let state = reopened.parts().1;
    assert_eq!(state.version(), 4);
    assert!(matches!(
        state
            .instance(&fixture.owner)
            .and_then(|step| step.waiting_on()),
        Some(WaitingOn::Loop)
    ));
    assert_eq!(
        state
            .instance(&fixture.entry)
            .map(sagat_domain::execution::StepState::status),
        Some(StepStatus::Ready)
    );
    let loop_state = state.loop_state(&fixture.owner).expect("restored loop");
    assert_eq!(loop_state.phase(), LoopPhase::Running);
    assert_eq!(loop_state.next_iteration(), 0);
    assert_eq!(loop_state.current_carry_digest(), carry_digest);
    drop(reopened);
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("journal and loop entry projection agree");
    let error = MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_ENTRY_RECEIPTS)
        .await
        .expect_err("retained loop entry receipt must block downgrade");
    assert!(
        error
            .to_string()
            .contains("loop entry receipts are retained")
    );
}

#[tokio::test]
async fn loop_discovery_filters_unrelated_pending_roots_before_pagination() {
    let fixture = seed_pending_loop("loop-discovery-page").await;
    for index in 0..16 {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
              instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,$4,$5,'PENDING',$6 - interval '1 day','{}','{}')",
        )
        .bind(fixture.tenant)
        .bind(fixture.run.as_uuid())
        .bind(format!("noise-{index}"))
        .bind(Uuid::now_v7())
        .bind(fixture.namespace.as_uuid())
        .bind(crate::support::now())
        .execute(&fixture.db.pool)
        .await
        .expect("unrelated pending root");
    }

    let candidates = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
        .loop_candidates(&Window::everything(1))
        .await
        .expect("loop candidates");
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].step.run_id, fixture.run);
    assert_eq!(candidates[0].step.node.as_str(), "poll");
}

#[tokio::test]
async fn false_while_finishes_without_admitting_a_body_and_replays_exactly() {
    let fixture = seed_pending_loop("loop-entry-false").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let occurred_at = crate::support::now();
    let false_while = LoopEntryCommand::new(
        transition(
            &fixture,
            1,
            fixture.authority.clone(),
            audit(&fixture),
            occurred_at,
        ),
        LoopKind::While,
        3,
        occurred_at + Duration::minutes(1),
        fixture.initial_carry,
        Some(false),
        None,
        Uuid::now_v7(),
    )
    .expect("false while decision");
    let applied = store.enter_loop(&false_while).await.expect("finish loop");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 5);
    let replay = store.enter_loop(&false_while).await.expect("entry replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 5);
    let projection: (String, String, bool, String, i64, i32, i64, i64) = sqlx::query_as(
        "SELECT run.status,owner.status,owner.routed,scope.lifecycle, \
                (SELECT count(*) FROM loop_iterations WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id),usage.active_iterations, \
                (SELECT count(*) FROM run_events WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=scope.tenant_id \
                   AND aggregate_id=scope.run_id AND event_type='run.finished') \
         FROM loop_scopes scope \
         JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=scope.tenant_id \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("terminal loop projection");
    assert_eq!(
        projection,
        (
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            "SUCCEEDED".into(),
            0,
            0,
            5,
            1,
        )
    );
    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("false while journal and projection agree");
}

#[tokio::test]
async fn repeat_until_always_admits_its_first_body_before_checking() {
    let fixture = seed_pending_loop_with_kind("loop-entry-repeat", "repeatUntil").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let occurred_at = crate::support::now();
    let command = LoopEntryCommand::new(
        transition(
            &fixture,
            1,
            fixture.authority.clone(),
            audit(&fixture),
            occurred_at,
        ),
        LoopKind::RepeatUntil,
        3,
        occurred_at + Duration::minutes(1),
        fixture.initial_carry,
        None,
        Some(next_activity(&fixture, 0)),
        Uuid::now_v7(),
    )
    .expect("repeat-until entry");
    let applied = store
        .enter_loop(&command)
        .await
        .expect("enter repeat-until");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 4);
    let projection: (String, String, i64) = sqlx::query_as(
        "SELECT scope.kind,scope.lifecycle, \
                (SELECT count(*) FROM loop_iterations WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id AND owner_instance_id=scope.owner_instance_id) \
         FROM loop_scopes scope WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("repeat-until projection");
    assert_eq!(projection, ("REPEAT_UNTIL".into(), "RUNNING".into(), 1));
    PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("repeat-until journal and projection agree");
}

#[tokio::test]
async fn stale_fence_and_outbox_failure_leave_entry_unchanged() {
    let fixture = seed_pending_loop("loop-entry-rollback").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());

    let stale = SchedulerAuthority::for_run(
        fixture.authority.owner(),
        fixture.run,
        fixture.authority.lease().generation + 1,
    );
    let stale_command = entry_command(&fixture, stale, Uuid::now_v7(), crate::support::now());
    assert_eq!(
        store.enter_loop(&stale_command).await,
        Err(LoopProgressError::SchedulerFenceLost)
    );

    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id,resource,max_value,granted_by) \
         VALUES ($1,'iteration_concurrency',0,'loop-test') \
         ON CONFLICT (tenant_id,resource) DO UPDATE SET max_value=0,granted_by='loop-test'",
    )
    .bind(fixture.tenant)
    .execute(&fixture.db.pool)
    .await
    .expect("restrict tenant iteration quota");
    let quota_command = entry_command(
        &fixture,
        fixture.authority.clone(),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        store.enter_loop(&quota_command).await,
        Err(LoopProgressError::AtCapacity)
    );
    sqlx::query(
        "DELETE FROM quota_ceilings WHERE tenant_id=$1 AND resource='iteration_concurrency'",
    )
    .bind(fixture.tenant)
    .execute(&fixture.db.pool)
    .await
    .expect("restore tenant iteration quota");

    sqlx::query("UPDATE run_instance_usage SET max_instances=2 WHERE tenant_id=$1 AND run_id=$2")
        .bind(fixture.tenant)
        .bind(fixture.run.as_uuid())
        .execute(&fixture.db.pool)
        .await
        .expect("restrict loop instance budget");
    let budget_command = entry_command(
        &fixture,
        fixture.authority.clone(),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        store.enter_loop(&budget_command).await,
        Err(LoopProgressError::InstanceBudgetExceeded)
    );
    sqlx::query(
        "UPDATE run_instance_usage SET max_instances=100000 WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("restore loop instance budget");

    let retained_bytes: i64 = sqlx::query_scalar(
        "SELECT content_bytes + content_reserved_bytes FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(fixture.tenant)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("retained usage");
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id,resource,max_value,granted_by) \
         VALUES ($1,'tenant_content_bytes',$2,'loop-test') \
         ON CONFLICT (tenant_id,resource) DO UPDATE SET max_value=$2,granted_by='loop-test'",
    )
    .bind(fixture.tenant)
    .bind(retained_bytes)
    .execute(&fixture.db.pool)
    .await
    .expect("restrict retained content quota");
    let quota_carry = b"40".to_vec();
    let quota_digest = digest(&quota_carry);
    let occurred_at = crate::support::now();
    let content_quota_command = LoopEntryCommand::from_carry(
        transition(
            &fixture,
            1,
            fixture.authority.clone(),
            audit(&fixture),
            occurred_at,
        ),
        LoopKind::While,
        3,
        occurred_at + Duration::minutes(1),
        quota_carry,
        Some(true),
        Some(next_activity(&fixture, 0)),
        Uuid::now_v7(),
    )
    .expect("content quota entry");
    assert!(matches!(
        store.enter_loop(&content_quota_command).await,
        Err(LoopProgressError::Storage(StoreError::QuotaExceeded { what }))
            if what == QuotaResource::TenantContentBytes.limit_name()
    ));
    let quota_projection: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM payloads WHERE tenant_id=$1 AND digest=$2), \
           (SELECT count(*) FROM loop_scopes WHERE tenant_id=$1 AND run_id=$3)",
    )
    .bind(fixture.tenant)
    .bind(quota_digest.as_bytes().as_slice())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("content quota rollback");
    assert_eq!(quota_projection, (0, 0));
    sqlx::query(
        "DELETE FROM quota_ceilings WHERE tenant_id=$1 AND resource='tenant_content_bytes'",
    )
    .bind(fixture.tenant)
    .execute(&fixture.db.pool)
    .await
    .expect("restore retained content quota");

    let collision = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished','{}')",
    )
    .bind(collision)
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("outbox collision");
    let collision_carry = b"41".to_vec();
    let collision_digest = digest(&collision_carry);
    let occurred_at = crate::support::now();
    let collision_command = LoopEntryCommand::from_carry(
        transition(
            &fixture,
            1,
            fixture.authority.clone(),
            audit(&fixture),
            occurred_at,
        ),
        LoopKind::While,
        3,
        occurred_at + Duration::minutes(1),
        collision_carry,
        Some(true),
        Some(next_activity(&fixture, 0)),
        collision,
    )
    .expect("entry with inline carry");
    assert!(matches!(
        store.enter_loop(&collision_command).await,
        Err(LoopProgressError::Storage(_))
    ));
    let unchanged: (i64, i64, String, i32, i32) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='poll'), \
           (SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1), \
           (SELECT reserved_instances FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2)",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("rolled back loop entry");
    assert_eq!(unchanged, (0, 1, "PENDING".into(), 0, 0));
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id=$1 AND digest=$2")
            .bind(fixture.tenant)
            .bind(collision_digest.as_bytes().as_slice())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("rolled back inline entry carry");
    assert_eq!(retained, 0);
}

#[tokio::test]
async fn populated_loop_schema_upgrades_and_backfills_the_initial_decision() {
    let fixture = seed_running_loop("loop-entry-migration").await;
    MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_ENTRY_RECEIPTS)
        .await
        .expect("remove entry receipts from legacy loop projection");
    migrate(&fixture.db.pool)
        .await
        .expect("upgrade populated loop schema");

    let decision: (Option<bool>, Option<Uuid>) = sqlx::query_as(
        "SELECT initial_condition,entry_outbox_id FROM loop_scopes \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("backfilled loop entry decision");
    assert_eq!(decision, (Some(true), None));
}

#[tokio::test]
async fn delay_checkpoint_restart_and_duplicate_timer_fire_are_atomic() {
    let fixture = seed_running_loop("loop-delay-atomic").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let checkpoint = delay_checkpoint(&fixture, Uuid::now_v7(), Uuid::now_v7(), audit(&fixture));
    let applied = store.checkpoint(&checkpoint).await.expect("checkpoint");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 10);
    let replay = store
        .checkpoint(&checkpoint)
        .await
        .expect("checkpoint replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 10);
    let checkpoint_state: (String, String, i32, i32, i32, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,i.lifecycle,u.active_iterations,r.admitted_instances,r.reserved_instances, \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=s.tenant_id AND aggregate_id=s.run_id), \
                (SELECT count(*) FROM audit_records WHERE tenant_id=s.tenant_id AND resource_id=s.run_id::text), \
                (SELECT count(*) FROM steps WHERE tenant_id=s.tenant_id AND run_id=s.run_id \
                   AND (status IN ('LEASED','RUNNING') OR lease_token_hash IS NOT NULL)), \
                (SELECT head_seq FROM telemetry_pending WHERE tenant_id=s.tenant_id AND run_id=s.run_id) \
         FROM loop_scopes s \
         JOIN loop_iterations i USING (tenant_id,run_id,owner_instance_id) \
         JOIN tenant_iteration_usage u USING (tenant_id) \
         JOIN run_instance_usage r USING (tenant_id,run_id) \
         WHERE s.tenant_id=$1 AND s.run_id=$2 AND i.iteration_index=0",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("checkpoint projection");
    assert_eq!(
        checkpoint_state,
        (
            "DELAYING".into(),
            "CHECKPOINTED".into(),
            0,
            3,
            0,
            1,
            1,
            0,
            10
        )
    );

    let restarted = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let fire_audit = audit(&fixture);
    let fire = LoopDelayElapsedCommand::new(
        transition(
            &fixture,
            10,
            fixture.authority.clone(),
            fire_audit,
            crate::support::now(),
        ),
        checkpoint.timer_id.expect("timer"),
        1,
        next_activity(&fixture, 1),
        Uuid::now_v7(),
    )
    .expect("fire command");
    let (left, right) = tokio::join!(restarted.fire_delay(&fire), restarted.fire_delay(&fire));
    let left = left.expect("first timeout contender");
    let right = right.expect("second timeout contender");
    assert_ne!(left.applied, right.applied);
    assert_eq!(left.run_version, 13);
    assert_eq!(right.run_version, 13);
    let final_state: (String, String, i32, i32, i32, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,t.lifecycle,u.active_iterations,r.admitted_instances,r.reserved_instances, \
                (SELECT count(*) FROM loop_iterations WHERE tenant_id=s.tenant_id AND run_id=s.run_id), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=s.tenant_id AND aggregate_id=s.run_id), \
                (SELECT count(*) FROM run_events WHERE tenant_id=s.tenant_id AND run_id=s.run_id), \
                (SELECT head_seq FROM telemetry_pending WHERE tenant_id=s.tenant_id AND run_id=s.run_id) \
         FROM loop_scopes s JOIN loop_timers t USING (tenant_id,run_id,owner_instance_id) \
         JOIN tenant_iteration_usage u USING (tenant_id) \
         JOIN run_instance_usage r USING (tenant_id,run_id) \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("fired timer projection");
    assert_eq!(
        final_state,
        ("RUNNING".into(), "FIRED".into(), 1, 4, 1, 2, 2, 13, 13)
    );
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.db.pool).await;
    crate::hint_contract::assert_published(&published, &["loop.timer.scheduled"]);
}

#[tokio::test]
async fn stale_fence_capacity_and_instance_budget_fail_without_timer_side_effects() {
    let fixture = seed_running_loop("loop-delay-budgets").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let stale = SchedulerAuthority::for_run(
        fixture.authority.owner(),
        fixture.run,
        fixture.authority.lease().generation + 1,
    );
    let stale_checkpoint = LoopCheckpointCommand::new(
        transition(&fixture, 8, stale, audit(&fixture), crate::support::now()),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        true,
        crate::support::now() - Duration::seconds(2),
        LoopContinuation::Continue,
        Some(next_activity(&fixture, 1)),
        None,
        Uuid::now_v7(),
    )
    .expect("stale checkpoint");
    assert_eq!(
        store.checkpoint(&stale_checkpoint).await,
        Err(LoopProgressError::SchedulerFenceLost)
    );
    assert_eq!(run_version(&fixture).await, 8);

    sqlx::query("DELETE FROM tenant_collection_policies WHERE tenant_id=$1")
        .bind(fixture.tenant)
        .execute(&fixture.db.pool)
        .await
        .expect("remove policy after admission");
    let checkpoint = delay_checkpoint(&fixture, Uuid::now_v7(), Uuid::now_v7(), audit(&fixture));
    store
        .checkpoint(&checkpoint)
        .await
        .expect("accepted work can release its slot after policy removal");
    let fire = LoopDelayElapsedCommand::new(
        transition(
            &fixture,
            10,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        checkpoint.timer_id.expect("timer"),
        1,
        next_activity(&fixture, 1),
        Uuid::now_v7(),
    )
    .expect("fire command");
    assert_eq!(
        store.fire_delay(&fire).await,
        Err(LoopProgressError::PolicyMissing)
    );
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,2,'loop-test')",
    )
    .bind(fixture.tenant)
    .execute(&fixture.db.pool)
    .await
    .expect("restore loop capacity policy");
    sqlx::query("UPDATE tenant_iteration_usage SET active_iterations=2 WHERE tenant_id=$1")
        .bind(fixture.tenant)
        .execute(&fixture.db.pool)
        .await
        .expect("fill tenant capacity");
    assert_eq!(
        store.fire_delay(&fire).await,
        Err(LoopProgressError::AtCapacity)
    );
    assert_eq!(run_version(&fixture).await, 10);
    sqlx::query("UPDATE tenant_iteration_usage SET active_iterations=0 WHERE tenant_id=$1")
        .bind(fixture.tenant)
        .execute(&fixture.db.pool)
        .await
        .expect("release tenant capacity");
    sqlx::query("UPDATE run_instance_usage SET max_instances=3 WHERE tenant_id=$1 AND run_id=$2")
        .bind(fixture.tenant)
        .bind(fixture.run.as_uuid())
        .execute(&fixture.db.pool)
        .await
        .expect("exhaust run budget");
    assert_eq!(
        store.fire_delay(&fire).await,
        Err(LoopProgressError::InstanceBudgetExceeded)
    );
    let unchanged: (String, i64, i64) = sqlx::query_as(
        "SELECT lifecycle, \
                (SELECT count(*) FROM loop_iterations WHERE tenant_id=$1 AND run_id=$2), \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2) \
         FROM loop_timers WHERE tenant_id=$1 AND timer_id=$3",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(fire.timer_id)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("unchanged timer");
    assert_eq!(unchanged, ("WAITING".into(), 1, 10));
}

#[tokio::test]
async fn outbox_and_audit_failures_roll_back_the_whole_checkpoint() {
    let fixture = seed_running_loop("loop-checkpoint-rollback").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished','{}')",
    )
    .bind(outbox)
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("outbox collision");
    let result_content = br#"{"inline":"result"}"#.to_vec();
    let carry_content = b"42".to_vec();
    let result_digest = digest(&result_content);
    let carry_digest = digest(&carry_content);
    let checked_at = crate::support::now() - Duration::seconds(2);
    let command = LoopCheckpointCommand::from_evaluation(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        LoopCheckpointEvaluation {
            result: result_content,
            carry: carry_content,
            condition: true,
        },
        checked_at,
        LoopContinuation::DelayUntil(checked_at + Duration::seconds(1)),
        None,
        Some(Uuid::now_v7()),
        outbox,
    )
    .expect("checkpoint with inline values");
    assert!(matches!(
        store.checkpoint(&command).await,
        Err(LoopProgressError::Storage(_))
    ));
    assert_eq!(run_version(&fixture).await, 8);
    let after_outbox: (String, i32, i32, i64) = sqlx::query_as(
        "SELECT lifecycle,u.active_iterations,r.reserved_instances, \
                (SELECT count(*) FROM loop_checkpoints WHERE tenant_id=s.tenant_id AND run_id=s.run_id) \
         FROM loop_scopes s JOIN tenant_iteration_usage u USING (tenant_id) \
         JOIN run_instance_usage r USING (tenant_id,run_id) \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("outbox rollback");
    assert_eq!(after_outbox, ("RUNNING".into(), 1, 1, 0));
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE tenant_id=$1 AND digest IN ($2,$3)",
    )
    .bind(fixture.tenant)
    .bind(result_digest.as_bytes().as_slice())
    .bind(carry_digest.as_bytes().as_slice())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("rolled back inline checkpoint values");
    assert_eq!(retained, 0);

    let duplicate_audit = audit(&fixture);
    sqlx::query(
        "INSERT INTO audit_records \
         (id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
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
    .execute(&fixture.db.pool)
    .await
    .expect("audit collision");
    let command = delay_checkpoint(&fixture, Uuid::now_v7(), Uuid::now_v7(), duplicate_audit);
    assert!(matches!(
        store.checkpoint(&command).await,
        Err(LoopProgressError::Storage(_))
    ));
    assert_eq!(run_version(&fixture).await, 8);
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM loop_checkpoints WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT count(*) FROM loop_timers WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2)",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("audit rollback counts");
    assert_eq!(counts, (0, 0, 8));
}

#[tokio::test]
async fn complete_checkpoint_routes_the_terminal_successor_atomically() {
    let fixture = seed_running_loop("loop-terminal-success").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let complete = LoopCheckpointCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        false,
        crate::support::now(),
        LoopContinuation::Complete,
        None,
        None,
        Uuid::now_v7(),
    )
    .expect("complete checkpoint");
    let applied = store.checkpoint(&complete).await.expect("finish loop");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 13);
    let replay = store.checkpoint(&complete).await.expect("finish replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 13);
    let projection: (String, String, bool, String, i32, i32, i64, i64, i64) = sqlx::query_as(
        "SELECT run.status,owner.status,owner.routed,scope.lifecycle, \
                usage.active_iterations,run_usage.reserved_instances, \
                (SELECT count(*) FROM run_events WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=scope.tenant_id \
                   AND aggregate_id=scope.run_id AND event_type='run.finished'), \
                (SELECT head_seq FROM telemetry_pending WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id) \
         FROM loop_scopes scope \
         JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=scope.tenant_id \
         JOIN run_instance_usage run_usage ON run_usage.tenant_id=scope.tenant_id \
           AND run_usage.run_id=scope.run_id \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("finished loop projection");
    assert_eq!(
        projection,
        (
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            "SUCCEEDED".into(),
            0,
            0,
            13,
            1,
            13,
        )
    );
    PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("finished loop journal and projection agree");
    let error = MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_PROGRESS)
        .await
        .expect_err("retained loop progress must block downgrade");
    assert!(error.to_string().contains("loop progress is retained"));
}

/// A run that a loop finishes is a finished run for its subscribers too: the same transaction that
/// records `SUCCEEDED` schedules the `run.finished` delivery of every matching subscription, as
/// the activity and cancellation paths do.
#[tokio::test]
async fn a_run_finished_by_its_loop_is_delivered_to_run_finished_subscribers() {
    let fixture = seed_running_loop("loop-terminal-subscribers").await;
    let subscription = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO webhook_subscriptions \
         (id,tenant_id,namespace_id,name,destination_kind,destination_settings,url,secret_name, \
          kinds,rate_per_second,timeout_ms,status,created_by) \
         VALUES ($1,$2,$3,'loop-finished','WEBHOOK','{}','https://example.invalid/loop', \
                 'loop-key',ARRAY['run.finished'],100,1000,'ACTIVE','test')",
    )
    .bind(subscription)
    .bind(fixture.tenant)
    .bind(fixture.namespace.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("run finished subscription");
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let complete = LoopCheckpointCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        false,
        crate::support::now(),
        LoopContinuation::Complete,
        None,
        None,
        Uuid::now_v7(),
    )
    .expect("complete checkpoint");
    assert!(
        store
            .checkpoint(&complete)
            .await
            .expect("finish loop")
            .applied
    );
    assert!(!store.checkpoint(&complete).await.expect("replay").applied);

    let delivered: (String, i64, Option<String>) = sqlx::query_as(
        "SELECT run.status, \
                (SELECT count(*) FROM webhook_deliveries d \
                  WHERE d.tenant_id=run.tenant_id AND d.subscription_id=$3), \
                (SELECT o.payload->>'node' FROM event_outbox o WHERE o.tenant_id=run.tenant_id \
                  AND o.aggregate_id=run.id AND o.event_type='run.finished') \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(subscription)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("terminal delivery");
    assert_eq!(
        delivered,
        ("SUCCEEDED".to_owned(), 1, None),
        "one delivery, and the run.finished hint has the documented RunFinished shape"
    );
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.db.pool).await;
    crate::hint_contract::assert_published(&published, &["run.finished"]);
}

#[tokio::test]
async fn terminal_outbox_failure_rolls_back_checkpoint_routing_and_slot_release() {
    let fixture = seed_running_loop("loop-terminal-rollback").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let collision = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished','{}')",
    )
    .bind(collision)
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("outbox collision");
    let command = LoopCheckpointCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        false,
        crate::support::now(),
        LoopContinuation::Complete,
        None,
        None,
        collision,
    )
    .expect("complete checkpoint");
    assert!(matches!(
        store.checkpoint(&command).await,
        Err(LoopProgressError::Storage(_))
    ));
    let projection: (i64, String, String, Option<String>, i32, i32, i64) = sqlx::query_as(
        "SELECT run.run_version,run.status,owner.status,owner.waiting_kind, \
                usage.active_iterations,run_usage.reserved_instances, \
                (SELECT count(*) FROM loop_checkpoints WHERE tenant_id=scope.tenant_id \
                   AND run_id=scope.run_id) \
         FROM loop_scopes scope \
         JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=scope.tenant_id \
         JOIN run_instance_usage run_usage ON run_usage.tenant_id=scope.tenant_id \
           AND run_usage.run_id=scope.run_id \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("rolled back terminal checkpoint");
    assert_eq!(
        projection,
        (
            8,
            "RUNNING".into(),
            "WAITING".into(),
            Some("LOOP".into()),
            1,
            1,
            0,
        )
    );
}

#[tokio::test]
async fn iteration_limit_stops_the_run_without_routing_a_successor() {
    let fixture = seed_running_loop_with_limit("loop-iteration-limit", 1).await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let command = LoopCheckpointCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        true,
        crate::support::now(),
        LoopContinuation::IterationLimit,
        None,
        None,
        Uuid::now_v7(),
    )
    .expect("iteration-limit checkpoint");
    let applied = store
        .checkpoint(&command)
        .await
        .expect("stop exhausted loop");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 11);
    let replay = store.checkpoint(&command).await.expect("limit replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 11);
    let projection: IterationLimitProjection = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code,owner.status,owner.routed, \
                owner.failure_code,scope.lifecycle,usage.active_iterations, \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=scope.tenant_id \
                   AND aggregate_id=scope.run_id AND event_type='loop.iteration_limit') \
         FROM loop_scopes scope \
         JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         JOIN tenant_iteration_usage usage ON usage.tenant_id=scope.tenant_id \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("iteration-limit projection");
    assert_eq!(
        projection,
        (
            "CANCELLING".into(),
            Some("FAILED".into()),
            Some("ITERATION_LIMIT".into()),
            "FAILED".into(),
            false,
            Some("ITERATION_LIMIT".into()),
            "FAILED".into(),
            0,
            1,
        )
    );
    PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("iteration-limit journal and projection agree");
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.db.pool).await;
    crate::hint_contract::assert_published(&published, &["loop.iteration_limit"]);
}

#[tokio::test]
async fn expression_failure_drains_once_restarts_and_finishes_from_the_journal() {
    let fixture = seed_running_loop("loop-expression-drain").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let drain_outbox = Uuid::now_v7();
    let start = LoopDrainCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        LoopDrainReason::Failed {
            iteration: 0,
            code: FailureCode::expression_error(),
        },
        drain_outbox,
    )
    .expect("expression drain");
    let started = store.start_drain(&start).await.expect("start drain");
    assert!(started.applied);
    assert_eq!(started.run_version, 10);
    let replay = store.start_drain(&start).await.expect("start replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 10);

    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let mut reopened = verifier
        .begin(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
            Lock::Wait,
        )
        .await
        .expect("reopen draining loop")
        .expect("draining run");
    let scope = reopened
        .parts()
        .1
        .loop_state(&fixture.owner)
        .expect("restored draining loop");
    assert_eq!(scope.phase(), LoopPhase::Draining);
    assert_eq!(scope.active_iteration(), Some(0));
    drop(reopened);
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("drain start projection matches journal");

    let drained = store
        .drain_iteration(
            &LoopIterationDrainCommand::new(
                transition(
                    &fixture,
                    10,
                    fixture.authority.clone(),
                    audit(&fixture),
                    crate::support::now(),
                ),
                0,
                Uuid::now_v7(),
            )
            .expect("iteration drain"),
        )
        .await
        .expect("drain iteration");
    assert!(drained.applied);
    assert_eq!(drained.run_version, 11);
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("drained iteration projection matches journal");

    let finish_outbox = Uuid::now_v7();
    let finish = LoopDrainFinishCommand::new(
        transition(
            &fixture,
            11,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        finish_outbox,
    );
    let finished = store.finish_drain(&finish).await.expect("finish drain");
    assert!(finished.applied);
    assert_eq!(finished.run_version, 13);
    let finish_replay = store.finish_drain(&finish).await.expect("finish replay");
    assert!(!finish_replay.applied);
    assert_eq!(finish_replay.run_version, 13);
    let projection: (
        String,
        String,
        Option<String>,
        String,
        String,
        i32,
        i32,
        i64,
    ) = sqlx::query_as(
        "SELECT run.status,owner.status,owner.failure_code,scope.lifecycle,iteration.lifecycle, \
                    usage.active_iterations,run_usage.reserved_instances, \
                    (SELECT head_seq FROM telemetry_pending WHERE tenant_id=scope.tenant_id \
                       AND run_id=scope.run_id) \
             FROM loop_scopes scope \
             JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
             JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
               AND owner.step_id=scope.owner_instance_id \
             JOIN loop_iterations iteration \
               ON iteration.tenant_id=scope.tenant_id AND iteration.run_id=scope.run_id \
              AND iteration.owner_instance_id=scope.owner_instance_id \
             JOIN tenant_iteration_usage usage ON usage.tenant_id=scope.tenant_id \
             JOIN run_instance_usage run_usage \
               ON run_usage.tenant_id=scope.tenant_id AND run_usage.run_id=scope.run_id \
             WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("terminal drain projection");
    assert_eq!(
        projection,
        (
            "FAILED".into(),
            "FAILED".into(),
            Some("EXPRESSION_ERROR".into()),
            "FAILED".into(),
            "DRAINED".into(),
            0,
            0,
            13,
        )
    );
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("terminal drain projection matches journal");
    let error = MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_DRAIN)
        .await
        .expect_err("retained loop drain must block downgrade");
    assert!(
        error
            .to_string()
            .contains("loop drain progress is retained")
    );
}

#[tokio::test]
async fn cancellation_drains_the_active_iteration_and_finishes_after_restart() {
    let fixture = seed_running_loop("loop-cancellation-drain").await;
    let requested_at = crate::support::now();
    let mut cancellation = fixture
        .db
        .pool
        .begin()
        .await
        .expect("cancellation transaction");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,9,'run.stop_requested',$3,'loop-fixture',$4)",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(json!({"reason":"CANCELLED","code":null}))
    .bind(requested_at)
    .execute(&mut *cancellation)
    .await
    .expect("cancellation journal");
    sqlx::query(
        "UPDATE runs SET status='CANCELLING',stop_reason='CANCELLED',stop_code=NULL, \
                run_version=9,updated_at=$3 WHERE tenant_id=$1 AND id=$2 AND run_version=8",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(requested_at)
    .execute(&mut *cancellation)
    .await
    .expect("cancellation projection");
    cancellation.commit().await.expect("commit cancellation");

    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("canonical cancellation fixture replays");
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let started = store
        .start_drain(
            &LoopDrainCommand::new(
                transition(
                    &fixture,
                    9,
                    fixture.authority.clone(),
                    audit(&fixture),
                    crate::support::now(),
                ),
                LoopDrainReason::Cancelled,
                Uuid::now_v7(),
            )
            .expect("cancellation drain"),
        )
        .await
        .expect("start cancellation drain");
    assert_eq!(started.run_version, 10);
    store
        .drain_iteration(
            &LoopIterationDrainCommand::new(
                transition(
                    &fixture,
                    10,
                    fixture.authority.clone(),
                    audit(&fixture),
                    crate::support::now(),
                ),
                0,
                Uuid::now_v7(),
            )
            .expect("cancel active iteration"),
        )
        .await
        .expect("drain cancelled iteration");
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("cancelled drain restarts before finish");
    let finished = store
        .finish_drain(&LoopDrainFinishCommand::new(
            transition(
                &fixture,
                11,
                fixture.authority.clone(),
                audit(&fixture),
                crate::support::now(),
            ),
            Uuid::now_v7(),
        ))
        .await
        .expect("finish cancellation drain");
    assert_eq!(finished.run_version, 13);
    let terminal: (String, String, String, String) = sqlx::query_as(
        "SELECT run.status,owner.status,scope.lifecycle,iteration.lifecycle \
         FROM runs run JOIN loop_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         JOIN loop_iterations iteration \
           ON iteration.tenant_id=scope.tenant_id AND iteration.run_id=scope.run_id \
          AND iteration.owner_instance_id=scope.owner_instance_id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("cancelled terminal projection");
    assert_eq!(
        terminal,
        (
            "CANCELLED".into(),
            "CANCELLED".into(),
            "CANCELLED".into(),
            "DRAINED".into(),
        )
    );
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("cancelled terminal projection matches journal");
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.db.pool).await;
    crate::hint_contract::assert_published(
        &published,
        &["loop.drain_started", "loop.finished", "run.finished"],
    );
}

#[tokio::test]
async fn leased_member_observes_cancellation_and_drain_resumes_after_restart() {
    let fixture = seed_leasable_loop("loop-leased-cancellation").await;
    let (workers, _registered, task) = lease_loop_member(&fixture).await;
    let requested_at = crate::support::now();
    let mut cancellation = fixture
        .db
        .pool
        .begin()
        .await
        .expect("cancellation transaction");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,6,'run.stop_requested',$3,'loop-fixture',$4)",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(json!({"reason":"CANCELLED","code":null}))
    .bind(requested_at)
    .execute(&mut *cancellation)
    .await
    .expect("cancellation journal");
    sqlx::query(
        "UPDATE runs SET status='CANCELLING',stop_reason='CANCELLED',stop_code=NULL, \
                run_version=6,updated_at=$3 WHERE tenant_id=$1 AND id=$2 AND run_version=5",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(requested_at)
    .execute(&mut *cancellation)
    .await
    .expect("cancellation projection");
    cancellation.commit().await.expect("commit cancellation");

    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let drain = LoopDrainCommand::new(
        transition(
            &fixture,
            6,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        LoopDrainReason::Cancelled,
        Uuid::now_v7(),
    )
    .expect("leased cancellation drain");
    let started = store.start_drain(&drain).await.expect("start leased drain");
    assert_eq!(started.run_version, 8);
    let cancellation_projection: (String, bool, i64, i64) = sqlx::query_as(
        "SELECT step.status,step.cancel_requested,run.run_version, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='step.cancel_requested') \
         FROM runs run JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2 AND step.step_id=$3",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(task.step_id)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("leased cancellation projection");
    assert_eq!(cancellation_projection, ("LEASED".into(), true, 8, 1));
    let beat = workers
        .heartbeat(&fixture.worker, &task_ref(&task))
        .await
        .expect("leased loop member observes cancellation");
    assert!(beat.cancel_requested);
    workers
        .fail(
            &fixture.worker,
            &task_ref(&task),
            "CANCELLED",
            false,
            "loop-cancel-ack",
        )
        .await
        .expect("worker acknowledges loop cancellation");

    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("worker cancellation projection replays");
    let version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.tenant)
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("version after cancellation acknowledgement");
    let drained = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone())
        .drain_iteration(
            &LoopIterationDrainCommand::new(
                transition(
                    &fixture,
                    u64::try_from(version).expect("run version"),
                    fixture.authority.clone(),
                    audit(&fixture),
                    crate::support::now(),
                ),
                0,
                Uuid::now_v7(),
            )
            .expect("resume drained iteration"),
        )
        .await
        .expect("drain acknowledged iteration");
    assert_eq!(
        drained.run_version,
        u64::try_from(version).expect("run version") + 1
    );
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("drain resumes after worker acknowledgement and restart");
}

#[tokio::test]
async fn running_member_observes_timeout_and_releases_capacity_after_restart() {
    let fixture = seed_leasable_loop("loop-running-timeout").await;
    let (workers, _registered, task) = lease_loop_member(&fixture).await;
    let initial_beat = workers
        .heartbeat(&fixture.worker, &task_ref(&task))
        .await
        .expect("first heartbeat starts loop member");
    assert!(!initial_beat.cancel_requested);
    let started_at = crate::support::now();
    let after_deadline = started_at + Duration::minutes(2);
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let started = store
        .start_drain(
            &LoopDrainCommand::new(
                transition(
                    &fixture,
                    6,
                    fixture.authority.clone(),
                    audit(&fixture),
                    after_deadline,
                ),
                LoopDrainReason::TimedOut,
                Uuid::now_v7(),
            )
            .expect("running timeout drain"),
        )
        .await
        .expect("start running timeout drain");
    assert_eq!(started.run_version, 9);
    let cancel_requested: bool = sqlx::query_scalar(
        "SELECT cancel_requested FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(task.step_id)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("running member cancellation request");
    assert!(cancel_requested);
    let cancellation_beat = workers
        .heartbeat(&fixture.worker, &task_ref(&task))
        .await
        .expect("running loop member observes timeout cancellation");
    assert!(cancellation_beat.cancel_requested);
    workers
        .fail(
            &fixture.worker,
            &task_ref(&task),
            "CANCELLED",
            false,
            "loop-timeout-ack",
        )
        .await
        .expect("worker acknowledges loop timeout");

    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("timed-out worker acknowledgement replays");
    let version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.tenant)
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("version after timeout acknowledgement");
    let drained = store
        .drain_iteration(
            &LoopIterationDrainCommand::new(
                transition(
                    &fixture,
                    u64::try_from(version).expect("run version"),
                    fixture.authority.clone(),
                    audit(&fixture),
                    crate::support::now(),
                ),
                0,
                Uuid::now_v7(),
            )
            .expect("drain timed-out iteration"),
        )
        .await
        .expect("drain timed-out worker iteration");
    let capacity: (i32, i32, String) = sqlx::query_as(
        "SELECT tenant.active_iterations,run_usage.reserved_instances,iteration.lifecycle \
         FROM tenant_iteration_usage tenant JOIN run_instance_usage run_usage \
           ON run_usage.tenant_id=tenant.tenant_id \
         JOIN loop_iterations iteration ON iteration.tenant_id=run_usage.tenant_id \
           AND iteration.run_id=run_usage.run_id \
         WHERE tenant.tenant_id=$1 AND run_usage.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("released timed-out capacity");
    assert_eq!(capacity, (0, 0, "DRAINED".into()));
    assert_eq!(
        drained.run_version,
        u64::try_from(version).expect("run version") + 1
    );
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("timed-out drain resumes after restart");
}

#[tokio::test]
async fn deadline_cancels_a_suspended_timer_and_fences_the_stale_fire() {
    let fixture = seed_running_loop("loop-timeout-drain").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let timer_id = Uuid::now_v7();
    let checked_at = crate::support::now();
    let checkpoint = LoopCheckpointCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            checked_at,
        ),
        0,
        fixture.returned.clone(),
        fixture.initial_carry,
        fixture.initial_carry,
        true,
        checked_at,
        LoopContinuation::DelayUntil(checked_at + Duration::seconds(1)),
        None,
        Some(timer_id),
        Uuid::now_v7(),
    )
    .expect("delay checkpoint");
    store
        .checkpoint(&checkpoint)
        .await
        .expect("schedule loop delay");
    let after_deadline = crate::support::now() + Duration::minutes(2);
    let start = LoopDrainCommand::new(
        transition(
            &fixture,
            10,
            fixture.authority.clone(),
            audit(&fixture),
            after_deadline,
        ),
        LoopDrainReason::TimedOut,
        Uuid::now_v7(),
    )
    .expect("timeout drain");
    let started = store
        .start_drain(&start)
        .await
        .expect("start timeout drain");
    assert_eq!(started.run_version, 12);
    let timer_state: (String, String, Option<i32>, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT timer.lifecycle,scope.lifecycle,scope.active_iteration,scope.delay_due_at \
         FROM loop_timers timer JOIN loop_scopes scope USING (tenant_id,run_id,owner_instance_id) \
         WHERE timer.tenant_id=$1 AND timer.timer_id=$2",
    )
    .bind(fixture.tenant)
    .bind(timer_id)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("cancelled timer projection");
    assert_eq!(
        timer_state,
        ("CANCELLED".into(), "DRAINING".into(), None, None)
    );
    let stale_fire = LoopDelayElapsedCommand::new(
        transition(
            &fixture,
            10,
            fixture.authority.clone(),
            audit(&fixture),
            after_deadline,
        ),
        timer_id,
        1,
        next_activity(&fixture, 1),
        Uuid::now_v7(),
    )
    .expect("stale timer fire");
    assert_eq!(
        store.fire_delay(&stale_fire).await,
        Err(LoopProgressError::InvalidTransition)
    );
    let verifier = PgRunStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("timeout projection matches journal");
    let finished = store
        .finish_drain(&LoopDrainFinishCommand::new(
            transition(
                &fixture,
                12,
                fixture.authority.clone(),
                audit(&fixture),
                crate::support::now(),
            ),
            Uuid::now_v7(),
        ))
        .await
        .expect("finish timeout drain");
    assert_eq!(finished.run_version, 14);
    let mut reopened = verifier
        .begin(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
            Lock::Wait,
        )
        .await
        .expect("reopen timeout run")
        .expect("terminal timeout run");
    assert_eq!(reopened.parts().1.status(), RunStatus::Failed);
    let scope = reopened
        .parts()
        .1
        .loop_state(&fixture.owner)
        .expect("restored timed-out loop");
    assert_eq!(scope.phase(), LoopPhase::Failed);
    assert_eq!(scope.failure(), Some(&FailureCode::run_timed_out()));
    drop(reopened);
    let terminal: (String, Option<String>, String, String) = sqlx::query_as(
        "SELECT run.status,run.stop_code,owner.status,scope.lifecycle \
         FROM runs run JOIN loop_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.step_id=scope.owner_instance_id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("terminal timeout projection");
    assert_eq!(
        terminal,
        (
            "FAILED".into(),
            Some("RUN_TIMED_OUT".into()),
            "FAILED".into(),
            "FAILED".into(),
        )
    );
    verifier
        .verify_run(
            sagat_domain::identity::TenantId::new(fixture.tenant),
            fixture.run,
        )
        .await
        .expect("terminal timeout projection matches journal");
}

#[tokio::test]
async fn drain_outbox_failure_rolls_back_stop_projection_audit_and_journal() {
    let fixture = seed_running_loop("loop-drain-rollback").await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let collision = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'run.finished','{}')",
    )
    .bind(collision)
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.db.pool)
    .await
    .expect("outbox collision");
    let command = LoopDrainCommand::new(
        transition(
            &fixture,
            8,
            fixture.authority.clone(),
            audit(&fixture),
            crate::support::now(),
        ),
        LoopDrainReason::Failed {
            iteration: 0,
            code: FailureCode::expression_error(),
        },
        collision,
    )
    .expect("expression drain");
    assert!(matches!(
        store.start_drain(&command).await,
        Err(LoopProgressError::Storage(_))
    ));
    let unchanged: (
        i64,
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT run.run_version,run.status,run.stop_reason,scope.lifecycle,scope.drain_kind, \
                    (SELECT count(*) FROM run_events WHERE tenant_id=scope.tenant_id \
                       AND run_id=scope.run_id), \
                    (SELECT count(*) FROM audit_records WHERE tenant_id=scope.tenant_id \
                       AND resource_id=scope.run_id::text) \
             FROM loop_scopes scope JOIN runs run \
               ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
             WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("rolled back drain");
    assert_eq!(
        unchanged,
        (8, "RUNNING".into(), None, "RUNNING".into(), None, 8, 0)
    );
}
