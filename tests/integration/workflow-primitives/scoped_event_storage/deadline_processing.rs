//! Qualified deadline candidates are expectations; current native ingress owns the outcome.
use super::*;
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_scheduler::CollectionAdvance;
use std::{collections::BTreeSet, time::Duration};

async fn due_members(name: &str) -> (Fixture, PgRunStore) {
    let (fixture, store) = pending_wait_with_timeout(name, true, 1).await;
    for _ in 0..2 {
        assert_eq!(
            scheduler::advance(&fixture, &store, false).await,
            CollectionAdvance::BodyProgress
        );
    }
    crate::support::bounded::within("both recorded scoped deadlines are actually due", Duration::from_secs(30), async {
        loop {
            let due:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND waiting_kind='EVENT' AND due_at<=clock_timestamp() AND cardinality(instance_path_nodes)>0").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("current database deadline barrier");
            if due == 2 { break; }
            tokio::task::yield_now().await;
        }
    }).await;
    (fixture, store)
}

#[tokio::test]
async fn deadline_discovery_preserves_both_same_name_scoped_instances_and_tenant_exclusion() {
    let (fixture, store) = due_members("event-deadline-qualified-query").await;
    let due = store
        .due_wait_deadlines(&Window::everything(32))
        .await
        .expect("native deadline discovery");
    assert_eq!(
        due.len(),
        2,
        "deadline discovery must return each qualified wait"
    );
    assert!(due.iter().all(
        |candidate| candidate.tenant_id == fixture.world.admin.tenant_id()
            && candidate.run_id == fixture.run
    ));
    assert_eq!(
        due.into_iter()
            .map(|candidate| candidate.instance())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([wait(0), wait(1)])
    );
    let excluded = Window {
        exclude: vec![fixture.world.admin.tenant_id()],
        ..Window::everything(32)
    };
    assert!(
        store
            .due_wait_deadlines(&excluded)
            .await
            .expect("tenant exclusion")
            .is_empty()
    );
    assert_eq!(
        store
            .due_wait_deadlines(&Window::everything(1))
            .await
            .expect("bounded candidate page")
            .len(),
        1
    );
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("discovery is read-only");
}

#[tokio::test]
async fn a_paused_failed_wait_without_on_error_waits_for_resume_before_collection_failure() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store) =
            pending_wait_with_timeout(&format!("event-paused-unhandled-deadline-{full}"), false, 1)
                .await;
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress
        );
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-unhandled-deadline",
        )
        .await;
        crate::support::bounded::within("recorded unhandled deadline is actually due",Duration::from_secs(30),async {
            loop {
                let due:bool=sqlx::query_scalar("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("database deadline");
                if due {break;}
                tokio::task::yield_now().await;
            }
        }).await;
        timeouts::expire(&fixture, &store, full)
            .await
            .expect("actual paused unhandled timeout");
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::Waiting,
            "pause must not attempt premature failure/drain even without a local handler"
        );
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Resume,
            "resume-before-unhandled-deadline-route",
        )
        .await;
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress,
            "release the deferred outcome first"
        );
        let snapshot = store
            .load_path(
                fixture.world.admin.tenant_id(),
                fixture.run,
                wait(0).path().clone(),
            )
            .await
            .expect("resumed exact source")
            .expect("path");
        let step = snapshot
            .state
            .local_instance(wait(0).node())
            .expect("failed wait");
        assert!(step.routed());
        assert!(
            step.unhandled_instance_failure(&snapshot.definition, &wait(0))
                .is_some(),
            "routing without onError never turns failure into success"
        );
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::DrainStarted
        );
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("deferred outcome then ordinary failure replay");
    }
}

