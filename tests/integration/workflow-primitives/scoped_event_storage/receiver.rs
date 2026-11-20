//! Actual production dispatcher over internal immutable scoped workflow fixtures.
use super::*;
use sagat_application::{entropy::SystemEntropy, events::PublishRequest, execution::Engine};
use sagat_domain::{events::EventId, workflow::EventType};
use sagat_scheduler::{CollectionAdvance, EventDispatchSettings, EventDispatcher};

pub(super) fn dispatcher(
    store: &PgRunStore,
    scan_batch: u32,
    batch: u32,
) -> EventDispatcher<PgRunStore, JsonEvaluator, SystemEntropy> {
    EventDispatcher::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        EventDispatchSettings {
            batch,
            scan_batch,
            fan_out: 1,
            ..EventDispatchSettings::default()
        },
    )
}

async fn publish(fixture: &Fixture, key: &str) {
    publish_id(fixture, key, "collection-receiver-event").await;
}

pub(super) async fn publish_id(fixture: &Fixture, key: &str, id: &str) {
    fixture
        .world
        .events
        .publish(
            &fixture.world.admin,
            PublishRequest {
                namespace: "payments".into(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse("payment.confirmed").expect("type"),
                subject: None,
                correlation_key: Some(execution::CorrelationKey::parse(key).expect("key")),
                occurred_at: None,
                data: br#"{"value":"received"}"#.to_vec(),
                dedupe_ttl: None,
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("actual ingress");
}

async fn run_delivery(lookback: bool) {
    let name = format!("collection-receiver-{lookback}");
    let mut document = wait_document(&name, true, 60000);
    document["bodies"]["line"]["nodes"][1]["correlationExpr"] = json!("input.value");
    let (fixture, store) = pending_wait_document(&name, document).await;
    if lookback {
        publish(&fixture, &name).await;
        let round = dispatcher(&store, 0, 1).tick().await;
        assert_eq!(
            (round.dispatched, round.steps_woken, round.retried),
            (1, 0, 0)
        );
    }
    for _ in 0..2 {
        assert_eq!(
            scheduler::advance(&fixture, &store, false).await,
            CollectionAdvance::BodyProgress
        );
    }
    if !lookback {
        publish(&fixture, &name).await;
    }
    let receiver = dispatcher(&store, u32::from(lookback), u32::from(!lookback));
    let first = receiver.tick().await;
    let expected_first = if lookback { 1 } else { 2 };
    assert_eq!(
        first.steps_woken, expected_first,
        "actual dispatcher must deliver qualified instances"
    );
    assert_eq!(first.retried, 0);
    if lookback {
        let second = dispatcher(&store, 1, 0).tick().await;
        assert_eq!(
            (second.steps_woken, second.retried),
            (1, 0),
            "cold lookback continues with the other instance"
        );
    }
    let duplicate = dispatcher(&store, 1, 1).tick().await;
    assert_eq!((duplicate.steps_woken, duplicate.retried), (0, 0));
    let rows:Vec<(String,bool,Vec<u8>,uuid::Uuid,String)>=sqlx::query_as("SELECT s.status,s.routed,p.content,s.step_id,d.consumer FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest JOIN event_deliveries d ON d.tenant_id=s.tenant_id AND d.run_id=s.run_id AND d.instance_id=s.step_id WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' ORDER BY s.instance_path_indices")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_all(&fixture.world.db.pool).await.expect("independent exact outcomes, retained canonical values and receipts");
    assert_eq!(rows.len(), 2);
    for (status, routed, bytes, id, consumer) in rows {
        assert_eq!((status.as_str(), routed), ("SUCCEEDED", true));
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("canonical event document");
        assert_eq!(value["data"], json!({"value":"received"}));
        assert_eq!(consumer, format!("wait-instance:{}/{}", fixture.run, id));
    }
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2),(SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.satisfied')")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("single receipts and outcomes");
    assert_eq!(counts, (2, 2));
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold replay of production delivery");
}

#[tokio::test]
async fn the_event_dispatcher_pages_same_name_scoped_waits_without_aliasing() {
    run_delivery(false).await;
}

#[tokio::test]
async fn the_event_dispatcher_recovers_finished_inbox_events_for_late_scoped_waits() {
    run_delivery(true).await;
}

async fn subscribed(name: &str) -> (Fixture, PgRunStore) {
    let mut document = wait_document(name, true, 60000);
    document["bodies"]["line"]["nodes"][1]["correlationExpr"] = json!("input.value");
    let (fixture, store) = pending_wait_document(name, document).await;
    for _ in 0..2 {
        assert_eq!(
            scheduler::advance(&fixture, &store, false).await,
            CollectionAdvance::BodyProgress
        );
    }
    (fixture, store)
}

#[tokio::test]
async fn scoped_lookback_and_scan_markers_require_each_complete_current_candidate() {
    use sagat_application::dispatch::EventDispatchStore;
    let name = "receiver-current-candidate";
    let (fixture, store) = subscribed(name).await;
    publish(&fixture, name).await;
    let event = store
        .claim_events(1, std::time::Duration::from_secs(60))
        .await
        .expect("real claim")
        .remove(0);
    let candidates = store
        .waits_for_event(&event, None, 8)
        .await
        .expect("native qualified candidates");
    assert_eq!(candidates.len(), 2);
    let candidate = &candidates[0];
    assert_eq!(candidate.instance, wait(0));
    for mutation in 0..8 {
        let mut stale = candidate.clone();
        match mutation {
            0 => stale.step_id = uuid::Uuid::new_v4(),
            1 => stale.instance = wait(1),
            2 => stale.tenant_id = fixture.world.other_admin.tenant_id(),
            3 => stale.namespace_id = fixture.world.other_namespace.id(),
            4 => stale.event_type = EventType::parse("payment.other").expect("type"),
            5 => stale.correlation_key = execution::CorrelationKey::parse("other").expect("key"),
            6 => stale.since += time::Duration::seconds(1),
            7 => stale.run_id = execution::RunId::generate(),
            _ => unreachable!(),
        }
        assert!(
            store
                .undelivered_event_for(&stale)
                .await
                .expect("stale lookback")
                .is_none()
        );
        store
            .mark_wait_scanned(&stale)
            .await
            .expect("stale scan is inert");
    }
    let markers:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND event_scanned_at IS NOT NULL").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no stale markers");
    assert_eq!(markers, 0);
    for current in &candidates {
        assert!(
            store
                .undelivered_event_for(current)
                .await
                .expect("current exact lookback")
                .is_some()
        );
    }
    store
        .mark_wait_scanned(candidate)
        .await
        .expect("mark one exact current wait");
    let unscanned = store
        .unscanned_waits(None, 8)
        .await
        .expect("remaining exact candidate");
    assert_eq!(unscanned, vec![candidates[1].clone()]);
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("markers do not mutate execution");
}

#[tokio::test]
async fn a_dispatcher_quota_refusal_rolls_back_receipts_and_retries_pending_lookback() {
    quota_refusal(false).await;
}

#[tokio::test]
async fn a_dispatcher_quota_refusal_with_held_inbox_keeps_lookback_pending() {
    quota_refusal(true).await;
}

async fn quota_refusal(hold_inbox: bool) {
    crate::support::bounded::within(
        "scoped dispatcher quota rollback and cold recovery",
        crate::support::bounded::LIFECYCLE_LIMIT,
        quota_refusal_scenario(hold_inbox),
    )
    .await;
}

async fn quota_refusal_scenario(hold_inbox: bool) {
    let name = if hold_inbox {
        "receiver-quota-held-inbox"
    } else {
        "receiver-quota-rollback"
    };
    let (fixture, store) = subscribed(name).await;
    publish(&fixture, name).await;
    sqlx::query("INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) SELECT tenant_id,NULL,'tenant_content_bytes',content_bytes+content_reserved_bytes,'test' FROM tenant_retained_usage WHERE tenant_id=$1").bind(fixture.world.admin.tenant_id().as_uuid()).execute(&fixture.world.db.pool).await.expect("close actual current quota");
    let held_inbox = if hold_inbox {
        let mut tx = fixture
            .world
            .db
            .pool
            .begin()
            .await
            .expect("owned inbox barrier");
        // Compatible with scoped source validation, conflicting with UPDATE SKIP LOCKED.
        sqlx::query("SELECT event_id FROM inbox_events WHERE tenant_id=$1 AND event_id='collection-receiver-event' FOR SHARE")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("hold compatible scoped source lock before the mixed tick");
        Some(tx)
    } else {
        None
    };
    let before = guards::snapshot(&fixture).await;
    let (_log_guard, captured) = crate::support::logs::capture();
    let refused = crate::support::bounded::within(
        "native scoped event quota refusal",
        std::time::Duration::from_secs(30),
        // Test dispatch retry independently of any failed lookback's source-lock cleanup.
        dispatcher(&store, u32::from(hold_inbox), 1).tick(),
    )
    .await;
    let diagnostic = captured.text();
    if hold_inbox {
        assert!(
            diagnostic
                .lines()
                .any(|line| line.contains("delivering a past event failed")
                    && line.contains("limit reached: retained content bytes of the tenant")),
            "the real lookback must reach its quota refusal: {diagnostic}"
        );
    }
    assert_eq!(
        (
            refused.steps_woken,
            refused.retried,
            refused.dispatched,
            refused.dead
        ),
        (0, u32::from(!hold_inbox), 0, 0),
        "retry is required for a claimed event; a held inbox remains pending: {diagnostic}"
    );
    if let Some(tx) = held_inbox {
        tx.rollback().await.expect("release owned inbox barrier");
    }
    let inbox: (String, i32, Option<String>) = sqlx::query_as(
        "SELECT status,attempts,last_error FROM inbox_events WHERE tenant_id=$1 AND event_id='collection-receiver-event'",
    ).bind(fixture.world.admin.tenant_id().as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("durable refusal class and attempt budget");
    assert_eq!((inbox.0.as_str(), inbox.1), ("PENDING", 0));
    assert_eq!(
        inbox.2.as_deref(),
        (!hold_inbox).then_some("a quota of the namespace or tenant was reached; will retry")
    );
    let mut after = guards::snapshot(&fixture).await;
    let retry_log_bytes:i64=sqlx::query_scalar("SELECT coalesce(octet_length(last_error),0)::bigint FROM inbox_events WHERE tenant_id=$1 AND event_id='collection-receiver-event'").bind(fixture.world.admin.tenant_id().as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("expected retained retry metadata");
    assert_eq!(
        after["retained"]["log_bytes"].as_i64().expect("log bytes")
            - before["retained"]["log_bytes"]
                .as_i64()
                .expect("prior log bytes"),
        retry_log_bytes
    );
    // Event retry metadata is deliberately committed after the failed outcome transaction.
    after["retained"]["log_bytes"] = before["retained"]["log_bytes"].clone();
    assert_eq!(after, before);
    let effects:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2),(SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND event_scanned_at IS NOT NULL)").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no orphan receipt or lost lookback scan");
    assert_eq!(effects, (0, 0));
    sqlx::query("DELETE FROM quota_limits WHERE tenant_id=$1 AND resource='tenant_content_bytes'")
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore owned fixture quota");
    let recovered = dispatcher(&store, 2, 0).tick().await;
    assert_eq!(
        (recovered.steps_woken, recovered.retried),
        (2, 0),
        "pending lookback recovers without waiting for the claimed event retry"
    );
    let duplicate = dispatcher(&store, 2, 0).tick().await;
    assert_eq!(duplicate.steps_woken, 0);
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("quota rollback and cold recovery replay");
}

#[tokio::test]
async fn the_dispatcher_records_paused_scoped_outcomes_and_routes_only_after_resume() {
    use sagat_application::runs::RunOperation;
    let name = "receiver-paused-outcomes";
    let (fixture, store) = subscribed(name).await;
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Pause,
        "pause-before-dispatch",
    )
    .await;
    publish(&fixture, name).await;
    let round = dispatcher(&store, 0, 1).tick().await;
    assert_eq!((round.steps_woken, round.retried), (2, 0));
    let unrouted:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND status='SUCCEEDED' AND NOT routed AND output_digest IS NOT NULL").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("retained paused outcomes");
    assert_eq!(unrouted, 2);
    assert_eq!(
        scheduler::advance(&fixture, &store, false).await,
        CollectionAdvance::Waiting
    );
    assert_eq!(dispatcher(&store, 2, 1).tick().await.steps_woken, 0);
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Resume,
        "resume-after-dispatch",
    )
    .await;
    for _ in 0..2 {
        assert_eq!(
            scheduler::advance(&fixture, &store, false).await,
            CollectionAdvance::BodyProgress
        );
    }
    let routed:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND status='SUCCEEDED' AND routed AND output_digest IS NOT NULL").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("exact resumed routes");
    assert_eq!(routed, 2);
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("paused production outcomes and resumed routes replay");
}

