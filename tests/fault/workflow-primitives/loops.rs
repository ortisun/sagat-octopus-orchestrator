//! Loop checkpoint, timer and recovery failures against real PostgreSQL.
//!
//! Cancel, signal and declared-update transactions are not wired to this private loop store yet.
//! Their race gate therefore proves only that a timer delivery loses on the shared run CAS/status
//! barrier after another transition commits; it does not claim the reciprocal integrated race.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::loop_progress::{
    LoopCheckpointCommand, LoopDelayElapsedCommand, LoopDrainCommand, LoopDrainFinishCommand,
    LoopIterationDrainCommand, LoopNextActivity, LoopProgressError, LoopProgressStore,
    LoopTransition,
};
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ExecutionInstanceId, FailureCode, InstancePath, LoopContinuation, LoopDrainReason, RunId,
};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{MIGRATOR, PgLoopProgressStore, PgRunStore};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{chain, world};
use crate::support::scope_flow::{Flow, Sched, activity};

const BEFORE_LOOP_PROGRESS: i64 = 20_261_030_000_000;

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

fn loop_workflow(name: &str) -> serde_json::Value {
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
                "type": "while",
                "condition": "carry < 3",
                "body": "iteration",
                "initial": "0",
                "carry": "bodyResult",
                "maxIterations": 3,
                "timeoutMs": 60000,
                "delayMs": 1,
                "next": "done"
            },
            { "id": "done", "type": "end", "outputExpr": "steps.poll.output" }
        ]
    })
}

/// The loop workflow with an `onError` edge to a recovery activity followed by an `end` (D294).
fn loop_workflow_with_error_edge(name: &str) -> serde_json::Value {
    let mut document = loop_workflow(name);
    let nodes = document["nodes"].as_array_mut().expect("workflow nodes");
    nodes[0]["onError"] = json!("recover");
    nodes.push(json!({
        "id": "recover",
        "type": "activity",
        "activity": "poll.recover",
        "next": "recovered"
    }));
    nodes.push(json!({"id": "recovered", "type": "end"}));
    document
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
    returned: ExecutionInstanceId,
    definitions: DefinitionCache,
    authority: SchedulerAuthority,
    initial_carry: Digest,
}

async fn seed_running_loop(name: &str) -> LoopFixture {
    seed_running_loop_with(name, loop_workflow(name)).await
}

async fn seed_running_loop_with_error_edge(name: &str) -> LoopFixture {
    seed_running_loop_with(name, loop_workflow_with_error_edge(name)).await
}