#[tokio::test]
async fn one_scheduler_tick_processes_both_same_name_scoped_deadlines_and_cold_repeat_is_no_work() {
    use sagat_application::{entropy::SystemEntropy, execution::Engine};
    use sagat_scheduler::{Scheduler, SchedulerSettings};
    let (fixture, store) = due_members("event-deadline-actual-tick").await;
    let scheduler = Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            concurrency: 1,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(fixture.placement.clone());
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        report.waits_timed_out, 2,
        "both qualified candidates must survive selection and reach native path processing: {report:?}"
    );
    let closed:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND cardinality(instance_path_nodes)>0 AND status='SUCCEEDED' AND timed_out AND output_digest IS NULL AND waiting_kind IS NULL AND due_at IS NULL AND event_type IS NULL AND correlation_key IS NULL AND event_since IS NULL AND event_scanned_at IS NULL AND lease_token_hash IS NULL").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("both current qualified outcomes closed without worker or event value");
    assert_eq!(closed, 2);
    let cold = PgRunStore::new(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("actual timeout tick cold replay");
    let scheduler = Scheduler::new(
        cold.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            concurrency: 1,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(fixture.placement.clone());
    let repeated = scheduler.tick().await;
    assert_eq!(repeated.failed, 0, "{repeated:?}");
    assert_eq!(
        repeated.waits_timed_out, 0,
        "cold duplicate deadlines are no work"
    );
    let outcomes:i64=sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.timed_out'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one recorded outcome per exact wait");
    assert_eq!(outcomes, 2);
    cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold repeat replay");
}

#[tokio::test]
async fn actual_scoped_deadline_ticks_refuse_missing_or_stale_authority_without_mutation() {
    use sagat_application::{entropy::SystemEntropy, execution::Engine};
    use sagat_scheduler::{Scheduler, SchedulerSettings};
    for stale in [false, true] {
        let (fixture, store) = due_members(&format!("event-deadline-current-fence-{stale}")).await;
        if stale {
            sqlx::query(
                "UPDATE scheduler_shards SET generation=generation+1 WHERE shard=run_shard($1)",
            )
            .bind(fixture.run.as_uuid())
            .execute(&fixture.world.db.pool)
            .await
            .expect("take over the actual shard without refreshing the old cached view");
        }
        let before = guards::snapshot(&fixture).await;
        let scheduler = Scheduler::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings {
                concurrency: 1,
                ..SchedulerSettings::default()
            },
        );
        let scheduler = if stale {
            scheduler.with_placement(fixture.placement.clone())
        } else {
            scheduler
        };
        let report = scheduler.tick().await;
        assert_eq!(
            report.failed, 0,
            "an authority refusal must be fenced, not a clock storage failure: {report:?}"
        );
        assert_eq!(report.waits_timed_out, 0);
        assert!(report.fenced > 0, "{report:?}");
        assert_eq!(guards::snapshot(&fixture).await, before);
    }
}

#[tokio::test]
async fn the_qualified_consumer_preserves_legacy_root_timeout_in_singleton_mode() {
    use crate::support::engine::{document, end_node, wait_node, world};
    let world = world().await;
    let (workflow, _) = world
        .publish(document(
            "event-deadline-root-legacy",
            "b",
            vec![
                wait_node("b", "root.confirmed", "input.orderId", 1, Some("done")),
                end_node("done"),
            ],
        ))
        .await;
    let run = world.start(workflow, &json!({"orderId":"root-1"})).await;
    crate::support::bounded::within("actual legacy root deadline is due",Duration::from_secs(30),async {
        loop {
            let due:bool=sqlx::query_scalar("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND cardinality(instance_path_nodes)=0").bind(world.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&world.db.pool).await.expect("legacy root database deadline");
            if due {break;}
            tokio::task::yield_now().await;
        }
    }).await;
    let report = world.scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        report.waits_timed_out, 1,
        "root singleton behavior must use its existing command"
    );
    world.assert_consistent(run).await;
    let again = world.scheduler.tick().await;
    assert_eq!(again.waits_timed_out, 0);
    assert_eq!(again.failed, 0, "{again:?}");
}

#[tokio::test]
async fn actual_paused_deadline_ticks_defer_routes_until_resume_for_every_timeout_policy() {
    use sagat_application::{entropy::SystemEntropy, execution::Engine, runs::RunOperation};
    use sagat_scheduler::{Scheduler, SchedulerSettings};
    for (handled, recover) in [(true, false), (false, true), (false, false)] {
        let name = format!("event-deadline-paused-ticks-{handled}-{recover}");
        let mut document = wait_document(&name, handled, 1);
        // This policy scenario ends at the collection; the shared worker fixture otherwise
        // declares an additional real root activity, which would correctly remain READY.
        document["nodes"][0]["next"] = json!("done");
        document["nodes"]
            .as_array_mut()
            .expect("root nodes")
            .retain(|node| node["id"] != "a");
        if recover {
            document["bodies"]["line"]["nodes"][1]["onError"] = json!("returned");
        }
        let (fixture, store) = pending_wait_document(&name, document).await;
        for _ in 0..2 {
            assert_eq!(
                scheduler::advance(&fixture, &store, false).await,
                CollectionAdvance::BodyProgress
            );
        }
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-actual-deadline-tick",
        )
        .await;
        crate::support::bounded::within("actual paused deadlines are due",Duration::from_secs(30),async {
            loop {
                let due:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND waiting_kind='EVENT' AND due_at<=clock_timestamp()").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual paused deadline barrier");
                if due==2 {break;}
                tokio::task::yield_now().await;
            }
        }).await;
        let scheduler = Scheduler::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings {
                concurrency: 1,
                ..SchedulerSettings::default()
            },
        )
        .with_placement(fixture.placement.clone());
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.waits_timed_out, 2, "{report:?}");
        let unrouted:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND NOT routed AND status IN ('SUCCEEDED','FAILED') AND output_digest IS NULL AND waiting_kind IS NULL").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("both outcomes recorded but routes deferred");
        assert_eq!(unrouted, 2);
        let again = scheduler.tick().await;
        assert_eq!(
            again.failed, 0,
            "a paused failed wait cannot produce a premature collection error: {again:?}"
        );
        assert_eq!(again.waits_timed_out, 0);
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Resume,
            "resume-after-actual-deadline-tick",
        )
        .await;
        let cold = PgRunStore::new(
            fixture.world.db.runtime_pool().await,
            DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        )
        .with_scoped_worker_protocol();
        let scheduler = Scheduler::new(
            cold.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings {
                concurrency: 1,
                ..SchedulerSettings::default()
            },
        )
        .with_placement(fixture.placement.clone());
        let expected = if handled || recover {
            "SUCCEEDED"
        } else {
            "FAILED"
        };
        let mut status = String::new();
        let mut reports = Vec::new();
        for _ in 0..16 {
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            reports.push(report);
            status = sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("actual resumed run status");
            if status == expected {
                break;
            }
        }
        let steps:serde_json::Value=sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_object('node',node,'path',instance_path_indices,'status',status,'routed',routed,'waiting',waiting_kind,'timedOut',timed_out) ORDER BY instance_path_indices,node) FROM steps WHERE tenant_id=$1 AND run_id=$2").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("metadata-only progress diagnosis");
        assert_eq!(
            status, expected,
            "handled={handled}, recover={recover}, reports={reports:?}, steps={steps}"
        );
        cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("actual paused outcomes and cold resumed policy replay");
    }
}