#[tokio::test]
async fn a_failed_scoped_lookback_cannot_monopolize_the_next_bounded_round() {
    let name = "receiver-lookback-fairness";
    let (fixture, store) = subscribed(name).await;
    publish(&fixture, name).await;
    sqlx::raw_sql("CREATE FUNCTION reject_first_scoped_delivery() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only first consumer failure' USING ERRCODE='54000', CONSTRAINT='tenant_content_quota_exceeded'; END $$; CREATE TRIGGER reject_first_scoped_delivery BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='wait.satisfied' AND NEW.instance_path_indices=ARRAY[0]::int[]) EXECUTE FUNCTION reject_first_scoped_delivery()")
        .execute(&fixture.world.db.pool).await.expect("owned per-consumer persistence fault, no real quota change");
    let receiver = dispatcher(&store, 1, 0);
    assert_eq!(receiver.tick().await.steps_woken, 0);
    assert_eq!(
        receiver.tick().await.steps_woken,
        1,
        "a failed first wait must not suppress its healthy sibling"
    );
    sqlx::raw_sql("DROP TRIGGER reject_first_scoped_delivery ON run_events; DROP FUNCTION reject_first_scoped_delivery()")
        .execute(&fixture.world.db.pool).await.expect("remove owned consumer fault");
    assert_eq!(
        receiver.tick().await.steps_woken,
        1,
        "wrap around and retry the still-pending first wait"
    );
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("fair lookback and retried outcome replay");
}