async fn seed_running_loop_with(name: &str, workflow_document: serde_json::Value) -> LoopFixture {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let (workflow, _) = w.publish(chain(name)).await;
    let document = serde_json::to_vec(&workflow_document).expect("loop document");
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
         VALUES ($1,$2,$3,$4,$5,'RUNNING',8,$6,'loop-fixture', \
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
    .expect("loop run");
    let (entry_nodes, entry_indices) = path_parts(&entry);
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
    .expect("loop steps");
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
         VALUES ($1,$2,$3,$4,'{}','{}','poll','WHILE','RUNNING',3,$5,$6,$6,true,0,0,2,$7,$7)",
    )
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
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
                "max_iterations": 3,
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
    ] {
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
        returned,
        definitions: DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        authority,
        initial_carry,
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
    let checkpoint_state: (String, String, i32, i32, i32, i64, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,i.lifecycle,u.active_iterations,r.admitted_instances,r.reserved_instances, \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=s.tenant_id AND aggregate_id=s.run_id), \
                (SELECT count(*) FROM audit_records WHERE tenant_id=s.tenant_id AND resource_id=s.run_id::text), \
                (SELECT count(*) FROM steps WHERE tenant_id=s.tenant_id AND run_id=s.run_id \
                   AND (status IN ('LEASED','RUNNING') OR lease_token_hash IS NOT NULL)) \
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
        ("DELAYING".into(), "CHECKPOINTED".into(), 0, 3, 0, 1, 1, 0)
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
    let final_state: (String, String, i32, i32, i32, i64, i64, i64) = sqlx::query_as(
        "SELECT s.lifecycle,t.lifecycle,u.active_iterations,r.admitted_instances,r.reserved_instances, \
                (SELECT count(*) FROM loop_iterations WHERE tenant_id=s.tenant_id AND run_id=s.run_id), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=s.tenant_id AND aggregate_id=s.run_id), \
                (SELECT count(*) FROM run_events WHERE tenant_id=s.tenant_id AND run_id=s.run_id) \
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
        ("RUNNING".into(), "FIRED".into(), 1, 4, 1, 2, 2, 13)
    );
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
    let command = delay_checkpoint(&fixture, outbox, Uuid::now_v7(), audit(&fixture));
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

async fn assert_timer_fails_closed_after_competing_commit(
    name: &str,
    committed_status: &str,
    expected: LoopProgressError,
) {
    let fixture = seed_running_loop(name).await;
    let store = PgLoopProgressStore::new(fixture.db.pool.clone(), fixture.definitions.clone());
    let checkpoint = delay_checkpoint(&fixture, Uuid::now_v7(), Uuid::now_v7(), audit(&fixture));
    store
        .checkpoint(&checkpoint)
        .await
        .expect("delay checkpoint");
    let timer = LoopDelayElapsedCommand::new(
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
    .expect("timer delivery");

    sqlx::query(
        "UPDATE runs SET run_version=run_version+1,status=$3, \
                finished_at=CASE WHEN $3='CANCELLED' THEN clock_timestamp() ELSE finished_at END, \
                updated_at=clock_timestamp() \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .bind(committed_status)
    .execute(&fixture.db.pool)
    .await
    .expect("inject competing committed transition");

    assert_eq!(store.fire_delay(&timer).await, Err(expected));
    let evidence: (String, i64, i64, i64) = sqlx::query_as(
        "SELECT t.lifecycle, \
           (SELECT count(*) FROM loop_iterations WHERE tenant_id=t.tenant_id AND run_id=t.run_id), \
           (SELECT count(*) FROM run_events WHERE tenant_id=t.tenant_id AND run_id=t.run_id \
              AND event_type='loop.delay_elapsed'), \
           (SELECT count(*) FROM steps WHERE tenant_id=t.tenant_id AND run_id=t.run_id \
              AND node='work' AND instance_path_indices=ARRAY[1]::integer[]) \
         FROM loop_timers t WHERE t.tenant_id=$1 AND t.timer_id=$2",
    )
    .bind(fixture.tenant)
    .bind(timer.timer_id)
    .fetch_one(&fixture.db.pool)
    .await
    .expect("timer loser evidence");
    assert_eq!(evidence, ("WAITING".into(), 1, 0, 0));
}

#[tokio::test]
async fn timer_delivery_fails_closed_after_cancel_signal_or_update_commit() {
    // The cancel, signal and update runtime ports are intentionally still closed. These injected
    // commits prove only the shared run CAS/status barrier; they do not claim an integrated race.
    assert_timer_fails_closed_after_competing_commit(
        "loop-cancel-gate",
        "CANCELLED",
        LoopProgressError::InvalidTransition,
    )
    .await;
    assert_timer_fails_closed_after_competing_commit(
        "loop-signal-gate",
        "WAITING",
        LoopProgressError::RevisionConflict { current: 11 },
    )
    .await;
    assert_timer_fails_closed_after_competing_commit(
        "loop-update-gate",
        "WAITING",
        LoopProgressError::RevisionConflict { current: 11 },
    )
    .await;
}

#[tokio::test]
async fn terminal_routing_replays_and_destructive_down_migration_fails_closed() {
    let fixture = seed_running_loop("loop-terminal-gates").await;
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
    let applied = store.checkpoint(&complete).await.expect("terminal routing");
    assert!(applied.applied);
    assert_eq!(applied.run_version, 13);
    let replay = store.checkpoint(&complete).await.expect("terminal replay");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, 13);
    let evidence: (String, String, bool, String, String, bool, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT run.status,owner.status,owner.routed,scope.lifecycle,ended.status,ended.routed, \
                    (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id \
                       AND event_type='loop.iteration_returned'), \
                    (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id \
                       AND event_type='loop.iteration_checkpointed'), \
                    (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id \
                       AND event_type='end.reached'), \
                    (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id AND run_id=run.id \
                       AND event_type='run.finished'), \
                    (SELECT count(*) FROM event_outbox WHERE tenant_id=run.tenant_id \
                       AND aggregate_id=run.id AND event_type='run.finished') \
             FROM runs run JOIN loop_scopes scope \
               ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
             JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
               AND owner.step_id=scope.owner_instance_id \
             JOIN steps ended ON ended.tenant_id=run.tenant_id AND ended.run_id=run.id \
               AND ended.node='done' AND cardinality(ended.instance_path_nodes)=0 \
             WHERE run.tenant_id=$1 AND run.id=$2",
        )
        .bind(fixture.tenant)
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.db.pool)
        .await
        .expect("terminal projection and journal");
    assert_eq!(
        evidence,
        (
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            1,
            1,
            1,
            1,
            1,
        )
    );
    assert_eq!(run_version(&fixture).await, 13);
    let error = MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_PROGRESS)
        .await
        .expect_err("retained loop progress must block downgrade");
    assert!(error.to_string().contains("loop progress is retained"));
}

