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