#[tokio::test]
async fn an_inbox_removed_after_lookback_selection_does_not_close_the_scan() {
    use sagat_application::dispatch::EventDispatchStore;
    let (fixture, store) = pending_wait("receiver-stale-lookback").await;
    assert_eq!(
        scheduler::advance(&fixture, &store, false).await,
        CollectionAdvance::BodyProgress
    );
    publish_id(&fixture, "first", "expired-first-event").await;
    publish_id(&fixture, "first", "retained-second-event").await;
    for event in store
        .claim_events(8, std::time::Duration::from_secs(60))
        .await
        .expect("actual pre-lookback claims")
    {
        store
            .finish_event(&event)
            .await
            .expect("actual finished inbox admissions");
    }
    sqlx::query("UPDATE inbox_events SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE tenant_id=$1 AND event_id='expired-first-event'").bind(fixture.world.admin.tenant_id().as_uuid()).execute(&fixture.world.db.pool).await.expect("age only first inbox retention window");
    let holder_pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&holder_pool)
    .await
    .expect("exact owned holder identity");
    let holder_store = PgRunStore::new(
        holder_pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    let tx = holder_store
        .begin_current_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            wait(0).path().clone(),
            Lock::Wait,
        )
        .await
        .expect("held current path")
        .expect("run");
    let receiver = dispatcher(&store, 1, 0);
    let delivery = receiver.tick();
    let release = async {
        cancellation::observe_run_waiter(&fixture, holder).await;
        assert_eq!(
            store
                .purge_events(8)
                .await
                .expect("actual inbox cleanup before receiver entry"),
            1
        );
        tx.commit().await.expect("release unchanged Run lock");
    };
    let (round, ()) = crate::support::bounded::within(
        "selected inbox removal while dispatcher waits on exact Run",
        std::time::Duration::from_secs(30),
        async { tokio::join!(delivery, release) },
    )
    .await;
    assert_eq!(round.steps_woken, 0);
    let scanned:bool=sqlx::query_scalar("SELECT event_scanned_at IS NOT NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("lookback remains pending after stale selection");
    assert!(
        !scanned,
        "a removed selected admission must not hide the later retained event"
    );
    assert_eq!(dispatcher(&store, 1, 0).tick().await.steps_woken, 1);
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("recovered next retained admission replay");
    holder_pool.close().await;
}