// ---------------------------------------------------------------------------------------------
// D294 faults: a loop failure routed over `onError` after its drain.
// ---------------------------------------------------------------------------------------------

fn drain_command(fixture: &LoopFixture, version: u64, at: OffsetDateTime) -> LoopDrainCommand {
    LoopDrainCommand::new(
        transition(
            fixture,
            version,
            fixture.authority.clone(),
            audit(fixture),
            at,
        ),
        LoopDrainReason::Failed {
            iteration: 0,
            code: FailureCode::expression_error(),
        },
        Uuid::now_v7(),
    )
    .expect("failure drain")
}

fn iteration_command(
    fixture: &LoopFixture,
    version: u64,
    authority: SchedulerAuthority,
) -> LoopIterationDrainCommand {
    LoopIterationDrainCommand::new(
        transition(
            fixture,
            version,
            authority,
            audit(fixture),
            crate::support::now(),
        ),
        0,
        Uuid::now_v7(),
    )
    .expect("iteration drain")
}

fn finish_command(
    fixture: &LoopFixture,
    version: u64,
    authority: SchedulerAuthority,
    outbox: Uuid,
) -> LoopDrainFinishCommand {
    LoopDrainFinishCommand::new(
        transition(
            fixture,
            version,
            authority,
            audit(fixture),
            crate::support::now(),
        ),
        outbox,
    )
}

