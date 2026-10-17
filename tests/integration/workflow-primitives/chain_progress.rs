//! Real PostgreSQL evidence for private execution-chain handoff transactions.
#![allow(clippy::expect_used)]

use sagat_application::chain_progress::{
    ChainCommandService, ChainFinalizationCommand, ChainHandoffCommand, ChainProgressError,
    ChainProgressStore, ChildChainSettlementCommand,
};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::execution::{Engine, instance_step_id};
use sagat_application::primitives::SchedulerAuthority;
use sagat_application::runs::{RunOperation, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    ChainError, ChainFinalOutcome, ChainPolicy, ChainScope, ContinuationTransfer, RunId, RunStatus,
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::{PgChainProgressStore, initialize_chain_in_transaction};
use serde_json::json;
use sqlx::Row;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{
    World, activity, child_node, ctx, document, end_node, wait_node, with, world,
};

pub(crate) struct ChainFixture {
    pub(crate) world: World,
    store: PgChainProgressStore,
    pub(crate) chain: RunId,
    input: sagat_domain::workflow::Digest,
    run_version: u64,
    deadline: OffsetDateTime,
    authority: SchedulerAuthority,
    trace_id: Option<String>,
}

async fn scheduler(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "chain-progress-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation=generation+1,owner=$2, \
                expires_at=clock_timestamp()+interval '1 hour',acquired_at=clock_timestamp() \
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

pub(crate) async fn seed_quiescent_chain(name: &str, deadline_after: Duration) -> ChainFixture {
    let world = world().await;
    let (workflow, _) = world
        .publish(document(name, "work", vec![activity("work")]))
        .await;
    let deadline = crate::support::now() + deadline_after;
    let chain = world
        .start_with(
            workflow,
            &json!({ "batch": name }),
            &format!("chain-{name}-{}", Uuid::now_v7()),
            Some(deadline),
        )
        .await
        .expect("initial run");
    let tenant = world.admin.tenant_id();
    let scope = ChainScope::new(tenant, world.namespace.id());
    let mut tx = world.db.pool.begin().await.expect("chain transaction");
    initialize_chain_in_transaction(
        &mut tx,
        &sagat_application::chain_progress::ChainInitialization::new(
            scope,
            chain,
            ChainPolicy::new(3).expect("chain policy"),
        ),
    )
    .await
    .expect("initialize chain");
    let first_sequence: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant.as_uuid())
            .bind(chain.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("chain run version");
    let now = crate::support::now();
    for (offset, event_type, detail) in [
        (
            1_i64,
            "step.leased",
            json!({ "attempt": 1, "generation": 1 }),
        ),
        (2, "step.started", json!({ "attempt": 1, "generation": 1 })),
        (
            3,
            "step.completed",
            json!({ "attempt": 1, "generation": 1 }),
        ),
        (4, "step.routed", json!({ "activated": [] })),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,attempt,detail,actor,occurred_at) \
             VALUES ($1,$2,$3,$4,'work',$5,$6,$7,$8)",
        )
        .bind(tenant.as_uuid())
        .bind(chain.as_uuid())
        .bind(first_sequence + offset)
        .bind(event_type)
        .bind(if event_type == "step.routed" {
            None
        } else {
            Some(1_i32)
        })
        .bind(detail)
        .bind(world.admin.subject().as_str())
        .bind(now)
        .execute(&mut *tx)
        .await
        .expect("complete journal event");
    }
    let final_version = first_sequence + 4;
    sqlx::query(
        "UPDATE steps SET status='SUCCEEDED',attempt=1,generation=1,routed=true,updated_at=$3 \
         WHERE tenant_id=$1 AND run_id=$2 AND node='work'",
    )
    .bind(tenant.as_uuid())
    .bind(chain.as_uuid())
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("quiescent step");
    sqlx::query("UPDATE runs SET run_version=$3,updated_at=$4 WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid())
        .bind(chain.as_uuid())
        .bind(final_version)
        .bind(now)
        .execute(&mut *tx)
        .await
        .expect("quiescent run");
    tx.commit().await.expect("chain fixture commit");
    let (input, trace_id): (Vec<u8>, Option<String>) =
        sqlx::query_as("SELECT input_digest,trace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid())
            .bind(chain.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("input digest");
    let input = sagat_domain::workflow::Digest::new(input.try_into().expect("sha-256 digest"));
    let authority = scheduler(&world.db.pool, chain).await;
    let store = PgChainProgressStore::new(world.db.pool.clone(), world.cache.clone());
    ChainFixture {
        world,
        store,
        chain,
        input,
        run_version: u64::try_from(final_version).expect("positive version"),
        deadline,
        authority,
        trace_id,
    }
}

fn command(
    fixture: &ChainFixture,
    successor: RunId,
    authority: SchedulerAuthority,
    occurred_at: OffsetDateTime,
    audit: AuditRecord,
    continuation_outbox: Uuid,
    successor_outbox: Uuid,
) -> ChainHandoffCommand {
    ChainHandoffCommand::new(
        ChainScope::new(
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
        ),
        fixture.chain,
        fixture.chain,
        0,
        fixture.run_version,
        successor,
        ContinuationTransfer::new(fixture.input, None),
        authority,
        fixture.world.admin.subject().clone(),
        None,
        occurred_at,
        audit,
        continuation_outbox,
        successor_outbox,
    )
}

fn audit(fixture: &ChainFixture, occurred_at: OffsetDateTime) -> AuditRecord {
    AuditRecord::by(
        &fixture.world.admin,
        AuditAction::RunContinued,
        ResourceType::Run,
        fixture.chain.to_string(),
        None,
        occurred_at,
    )
}

fn finalization(
    fixture: &ChainFixture,
    run_outbox: Uuid,
    chain_outbox: Uuid,
) -> ChainFinalizationCommand {
    finalization_with_authority(fixture, fixture.authority.clone(), run_outbox, chain_outbox)
}

fn finalization_with_authority(
    fixture: &ChainFixture,
    authority: SchedulerAuthority,
    run_outbox: Uuid,
    chain_outbox: Uuid,
) -> ChainFinalizationCommand {
    let occurred_at = crate::support::now();
    ChainFinalizationCommand::new(
        ChainScope::new(
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
        ),
        fixture.chain,
        fixture.chain,
        0,
        fixture.run_version,
        RunStatus::Succeeded,
        authority,
        occurred_at,
        AuditRecord::engine(
            fixture.world.admin.tenant_id(),
            AuditAction::RunFinished,
            ResourceType::Run,
            fixture.chain.to_string(),
            "terminal chain outcome committed",
            fixture.trace_id.clone(),
            occurred_at,
        ),
        run_outbox,
        chain_outbox,
    )
}

async fn seed_waiting_parent(
    fixture: &ChainFixture,
    child_name: &str,
) -> (RunId, u64, SchedulerAuthority, Uuid) {
    let (workflow, version) = fixture
        .world
        .publish(document(
            &format!("parent-{child_name}"),
            "child",
            vec![
                child_node("child", child_name, Some("after")),
                activity("after"),
            ],
        ))
        .await;
    let parent = RunId::generate();
    let tenant = fixture.world.admin.tenant_id();
    let now = crate::support::now();
    let deadline = now + Duration::hours(1);
    let child = sagat_domain::workflow::NodeId::parse("child").expect("child node");
    let child_instance = sagat_domain::execution::ExecutionInstanceId::root(child.clone());
    let child_step_id = instance_step_id(version, parent, &child_instance);
    let mut tx = fixture
        .world
        .db
        .pool
        .begin()
        .await
        .expect("parent transaction");
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version,paused, \
          input_digest,started_by,deadline_at,created_at,updated_at,root_run_id,depth) \
         VALUES ($1,$2,$3,$4,$5,'WAITING',2,false,$6,$7,$8,$9,$9,$1,0)",
    )
    .bind(parent.as_uuid())
    .bind(tenant.as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(fixture.input.as_bytes().as_slice())
    .bind(fixture.world.admin.subject().as_str())
    .bind(deadline)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("parent run");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,waiting_kind,child_run_id,child_started, \
          updated_at,instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'child',$3,$4,'WAITING','CHILD',$5,true,$6,'{}','{}')",
    )
    .bind(tenant.as_uuid())
    .bind(parent.as_uuid())
    .bind(child_step_id)
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("waiting parent step");
    for (sequence, event_type, detail) in [
        (1_i64, "run.started", json!({ "entrypoint": "child" })),
        (
            2,
            "child.started",
            json!({ "child": fixture.chain.to_string() }),
        ),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,detail,actor,occurred_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(tenant.as_uuid())
        .bind(parent.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind((event_type == "child.started").then_some("child"))
        .bind(detail)
        .bind(fixture.world.admin.subject().as_str())
        .bind(now)
        .execute(&mut *tx)
        .await
        .expect("parent journal");
    }
    initialize_chain_in_transaction(
        &mut tx,
        &sagat_application::chain_progress::ChainInitialization::new(
            ChainScope::new(tenant, fixture.world.namespace.id()),
            parent,
            ChainPolicy::new(3).expect("parent chain policy"),
        ),
    )
    .await
    .expect("parent chain");
    sqlx::query(
        "INSERT INTO child_chain_links \
         (tenant_id,namespace_id,parent_run_id,parent_instance_id,child_chain_id,created_at,updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$6)",
    )
    .bind(tenant.as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(parent.as_uuid())
    .bind(child_step_id)
    .bind(fixture.chain.as_uuid())
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("parent child-chain link");
    tx.commit().await.expect("waiting parent commit");
    let authority = scheduler(&fixture.world.db.pool, parent).await;
    (parent, 3, authority, child_step_id)
}

#[tokio::test]
async fn handoff_replays_after_restart_without_releasing_a_second_successor() {
    let fixture = seed_quiescent_chain("chain-restart", Duration::hours(1)).await;
    let successor = RunId::generate();
    let now = crate::support::now();
    let continuation_outbox = Uuid::now_v7();
    let successor_outbox = Uuid::now_v7();
    let command = command(
        &fixture,
        successor,
        fixture.authority.clone(),
        now,
        audit(&fixture, now),
        continuation_outbox,
        successor_outbox,
    );
    let written = fixture
        .store
        .continue_as_new(&command)
        .await
        .expect("atomic handoff");
    assert!(!written.replayed());
    assert_eq!(written.successor(), successor);
    assert_eq!(written.successor_index(), 1);

    fixture
        .store
        .verify_chain_journal(fixture.world.admin.tenant_id(), fixture.chain)
        .await
        .expect("predecessor journal equals chain projection");
    fixture
        .store
        .verify_chain_journal(fixture.world.admin.tenant_id(), successor)
        .await
        .expect("successor journal equals chain projection");

    let restarted =
        PgChainProgressStore::new(fixture.world.db.pool.clone(), fixture.world.cache.clone());
    let replayed = restarted
        .continue_as_new(&command)
        .await
        .expect("duplicate after restart");
    assert!(replayed.replayed());
    assert_eq!(replayed.successor(), successor);

    let predecessor_graph = fixture
        .world
        .store
        .graph(fixture.world.admin.tenant_id(), fixture.chain)
        .await
        .expect("predecessor graph")
        .expect("predecessor exists");
    let predecessor_chain = predecessor_graph.chain.expect("predecessor chain links");
    assert_eq!(predecessor_chain.chain_id, fixture.chain);
    assert_eq!(predecessor_chain.segment_index, 0);
    assert_eq!(predecessor_chain.current_run_id, successor);
    assert_eq!(predecessor_chain.successor_run_id, Some(successor));
    let successor_graph = fixture
        .world
        .store
        .graph(fixture.world.admin.tenant_id(), successor)
        .await
        .expect("successor graph")
        .expect("successor exists");
    let successor_chain = successor_graph.chain.expect("successor chain links");
    assert_eq!(successor_chain.segment_index, 1);
    assert_eq!(successor_chain.predecessor_run_id, Some(fixture.chain));
    assert_eq!(successor_chain.current_run_id, successor);

    let row = sqlx::query(
        "SELECT ec.current_run_id,ec.segment_count,ec.revision,r.status AS run_status,r.deadline_at, \
                s.status AS step_status, \
                s.lease_token_hash,s.routed \
         FROM execution_chains ec JOIN runs r ON r.tenant_id=ec.tenant_id AND r.id=ec.current_run_id \
         JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
         WHERE ec.tenant_id=$1 AND ec.chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("successor projection");
    assert_eq!(row.get::<Uuid, _>("current_run_id"), successor.as_uuid());
    assert_eq!(row.get::<i32, _>("segment_count"), 2);
    assert_eq!(row.get::<i64, _>("revision"), 1);
    assert_eq!(row.get::<String, _>("run_status"), "RUNNING");
    assert_eq!(
        row.get::<OffsetDateTime, _>("deadline_at"),
        fixture.deadline
    );
    assert_eq!(row.get::<String, _>("step_status"), "READY");
    assert!(row.get::<Option<Vec<u8>>, _>("lease_token_hash").is_none());
    assert!(!row.get::<bool, _>("routed"));

    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id IN ($2,$3)), \
                (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.continued' \
                 AND resource_id=$4)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(continuation_outbox)
    .bind(successor_outbox)
    .bind(fixture.chain.to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("durable evidence counts");
    assert_eq!(counts, (2, 1));
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.world.db.pool).await;
    crate::hint_contract::assert_published(&published, &["run.continued"]);
}

#[tokio::test]
async fn final_head_closes_once_and_exposes_child_outcome_without_settling_parent() {
    let fixture = seed_quiescent_chain("chain-final", Duration::hours(1)).await;
    let parent = fixture
        .world
        .start(
            sqlx::query_scalar("SELECT workflow_id FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.chain.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .map(sagat_domain::definition::WorkflowId::new)
                .expect("workflow id"),
            &json!({}),
        )
        .await;
    sqlx::query(
        "INSERT INTO child_chain_links \
         (tenant_id,namespace_id,parent_run_id,parent_instance_id,child_chain_id,created_at,updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$6)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(parent.as_uuid())
    .bind(Uuid::now_v7())
    .bind(fixture.chain.as_uuid())
    .bind(crate::support::now())
    .execute(&fixture.world.db.pool)
    .await
    .expect("child chain link");
    let run_outbox = Uuid::now_v7();
    let chain_outbox = Uuid::now_v7();
    let command = finalization(&fixture, run_outbox, chain_outbox);
    let first = fixture
        .store
        .finalize_chain(&command)
        .await
        .expect("finalize chain");
    assert_eq!(first.outcome(), ChainFinalOutcome::Succeeded);
    assert!(!first.replayed());
    fixture
        .store
        .verify_chain_journal(fixture.world.admin.tenant_id(), fixture.chain)
        .await
        .expect("final journal equals projection");
    let restarted =
        PgChainProgressStore::new(fixture.world.db.pool.clone(), fixture.world.cache.clone());
    assert!(
        restarted
            .finalize_chain(&command)
            .await
            .expect("finalization replay")
            .replayed()
    );
    let row = sqlx::query(
        "SELECT r.status,r.finished_at,ec.final_outcome,ec.revision,cs.completion_kind, \
                l.final_outcome AS child_outcome,l.settled \
         FROM runs r JOIN execution_chains ec ON ec.tenant_id=r.tenant_id AND ec.chain_id=r.id \
         JOIN chain_segments cs ON cs.tenant_id=ec.tenant_id AND cs.chain_id=ec.chain_id \
         JOIN child_chain_links l ON l.tenant_id=ec.tenant_id AND l.child_chain_id=ec.chain_id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("final projection");
    assert_eq!(row.get::<String, _>("status"), "SUCCEEDED");
    assert!(
        row.get::<Option<OffsetDateTime>, _>("finished_at")
            .is_some()
    );
    assert_eq!(row.get::<String, _>("final_outcome"), "SUCCEEDED");
    assert_eq!(row.get::<i64, _>("revision"), 1);
    assert_eq!(row.get::<String, _>("completion_kind"), "FINAL");
    assert_eq!(row.get::<String, _>("child_outcome"), "SUCCEEDED");
    assert!(!row.get::<bool, _>("settled"));
    let outboxes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id IN ($2,$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(run_outbox)
    .bind(chain_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal outboxes");
    assert_eq!(outboxes, 2);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.finished' \
           AND resource_type='run' AND resource_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal audit");
    assert_eq!(audits, 1);
}

/// The final head of a chain is a finished run for `run.finished` subscribers, and both terminal
/// hints carry the run's trace.
#[tokio::test]
async fn final_head_is_delivered_to_run_finished_subscribers_once() {
    let fixture = seed_quiescent_chain("chain-final-subscribers", Duration::hours(1)).await;
    let subscription = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO webhook_subscriptions \
         (id,tenant_id,namespace_id,name,destination_kind,destination_settings,url,secret_name, \
          kinds,rate_per_second,timeout_ms,status,created_by) \
         VALUES ($1,$2,$3,'chain-final','WEBHOOK','{}','https://example.invalid/chain', \
                 'chain-key',ARRAY['run.finished'],100,1000,'ACTIVE','test')",
    )
    .bind(subscription)
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("run finished subscription");
    let run_outbox = Uuid::now_v7();
    let chain_outbox = Uuid::now_v7();
    let command = finalization(&fixture, run_outbox, chain_outbox);
    assert!(
        !fixture
            .store
            .finalize_chain(&command)
            .await
            .expect("finalize chain")
            .replayed()
    );
    assert!(
        fixture
            .store
            .finalize_chain(&command)
            .await
            .expect("finalization replay")
            .replayed()
    );

    let delivered: (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM webhook_deliveries d \
                  WHERE d.tenant_id=$1 AND d.subscription_id=$2), \
                (SELECT payload->>'traceId' FROM event_outbox WHERE tenant_id=$1 AND id=$3), \
                (SELECT payload->>'traceId' FROM event_outbox WHERE tenant_id=$1 AND id=$4)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(subscription)
    .bind(run_outbox)
    .bind(chain_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal deliveries");
    assert_eq!(
        delivered,
        (1, fixture.trace_id.clone(), fixture.trace_id.clone())
    );
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.world.db.pool).await;
    crate::hint_contract::assert_published(&published, &["run.finished", "chain.finished"]);
}

#[tokio::test]
async fn child_cancellation_latch_blocks_a_new_handoff() {
    let fixture = seed_quiescent_chain("chain-child-cancel", Duration::hours(1)).await;
    let workflow: Uuid =
        sqlx::query_scalar("SELECT workflow_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.chain.as_uuid())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("workflow");
    let parent = fixture
        .world
        .start(
            sagat_domain::definition::WorkflowId::new(workflow),
            &json!({}),
        )
        .await;
    sqlx::query(
        "INSERT INTO child_chain_links \
         (tenant_id,namespace_id,parent_run_id,parent_instance_id,child_chain_id,cancel_requested, \
          created_at,updated_at) VALUES ($1,$2,$3,$4,$5,true,$6,$6)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(parent.as_uuid())
    .bind(Uuid::now_v7())
    .bind(fixture.chain.as_uuid())
    .bind(crate::support::now())
    .execute(&fixture.world.db.pool)
    .await
    .expect("cancelled child link");
    let successor = RunId::generate();
    let now = crate::support::now();
    let result = fixture
        .store
        .continue_as_new(&command(
            &fixture,
            successor,
            fixture.authority.clone(),
            now,
            audit(&fixture, now),
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await;
    assert_eq!(
        result,
        Err(ChainProgressError::Chain(
            ChainError::ChildCancellationRequested
        ))
    );
}

#[tokio::test]
async fn terminal_outbox_conflict_rolls_back_the_final_event_and_chain_outcome() {
    let fixture = seed_quiescent_chain("chain-final-rollback", Duration::hours(1)).await;
    let run_outbox = Uuid::now_v7();
    let chain_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'conflict','{}')",
    )
    .bind(chain_outbox)
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("conflicting outbox identity");
    let result = fixture
        .store
        .finalize_chain(&finalization(&fixture, run_outbox, chain_outbox))
        .await;
    assert!(matches!(result, Err(ChainProgressError::Storage(_))));
    let unchanged: (String, i64, Option<String>, i64, Option<String>) = sqlx::query_as(
        "SELECT r.status,r.run_version,ec.final_outcome,ec.revision,cs.completion_kind \
         FROM runs r JOIN execution_chains ec ON ec.tenant_id=r.tenant_id AND ec.chain_id=r.id \
         JOIN chain_segments cs ON cs.tenant_id=ec.tenant_id AND cs.chain_id=ec.chain_id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back finalization");
    assert_eq!(unchanged.0, "RUNNING");
    assert_eq!(
        unchanged.1,
        i64::try_from(fixture.run_version).expect("version")
    );
    assert_eq!(unchanged.2, None);
    assert_eq!(unchanged.3, 0);
    assert_eq!(unchanged.4, None);
    let leaked_run_outbox: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM event_outbox WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(run_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("run outbox rollback");
    assert!(!leaked_run_outbox);
}

#[tokio::test]
async fn terminal_audit_failure_rolls_back_journal_projection_and_outbox() {
    let fixture = seed_quiescent_chain("chain-final-audit-rollback", Duration::hours(1)).await;
    let run_outbox = Uuid::now_v7();
    let chain_outbox = Uuid::now_v7();
    let command = finalization(&fixture, run_outbox, chain_outbox);
    sqlx::query(
        "INSERT INTO audit_records \
         (id,tenant_id,actor,action,resource_type,resource_id,created_at) \
         VALUES ($1,$2,'conflict','run.started','run',$3,$4)",
    )
    .bind(command.audit().id().as_uuid())
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.to_string())
    .bind(crate::support::now())
    .execute(&fixture.world.db.pool)
    .await
    .expect("conflicting audit identity");
    let result = fixture.store.finalize_chain(&command).await;
    assert!(matches!(result, Err(ChainProgressError::Storage(_))));
    let unchanged: (String, i64, Option<String>, i64, Option<String>) = sqlx::query_as(
        "SELECT r.status,r.run_version,ec.final_outcome,ec.revision,cs.completion_kind \
         FROM runs r JOIN execution_chains ec ON ec.tenant_id=r.tenant_id AND ec.chain_id=r.id \
         JOIN chain_segments cs ON cs.tenant_id=ec.tenant_id AND cs.chain_id=ec.chain_id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back audit failure");
    assert_eq!(unchanged.0, "RUNNING");
    assert_eq!(
        unchanged.1,
        i64::try_from(fixture.run_version).expect("version")
    );
    assert_eq!(unchanged.2, None);
    assert_eq!(unchanged.3, 0);
    assert_eq!(unchanged.4, None);
    let leaked_outboxes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id IN ($2,$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(run_outbox)
    .bind(chain_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("terminal outbox rollback");
    assert_eq!(leaked_outboxes, 0);
}

async fn initialize_chain(world: &World, chain: RunId) {
    let mut tx = world.db.pool.begin().await.expect("chain transaction");
    initialize_chain_in_transaction(
        &mut tx,
        &sagat_application::chain_progress::ChainInitialization::new(
            ChainScope::new(world.admin.tenant_id(), world.namespace.id()),
            chain,
            ChainPolicy::new(3).expect("chain policy"),
        ),
    )
    .await
    .expect("initialize chain");
    tx.commit().await.expect("chain commit");
}

#[tokio::test]
async fn chain_signal_is_delivered_only_to_an_existing_wait_and_replays_after_restart() {
    let world = world().await;
    let wait = with(
        wait_node("wait", "chain.release", "input.key", 300_000, None),
        "next",
        json!("done"),
    );
    let (workflow, _) = world
        .publish(document(
            "chain-signal",
            "wait",
            vec![wait, end_node("done")],
        ))
        .await;
    let chain = world
        .start_with(
            workflow,
            &json!({"key": "order-1"}),
            &format!("chain-signal-{}", Uuid::now_v7()),
            Some(crate::support::now() + Duration::hours(1)),
        )
        .await
        .expect("waiting chain head");
    initialize_chain(&world, chain).await;
    let service =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    let inspection = service
        .inspect(&world.admin, world.namespace.id(), chain, &ctx())
        .await
        .expect("inspect chain");
    assert_eq!(inspection.current_head, chain);
    assert_eq!(inspection.segments.len(), 1);

    let missing = service
        .signal(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "chain.unknown",
            br#"{"released":true}"#,
            "missing-signal",
            &ctx(),
        )
        .await;
    assert_eq!(
        missing,
        Err(AppError::StateConflict(
            "the chain head is not waiting for that signal"
        ))
    );

    let payload = br#"{"released":true}"#;
    let delivered = service
        .signal(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "chain.release",
            payload,
            "release-signal",
            &ctx(),
        )
        .await
        .expect("deliver chain signal");
    assert_eq!(
        delivered
            .iter()
            .map(|node| node.as_str())
            .collect::<Vec<_>>(),
        ["wait"]
    );
    let restarted =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    assert_eq!(
        restarted
            .signal(
                &world.admin,
                world.namespace.id(),
                chain,
                Some(chain),
                "chain.release",
                payload,
                "release-signal",
                &ctx(),
            )
            .await
            .expect("replay chain signal"),
        delivered
    );
    world
        .store
        .verify_run(world.admin.tenant_id(), chain)
        .await
        .expect("signal journal projection");
}

#[tokio::test]
async fn chain_cancel_latches_before_stopping_the_head_and_replays_after_restart() {
    let world = world().await;
    let (workflow, _) = world
        .publish(document("chain-cancel", "work", vec![activity("work")]))
        .await;
    let chain = world
        .start_with(
            workflow,
            &json!({"batch": "cancel"}),
            &format!("chain-cancel-{}", Uuid::now_v7()),
            Some(crate::support::now() + Duration::hours(1)),
        )
        .await
        .expect("chain head");
    initialize_chain(&world, chain).await;
    let service =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    service
        .cancel(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "operator cancelled the chain",
            "cancel-chain",
            &ctx(),
        )
        .await
        .expect("cancel chain");
    let restarted =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    restarted
        .cancel(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "operator cancelled the chain",
            "cancel-chain",
            &ctx(),
        )
        .await
        .expect("replay chain cancel");
    let inspection = restarted
        .inspect(&world.admin, world.namespace.id(), chain, &ctx())
        .await
        .expect("inspect cancelled chain");
    assert!(inspection.cancel_requested);
    assert_eq!(inspection.revision, 2);
    assert_eq!(world.run_status(chain).await, RunStatus::Cancelled);
    world
        .store
        .verify_run(world.admin.tenant_id(), chain)
        .await
        .expect("cancel journal projection");
}

#[tokio::test]
async fn chain_cancel_audit_failure_rolls_back_latch_journal_and_projection() {
    let world = world().await;
    let (workflow, _) = world
        .publish(document(
            "chain-cancel-rollback",
            "work",
            vec![activity("work")],
        ))
        .await;
    let chain = world
        .start_with(
            workflow,
            &json!({"batch": "rollback"}),
            &format!("chain-cancel-rollback-{}", Uuid::now_v7()),
            Some(crate::support::now() + Duration::hours(1)),
        )
        .await
        .expect("chain head");
    initialize_chain(&world, chain).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_chain_cancel_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action='run.cancel_requested' THEN RAISE EXCEPTION 'audit rejected'; \
         END IF; RETURN NEW; END $$; \
         CREATE TRIGGER reject_chain_cancel_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_chain_cancel_audit()",
    )
    .execute(&world.db.pool)
    .await
    .expect("audit fault");
    let service =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    assert_eq!(
        service
            .cancel(
                &world.admin,
                world.namespace.id(),
                chain,
                Some(chain),
                "must roll back",
                "cancel-rollback",
                &ctx(),
            )
            .await,
        Err(AppError::Internal)
    );
    let unchanged: (bool, i64, String) = sqlx::query_as(
        "SELECT ec.cancel_requested,ec.revision,r.status FROM execution_chains ec \
         JOIN runs r ON r.tenant_id=ec.tenant_id AND r.id=ec.current_run_id \
         WHERE ec.tenant_id=$1 AND ec.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("rollback state");
    assert_eq!(unchanged, (false, 0, "RUNNING".to_owned()));
    world
        .store
        .verify_run(world.admin.tenant_id(), chain)
        .await
        .expect("unchanged journal projection");
}

#[tokio::test]
async fn chain_inspection_is_tenant_scoped_and_audits_a_foreign_probe() {
    let fixture = seed_quiescent_chain("chain-inspection-tenant", Duration::hours(1)).await;
    let service = ChainCommandService::new(
        fixture.world.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
    );
    assert_eq!(
        service
            .inspect(
                &fixture.world.other_admin,
                fixture.world.other_namespace.id(),
                fixture.chain,
                &ctx(),
            )
            .await,
        Err(AppError::NotFound)
    );
    fixture.world.store.flush_denial_writes().await;
    let denied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM audit_records WHERE tenant_id=$1 \
         AND action='access.denied.cross_tenant' AND resource_id=$2)",
    )
    .bind(fixture.world.other_admin.tenant_id().as_uuid())
    .bind(fixture.chain.to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("denial audit");
    assert!(denied);
}

#[tokio::test]
async fn direct_run_signal_and_cancel_preserve_chain_lock_and_terminal_projection() {
    let world = world().await;
    let (signal_workflow, _) = world
        .publish(document(
            "direct-chain-signal",
            "wait",
            vec![wait_node(
                "wait",
                "chain.direct",
                "input.key",
                300_000,
                None,
            )],
        ))
        .await;
    let signalled = world
        .start_with(
            signal_workflow,
            &json!({"key": "direct"}),
            &format!("direct-chain-signal-{}", Uuid::now_v7()),
            Some(crate::support::now() + Duration::hours(1)),
        )
        .await
        .expect("signal chain");
    initialize_chain(&world, signalled).await;
    world
        .runs
        .signal(
            &world.admin,
            signalled,
            "chain.direct",
            br#"{"ok":true}"#,
            "direct-chain-signal",
            &ctx(),
        )
        .await
        .expect("direct run signal");

    let (cancel_workflow, _) = world
        .publish(document(
            "direct-chain-cancel",
            "work",
            vec![activity("work")],
        ))
        .await;
    let cancelled = world
        .start_with(
            cancel_workflow,
            &json!({"batch": "direct"}),
            &format!("direct-chain-cancel-{}", Uuid::now_v7()),
            Some(crate::support::now() + Duration::hours(1)),
        )
        .await
        .expect("cancel chain");
    initialize_chain(&world, cancelled).await;
    world
        .runs
        .operate(
            &world.admin,
            cancelled,
            RunOperation::Cancel,
            "cancel through the run command",
            None,
            "direct-chain-cancel",
            &ctx(),
        )
        .await
        .expect("direct run cancel");

    let service =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    let signal_view = service
        .inspect(&world.admin, world.namespace.id(), signalled, &ctx())
        .await
        .expect("signalled chain projection");
    assert_eq!(
        signal_view.final_outcome,
        Some(ChainFinalOutcome::Succeeded)
    );
    assert_eq!(signal_view.revision, 1);
    let cancel_view = service
        .inspect(&world.admin, world.namespace.id(), cancelled, &ctx())
        .await
        .expect("cancelled chain projection");
    assert!(cancel_view.cancel_requested);
    assert_eq!(
        cancel_view.final_outcome,
        Some(ChainFinalOutcome::Cancelled)
    );
    assert_eq!(cancel_view.revision, 2);
    world
        .store
        .verify_run(world.admin.tenant_id(), signalled)
        .await
        .expect("direct signal replay");
    world
        .store
        .verify_run(world.admin.tenant_id(), cancelled)
        .await
        .expect("direct cancel replay");
}

#[tokio::test]
async fn chain_cancel_and_handoff_serialize_on_one_head() {
    let fixture = seed_quiescent_chain("chain-cancel-handoff-race", Duration::hours(1)).await;
    let service = ChainCommandService::new(
        fixture.world.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
    );
    let successor = RunId::generate();
    let now = crate::support::now();
    let handoff_command = command(
        &fixture,
        successor,
        fixture.authority.clone(),
        now,
        audit(&fixture, now),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let request = ctx();
    let cancel = service.cancel(
        &fixture.world.admin,
        fixture.world.namespace.id(),
        fixture.chain,
        Some(fixture.chain),
        "cancel racing with continuation",
        "cancel-handoff-race",
        &request,
    );
    let handoff = fixture.store.continue_as_new(&handoff_command);
    let (cancelled, continued) = tokio::join!(cancel, handoff);
    assert_ne!(cancelled.is_ok(), continued.is_ok());

    let inspection = service
        .inspect(
            &fixture.world.admin,
            fixture.world.namespace.id(),
            fixture.chain,
            &ctx(),
        )
        .await
        .expect("inspect serialized chain");
    if cancelled.is_ok() {
        assert!(inspection.cancel_requested);
        assert_eq!(inspection.current_head, fixture.chain);
        assert_eq!(
            fixture.world.run_status(fixture.chain).await,
            RunStatus::Cancelled
        );
    } else {
        assert_eq!(inspection.current_head, successor);
        assert!(!inspection.cancel_requested);
    }
}

#[tokio::test]
async fn final_child_outcome_wakes_parent_once_after_restart() {
    let fixture = seed_quiescent_chain("chain-child-settlement", Duration::hours(1)).await;
    let (parent, parent_version, _, _) =
        seed_waiting_parent(&fixture, "chain-child-settlement").await;
    let child_authority = scheduler(&fixture.world.db.pool, fixture.chain).await;
    fixture
        .store
        .finalize_chain(&finalization_with_authority(
            &fixture,
            child_authority,
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await
        .expect("final child outcome");
    let parent_authority = scheduler(&fixture.world.db.pool, parent).await;
    let task_outbox = Uuid::now_v7();
    let command = ChildChainSettlementCommand::new(
        ChainScope::new(
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
        ),
        parent,
        parent,
        0,
        parent_version,
        fixture.chain,
        parent_authority,
        crate::support::now(),
        task_outbox,
    );
    let applied = fixture
        .store
        .settle_child_chain(&command)
        .await
        .expect("settle child chain");
    assert!(!applied.replayed());
    assert_eq!(applied.parent_run_id(), parent);
    let restarted =
        PgChainProgressStore::new(fixture.world.db.pool.clone(), fixture.world.cache.clone());
    assert!(
        restarted
            .settle_child_chain(&command)
            .await
            .expect("settlement replay")
            .replayed()
    );
    restarted
        .verify_chain_journal(fixture.world.admin.tenant_id(), parent)
        .await
        .expect("parent journal equals projection");
    let rows = sqlx::query(
        "SELECT l.settled,child.status AS child_status,child.routed,ready.status AS ready_status, \
                ready.lease_token_hash,r.run_version \
         FROM child_chain_links l JOIN runs r ON r.tenant_id=l.tenant_id AND r.id=l.parent_run_id \
         JOIN steps child ON child.tenant_id=r.tenant_id AND child.run_id=r.id AND child.node='child' \
         JOIN steps ready ON ready.tenant_id=r.tenant_id AND ready.run_id=r.id AND ready.node='after' \
         WHERE l.tenant_id=$1 AND l.child_chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("settled parent projection");
    assert!(rows.get::<bool, _>("settled"));
    assert_eq!(rows.get::<String, _>("child_status"), "SUCCEEDED");
    assert!(rows.get::<bool, _>("routed"));
    assert_eq!(rows.get::<String, _>("ready_status"), "READY");
    assert!(rows.get::<Option<Vec<u8>>, _>("lease_token_hash").is_none());
    assert_eq!(rows.get::<i64, _>("run_version"), 6);
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id=$2 \
           AND aggregate_id=$3 AND event_type='task.ready'",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(task_outbox)
    .bind(parent.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("one parent wake");
    assert_eq!(outbox_count, 1);
}

#[tokio::test]
async fn parent_wake_outbox_failure_rolls_back_settlement_and_journal() {
    let fixture = seed_quiescent_chain("chain-parent-rollback", Duration::hours(1)).await;
    let (parent, parent_version, _, _) =
        seed_waiting_parent(&fixture, "chain-parent-rollback").await;
    let child_authority = scheduler(&fixture.world.db.pool, fixture.chain).await;
    fixture
        .store
        .finalize_chain(&finalization_with_authority(
            &fixture,
            child_authority,
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await
        .expect("final child outcome");
    let parent_authority = scheduler(&fixture.world.db.pool, parent).await;
    let task_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
         VALUES ($1,$2,$3,'conflict','{}')",
    )
    .bind(task_outbox)
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(parent.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("conflicting task outbox");
    let result = fixture
        .store
        .settle_child_chain(&ChildChainSettlementCommand::new(
            ChainScope::new(
                fixture.world.admin.tenant_id(),
                fixture.world.namespace.id(),
            ),
            parent,
            parent,
            0,
            parent_version,
            fixture.chain,
            parent_authority,
            crate::support::now(),
            task_outbox,
        ))
        .await;
    assert!(matches!(result, Err(ChainProgressError::Storage(_))));
    let unchanged: (bool, String, bool, i64, bool) = sqlx::query_as(
        "SELECT l.settled,s.status,s.routed,r.run_version, \
                EXISTS(SELECT 1 FROM steps ready WHERE ready.tenant_id=r.tenant_id \
                  AND ready.run_id=r.id AND ready.node='after') \
         FROM child_chain_links l JOIN runs r ON r.tenant_id=l.tenant_id AND r.id=l.parent_run_id \
         JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node='child' \
         WHERE l.tenant_id=$1 AND l.child_chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back settlement");
    assert_eq!(unchanged, (false, "WAITING".to_owned(), false, 3, false));
}

#[tokio::test]
async fn non_successful_child_outcome_remains_closed_without_mutating_parent() {
    let fixture = seed_quiescent_chain("chain-parent-failed", Duration::hours(1)).await;
    let (parent, parent_version, parent_authority, _) =
        seed_waiting_parent(&fixture, "chain-parent-failed").await;
    sqlx::query(
        "UPDATE child_chain_links SET final_outcome='FAILED',updated_at=$3 \
         WHERE tenant_id=$1 AND child_chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .bind(crate::support::now())
    .execute(&fixture.world.db.pool)
    .await
    .expect("failed child outcome");
    let task_outbox = Uuid::now_v7();
    let result = fixture
        .store
        .settle_child_chain(&ChildChainSettlementCommand::new(
            ChainScope::new(
                fixture.world.admin.tenant_id(),
                fixture.world.namespace.id(),
            ),
            parent,
            parent,
            0,
            parent_version,
            fixture.chain,
            parent_authority,
            crate::support::now(),
            task_outbox,
        ))
        .await;
    assert_eq!(result, Err(ChainProgressError::RuntimeNotIntegrated));
    let unchanged: (bool, String, bool, i64, bool) = sqlx::query_as(
        "SELECT l.settled,s.status,s.routed,r.run_version, \
                EXISTS(SELECT 1 FROM event_outbox WHERE tenant_id=l.tenant_id AND id=$3) \
         FROM child_chain_links l JOIN runs r ON r.tenant_id=l.tenant_id AND r.id=l.parent_run_id \
         JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node='child' \
         WHERE l.tenant_id=$1 AND l.child_chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .bind(task_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("unchanged failed settlement");
    assert_eq!(unchanged, (false, "WAITING".to_owned(), false, 3, false));
}

#[tokio::test]
async fn stale_scheduler_fence_cannot_move_the_chain_head() {
    let fixture = seed_quiescent_chain("chain-stale-fence", Duration::hours(1)).await;
    let stale = fixture.authority.clone();
    let _current = scheduler(&fixture.world.db.pool, fixture.chain).await;
    let successor = RunId::generate();
    let now = crate::support::now();
    let result = fixture
        .store
        .continue_as_new(&command(
            &fixture,
            successor,
            stale,
            now,
            audit(&fixture, now),
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await;
    assert_eq!(result, Err(ChainProgressError::SchedulerFenceLost));
    let head: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("unchanged head");
    assert_eq!(head, fixture.chain.as_uuid());
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2)")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(successor.as_uuid())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("successor absence");
    assert!(!exists);
}

#[tokio::test]
async fn deadline_boundary_fails_closed_before_any_successor_write() {
    let fixture = seed_quiescent_chain("chain-deadline", Duration::seconds(1)).await;
    let successor = RunId::generate();
    let result = fixture
        .store
        .continue_as_new(&command(
            &fixture,
            successor,
            fixture.authority.clone(),
            fixture.deadline,
            audit(&fixture, fixture.deadline),
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await;
    assert_eq!(result, Err(ChainProgressError::RuntimeNotIntegrated));
    let projection: (Uuid, i32, i64) = sqlx::query_as(
        "SELECT current_run_id,segment_count,revision FROM execution_chains \
         WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("unchanged chain");
    assert_eq!(projection, (fixture.chain.as_uuid(), 1, 0));
}

#[tokio::test]
async fn persisted_active_run_limit_blocks_replacement_without_partial_writes() {
    let fixture = seed_quiescent_chain("chain-capacity", Duration::hours(1)).await;
    sqlx::query(
        "INSERT INTO quota_limits \
         (tenant_id,namespace_id,resource,max_value,updated_by,updated_at) \
         VALUES ($1,$2,'active_runs',0,$3,$4) \
         ON CONFLICT (tenant_id,coalesce(namespace_id, \
           '00000000-0000-0000-0000-000000000000'::uuid),resource) \
         DO UPDATE SET max_value=0,updated_by=EXCLUDED.updated_by,updated_at=EXCLUDED.updated_at",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.world.namespace.id().as_uuid())
    .bind(fixture.world.admin.subject().as_str())
    .bind(crate::support::now())
    .execute(&fixture.world.db.pool)
    .await
    .expect("persist zero active-run limit");
    let successor = RunId::generate();
    let now = crate::support::now();
    let result = fixture
        .store
        .continue_as_new(&command(
            &fixture,
            successor,
            fixture.authority.clone(),
            now,
            audit(&fixture, now),
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await;
    assert_eq!(result, Err(ChainProgressError::AtCapacity));
    let projection: (Uuid, i32, i64) = sqlx::query_as(
        "SELECT current_run_id,segment_count,revision FROM execution_chains \
         WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("unchanged chain");
    assert_eq!(projection, (fixture.chain.as_uuid(), 1, 0));
}

#[tokio::test]
async fn audit_insert_failure_rolls_back_runs_journal_outbox_and_lineage() {
    let fixture = seed_quiescent_chain("chain-audit-rollback", Duration::hours(1)).await;
    let successor = RunId::generate();
    let now = crate::support::now();
    let audit = audit(&fixture, now);
    sqlx::query(
        "INSERT INTO audit_records \
         (id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(audit.id().as_uuid())
    .bind(audit.tenant_id().as_uuid())
    .bind(audit.actor().as_str())
    .bind(audit.action().as_str())
    .bind(audit.resource_type().as_str())
    .bind(audit.resource_id())
    .bind(audit.reason())
    .bind(audit.trace_id())
    .bind(audit.created_at())
    .execute(&fixture.world.db.pool)
    .await
    .expect("conflicting audit identity");
    let continuation_outbox = Uuid::now_v7();
    let successor_outbox = Uuid::now_v7();
    let result = fixture
        .store
        .continue_as_new(&command(
            &fixture,
            successor,
            fixture.authority.clone(),
            now,
            audit,
            continuation_outbox,
            successor_outbox,
        ))
        .await;
    assert!(matches!(result, Err(ChainProgressError::Storage(_))));
    let unchanged: (String, i64, Uuid, i32, i64) = sqlx::query_as(
        "SELECT r.status,r.run_version,ec.current_run_id,ec.segment_count,ec.revision \
         FROM runs r JOIN execution_chains ec ON ec.tenant_id=r.tenant_id AND ec.chain_id=r.id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rolled back projection");
    assert_eq!(unchanged.0, "RUNNING");
    assert_eq!(
        unchanged.1,
        i64::try_from(fixture.run_version).expect("version")
    );
    assert_eq!(unchanged.2, fixture.chain.as_uuid());
    assert_eq!((unchanged.3, unchanged.4), (1, 0));
    let leaked: (bool, i64) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2), \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id IN ($3,$4))",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(successor.as_uuid())
    .bind(continuation_outbox)
    .bind(successor_outbox)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rollback evidence");
    assert_eq!(leaked, (false, 0));
}