#[tokio::test]
async fn a_post_admission_routing_error_is_retried_instead_of_finishing_the_event() {
    let (fixture, store) = pending_wait("receiver-invalid-local-route").await;
    assert_eq!(
        scheduler::advance(&fixture, &store, false).await,
        CollectionAdvance::BodyProgress
    );
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            wait(0).path().clone(),
        )
        .await
        .expect("current path")
        .expect("path");
    let returned = ExecutionInstanceId::in_path(
        wait(0).path().clone(),
        NodeId::parse("returned").expect("return"),
    );
    let id = sagat_application::execution::instance_step_id(
        snapshot.info.version_id,
        fixture.run,
        &returned,
    );
    // Explicit corruption fixture: a structurally valid but prematurely admitted successor.
    // No normal production transition or replay guarantee is claimed for this source.
    sqlx::query("INSERT INTO steps SELECT (jsonb_populate_record(NULL::steps,to_jsonb(s)||jsonb_build_object('step_id',$3::uuid,'node','returned','node_kind','return','status','PENDING','waiting_kind',NULL,'due_at',NULL,'correlation_key',NULL,'event_type',NULL,'event_since',NULL))).* FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).bind(id).execute(&fixture.world.db.pool).await.expect("owned malformed route fixture");
    publish(&fixture, "first").await;
    let before: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("current revision");
    let round = dispatcher(&store, 0, 1).tick().await;
    assert_eq!(
        (round.steps_woken, round.dispatched, round.retried),
        (0, 0, 1),
        "validated receipt does not turn a deterministic route error into a lost race"
    );
    let after:(i64,i64,i64)=sqlx::query_as("SELECT r.run_version,(SELECT count(*) FROM event_deliveries WHERE tenant_id=r.tenant_id AND run_id=r.id),(SELECT count(*) FROM run_events WHERE tenant_id=r.tenant_id AND run_id=r.id AND event_type='wait.satisfied') FROM runs r WHERE r.tenant_id=$1 AND r.id=$2").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("complete outcome rollback");
    assert_eq!(after, (before, 0, 0));
}

