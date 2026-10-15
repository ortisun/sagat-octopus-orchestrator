//! Real PostgreSQL evidence for sequential-loop checkpoint and delay transactions.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::loop_progress::{
    LoopCheckpointCommand, LoopDelayElapsedCommand, LoopNextActivity, LoopProgressError,
    LoopProgressStore, LoopTransition,
};
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, LoopContinuation, RunId};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{MIGRATOR, PgLoopProgressStore};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{chain, world};

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
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let (workflow, _) = w.publish(chain(name)).await;
    let document = serde_json::to_vec(&loop_workflow(name)).expect("loop document");
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
          current_carry_digest,next_iteration,active_iteration,entered_sequence,created_at,updated_at) \
         VALUES ($1,$2,$3,$4,'{}','{}','poll','WHILE','RUNNING',3,$5,$6,$6,0,0,2,$7,$7)",
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

#[tokio::test]
async fn terminal_routing_and_destructive_down_migration_fail_closed() {
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
    assert_eq!(
        store.checkpoint(&complete).await,
        Err(LoopProgressError::RuntimeNotIntegrated)
    );
    assert_eq!(run_version(&fixture).await, 8);
    let error = MIGRATOR
        .undo(&fixture.db.pool, BEFORE_LOOP_PROGRESS)
        .await
        .expect_err("retained loop progress must block downgrade");
    assert!(error.to_string().contains("loop progress is retained"));
}
