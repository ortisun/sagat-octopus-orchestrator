//! Public continuation-chain races, rollback and restart recovery against real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::chain_progress::ChainCommandService;
use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{ctx, world};

fn continuation(name: &str, activity: &str, max_segments: u16) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": activity,
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": {"maxSegments": max_segments, "timeoutMs": 3_600_000}
    })
}

async fn placement(world: &crate::support::engine::World, name: &str) -> Placement {
    let placement = Placement::new(format!("chain-fault-{name}-{}", Uuid::now_v7()));
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
    placement
}

fn scheduler(
    world: &crate::support::engine::World,
    placement: Placement,
) -> Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement)
}

#[tokio::test]
async fn handoff_and_cancel_race_commit_exactly_one_chain_transition() {
    let world = world().await;
    let name = format!("chain-cancel-race-{}", Uuid::now_v7());
    let (workflow, _) = world.publish(continuation(&name, "chain.race", 3)).await;
    let chain = world.start(workflow, &json!({"next": {"next": {}}})).await;
    let session = world.session(&["chain.race"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete predecessor activity");

    let placement = placement(&world, "cancel-race").await;
    let scheduler = scheduler(&world, placement);
    let commands =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    let boundary = NodeId::parse("rollover").expect("continuation node");
    let request = ctx();
    let (handoff, cancel) = tokio::join!(
        scheduler.advance_continuation(
            world.admin.tenant_id(),
            world.namespace.id(),
            chain,
            &boundary,
        ),
        commands.cancel(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "cancel during handoff",
            "chain-cancel-race-key",
            &request,
        ),
    );
    assert_ne!(handoff.is_ok(), cancel.is_ok(), "{handoff:?} {cancel:?}");

    let projection: (Uuid, i32, bool, Option<String>, i64) = sqlx::query_as(
        "SELECT chain.current_run_id,chain.segment_count,chain.cancel_requested, \
                chain.final_outcome, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=chain.tenant_id \
                   AND event.run_id=$2 AND event.event_type='continuation.selected') \
         FROM execution_chains chain WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("serialized chain result");
    if handoff.is_ok() {
        assert_ne!(projection.0, chain.as_uuid());
        assert_eq!(projection.1, 2);
        assert!(!projection.2);
        assert_eq!(projection.3, None);
        assert_eq!(projection.4, 1);
    } else {
        assert_eq!(projection.0, chain.as_uuid());
        assert_eq!(projection.1, 1);
        assert!(projection.2);
        assert_eq!(projection.3.as_deref(), Some("CANCELLED"));
        assert_eq!(projection.4, 0);
    }
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn restart_recovers_the_successor_when_the_handoff_hint_is_lost() {
    let world = world().await;
    let name = format!("chain-lost-hint-{}", Uuid::now_v7());
    let (workflow, _) = world.publish(continuation(&name, "chain.recover", 3)).await;
    let chain = world.start(workflow, &json!({"next": {"next": {}}})).await;
    let session = world.session(&["chain.recover"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete predecessor activity");

    let first = scheduler(&world, placement(&world, "lost-hint-first").await);
    let committed = first.tick().await;
    assert_eq!(committed.continuations_advanced, 1, "{committed:?}");
    assert_eq!(committed.failed, 0, "{committed:?}");
    let successor: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("successor head");
    let pending_hint: (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER (WHERE delivered_at IS NULL) FROM event_outbox \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(successor)
    .fetch_one(&world.db.pool)
    .await
    .expect("undelivered successor hint");
    assert_eq!(pending_hint, (1, 1));

    let restarted = world.replica().await;
    sqlx::query(
        "UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second' \
         WHERE expires_at>clock_timestamp()",
    )
    .execute(&world.db.pool)
    .await
    .expect("expire crashed scheduler leases");
    let restarted_placement = Placement::new(format!("chain-fault-restart-{}", Uuid::now_v7()));
    Keeper::new(
        restarted.store.clone(),
        restarted_placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("restart acquires scheduler shards");
    let replay = Scheduler::new(
        restarted.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(restarted_placement)
    .tick()
    .await;
    assert_eq!(replay.continuations_advanced, 0, "{replay:?}");
    assert_eq!(replay.failed, 0, "{replay:?}");
    let tasks = restarted
        .workers
        .poll(
            &session.principal,
            session.worker_id,
            &session.token,
            8,
            None,
        )
        .await
        .expect("fresh replica polls durable successor");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].run_id.as_uuid(), successor);
    assert_eq!(tasks[0].activity.as_str(), "chain.recover");
    let lineage: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND chain_id=$2), \
            (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
               AND event_type='continuation.selected')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("single durable handoff");
    assert_eq!(lineage, (2, 1));
}

#[tokio::test]
async fn signalled_wait_handoff_rolls_back_with_outbox_and_retries_once() {
    let world = world().await;
    let name = format!("chain-wait-rollback-{}", Uuid::now_v7());
    let mut definition = continuation(&name, "chain.prepare", 3);
    definition["nodes"][0]["next"] = json!("release");
    definition["nodes"].as_array_mut().expect("nodes").insert(
        1,
        json!({
            "id": "release", "type": "waitEvent", "eventType": "chain.release",
            "correlationExpr": "input.key", "timeoutMs": 300_000, "next": "rollover"
        }),
    );
    let (workflow, _) = world.publish(definition).await;
    let chain = world
        .start(
            workflow,
            &json!({"key": "order-1", "next": {"key": "order-2"}}),
        )
        .await;
    let session = world.session(&["chain.prepare"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete entry activity");
    let commands =
        ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator));
    let delivered = commands
        .signal(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "chain.release",
            br#"{"released":true}"#,
            "chain-wait-rollback-signal",
            &ctx(),
        )
        .await
        .expect("deliver signal");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].as_str(), "release");

    sqlx::raw_sql(
        "CREATE FUNCTION reject_signalled_handoff_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected handoff outbox failure'; END $$; \
         CREATE TRIGGER reject_signalled_handoff_outbox BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type='run.continued') \
         EXECUTE FUNCTION reject_signalled_handoff_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("install handoff fault");
    let scheduler = scheduler(&world, placement(&world, "wait-rollback").await);
    let boundary = NodeId::parse("rollover").expect("continuation node");
    assert!(
        scheduler
            .advance_continuation(
                world.admin.tenant_id(),
                world.namespace.id(),
                chain,
                &boundary
            )
            .await
            .is_err()
    );
    let rolled_back: (Uuid, i32, i64) = sqlx::query_as(
        "SELECT current_run_id,segment_count, \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
                   AND event_type='continuation.selected') \
         FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("rolled back handoff");
    assert_eq!(rolled_back, (chain.as_uuid(), 1, 0));

    sqlx::raw_sql(
        "DROP TRIGGER reject_signalled_handoff_outbox ON event_outbox; \
         DROP FUNCTION reject_signalled_handoff_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("remove handoff fault");
    let advanced = scheduler
        .advance_continuation(
            world.admin.tenant_id(),
            world.namespace.id(),
            chain,
            &boundary,
        )
        .await
        .expect("retry handoff");
    assert_eq!(advanced.predecessor(), chain);
    assert!(!advanced.replayed());
    let committed: (Uuid, i32, i64) = sqlx::query_as(
        "SELECT current_run_id,segment_count, \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
                   AND event_type='continuation.selected') \
         FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("committed handoff");
    assert_ne!(committed.0, chain.as_uuid());
    assert_eq!((committed.1, committed.2), (2, 1));
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn chain_limit_outbox_failure_rolls_back_and_retry_finalizes_once() {
    let world = world().await;
    let name = format!("chain-limit-rollback-{}", Uuid::now_v7());
    let (workflow, _) = world.publish(continuation(&name, "chain.limit", 2)).await;
    let chain = world
        .start(workflow, &json!({"next": {"next": {"unexpected": true}}}))
        .await;
    let session = world.session(&["chain.limit"]).await;
    let scheduler = scheduler(&world, placement(&world, "limit").await);
    let first = world.one_task(&session).await;
    world
        .complete(&session, &first, &json!({"done": true}))
        .await
        .expect("complete first segment");
    let handoff = scheduler.tick().await;
    assert_eq!(handoff.continuations_advanced, 1, "{handoff:?}");
    let second = world.one_task(&session).await;
    world
        .complete(&session, &second, &json!({"done": true}))
        .await
        .expect("complete last admitted segment");

    sqlx::raw_sql(
        "CREATE FUNCTION reject_chain_limit_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected chain limit outbox failure'; END $$; \
         CREATE TRIGGER reject_chain_limit_outbox BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type='chain.finished') \
         EXECUTE FUNCTION reject_chain_limit_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("install chain limit fault");
    let rejected = scheduler.tick().await;
    assert_eq!(rejected.continuations_advanced, 0, "{rejected:?}");
    assert_eq!(rejected.failed, 1, "{rejected:?}");
    let rolled_back: (Option<String>, String, Option<String>, String, i64, i64) = sqlx::query_as(
        "SELECT chain.final_outcome,run.status,run.stop_code,step.status, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                   AND event.run_id=run.id AND event.event_type='run.finished'), \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                   AND audit.resource_id=run.id::text AND audit.reason='chain segment limit exhausted') \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
              AND step.node='rollover' AND cardinality(step.instance_path_nodes)=0 \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("rolled back chain limit");
    assert_eq!(
        rolled_back,
        (None, "RUNNING".into(), None, "PENDING".into(), 0, 0)
    );
    sqlx::raw_sql(
        "DROP TRIGGER reject_chain_limit_outbox ON event_outbox; \
         DROP FUNCTION reject_chain_limit_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("remove chain limit fault");

    let retried = scheduler.tick().await;
    assert_eq!(retried.continuations_advanced, 1, "{retried:?}");
    assert_eq!(retried.failed, 0, "{retried:?}");
    let terminal: (Option<String>, String, Option<String>, i64, i64) = sqlx::query_as(
        "SELECT chain.final_outcome,run.status,run.stop_code, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                   AND event.run_id=run.id AND event.event_type='run.finished'), \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                   AND audit.resource_id=run.id::text AND audit.reason='chain segment limit exhausted') \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("terminal chain limit");
    assert_eq!(
        terminal,
        (
            Some("FAILED".into()),
            "FAILED".into(),
            Some("CHAIN_LIMIT".into()),
            1,
            1,
        )
    );
    let repeated = scheduler.tick().await;
    assert_eq!(repeated.continuations_advanced, 0, "{repeated:?}");
    assert_eq!(repeated.failed, 0, "{repeated:?}");
}

#[tokio::test]
async fn continuation_expression_failure_rolls_back_and_retry_finalizes_once() {
    let world = world().await;
    let name = format!("chain-expression-rollback-{}", Uuid::now_v7());
    let mut definition = continuation(&name, "chain.expression", 2);
    definition["nodes"][1]["input"] = json!("input.next.value");
    let (workflow, _) = world.publish(definition).await;
    let chain = world.start(workflow, &json!({"next": {}})).await;
    let session = world.session(&["chain.expression"]).await;
    let scheduler = scheduler(&world, placement(&world, "expression").await);
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete activity");

    sqlx::raw_sql(
        "CREATE FUNCTION reject_chain_expression_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected chain expression outbox failure'; END $$; \
         CREATE TRIGGER reject_chain_expression_outbox BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type='chain.finished') \
         EXECUTE FUNCTION reject_chain_expression_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("install expression fault");
    let rejected = scheduler.tick().await;
    assert_eq!(rejected.continuations_advanced, 0, "{rejected:?}");
    assert_eq!(rejected.failed, 1, "{rejected:?}");
    let rolled_back: (Option<String>, String, Option<String>, String, i64, i64) = sqlx::query_as(
        "SELECT chain.final_outcome,run.status,run.stop_code,step.status, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                   AND event.run_id=run.id AND event.event_type='run.stop_requested'), \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                   AND audit.resource_id=run.id::text AND audit.reason='continuation expression failed') \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
              AND step.node='rollover' AND cardinality(step.instance_path_nodes)=0 \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("rolled back expression failure");
    assert_eq!(
        rolled_back,
        (None, "RUNNING".into(), None, "PENDING".into(), 0, 0)
    );
    sqlx::raw_sql(
        "DROP TRIGGER reject_chain_expression_outbox ON event_outbox; \
         DROP FUNCTION reject_chain_expression_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("remove expression fault");

    let retried = scheduler.tick().await;
    assert_eq!(retried.continuations_advanced, 1, "{retried:?}");
    assert_eq!(retried.failed, 0, "{retried:?}");
    let terminal: (Option<String>, String, Option<String>, i64, i64) = sqlx::query_as(
        "SELECT chain.final_outcome,run.status,run.stop_code, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                   AND event.run_id=run.id AND event.event_type='run.stop_requested' \
                   AND event.detail->>'code'='EXPRESSION_ERROR'), \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                   AND audit.resource_id=run.id::text AND audit.reason='continuation expression failed') \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("terminal expression failure");
    assert_eq!(
        terminal,
        (
            Some("FAILED".into()),
            "FAILED".into(),
            Some("EXPRESSION_ERROR".into()),
            1,
            1
        )
    );
    let repeated = scheduler.tick().await;
    assert_eq!(repeated.continuations_advanced, 0, "{repeated:?}");
    assert_eq!(repeated.failed, 0, "{repeated:?}");
}

#[tokio::test]
async fn handoff_keeps_the_started_definition_version_after_a_new_publish() {
    let world = world().await;
    let name = format!("chain-version-pin-{}", Uuid::now_v7());
    let (workflow, first_version) = world
        .publish(continuation(&name, "chain.version-one", 3))
        .await;
    let chain = world.start(workflow, &json!({"next": {"next": {}}})).await;
    let second_version = world
        .publish_new_version(workflow, continuation(&name, "chain.version-two", 3))
        .await;
    assert_ne!(first_version, second_version);
    let session = world
        .session(&["chain.version-one", "chain.version-two"])
        .await;
    let first = world.one_task(&session).await;
    assert_eq!(first.activity.as_str(), "chain.version-one");
    world
        .complete(&session, &first, &json!({"done": true}))
        .await
        .expect("complete pinned predecessor");
    let scheduler = scheduler(&world, placement(&world, "version-pin").await);
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    let pinned: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT chain.current_run_id,run.workflow_version_id,step.activity \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
              AND step.node='work' AND cardinality(step.instance_path_nodes)=0 \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("pinned successor definition");
    assert_ne!(pinned.0, chain.as_uuid());
    assert_eq!(pinned.1, first_version.as_uuid());
    assert_eq!(pinned.2, "chain.version-one");
}

// ------------------------------------------------------------------ ordered races and real faults
//
// The races below do not sleep to decide who goes first. A test transaction holds a lock; each
// competitor is started, and the test waits until PostgreSQL reports that backend blocked on a lock
// before it starts the next one. Locks are granted in arrival order, so releasing the gate commits
// the competitors in exactly the order they were started, and the opposite order is a second run.
//
// A hand-off reads the boundary in one transaction and writes the successor in a second one, and
// a command may commit between the two. The chain gate therefore orders the *reads*: it decides
// whether the hand-off or a command is first to look. The quota gate (the advisory lock the write
// takes after the chain and run locks) holds the hand-off *inside* its write, so a command started
// meanwhile queues behind a hand-off that has already committed to winning.

use std::sync::Arc;

use sagat_application::chain_progress::ChainHandoffWrite;
use sagat_application::error::AppError;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_scheduler::ContinuationAdvanceError;
use sqlx::PgPool;

type Sched = Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>;

struct Gate {
    transaction: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl Gate {
    /// Takes the chain row `FOR UPDATE`, as a command that has started and not yet committed.
    async fn on_chain(pool: &PgPool, tenant: Uuid, chain: RunId) -> Self {
        let mut transaction = pool.begin().await.expect("gate transaction");
        sqlx::query("SELECT 1 FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(chain.as_uuid())
            .execute(&mut *transaction)
            .await
            .expect("hold the chain row");
        Self { transaction }
    }

    /// Holds the advisory lock a hand-off takes for the namespace's active-run quota, after it has
    /// locked the chain and the head run: a hand-off that reaches it is past its checks and waits
    /// with both locks held.
    async fn inside_the_write(pool: &PgPool, tenant: TenantId, namespace: NamespaceId) -> Self {
        let mut transaction = pool.begin().await.expect("gate transaction");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "quota:{}:{tenant}:{namespace}",
                sagat_domain::quota::QuotaResource::ActiveRuns
            ))
            .execute(&mut *transaction)
            .await
            .expect("hold the quota lock");
        Self { transaction }
    }

    /// Takes the child-chain link row `FOR UPDATE`.
    async fn on_link(pool: &PgPool, tenant: Uuid, chain: RunId) -> Self {
        let mut transaction = pool.begin().await.expect("gate transaction");
        sqlx::query(
            "SELECT 1 FROM child_chain_links WHERE tenant_id=$1 AND child_chain_id=$2 FOR UPDATE",
        )
        .bind(tenant)
        .bind(chain.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("hold the link row");
        Self { transaction }
    }

    async fn release(self) {
        self.transaction.rollback().await.expect("release the gate");
    }
}

/// Waits until exactly `count` backends of this database wait for a lock.
async fn until_blocked(pool: &PgPool, count: i64) {
    crate::support::eventually("backends blocked on a lock", || async {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname=current_database() AND wait_event_type='Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        (waiting == count).then_some(())
    })
    .await;
}

fn boundary() -> NodeId {
    NodeId::parse("rollover").expect("continuation node")
}

async fn handoff(
    scheduler: Arc<Sched>,
    world: Arc<crate::support::engine::World>,
    chain: RunId,
) -> Result<ChainHandoffWrite, ContinuationAdvanceError> {
    scheduler
        .advance_continuation(
            world.admin.tenant_id(),
            world.namespace.id(),
            chain,
            &boundary(),
        )
        .await
}

type Commands = ChainCommandService<sagat_infra_postgres::PgRunStore, SystemClock, JsonEvaluator>;

fn commands(world: &crate::support::engine::World) -> Commands {
    ChainCommandService::new(world.store.clone(), SystemClock, Engine::new(JsonEvaluator))
}

/// A chain whose first activity is done and whose boundary is pending.
async fn pending_boundary(
    name: &str,
    max_segments: u16,
) -> (Arc<crate::support::engine::World>, RunId, Arc<Sched>) {
    let world = world().await;
    let workflow_name = format!("chain-{name}-{}", Uuid::now_v7());
    let (workflow, _) = world
        .publish(continuation(&workflow_name, "chain.fault", max_segments))
        .await;
    let chain = world.start(workflow, &json!({"next": {"next": {}}})).await;
    let session = world.session(&["chain.fault"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete the activity");
    let placement = placement(&world, name).await;
    let scheduler = Arc::new(scheduler(&world, placement));
    (Arc::new(world), chain, scheduler)
}

async fn chain_state(
    world: &crate::support::engine::World,
    chain: RunId,
) -> (Uuid, i32, bool, Option<String>) {
    sqlx::query_as(
        "SELECT current_run_id,segment_count,cancel_requested,final_outcome \
         FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain state")
}

async fn count_events(world: &crate::support::engine::World, run: RunId, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type=$3",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(kind)
    .fetch_one(&world.db.pool)
    .await
    .expect("event count")
}

#[tokio::test]
async fn a_dropped_connection_before_commit_leaves_nothing_and_one_retry_commits_once() {
    let (world, chain, scheduler) = pending_boundary("dropped", 3).await;
    let gate = Gate::on_chain(&world.db.pool, world.admin.tenant_id().as_uuid(), chain).await;
    let attempt = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;

    // The connection of the hand-off dies while it waits for the chain: the process "crashed"
    // before it wrote anything.
    let terminated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
           WHERE datname=current_database() AND wait_event_type='Lock') killed",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("terminate the waiting backend");
    assert_eq!(terminated, 1);
    let crashed = attempt.await.expect("task");
    assert!(crashed.is_err(), "{crashed:?}");
    gate.release().await;
    assert_eq!(
        chain_state(&world, chain).await,
        (chain.as_uuid(), 1, false, None)
    );
    assert_eq!(
        count_events(&world, chain, "continuation.selected").await,
        0
    );
    let strays: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE tenant_id=$1 AND id<>$2 AND started_by=$3 \
           AND created_at>(SELECT created_at FROM runs WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .bind(world.admin.subject().as_str())
    .fetch_one(&world.db.pool)
    .await
    .expect("stray runs");
    assert_eq!(strays, 0);

    // The restarted scheduler finds the same boundary and commits it once; a repeat finds nothing.
    let committed = handoff(scheduler.clone(), world.clone(), chain)
        .await
        .expect("retry");
    assert!(!committed.replayed());
    let repeat = handoff(scheduler, world.clone(), chain).await;
    assert!(
        matches!(repeat, Err(ContinuationAdvanceError::NotContinuation)),
        "{repeat:?}"
    );
    assert_eq!(chain_state(&world, chain).await.1, 2);
    assert_eq!(
        count_events(&world, chain, "continuation.selected").await,
        1
    );
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn duplicate_hand_offs_of_one_boundary_commit_one_successor() {
    let (world, chain, scheduler) = pending_boundary("duplicate", 3).await;
    // The first attempt is held inside its write; the second queues behind it for the chain.
    let gate = Gate::inside_the_write(
        &world.db.pool,
        world.admin.tenant_id(),
        world.namespace.id(),
    )
    .await;
    let first = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;
    let second = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    let (first, second) = (first.await.expect("task"), second.await.expect("task"));
    assert!(first.is_ok(), "the first in the queue commits: {first:?}");
    assert!(
        matches!(second, Err(ContinuationAdvanceError::NotContinuation)),
        "the duplicate finds the boundary already handed off: {second:?}"
    );
    let (head, segments, _, outcome) = chain_state(&world, chain).await;
    assert_ne!(head, chain.as_uuid());
    assert_eq!((segments, outcome), (2, None));
    assert_eq!(
        count_events(&world, chain, "continuation.selected").await,
        1
    );
    let outbox: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE event_type='run.continued'), \
                count(*) FILTER (WHERE event_type='task.ready' AND aggregate_id=$2) \
         FROM event_outbox WHERE tenant_id=$1",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(head)
    .fetch_one(&world.db.pool)
    .await
    .expect("outbox");
    assert_eq!(outbox, (1, 1), "one continuation hint and one task hint");
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn cancel_and_hand_off_commit_in_either_order_without_a_stray_successor() {
    // Hand-off first: the cancel names the head it saw and finds it replaced.
    let (world, chain, scheduler) = pending_boundary("cancel-after", 3).await;
    let gate = Gate::inside_the_write(
        &world.db.pool,
        world.admin.tenant_id(),
        world.namespace.id(),
    )
    .await;
    let moving = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;
    let (cancel_world, cancel_chain) = (world.clone(), chain);
    let cancelling = tokio::spawn(async move {
        commands(&cancel_world)
            .cancel(
                &cancel_world.admin,
                cancel_world.namespace.id(),
                cancel_chain,
                Some(cancel_chain),
                "cancel after the hand-off",
                "cancel-after-handoff-key",
                &ctx(),
            )
            .await
    });
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    assert!(moving.await.expect("task").is_ok());
    assert_eq!(
        cancelling.await.expect("task"),
        Err(AppError::StateConflict("the chain head changed"))
    );
    let (head, segments, latched, outcome) = chain_state(&world, chain).await;
    assert_ne!(head, chain.as_uuid());
    assert_eq!((segments, latched, outcome), (2, false, None));
    assert_eq!(world.run_status(RunId::new(head)).await, RunStatus::Running);

    // Cancel first: the latch wins and the hand-off never writes a successor.
    let (world, chain, scheduler) = pending_boundary("cancel-before", 3).await;
    let gate = Gate::on_chain(&world.db.pool, world.admin.tenant_id().as_uuid(), chain).await;
    let (cancel_world, cancel_chain) = (world.clone(), chain);
    let cancelling = tokio::spawn(async move {
        commands(&cancel_world)
            .cancel(
                &cancel_world.admin,
                cancel_world.namespace.id(),
                cancel_chain,
                Some(cancel_chain),
                "cancel before the hand-off",
                "cancel-before-handoff-key",
                &ctx(),
            )
            .await
    });
    until_blocked(&world.db.pool, 1).await;
    let moving = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    assert_eq!(cancelling.await.expect("task"), Ok(()));
    let refused = moving.await.expect("task");
    assert!(
        refused.is_err(),
        "a latched chain does not continue: {refused:?}"
    );
    assert_eq!(
        chain_state(&world, chain).await,
        (chain.as_uuid(), 1, true, Some("CANCELLED".into()))
    );
    assert_eq!(world.run_status(chain).await, RunStatus::Cancelled);
    assert_eq!(
        count_events(&world, chain, "continuation.selected").await,
        0
    );
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn a_chain_signal_is_never_held_for_the_successor_in_either_order() {
    // At a pending boundary there is no open wait, so a signal has nothing to wake: before the
    // hand-off it is refused as "not waiting", after it the expected head has changed. Either way
    // it is not buffered, and the successor's own wait later opens untouched.
    let signal = |world: Arc<crate::support::engine::World>, chain: RunId, key: &'static str| {
        tokio::spawn(async move {
            commands(&world)
                .signal(
                    &world.admin,
                    world.namespace.id(),
                    chain,
                    Some(chain),
                    "chain.release",
                    br#"{"released":true}"#,
                    key,
                    &ctx(),
                )
                .await
        })
    };
    let waiting_events = |world: Arc<crate::support::engine::World>, chain: RunId| async move {
        let head = RunId::new(chain_state(&world, chain).await.0);
        count_events(&world, head, "wait.satisfied").await
    };

    // Signal first.
    let (world, chain, scheduler) = pending_boundary("signal-first", 3).await;
    let gate = Gate::on_chain(&world.db.pool, world.admin.tenant_id().as_uuid(), chain).await;
    let signalling = signal(world.clone(), chain, "signal-before-handoff");
    until_blocked(&world.db.pool, 1).await;
    let moving = tokio::spawn(handoff(scheduler, world.clone(), chain));
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    assert_eq!(
        signalling.await.expect("task"),
        Err(AppError::StateConflict(
            "the chain head is not waiting for that signal"
        ))
    );
    assert!(moving.await.expect("task").is_ok());
    assert_eq!(waiting_events(world.clone(), chain).await, 0);

    // Hand-off first.
    let (world, chain, scheduler) = pending_boundary("signal-after", 3).await;
    let gate = Gate::inside_the_write(
        &world.db.pool,
        world.admin.tenant_id(),
        world.namespace.id(),
    )
    .await;
    let moving = tokio::spawn(handoff(scheduler, world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;
    let signalling = signal(world.clone(), chain, "signal-after-handoff");
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    assert!(moving.await.expect("task").is_ok());
    assert_eq!(
        signalling.await.expect("task"),
        Err(AppError::StateConflict("the chain head changed"))
    );
    assert_eq!(waiting_events(world.clone(), chain).await, 0);
    let successor = RunId::new(chain_state(&world, chain).await.0);
    assert_eq!(count_events(&world, successor, "wait.satisfied").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM command_dedupe WHERE tenant_id=$1 AND scope LIKE 'chain.signal:%'"
        )
        .bind(world.admin.tenant_id().as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("dedupe rows"),
        0,
        "a refused signal leaves no receipt that could replay it onto another head"
    );
}

#[tokio::test]
async fn a_quota_lowered_while_the_hand_off_waits_is_honoured_by_the_write() {
    let (world, chain, scheduler) = pending_boundary("quota-race", 3).await;
    let gate = Gate::on_chain(&world.db.pool, world.admin.tenant_id().as_uuid(), chain).await;
    let moving = tokio::spawn(handoff(scheduler.clone(), world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;
    // The tenant's limit drops to nothing after the scheduler picked the boundary up.
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by,updated_at) \
         VALUES ($1,NULL,'tenant_active_runs',0,'test',now()) \
         ON CONFLICT (tenant_id,coalesce(namespace_id,'00000000-0000-0000-0000-000000000000'::uuid),resource) \
         DO UPDATE SET max_value=0",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("lower the tenant limit");
    gate.release().await;
    let refused = moving.await.expect("task");
    assert!(
        matches!(
            refused,
            Err(ContinuationAdvanceError::Progress(
                sagat_application::chain_progress::ChainProgressError::AtCapacity
            ))
        ),
        "{refused:?}"
    );
    assert_eq!(
        chain_state(&world, chain).await,
        (chain.as_uuid(), 1, false, None)
    );
    sqlx::query("DELETE FROM quota_limits WHERE tenant_id=$1 AND resource='tenant_active_runs'")
        .bind(world.admin.tenant_id().as_uuid())
        .execute(&world.db.pool)
        .await
        .expect("restore the tenant limit");
    assert!(handoff(scheduler, world.clone(), chain).await.is_ok());
    assert_eq!(chain_state(&world, chain).await.1, 2);
}

// ------------------------------------------------------------------------------- child chains

use sagat_application::execution::child_run_id;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::StepStatus;

use crate::support::engine::{World, activity, child_node, document};

/// A continuation workflow that keeps going while `input.continue` is true and then ends.
fn rolling(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "chain.child", "next": "again"},
            {"id": "again", "type": "condition", "expression": "input.continue",
             "ifTrue": "rollover", "ifFalse": "done"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 5, "timeoutMs": 3_600_000}
    })
}

struct Family {
    world: Arc<World>,
    scheduler: Arc<Sched>,
    parent: RunId,
    chain: RunId,
}

impl Family {
    /// A parent waiting at a `child` node whose child is a three-segment chain, already started.
    async fn start(label: &str) -> Self {
        let world = world().await;
        let kid = format!("kid-{label}-{}", Uuid::now_v7());
        world.publish(rolling(&kid)).await;
        let (parent_workflow, _) = world
            .publish(document(
                &format!("mother-{label}-{}", Uuid::now_v7()),
                "kid",
                vec![child_node("kid", &kid, Some("after")), activity("after")],
            ))
            .await;
        let parent = world
            .start(
                parent_workflow,
                &json!({"continue": true, "next": {"continue": true, "next": {"continue": false}}}),
            )
            .await;
        let chain = child_run_id(parent, &NodeId::parse("kid").expect("kid node"));
        let placement = placement(&world, label).await;
        let scheduler = Arc::new(
            Scheduler::new(
                world.store.clone(),
                Engine::new(JsonEvaluator),
                SystemEntropy,
                SchedulerSettings::default(),
            )
            .with_children(world.cache.clone())
            .with_placement(placement),
        );
        let started = scheduler.tick().await;
        assert_eq!(started.children_started, 1, "{started:?}");
        Self {
            world: Arc::new(world),
            scheduler,
            parent,
            chain,
        }
    }

    async fn finish_segment(&self) {
        let session = self.world.session(&["chain.child"]).await;
        let task = self.world.one_task_of(&session, "chain.child").await;
        self.world
            .complete(&session, &task, &json!({"processed": true}))
            .await
            .expect("segment activity");
    }

    async fn head(&self) -> (Uuid, i32, bool, Option<String>) {
        chain_state(&self.world, self.chain).await
    }

    async fn link(&self) -> (bool, Option<String>, bool) {
        sqlx::query_as(
            "SELECT cancel_requested,final_outcome,settled FROM child_chain_links \
             WHERE tenant_id=$1 AND child_chain_id=$2",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("link")
    }
}

#[tokio::test]
async fn parent_cancellation_and_a_child_hand_off_commit_in_either_order_and_the_latch_never_gets_lost()
 {
    // The parent's cancellation first: the latch is committed before the hand-off looks at it.
    let family = Family::start("latch-first").await;
    family.finish_segment().await;
    let tenant = family.world.admin.tenant_id().as_uuid();
    let gate = Gate::on_link(&family.world.db.pool, tenant, family.chain).await;
    let cancelling = tokio::spawn({
        let world = family.world.clone();
        let parent = family.parent;
        async move { world.operate(parent, RunOperation::Cancel).await }
    });
    until_blocked(&family.world.db.pool, 1).await;
    let moving = tokio::spawn(handoff(
        family.scheduler.clone(),
        family.world.clone(),
        family.chain,
    ));
    until_blocked(&family.world.db.pool, 2).await;
    gate.release().await;
    let cancelled = cancelling.await.expect("task");
    assert!(cancelled.is_ok(), "{cancelled:?}");
    let refused = moving.await.expect("task");
    assert!(
        matches!(
            refused,
            Err(ContinuationAdvanceError::Progress(
                sagat_application::chain_progress::ChainProgressError::Chain(
                    sagat_domain::execution::ChainError::ChildCancellationRequested
                )
            ))
        ),
        "{refused:?}"
    );
    assert_eq!(family.head().await.1, 1, "no successor after the latch");
    assert!(family.link().await.0);
    let report = family.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1, "{report:?}");
    assert_eq!(
        family.world.run_status(family.chain).await,
        RunStatus::Cancelled
    );
    assert_eq!(family.head().await.3.as_deref(), Some("CANCELLED"));

    // The hand-off first: the successor exists when the latch lands, and the successor is stopped.
    let family = Family::start("handoff-first").await;
    family.finish_segment().await;
    let tenant = family.world.admin.tenant_id().as_uuid();
    let gate = Gate::on_link(&family.world.db.pool, tenant, family.chain).await;
    let moving = tokio::spawn(handoff(
        family.scheduler.clone(),
        family.world.clone(),
        family.chain,
    ));
    until_blocked(&family.world.db.pool, 1).await;
    let cancelling = tokio::spawn({
        let world = family.world.clone();
        let parent = family.parent;
        async move { world.operate(parent, RunOperation::Cancel).await }
    });
    until_blocked(&family.world.db.pool, 2).await;
    gate.release().await;
    assert!(moving.await.expect("task").is_ok());
    let cancelled = cancelling.await.expect("task");
    assert!(cancelled.is_ok(), "{cancelled:?}");
    let (head, segments, _, _) = family.head().await;
    assert_eq!(segments, 2, "the hand-off committed first");
    assert!(family.link().await.0, "and the latch landed after it");
    let report = family.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1, "{report:?}");
    assert_eq!(
        family.world.run_status(RunId::new(head)).await,
        RunStatus::Cancelled,
        "the successor, not the finished predecessor, is stopped"
    );
    assert_eq!(family.head().await.3.as_deref(), Some("CANCELLED"));
    let quiet = family.scheduler.tick().await;
    assert_eq!(
        (quiet.failed, quiet.continuations_advanced),
        (0, 0),
        "{quiet:?}"
    );
    assert_eq!(
        family.head().await.1,
        2,
        "nothing continued after the latch"
    );
}

#[tokio::test]
async fn the_final_outcome_of_a_child_chain_reaches_the_parent_once_despite_a_failed_write_and_a_duplicate()
 {
    let family = Family::start("final-once").await;
    for _ in 0..2 {
        family.finish_segment().await;
        family.scheduler.tick().await;
    }
    // The last segment ends the chain; the parent has not been told yet.
    family.finish_segment().await;
    assert_eq!(family.head().await.3.as_deref(), Some("SUCCEEDED"));
    assert_eq!(
        family.link().await,
        (false, Some("SUCCEEDED".into()), false)
    );

    // The write that wakes the parent fails: the parent's step, journal and the link stay as they
    // were, and the failure is counted rather than hidden.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION reject_parent_wake() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected parent wake failure'; END $$; \
         CREATE TRIGGER reject_parent_wake BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.aggregate_id='{}' AND NEW.event_type='task.ready') \
         EXECUTE FUNCTION reject_parent_wake()",
        family.parent.as_uuid()
    )))
    .execute(&family.world.db.pool)
    .await
    .expect("install the parent wake fault");
    let failed = family.scheduler.tick().await;
    assert_eq!(failed.children_finished, 0, "{failed:?}");
    assert_eq!(failed.failed, 1, "{failed:?}");
    assert_eq!(
        family.world.step_status(family.parent, "kid").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(
        count_events(&family.world, family.parent, "child.finished").await,
        0
    );
    assert_eq!(
        family.link().await,
        (false, Some("SUCCEEDED".into()), false)
    );

    // The fault clears; two deliveries of the same ending race for the parent's lock.
    sqlx::raw_sql(
        "DROP TRIGGER reject_parent_wake ON event_outbox; DROP FUNCTION reject_parent_wake()",
    )
    .execute(&family.world.db.pool)
    .await
    .expect("remove the parent wake fault");
    let (first, second) = tokio::join!(family.scheduler.tick(), family.scheduler.tick());
    assert_eq!(
        first.children_finished + second.children_finished,
        1,
        "{first:?} {second:?}"
    );
    assert_eq!(first.failed + second.failed, 0, "{first:?} {second:?}");
    assert_eq!(
        count_events(&family.world, family.parent, "child.finished").await,
        1
    );
    assert_eq!(
        family.world.step_status(family.parent, "kid").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(family.link().await, (false, Some("SUCCEEDED".into()), true));
    let again = family.scheduler.tick().await;
    assert_eq!(again.children_finished, 0, "{again:?}");
    family.world.assert_consistent(family.parent).await;
}

// --------------------------------------------------------------------- Update against a hand-off

use crate::support::worker::{Platform, platform};
use reqwest::Method;

fn stateful_chain(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "chain.state", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next", "state": "state"}
        ],
        "chainPolicy": {"maxSegments": 4, "timeoutMs": 3_600_000},
        "state": {
            "schema": {
                "type": "object", "additionalProperties": false,
                "properties": {"balance": {"type": "integer", "minimum": 0}},
                "required": ["balance"]
            },
            "initial": "input.account",
            "queries": {
                "balance.read": {
                    "argsSchema": {"type": "object", "additionalProperties": false},
                    "resultSchema": {"type": "integer"},
                    "expression": "state.balance",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance.replace": {
                    "argsSchema": {
                        "type": "object", "additionalProperties": false,
                        "properties": {
                            "allowed": {"type": "boolean"},
                            "nextState": {
                                "type": "object", "additionalProperties": false,
                                "properties": {"balance": {"type": "integer", "minimum": 0}},
                                "required": ["balance"]
                            }
                        },
                        "required": ["allowed", "nextState"]
                    },
                    "validate": "args.allowed",
                    "state": "args.nextState",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer", "minimum": 0},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

struct StatefulChain {
    platform: Platform,
    namespace: String,
    chain: RunId,
    scheduler: Arc<Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>>,
}

impl StatefulChain {
    /// A chain whose first activity is done: its boundary is pending with state `{balance: 10}`.
    async fn at_boundary(label: &str) -> Self {
        let platform = platform().await;
        let name = format!("state-{label}-{}", Uuid::now_v7());
        let workflow = platform.publish(&name, &stateful_chain(&name)).await;
        let run = platform
            .start(
                &workflow,
                &json!({"account": {"balance": 10}, "next": {"account": {"balance": 0}}}),
            )
            .await;
        let mut worker = platform.worker("worker-state").await;
        worker.register(&["chain.state"]).await;
        let lease = worker.one_task().await;
        worker
            .complete(&lease, &json!({"processed": true}))
            .await
            .expect("complete the segment activity");
        let placement = Placement::new(format!("chain-state-{label}-{}", Uuid::now_v7()));
        Keeper::new(
            platform.api.executions.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: Duration::from_secs(60),
            },
        )
        .round()
        .await
        .expect("acquire scheduler shards");
        let scheduler = Arc::new(
            Scheduler::new(
                platform.api.executions.clone(),
                Engine::new(JsonEvaluator),
                SystemEntropy,
                SchedulerSettings::default(),
            )
            .with_placement(placement),
        );
        let namespace = platform.namespace_id().await;
        Self {
            platform,
            namespace,
            chain: run.parse().expect("run id"),
            scheduler,
        }
    }

    fn tenant(&self) -> Uuid {
        self.platform.tenant.as_uuid()
    }

    fn update_path(&self) -> String {
        format!(
            "/namespaces/{}/chains/{}/updates/balance.replace",
            self.namespace, self.chain
        )
    }

    fn update_body(key: &str, balance: i64, expected_run: Option<RunId>) -> Value {
        let mut body = json!({
            "updateKey": key,
            "args": {"allowed": true, "nextState": {"balance": balance}}
        });
        if let Some(run) = expected_run {
            body["expectedRunId"] = json!(run.to_string());
        }
        body
    }

    fn handoff(
        &self,
    ) -> tokio::task::JoinHandle<Result<ChainHandoffWrite, ContinuationAdvanceError>> {
        let scheduler = self.scheduler.clone();
        let (tenant, namespace) = (
            sagat_domain::identity::TenantId::new(self.tenant()),
            NamespaceId::new(self.namespace.parse().expect("namespace id")),
        );
        let chain = self.chain;
        tokio::spawn(async move {
            scheduler
                .advance_continuation(tenant, namespace, chain, &boundary())
                .await
        })
    }

    fn update(&self, body: Value) -> tokio::task::JoinHandle<(u16, Value)> {
        let request = self
            .platform
            .api
            .as_user(Method::POST, &self.update_path(), "alice", "acme");
        tokio::spawn(async move {
            let response = request.json(&body).send().await.expect("update request");
            let status = response.status().as_u16();
            (status, crate::support::api::json_of(response).await)
        })
    }

    async fn head(&self) -> (Uuid, i32) {
        sqlx::query_as(
            "SELECT current_run_id,segment_count FROM execution_chains \
             WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(self.tenant())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.platform.api.db.pool)
        .await
        .expect("chain head")
    }

    async fn state_of(&self, run: Uuid) -> (i64, serde_json::Value) {
        sqlx::query_as(
            "SELECT operation.state_revision, convert_from(payload.content,'UTF8')::jsonb \
             FROM workflow_operation_states operation JOIN payloads payload \
               ON payload.tenant_id=operation.tenant_id AND payload.digest=operation.state_digest \
             WHERE operation.tenant_id=$1 AND operation.run_id=$2",
        )
        .bind(self.tenant())
        .bind(run)
        .fetch_one(&self.platform.api.db.pool)
        .await
        .expect("declared state")
    }
}

#[tokio::test]
async fn a_chain_update_and_a_hand_off_commit_in_either_order_without_losing_or_repeating_the_update()
 {
    // The update first: the successor starts from the state the update left.
    let chain = StatefulChain::at_boundary("update-first").await;
    let pool = &chain.platform.api.db.pool;
    let gate = Gate::on_chain(pool, chain.tenant(), chain.chain).await;
    let update = chain.update(StatefulChain::update_body(
        "update-before-handoff",
        27,
        None,
    ));
    until_blocked(pool, 1).await;
    let moving = chain.handoff();
    until_blocked(pool, 2).await;
    gate.release().await;
    let (status, receipt) = update.await.expect("task");
    assert_eq!(status, 200, "{receipt}");
    assert_eq!(
        receipt["runId"],
        chain.chain.to_string(),
        "applied on the head it found"
    );
    assert_eq!(receipt["replayed"], false);
    assert!(moving.await.expect("task").is_ok());
    let (head, segments) = chain.head().await;
    assert_eq!(segments, 2);
    assert_eq!(
        chain.state_of(head).await,
        (1, json!({"balance": 27})),
        "the successor carries the updated state and its revision"
    );

    // The same key after the roll-over is the original receipt, not a second application.
    let (status, replay) = chain
        .update(StatefulChain::update_body(
            "update-before-handoff",
            27,
            None,
        ))
        .await
        .expect("task");
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["runId"], chain.chain.to_string());
    assert_eq!(replay["replayed"], true);
    assert_eq!(chain.state_of(head).await.0, 1, "applied once");

    // The hand-off first: the update meets the successor, or is refused by its expected head.
    let chain = StatefulChain::at_boundary("handoff-first").await;
    let pool = &chain.platform.api.db.pool;
    let gate = Gate::inside_the_write(
        pool,
        sagat_domain::identity::TenantId::new(chain.tenant()),
        NamespaceId::new(chain.namespace.parse().expect("namespace id")),
    )
    .await;
    let moving = chain.handoff();
    until_blocked(pool, 1).await;
    let stale = chain.update(StatefulChain::update_body(
        "stale-head-update",
        31,
        Some(chain.chain),
    ));
    until_blocked(pool, 2).await;
    let open = chain.update(StatefulChain::update_body("open-head-update", 32, None));
    until_blocked(pool, 3).await;
    gate.release().await;
    assert!(moving.await.expect("task").is_ok());
    let (head, segments) = chain.head().await;
    assert_eq!(segments, 2);
    let (status, refused) = stale.await.expect("task");
    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["details"]["reason"], "CHAIN_HEAD_CHANGED");
    let (status, applied) = open.await.expect("task");
    assert_eq!(status, 200, "{applied}");
    assert_eq!(
        applied["runId"],
        head.to_string(),
        "applied once, on the successor"
    );
    assert_eq!(chain.state_of(head).await, (1, json!({"balance": 32})));
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operation_update_receipts WHERE tenant_id=$1")
            .bind(chain.tenant())
            .fetch_one(pool)
            .await
            .expect("receipts");
    assert_eq!(receipts, 1, "the refused update left no receipt behind");
    assert_eq!(
        chain.state_of(chain.chain.as_uuid()).await,
        (0, json!({"balance": 10})),
        "the finished predecessor kept the state it ended with"
    );
}

#[tokio::test]
async fn a_lost_wake_of_the_parent_after_a_child_chain_ends_is_found_again_and_runs_the_next_step_once()
 {
    let family = Family::start("lost-wake").await;
    for _ in 0..3 {
        family.finish_segment().await;
        family.scheduler.tick().await;
    }
    assert_eq!(
        family.world.step_status(family.parent, "kid").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(
        family.world.step_status(family.parent, "after").await,
        Some(StepStatus::Ready)
    );

    // The dispatcher marked the hint of the parent's commit delivered, but the broker lost it: no
    // worker is told. (An undelivered message cannot be deleted; "delivered and lost" is the fault.)
    let lost = sqlx::query(
        "UPDATE event_outbox SET delivered_at=now() \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready' AND delivered_at IS NULL",
    )
    .bind(family.world.admin.tenant_id().as_uuid())
    .bind(family.parent.as_uuid())
    .execute(&family.world.db.pool)
    .await
    .expect("lose the hint")
    .rows_affected();
    assert_eq!(lost, 1);

    // The durable READY step is found again by the reconciler and handed out exactly once.
    family.world.age_hint(family.parent, "after").await;
    let report = family.scheduler.tick().await;
    assert_eq!(report.hints_renewed, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    let session = family.world.session(&["act.after"]).await;
    let task = family.world.one_task_of(&session, "act.after").await;
    assert_eq!(task.run_id, family.parent);
    family
        .world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("the parent's next step");
    assert_eq!(
        family.world.run_status(family.parent).await,
        RunStatus::Succeeded
    );
    assert_eq!(
        count_events(&family.world, family.parent, "child.finished").await,
        1
    );
    family.world.assert_consistent(family.parent).await;
}

// -------------------------------------------------- a continuation that starts with an event wait

use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::execution::CorrelationKey;
use sagat_domain::workflow::EventType;

fn root_wait(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "release",
        "nodes": [
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": 600_000, "next": "rollover"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 4, "timeoutMs": 3_600_000}
    })
}

/// A chain whose entry wait was woken by a signal: the boundary is pending and no wait is open.
async fn woken_wait(name: &str) -> (Arc<crate::support::engine::World>, RunId, Arc<Sched>) {
    let world = world().await;
    let workflow_name = format!("chain-{name}-{}", Uuid::now_v7());
    let (workflow, _) = world.publish(root_wait(&workflow_name)).await;
    let chain = world
        .start(
            workflow,
            &json!({"key": "order-1", "next": {"key": "order-2", "next": {"key": "order-3"}}}),
        )
        .await;
    commands(&world)
        .signal(
            &world.admin,
            world.namespace.id(),
            chain,
            Some(chain),
            "chain.release",
            br#"{"released":true}"#,
            &format!("woken-{name}"),
            &ctx(),
        )
        .await
        .expect("wake the entry wait");
    let placement = placement(&world, name).await;
    let scheduler = Arc::new(scheduler(&world, placement));
    (Arc::new(world), chain, scheduler)
}

fn release_signal(
    world: Arc<crate::support::engine::World>,
    chain: RunId,
    expected: Option<RunId>,
    key: &'static str,
) -> tokio::task::JoinHandle<Result<Vec<NodeId>, AppError>> {
    tokio::spawn(async move {
        commands(&world)
            .signal(
                &world.admin,
                world.namespace.id(),
                chain,
                expected,
                "chain.release",
                br#"{"released":true}"#,
                key,
                &ctx(),
            )
            .await
    })
}

async fn subscriptions(world: &crate::support::engine::World, chain: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND status='WAITING' \
           AND waiting_kind='EVENT' AND run_id<>$2 AND run_id IN \
           (SELECT run_id FROM chain_segments WHERE tenant_id=$1 AND chain_id=$2)",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("subscriptions of the successors")
}

#[tokio::test]
async fn a_signal_and_the_hand_off_of_a_wait_chain_commit_in_either_order_and_the_next_signal_is_not_lost()
 {
    // Signal first: the boundary has no open wait, so the signal is refused and not buffered; the
    // hand-off then commits, and a signal sent to the new head finds the wait the hand-off opened.
    let (world, chain, scheduler) = woken_wait("wait-signal-first").await;
    let gate = Gate::on_chain(&world.db.pool, world.admin.tenant_id().as_uuid(), chain).await;
    let early = release_signal(world.clone(), chain, Some(chain), "wait-race-early");
    until_blocked(&world.db.pool, 1).await;
    let moving = tokio::spawn(handoff(scheduler, world.clone(), chain));
    until_blocked(&world.db.pool, 2).await;
    gate.release().await;
    assert_eq!(
        early.await.expect("task"),
        Err(AppError::StateConflict(
            "the chain head is not waiting for that signal"
        ))
    );
    assert!(moving.await.expect("task").is_ok());
    let (head, segments, _, _) = chain_state(&world, chain).await;
    let successor = RunId::new(head);
    assert_eq!(segments, 2);
    assert_eq!(count_events(&world, successor, "wait.satisfied").await, 0);
    assert_eq!(subscriptions(&world, chain).await, 1);
    let delivered = release_signal(world.clone(), chain, Some(successor), "wait-race-late")
        .await
        .expect("task")
        .expect("the successor's wait is open at once");
    assert_eq!(delivered.len(), 1);
    assert_eq!(count_events(&world, successor, "wait.satisfied").await, 1);
    world.assert_consistent(successor).await;

    // Hand-off first: a signal fenced to the old head finds it replaced; one that names no head,
    // queued behind the hand-off, reaches the wait the hand-off opened. Neither is lost or doubled.
    let (world, chain, scheduler) = woken_wait("wait-handoff-first").await;
    let gate = Gate::inside_the_write(
        &world.db.pool,
        world.admin.tenant_id(),
        world.namespace.id(),
    )
    .await;
    let moving = tokio::spawn(handoff(scheduler, world.clone(), chain));
    until_blocked(&world.db.pool, 1).await;
    let stale = release_signal(world.clone(), chain, Some(chain), "wait-race-stale");
    until_blocked(&world.db.pool, 2).await;
    let unfenced = release_signal(world.clone(), chain, None, "wait-race-unfenced");
    until_blocked(&world.db.pool, 3).await;
    gate.release().await;
    assert!(moving.await.expect("task").is_ok());
    assert_eq!(
        stale.await.expect("task"),
        Err(AppError::StateConflict("the chain head changed"))
    );
    let delivered = unfenced
        .await
        .expect("task")
        .expect("the signal queued behind the hand-off reaches the successor's wait");
    assert_eq!(delivered.len(), 1);
    let (head, segments, _, _) = chain_state(&world, chain).await;
    let successor = RunId::new(head);
    assert_eq!(segments, 2);
    assert_eq!(count_events(&world, successor, "wait.satisfied").await, 1);
    assert_eq!(
        count_events(&world, chain, "wait.satisfied").await,
        1,
        "the predecessor's wait was satisfied once, by the signal that woke it"
    );
    world.assert_consistent(successor).await;
}

#[tokio::test]
async fn a_failed_hand_off_of_a_wait_chain_leaves_no_subscription_and_the_retry_writes_one() {
    let (world, chain, scheduler) = woken_wait("wait-rollback").await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_wait_handoff_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected handoff outbox failure'; END $$; \
         CREATE TRIGGER reject_wait_handoff_outbox BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type='run.continued') \
         EXECUTE FUNCTION reject_wait_handoff_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("install the hand-off fault");
    assert!(
        handoff(scheduler.clone(), world.clone(), chain)
            .await
            .is_err()
    );
    // The step row, the run and the journal of the successor were written before the fault, and
    // all of them are gone: no subscription survives a hand-off that did not commit.
    assert_eq!(subscriptions(&world, chain).await, 0);
    assert_eq!(
        chain_state(&world, chain).await,
        (chain.as_uuid(), 1, false, None)
    );
    let strays: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND waiting_kind='EVENT' AND run_id<>$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("stray waits");
    assert_eq!(strays, 0);

    sqlx::raw_sql(
        "DROP TRIGGER reject_wait_handoff_outbox ON event_outbox; \
         DROP FUNCTION reject_wait_handoff_outbox()",
    )
    .execute(&world.db.pool)
    .await
    .expect("remove the hand-off fault");
    let committed = handoff(scheduler.clone(), world.clone(), chain)
        .await
        .expect("retry");
    assert!(!committed.replayed());
    assert_eq!(subscriptions(&world, chain).await, 1);
    assert_eq!(chain_state(&world, chain).await.1, 2);
    let repeat = handoff(scheduler, world.clone(), chain).await;
    assert!(
        matches!(repeat, Err(ContinuationAdvanceError::NotContinuation)),
        "{repeat:?}"
    );
    assert_eq!(subscriptions(&world, chain).await, 1, "still one");
    world.assert_consistent(chain).await;
}

#[tokio::test]
async fn a_restart_between_the_hand_off_and_the_event_still_wakes_the_successor_once() {
    let (world, chain, scheduler) = woken_wait("wait-restart").await;
    assert!(handoff(scheduler, world.clone(), chain).await.is_ok());
    let successor = RunId::new(chain_state(&world, chain).await.0);
    let hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 \
           AND event_type='task.ready'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(successor.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("hints");
    assert_eq!(
        hints, 0,
        "there is no hint to lose: the wait is the row itself"
    );

    // Everything in memory is gone: a new process, new pools, new scheduler shards and a new
    // dispatcher find the wait in the database and nowhere else.
    let restarted = world.replica().await;
    sqlx::query(
        "UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second' \
         WHERE expires_at>clock_timestamp()",
    )
    .execute(&world.db.pool)
    .await
    .expect("expire the crashed scheduler leases");
    // Its membership lapses with the leases. A dead process that still counts as a live replica
    // leaves the new one entitled to about half of the shards, and the chain may be in the other half.
    sqlx::query("UPDATE scheduler_members SET expires_at=now()-interval '1 second'")
        .execute(&world.db.pool)
        .await
        .expect("expire the crashed scheduler membership");
    let restarted_placement =
        Placement::new(format!("chain-fault-wait-restart-{}", Uuid::now_v7()));
    Keeper::new(
        restarted.store.clone(),
        restarted_placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("restart acquires the scheduler shards");
    let dispatcher = sagat_scheduler::EventDispatcher::new(
        restarted.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        sagat_application::execution::DefinitionCache::new(
            Arc::new(sagat_infra_ir::JsonIrCodec),
            64,
        ),
        sagat_scheduler::EventDispatchSettings::default(),
    );
    world
        .events
        .publish(
            &world.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse("restart-event").expect("id")),
                event_type: EventType::parse("chain.release").expect("type"),
                subject: None,
                correlation_key: Some(CorrelationKey::parse("order-2").expect("key")),
                occurred_at: None,
                data: br#"{"released":true}"#.to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("the event arrives after the restart");
    let round = dispatcher.tick().await;
    assert_eq!(round.steps_woken, 1, "{round:?}");
    let again = dispatcher.tick().await;
    assert_eq!(again.steps_woken, 0, "{again:?}");
    assert_eq!(count_events(&world, successor, "wait.satisfied").await, 1);
    let next = Scheduler::new(
        restarted.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(restarted_placement)
    .tick()
    .await;
    assert_eq!(next.continuations_advanced, 1, "{next:?}");
    assert_eq!(next.failed, 0, "{next:?}");
    assert_eq!(chain_state(&world, chain).await.1, 3);
    world.assert_consistent(successor).await;
}