#[tokio::test]
async fn the_lookback_index_migration_round_trips_with_admitted_waits_present() {
    use sagat_application::dispatch::EventDispatchStore;
    use sagat_infra_postgres::{MIGRATOR, migrate};
    let (fixture, store) = subscribed("receiver-index-roundtrip").await;
    let before = guards::snapshot(&fixture).await;
    let candidates = store
        .unscanned_waits(None, 8)
        .await
        .expect("qualified discovery before rollback");
    assert_eq!(candidates.len(), 2);
    let indexed: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.steps_event_unscanned_path_idx') IS NOT NULL",
    )
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("current keyset index");
    assert!(indexed);
    MIGRATOR
        .undo(&fixture.world.db.pool, 20261222000000)
        .await
        .expect("index-only rollback with current waits and retained payloads");
    assert_eq!(guards::snapshot(&fixture).await, before);
    assert_eq!(
        store
            .unscanned_waits(None, 8)
            .await
            .expect("legacy index remains sufficient for correctness"),
        candidates
    );
    migrate(&fixture.world.db.pool)
        .await
        .expect("upgrade with admitted waits present");
    assert_eq!(guards::snapshot(&fixture).await, before);
    assert_eq!(
        store
            .unscanned_waits(None, 8)
            .await
            .expect("restored qualified discovery"),
        candidates
    );
    let indexed: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.steps_event_unscanned_path_idx') IS NOT NULL",
    )
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("restored keyset index");
    assert!(indexed);
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("migration preserves native replay");
}