#[tokio::test]
async fn an_event_committed_ahead_of_the_actual_queued_deadline_entry_fences_its_old_revision_and_binding()
 {
    use sagat_application::{
        dispatch::{EventDispatchStore, WaitingStep},
        entropy::SystemEntropy,
        events::PublishRequest,
        execution::Engine,
    };
    use sagat_domain::{events::EventId, workflow::EventType};
    use sagat_scheduler::{Scheduler, SchedulerSettings};
    let (fixture, store) = due_members("event-before-actual-deadline-entry").await;
    fixture
        .world
        .events
        .publish(
            &fixture.world.admin,
            PublishRequest {
                namespace: "payments".into(),
                event_id: Some(EventId::parse("deadline-race-event").expect("id")),
                event_type: EventType::parse("payment.confirmed").expect("type"),
                subject: None,
                correlation_key: Some(execution::CorrelationKey::parse("first").expect("key")),
                occurred_at: None,
                data: br#"{"value":"event-wins-before-timeout"}"#.to_vec(),
                dedupe_ttl: None,
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("actual event admission before the timeout lock race");
    let mut events = store
        .claim_events(8, Duration::from_secs(60))
        .await
        .expect("actual event claim");
    assert_eq!(events.len(), 1);
    let event = events.pop().expect("one actual admission");
    let (step_id,since):(uuid::Uuid,time::OffsetDateTime)=sqlx::query_as("SELECT step_id,event_since FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual current exact subscription");
    let candidate = WaitingStep {
        tenant_id: fixture.world.admin.tenant_id(),
        namespace_id: fixture.world.namespace.id(),
        run_id: fixture.run,
        instance: wait(0),
        step_id,
        event_type: event.envelope.event_type.clone(),
        correlation_key: execution::CorrelationKey::parse("first").expect("key"),
        since,
    };
    let pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&pool)
    .await
    .expect("exact held ingress backend");
    let owner = PgRunStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let mut tx = owner
        .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
        .await
        .expect("held current ingress Run")
        .expect("run");
    assert!(
        receipts::persist_full_delivery(&mut tx, &candidate, &event)
            .await
            .is_some()
    );
    let scheduler = Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            concurrency: 1,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(fixture.placement.clone());
    let attempted = async { scheduler.tick().await };
    let release = async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity waiter JOIN pg_stat_activity holder ON holder.pid=$1 AND holder.backend_start=$2 WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' AND waiter.wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(waiter.pid)) AND waiter.query LIKE 'SELECT run_version FROM runs WHERE%FOR UPDATE%')").bind(holder.0).bind(holder.1).fetch_one(&fixture.world.db.pool).await.expect("exact bounded deadline entry waits after its snapshot");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
        tx.commit()
            .await
            .expect("event changes the actual Run revision and binding ahead of queued deadline");
    };
    let (report, ()) = crate::support::bounded::within(
        "actual timeout consumer observes an event committed before bounded entry",
        Duration::from_secs(30),
        async { tokio::join!(attempted, release) },
    )
    .await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        report.waits_timed_out, 1,
        "only the untouched same-name sibling times out: {report:?}"
    );
    let values:Vec<(i32,String,bool,bool)>=sqlx::query_as("SELECT instance_path_indices[1],status,timed_out,output_digest IS NOT NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND cardinality(instance_path_nodes)>0 ORDER BY instance_path_indices").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_all(&fixture.world.db.pool).await.expect("event winner and independent timeout sibling");
    assert_eq!(
        values,
        vec![
            (0, "SUCCEEDED".into(), false, true),
            (1, "SUCCEEDED".into(), true, false)
        ]
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.satisfied'),count(*) FILTER(WHERE event_type='wait.timed_out'),(SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2) FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no stale timeout or duplicate receipt");
    assert_eq!(counts, (1, 1, 1));
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("actual consumer revision/binding race replay");
    pool.close().await;
}