fn store(fixture: &LoopFixture) -> PgLoopProgressStore {
    PgLoopProgressStore::new(
        fixture.db.pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
}

async fn verify(fixture: &LoopFixture, what: &str) {
    PgRunStore::new(
        fixture.db.pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .verify_run(
        sagat_domain::identity::TenantId::new(fixture.tenant),
        fixture.run,
    )
    .await
    .unwrap_or_else(|error| panic!("{what}: {error}"));
}

/// Starts and drains a failure with the loop's own error edge; the run is left one finish away.
async fn drained_failure(fixture: &LoopFixture) -> u64 {
    let started = store(fixture)
        .start_drain(&drain_command(fixture, 8, crate::support::now()))
        .await
        .expect("start failure drain");
    store(fixture)
        .drain_iteration(&iteration_command(
            fixture,
            started.run_version,
            fixture.authority.clone(),
        ))
        .await
        .expect("drain the failed iteration")
        .run_version
}

/// `(run status, owner status, scope, recover step, step.routed of the owner, task.ready rows)`.
async fn edge_state(fixture: &LoopFixture) -> (String, String, String, Option<String>, i64, i64) {
    sqlx::query_as(
        "SELECT run.status,owner.status,scope.lifecycle, \
                (SELECT status FROM steps WHERE tenant_id=run.tenant_id AND run_id=run.id \
                   AND node='recover'), \
                (SELECT count(*) FROM run_events WHERE tenant_id=run.tenant_id \
                   AND run_id=run.id AND event_type='step.routed' AND node='poll'), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=run.tenant_id \
                   AND aggregate_id=run.id AND event_type='task.ready') \
         FROM runs run JOIN loop_scopes scope ON scope.tenant_id=run.tenant_id \
           AND scope.run_id=run.id \
         JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
           AND owner.step_id=scope.owner_instance_id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(fixture.tenant)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.db.pool)
    .await
    .expect("error edge state")
}

/// A failing audit insert is a different fault from a failing outbox: the owner, the scope, the
/// recovery step, the journal and the hint must all disappear with it, and a retry must succeed.
#[tokio::test]
async fn audit_failure_while_taking_the_error_edge_rolls_the_whole_finish_back() {
    let fixture = seed_running_loop_with_error_edge("loop-edge-audit").await;
    let version = drained_failure(&fixture).await;
    let events_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(fixture.tenant)
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("journal size");
    sqlx::raw_sql(
        "CREATE FUNCTION reject_edge_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected audit failure'; END $$; \
         CREATE TRIGGER reject_edge_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_edge_audit()",
    )
    .execute(&fixture.db.pool)
    .await
    .expect("install audit fault");
    let finish = finish_command(&fixture, version, fixture.authority.clone(), Uuid::now_v7());
    assert!(matches!(
        store(&fixture).finish_drain(&finish).await,
        Err(LoopProgressError::Storage(_))
    ));
    assert_eq!(
        edge_state(&fixture).await,
        (
            "WAITING".into(),
            "WAITING".into(),
            "DRAINING".into(),
            None,
            0,
            0
        )
    );
    let events_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(fixture.tenant)
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.db.pool)
            .await
            .expect("journal size after the fault");
    assert_eq!(events_after, events_before);
    assert_eq!(run_version(&fixture).await, version);
    sqlx::raw_sql(
        "DROP TRIGGER reject_edge_audit ON audit_records; DROP FUNCTION reject_edge_audit()",
    )
    .execute(&fixture.db.pool)
    .await
    .expect("remove audit fault");
    let finished = store(&fixture)
        .finish_drain(&finish)
        .await
        .expect("retry takes the edge");
    assert!(finished.applied);
    assert_eq!(
        edge_state(&fixture).await,
        (
            "RUNNING".into(),
            "FAILED".into(),
            "FAILED".into(),
            Some("READY".into()),
            1,
            1
        )
    );
    verify(&fixture, "edge taken after the audit fault").await;
}

/// A replica that lost the scheduler shard mid-drain is fenced out; the replica that took the shard
/// over resumes from the journal and takes the edge exactly once.
#[tokio::test]
async fn scheduler_takeover_mid_drain_fences_the_old_replica_and_the_new_one_finishes() {
    let fixture = seed_running_loop_with_error_edge("loop-edge-takeover").await;
    let started = store(&fixture)
        .start_drain(&drain_command(&fixture, 8, crate::support::now()))
        .await
        .expect("old replica starts the drain");
    // Another replica takes the shard over: the old authority's generation is now stale.
    let successor = scheduler(&fixture.db.pool, fixture.run).await;
    assert_eq!(
        store(&fixture)
            .drain_iteration(&iteration_command(
                &fixture,
                started.run_version,
                fixture.authority.clone()
            ))
            .await,
        Err(LoopProgressError::SchedulerFenceLost)
    );
    assert_eq!(
        edge_state(&fixture).await,
        (
            "RUNNING".into(),
            "WAITING".into(),
            "DRAINING".into(),
            None,
            0,
            0
        )
    );
    let drained = store(&fixture)
        .drain_iteration(&iteration_command(
            &fixture,
            started.run_version,
            successor.clone(),
        ))
        .await
        .expect("new replica drains");
    let outbox = Uuid::now_v7();
    assert_eq!(
        store(&fixture)
            .finish_drain(&finish_command(
                &fixture,
                drained.run_version,
                fixture.authority.clone(),
                outbox,
            ))
            .await,
        Err(LoopProgressError::SchedulerFenceLost),
        "the fenced replica cannot finish either"
    );
    let finish = finish_command(&fixture, drained.run_version, successor, outbox);
    let finished = store(&fixture)
        .finish_drain(&finish)
        .await
        .expect("new replica takes the edge");
    assert!(finished.applied);
    let replay = store(&fixture)
        .finish_drain(&finish)
        .await
        .expect("redelivered finish");
    assert!(!replay.applied);
    assert_eq!(replay.run_version, finished.run_version);
    assert_eq!(
        edge_state(&fixture).await,
        (
            "RUNNING".into(),
            "FAILED".into(),
            "FAILED".into(),
            Some("READY".into()),
            1,
            1
        )
    );
    verify(&fixture, "edge taken once after the takeover").await;
}

/// Two replicas finish the same drain at once with ids of their own: one commits, the other is
/// refused, and the edge is taken once.
#[tokio::test]
async fn concurrent_replicas_finishing_one_drain_take_the_error_edge_once() {
    let fixture = seed_running_loop_with_error_edge("loop-edge-race").await;
    let version = drained_failure(&fixture).await;
    let first = finish_command(&fixture, version, fixture.authority.clone(), Uuid::now_v7());
    let second = finish_command(&fixture, version, fixture.authority.clone(), Uuid::now_v7());
    let (replica_one, replica_two) = (store(&fixture), store(&fixture));
    let (left, right) = tokio::join!(
        replica_one.finish_drain(&first),
        replica_two.finish_drain(&second)
    );
    let applied = [&left, &right]
        .iter()
        .filter(|outcome| matches!(outcome, Ok(write) if write.applied))
        .count();
    assert_eq!(
        applied, 1,
        "exactly one replica commits: {left:?} / {right:?}"
    );
    assert!(
        [&left, &right].iter().all(|outcome| matches!(
            outcome,
            Ok(_)
                | Err(LoopProgressError::DuplicateMismatch
                    | LoopProgressError::RevisionConflict { .. }
                    | LoopProgressError::InvalidTransition)
        )),
        "the loser is refused, never an internal failure: {left:?} / {right:?}"
    );
    assert_eq!(
        edge_state(&fixture).await,
        (
            "RUNNING".into(),
            "FAILED".into(),
            "FAILED".into(),
            Some("READY".into()),
            1,
            1
        )
    );
    verify(&fixture, "one edge after the race").await;
}

fn complete_checkpoint(fixture: &LoopFixture) -> LoopCheckpointCommand {
    LoopCheckpointCommand::new(
        transition(
            fixture,
            8,
            fixture.authority.clone(),
            audit(fixture),
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
    .expect("complete checkpoint")
}

fn timeout_drain(fixture: &LoopFixture, version: u64) -> LoopDrainCommand {
    LoopDrainCommand::new(
        transition(
            fixture,
            version,
            fixture.authority.clone(),
            audit(fixture),
            crate::support::now() + Duration::minutes(2),
        ),
        LoopDrainReason::TimedOut,
        Uuid::now_v7(),
    )
    .expect("timeout drain")
}

/// The loop timeout wins the race against a completing body: the late checkpoint changes nothing.
#[tokio::test]
async fn a_timeout_that_commits_first_beats_a_completing_body() {
    let fixture = seed_running_loop_with_error_edge("loop-edge-timeout-first").await;
    let started = store(&fixture)
        .start_drain(&timeout_drain(&fixture, 8))
        .await
        .expect("timeout wins");
    assert!(matches!(
        store(&fixture)
            .checkpoint(&complete_checkpoint(&fixture))
            .await,
        Err(LoopProgressError::RevisionConflict { .. } | LoopProgressError::InvalidTransition)
    ));
    assert_eq!(run_version(&fixture).await, started.run_version);
    let drained = store(&fixture)
        .drain_iteration(&iteration_command(
            &fixture,
            started.run_version,
            fixture.authority.clone(),
        ))
        .await
        .expect("drain");
    store(&fixture)
        .finish_drain(&finish_command(
            &fixture,
            drained.run_version,
            fixture.authority.clone(),
            Uuid::now_v7(),
        ))
        .await
        .expect("take the edge");
    assert_eq!(
        edge_state(&fixture).await,
        (
            "RUNNING".into(),
            "FAILED".into(),
            "FAILED".into(),
            Some("READY".into()),
            1,
            1
        )
    );
    verify(&fixture, "timeout won the race").await;
}

/// The completing body wins the race against the timeout: the loop ends through `next`, the late
/// timeout is refused and no error edge is taken.
#[tokio::test]
async fn a_completing_body_that_commits_first_beats_the_timeout() {
    let fixture = seed_running_loop_with_error_edge("loop-edge-complete-first").await;
    let finished = store(&fixture)
        .checkpoint(&complete_checkpoint(&fixture))
        .await
        .expect("completion wins");
    assert!(finished.applied);
    assert!(matches!(
        store(&fixture)
            .start_drain(&timeout_drain(&fixture, 8))
            .await,
        Err(LoopProgressError::RevisionConflict { .. } | LoopProgressError::InvalidTransition)
    ));
    assert_eq!(run_version(&fixture).await, finished.run_version);
    let state = edge_state(&fixture).await;
    assert_eq!(
        (state.0.as_str(), state.1.as_str(), state.3.clone()),
        ("SUCCEEDED", "SUCCEEDED", None),
        "the loop completed and its error edge was not taken: {state:?}"
    );
    verify(&fixture, "completion won the race").await;
}

// ---------------------------------------------------------------------------------------------
// The public delay path: a run published over HTTP, advanced by the scheduler, with no worker
// connected while the loop waits on its durable timer (T014-021, FR-014-006, FR-014-014).
// ---------------------------------------------------------------------------------------------

/// A `while` loop that asks for a second iteration after its first one returns `0`, with a delay
/// between iterations.
fn delayed_loop(name: &str, delay_ms: u64, timeout_ms: u64, on_error: bool) -> serde_json::Value {
    let mut poll = json!({
        "id": "poll", "type": "while", "condition": "carry < 1", "body": "pass",
        "initial": "0", "carry": "bodyResult", "maxIterations": 3, "timeoutMs": timeout_ms,
        "delayMs": delay_ms, "next": "done"
    });
    let mut graph = vec![json!({"id": "done", "type": "end"})];
    if on_error {
        poll["onError"] = json!("failed");
        graph.push(json!({"id": "failed", "type": "end"}));
    }
    graph.insert(0, poll);
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "poll",
        "bodies": {"pass": {"entrypoint": "check", "nodes": [
            activity("check", "line.reserve", "returned", json!({})),
            json!({"id": "returned", "type": "return", "result": "steps.check.output"})
        ]}},
        "nodes": graph
    })
}

/// The shards of the flow's scheduler, taken again by a process that starts from the database
/// alone: the generation of the fence moves on, so the scheduler the flow started with is stale.
async fn retake_the_shards(flow: &Flow, name: &str) -> sagat_scheduler::Placement {
    let placement = sagat_scheduler::Placement::new(format!("scope-compensation-{name}"));
    sagat_scheduler::Keeper::new(
        flow.api.executions.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("the new process acquires the shards");
    placement
}

fn scheduler_on(flow: &Flow, placement: &sagat_scheduler::Placement) -> Sched {
    sagat_scheduler::Scheduler::new(
        flow.api.executions.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_placement(placement.clone())
}

/// Runs the first iteration to its checkpoint, which leaves only the delay timer.
async fn until_the_delay(flow: &Flow) {
    flow.drive().await;
    let first = flow.poll().await;
    assert_eq!(first.len(), 1);
    flow.complete(&first[0], json!(0)).await;
    flow.drive().await;
    assert_eq!(timer(flow).await.as_deref(), Some("WAITING"));
    assert!(
        flow.try_poll().await.expect("poll").is_empty(),
        "no task waits for a worker during the delay"
    );
}

async fn timer(flow: &Flow) -> Option<String> {
    sqlx::query_scalar(
        "SELECT lifecycle FROM loop_timers WHERE run_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(flow.run.as_uuid())
    .fetch_optional(&flow.api.db.pool)
    .await
    .expect("loop timer")
}

async fn events_of(flow: &Flow, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type=$2")
        .bind(flow.run.as_uuid())
        .bind(kind)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("event count")
}

async fn loop_usage(flow: &Flow) -> (i32, i32) {
    let reserved: i32 =
        sqlx::query_scalar("SELECT reserved_instances FROM run_instance_usage WHERE run_id=$1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("run instance usage");
    let active: i32 = sqlx::query_scalar("SELECT active_iterations FROM tenant_iteration_usage")
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("tenant iteration usage");
    (reserved, active)
}

/// Ticks `scheduler` until it advances a loop (a pass may rest its search for a moment after it
/// found nothing), returning how many loops it advanced and how many actions failed.
async fn tick_until_a_loop_advances(scheduler: &Sched) -> (u32, u32) {
    let (mut advanced, mut failed) = (0, 0);
    for _ in 0..8 {
        let report = scheduler.tick().await;
        advanced += report.loops_advanced;
        failed += report.failed;
        if advanced > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    (advanced, failed)
}

/// Ticks `scheduler` and completes what it releases until the run reaches `status`.
async fn finish_with(flow: &Flow, scheduler: &Sched, status: &str) {
    for _ in 0..40 {
        if flow.run_status().await == status {
            return;
        }
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        for task in flow.try_poll().await.expect("poll") {
            flow.complete(&task, json!(1)).await;
        }
    }
    panic!("the run never became {status}: {}", flow.run_status().await);
}

#[tokio::test]
async fn a_scheduler_restarted_during_the_delay_wakes_the_loop_exactly_once() {
    let name = format!("delay-restart-{}", Uuid::now_v7());
    let flow = Flow::start(&name, delayed_loop(&name, 2_000, 120_000, false), json!([])).await;
    until_the_delay(&flow).await;
    let version_before: i64 = sqlx::query_scalar("SELECT run_version FROM runs WHERE id=$1")
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("run version");

    // The process that scheduled the timer is gone; a new one starts from the database alone.
    let restarted = scheduler_on(&flow, &retake_the_shards(&flow, &name).await);
    let early = restarted.tick().await;
    assert_eq!((early.failed, early.loops_advanced), (0, 0), "{early:?}");
    assert_eq!(timer(&flow).await.as_deref(), Some("WAITING"));
    tokio::time::sleep(std::time::Duration::from_millis(2_300)).await;

    assert_eq!(tick_until_a_loop_advances(&restarted).await, (1, 0));
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    assert_eq!(timer(&flow).await.as_deref(), Some("FIRED"));
    let version_after: i64 = sqlx::query_scalar("SELECT run_version FROM runs WHERE id=$1")
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("run version");
    assert!(version_after > version_before);
    finish_with(&flow, &restarted, "SUCCEEDED").await;
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    assert_eq!(loop_usage(&flow).await, (0, 0));
    flow.verify().await;
}

#[tokio::test]
async fn two_replicas_delivering_the_same_due_timer_admit_one_iteration() {
    let name = format!("delay-duplicate-{}", Uuid::now_v7());
    let flow = Flow::start(&name, delayed_loop(&name, 1_500, 120_000, false), json!([])).await;
    until_the_delay(&flow).await;
    // Two replicas under the same fence both find the timer due and deliver it together.
    let placement = retake_the_shards(&flow, &name).await;
    let (first, second) = (
        scheduler_on(&flow, &placement),
        scheduler_on(&flow, &placement),
    );
    tokio::time::sleep(std::time::Duration::from_millis(1_800)).await;

    let mut advanced = 0;
    for _ in 0..8 {
        let (a, b) = tokio::join!(first.tick(), second.tick());
        advanced += a.loops_advanced + b.loops_advanced;
        if advanced > 0 {
            assert_eq!(
                advanced, 1,
                "only one replica commits the fire: {a:?} {b:?}"
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    assert_eq!(advanced, 1, "the due timer was delivered");
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    let iterations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM loop_iterations WHERE run_id=$1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("iterations");
    assert_eq!(iterations, 2, "the second iteration was admitted once");
    // A delivery that arrives after the fire is a no-op.
    let late = first.tick().await;
    assert_eq!((late.failed, late.loops_advanced), (0, 0), "{late:?}");
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    finish_with(&flow, &first, "SUCCEEDED").await;
    assert_eq!(loop_usage(&flow).await, (0, 0));
    flow.verify().await;
}

#[tokio::test]
async fn a_replica_that_lost_its_shards_cannot_fire_the_timer() {
    let name = format!("delay-stale-{}", Uuid::now_v7());
    let flow = Flow::start(&name, delayed_loop(&name, 1_500, 120_000, false), json!([])).await;
    until_the_delay(&flow).await;
    // The shards are taken over (their fence generation moves on) and the replacement process, which
    // keeps the identity, adopts the new generation: the process the flow started with is stale.
    sqlx::query(
        "UPDATE scheduler_shards SET generation=generation+1, \
                expires_at=clock_timestamp() + interval '1 hour', acquired_at=clock_timestamp()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("the fence generation moves on");
    let placement = retake_the_shards(&flow, &name).await;
    let successor = scheduler_on(&flow, &placement);
    tokio::time::sleep(std::time::Duration::from_millis(1_800)).await;

    for _ in 0..3 {
        let stale = flow.scheduler.tick().await;
        assert_eq!(stale.loops_advanced, 0, "{stale:?}");
        assert_eq!(
            stale.fenced, 1,
            "the stale replica is fenced out: {stale:?}"
        );
    }
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 0);
    assert_eq!(timer(&flow).await.as_deref(), Some("WAITING"));
    assert_eq!(tick_until_a_loop_advances(&successor).await, (1, 0));
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    finish_with(&flow, &successor, "SUCCEEDED").await;
    flow.verify().await;
}

async fn step_status(flow: &Flow, node: &str) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM steps WHERE run_id=$1 AND node=$2")
        .bind(flow.run.as_uuid())
        .bind(node)
        .fetch_optional(&flow.api.db.pool)
        .await
        .expect("step status")
}

async fn cancel(flow: &Flow) {
    let (status, body) = flow
        .operate(json!({"action": "CANCEL", "reason": "cancelled while the loop waits"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED, "{body}");
}

async fn scope_lifecycle(flow: &Flow) -> String {
    sqlx::query_scalar("SELECT lifecycle FROM loop_scopes WHERE run_id=$1")
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("loop scope")
}

/// The loop timeout of the two tests that need the deadline to pass during the delay. The window has to
/// outlast the time a loaded runner needs to reach the delay (first iteration, completion, checkpoint):
/// with 4 s the first iteration timed out before any timer existed and the test failed on CI.
const TIMEOUT_MS: u64 = 15_000;

/// The deadline passed during the delay and a cancel committed before the scheduler noticed: the
/// run-level cancellation wins and the `onError` edge is never taken (D342).
#[tokio::test]
async fn a_cancel_that_commits_before_the_timeout_is_noticed_ends_the_run_cancelled() {
    let name = format!("delay-cancel-first-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        delayed_loop(&name, 3_000, TIMEOUT_MS, true),
        json!([]),
    )
    .await;
    until_the_delay(&flow).await;
    tokio::time::sleep(std::time::Duration::from_millis(TIMEOUT_MS + 300)).await;
    cancel(&flow).await;

    flow.drive().await;
    assert_eq!(flow.run_status().await, "CANCELLED");
    assert_eq!(scope_lifecycle(&flow).await, "CANCELLED");
    assert_eq!(timer(&flow).await.as_deref(), Some("CANCELLED"));
    assert_ne!(
        step_status(&flow, "failed").await.as_deref(),
        Some("SUCCEEDED")
    );
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 0);
    assert_eq!(loop_usage(&flow).await, (0, 0));
    flow.verify().await;
}

/// The timeout drain started first; the cancel lands while it is in flight. The loop still ends
/// `FAILED` with its timeout code, the run ends `CANCELLED` and the edge is not taken (D342).
#[tokio::test]
async fn a_cancel_that_lands_after_the_timeout_started_draining_still_wins() {
    let name = format!("delay-cancel-second-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        delayed_loop(&name, 3_000, TIMEOUT_MS, true),
        json!([]),
    )
    .await;
    until_the_delay(&flow).await;
    tokio::time::sleep(std::time::Duration::from_millis(TIMEOUT_MS + 300)).await;
    let drain = flow.scheduler.tick().await;
    assert_eq!((drain.failed, drain.loops_advanced), (0, 1), "{drain:?}");
    assert_eq!(scope_lifecycle(&flow).await, "DRAINING");
    assert_ne!(
        flow.run_status().await,
        "CANCELLING",
        "a timeout drain stops nothing"
    );
    cancel(&flow).await;

    flow.drive().await;
    assert_eq!(flow.run_status().await, "CANCELLED");
    assert_eq!(scope_lifecycle(&flow).await, "FAILED");
    let owner: (String, Option<String>) =
        sqlx::query_as("SELECT status,failure_code FROM steps WHERE run_id=$1 AND node='poll'")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("loop owner");
    assert_eq!(owner.1.as_deref(), Some("RUN_TIMED_OUT"));
    assert_ne!(
        step_status(&flow, "failed").await.as_deref(),
        Some("SUCCEEDED")
    );
    assert_eq!(loop_usage(&flow).await, (0, 0));
    flow.verify().await;
}

/// A loop waits for no signal: one sent during the delay, and one sent exactly while the timer is
/// delivered, are refused with a typed `CONFLICT` and change neither the timer nor the outcome.
/// (Declared updates cannot race the timer: a declared state document is not publishable with a
/// loop, so a loop run has no update to send.)
#[tokio::test]
async fn a_signal_during_or_racing_the_delay_is_refused_and_leaves_the_loop_alone() {
    let name = format!("delay-signal-{}", Uuid::now_v7());
    let flow = Flow::start(&name, delayed_loop(&name, 1_500, 120_000, false), json!([])).await;
    until_the_delay(&flow).await;
    let signal = |key: &'static str| {
        flow.api
            .as_user(
                reqwest::Method::POST,
                &format!("/runs/{}/signals/nudge", flow.run),
                "alice",
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", key)
            .header("content-type", "application/json")
            .body("{}")
            .send()
    };
    let version = || async {
        sqlx::query_scalar::<_, i64>("SELECT run_version FROM runs WHERE id=$1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("run version")
    };
    let before = version().await;
    let during = signal("signal-during-the-delay").await.expect("signal");
    assert_eq!(during.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(version().await, before, "a refused signal writes nothing");
    assert_eq!(timer(&flow).await.as_deref(), Some("WAITING"));

    tokio::time::sleep(std::time::Duration::from_millis(1_800)).await;
    let (racing, tick) = tokio::join!(signal("signal-at-the-fire"), flow.scheduler.tick());
    assert_eq!(
        racing.expect("signal").status(),
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!((tick.failed, tick.loops_advanced), (0, 1), "{tick:?}");
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    finish_with(&flow, &flow.scheduler, "SUCCEEDED").await;
    assert_eq!(events_of(&flow, "loop.delay_elapsed").await, 1);
    assert_eq!(loop_usage(&flow).await, (0, 0));
    flow.verify().await;
}
