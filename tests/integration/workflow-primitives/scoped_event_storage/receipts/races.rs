//! Deterministic native outcome and inbox-cleanup ordering; no sleep-based races.
use super::*;
use sagat_application::execution::Engine;
use sagat_application::runs::RunOperation;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OtherOutcome {
    Timeout,
    Cancel,
}

async fn competing_outcomes(other: OtherOutcome, event_first: bool) {
    let name = format!("event-race-{other:?}-{event_first}").to_lowercase();
    let (fixture, store, candidate, event) = source_document(
        &name,
        wait_document(
            &name,
            true,
            if other == OtherOutcome::Timeout {
                1
            } else {
                60000
            },
        ),
    )
    .await;
    if other == OtherOutcome::Timeout {
        crate::support::bounded::within("actual scoped event deadline is due", Duration::from_secs(30), async {
            loop {
                let due:bool=sqlx::query_scalar("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("database deadline barrier");
                if due { break; }
                tokio::task::yield_now().await;
            }
        }).await;
    }
    let holder_pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&holder_pool)
    .await
    .expect("exact holder identity");
    let holder_store = PgRunStore::new(
        holder_pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let mut tx = holder_store
        .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
        .await
        .expect("held own Run")
        .expect("run");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(
            Shard::of(candidate.run_id.as_uuid()),
            std::time::Instant::now(),
        )
        .expect("own shard");
    if event_first {
        assert!(
            persist_full_delivery(&mut tx, &candidate, &event)
                .await
                .is_some()
        );
        let attempted = async {
            match other {
                OtherOutcome::Timeout => {
                    let mut next = store
                        .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                        .await
                        .expect("queued timeout")
                        .expect("run");
                    assert!(
                        next.holds_shard("scoped-worker-test", lease)
                            .await
                            .expect("current timeout authority")
                    );
                    let now = next.now();
                    let progress = {
                        let (definition, state) = next.parts();
                        execution::time_out_event_wait_instance(
                            definition,
                            state,
                            &candidate.instance,
                            now,
                        )
                    };
                    assert!(
                        progress.is_err(),
                        "a committed event must prevent a queued timeout"
                    );
                }
                OtherOutcome::Cancel => {
                    timeouts::operate(
                        &fixture,
                        &store,
                        RunOperation::Cancel,
                        "cancel-after-held-event",
                    )
                    .await
                }
            }
        };
        let release = async {
            cancellation::observe_run_waiter(&fixture, holder).await;
            tx.commit()
                .await
                .expect("event commits before actual queued contender");
        };
        crate::support::bounded::within(
            "event-first exact Run lock edge",
            Duration::from_secs(30),
            async { tokio::join!(attempted, release) },
        )
        .await;
    } else {
        assert!(
            tx.holds_shard("scoped-worker-test", lease)
                .await
                .expect("current holder authority")
        );
        match other {
            OtherOutcome::Timeout => {
                let now = tx.now();
                let progress = {
                    let (definition, state) = tx.parts();
                    execution::time_out_event_wait_instance(
                        definition,
                        state,
                        &candidate.instance,
                        now,
                    )
                    .expect("real current timeout")
                };
                tx.persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(Actor::System("scheduler"), None),
                )
                .await
                .expect("held timeout persistence");
            }
            OtherOutcome::Cancel => {
                Engine::new(JsonEvaluator)
                    .run(
                        &mut tx,
                        Effects::by(Actor::System("event-race-test"), None),
                        |definition, state, now| {
                            Ok((
                                (),
                                execution::stop_run(
                                    definition,
                                    state,
                                    execution::StopReason::Cancelled,
                                    None,
                                    now,
                                )?,
                            ))
                        },
                    )
                    .await
                    .expect("real uncommitted system cancellation");
            }
        }
        let attempted = async {
            let mut next = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("queued event ingress")
                .expect("run");
            assert!(
                next.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .expect("recheck current outcome authority")
                    .is_none(),
                "a committed earlier outcome must prevent a queued event"
            );
            next.commit()
                .await
                .expect("no-work contender retains no receipt or value");
        };
        let release = async {
            cancellation::observe_run_waiter(&fixture, holder).await;
            tx.commit()
                .await
                .expect("earlier outcome commits before actual queued event");
        };
        crate::support::bounded::within(
            "other-outcome-first exact Run lock edge",
            Duration::from_secs(30),
            async { tokio::join!(attempted, release) },
        )
        .await;
    }
    if other == OtherOutcome::Cancel {
        let scheduler = cancellation::scheduler(&fixture, &store);
        for _ in 0..16 {
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            if !report.did_work() {
                break;
            }
        }
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(candidate.tenant_id.as_uuid())
                .bind(candidate.run_id.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("fully drained cancellation");
        assert_eq!(status, "CANCELLED");
    }
    let actual:(String,bool,bool,bool)=sqlx::query_as("SELECT status,timed_out,output_digest IS NOT NULL,waiting_kind IS NULL AND due_at IS NULL AND event_type IS NULL AND correlation_key IS NULL AND event_since IS NULL AND event_scanned_at IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("winning exact outcome and closed subscription");
    assert_eq!(
        actual,
        (
            (if event_first || other == OtherOutcome::Timeout {
                "SUCCEEDED"
            } else {
                "CANCELLED"
            })
            .into(),
            !event_first && other == OtherOutcome::Timeout,
            event_first,
            true
        )
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.satisfied'),count(*) FILTER(WHERE event_type='wait.timed_out'),(SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2) FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one winning journal outcome and receipt");
    assert_eq!(
        counts,
        (
            i64::from(event_first),
            i64::from(!event_first && other == OtherOutcome::Timeout),
            i64::from(event_first)
        )
    );
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("winning outcome replay");
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    holder_pool.close().await;
}

#[tokio::test]
async fn event_commit_prevents_a_due_timeout_actually_queued_on_the_run_lock() {
    competing_outcomes(OtherOutcome::Timeout, true).await;
}
#[tokio::test]
async fn timeout_commit_prevents_an_event_actually_queued_on_the_run_lock() {
    competing_outcomes(OtherOutcome::Timeout, false).await;
}
#[tokio::test]
async fn event_commit_preserves_its_value_ahead_of_queued_public_cancellation() {
    competing_outcomes(OtherOutcome::Cancel, true).await;
}
#[tokio::test]
async fn cancellation_commit_prevents_an_event_actually_queued_on_the_run_lock() {
    competing_outcomes(OtherOutcome::Cancel, false).await;
}

async fn expire_dispatched(fixture: &Fixture, store: &PgRunStore, event: &ClaimedEvent) {
    store
        .finish_event(event)
        .await
        .expect("actual dispatched status");
    sqlx::query("UPDATE inbox_events SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4").bind(event.tenant_id.as_uuid()).bind(event.namespace_id.as_uuid()).bind(event.envelope.key.source.as_str()).bind(event.envelope.key.event_id.as_str()).execute(&fixture.world.db.pool).await.expect("owner ages only the deduplication window");
}

#[tokio::test]
async fn current_inbox_lock_keeps_cleanup_queued_until_the_value_commits() {
    let (fixture, store, candidate, event) = source("event-before-inbox-cleanup").await;
    expire_dispatched(&fixture, &store, &event).await;
    let pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&pool)
    .await
    .expect("exact holder identity");
    let owner = PgRunStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let mut tx = owner
        .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
        .await
        .expect("held Run")
        .expect("run");
    let document = persist_full_delivery(&mut tx, &candidate, &event)
        .await
        .expect("current dispatched lookback event");
    let purged = async { store.purge_events(8).await.expect("actual native cleanup") };
    let release = async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity waiter JOIN pg_stat_activity holder ON holder.pid=$1 AND holder.backend_start=$2 WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' AND waiter.wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(waiter.pid)) AND waiter.query LIKE '%DELETE FROM inbox_events%')").bind(holder.0).bind(holder.1).fetch_one(&fixture.world.db.pool).await.expect("exact cleanup-to-ingress lock edge");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
        tx.commit()
            .await
            .expect("value commits ahead of actual queued cleanup");
    };
    let (purged, ()) = crate::support::bounded::within(
        "inbox cleanup waits for current canonical value commit",
        Duration::from_secs(30),
        async { tokio::join!(purged, release) },
    )
    .await;
    assert_eq!(purged, 1);
    let remaining:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM inbox_events WHERE tenant_id=$1),(SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2)").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("cleanup and receipt cascade completed");
    assert_eq!(remaining, (0, 0));
    let retained:Vec<u8>=sqlx::query_scalar("SELECT p.content FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("retained run value survives inbox/receipt cleanup");
    assert_eq!(retained, document);
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("cold journal proof without live receipt");
    assert!(
        probe(&store, &candidate, &event, false)
            .await
            .expect("stale post-cleanup claim")
            .is_none()
    );
    pool.close().await;
}

#[tokio::test]
async fn cleanup_committed_first_prevents_a_stale_claim_from_creating_any_value() {
    let (fixture, store, candidate, event) = source("inbox-cleanup-before-event").await;
    expire_dispatched(&fixture, &store, &event).await;
    assert_eq!(
        store
            .purge_events(8)
            .await
            .expect("actual cleanup committed first"),
        1
    );
    let before = guards::snapshot(&fixture).await;
    for full in [false, true] {
        assert!(
            probe(&store, &candidate, &event, full)
                .await
                .expect("stale purged claim")
                .is_none()
        );
    }
    assert_eq!(guards::snapshot(&fixture).await, before);
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("unchanged waiting replay");
}

#[tokio::test]
async fn a_prepared_path_delivery_refuses_a_timeout_committed_while_its_entry_is_queued() {
    let name = "event-prepared-path-after-timeout";
    let (fixture, store, candidate, _) = source_document(name, wait_document(name, true, 1)).await;
    crate::support::bounded::within("prepared event source is actually due", Duration::from_secs(30), async {
        loop {
            let due:bool=sqlx::query_scalar("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("database deadline barrier");
            if due { break; }
            tokio::task::yield_now().await;
        }
    }).await;
    let prepared = store
        .load_path(
            candidate.tenant_id,
            candidate.run_id,
            candidate.instance.path().clone(),
        )
        .await
        .expect("pre-race snapshot")
        .expect("path");
    let pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&pool)
    .await
    .expect("exact holder identity");
    let owner = PgRunStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let mut tx = owner
        .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
        .await
        .expect("held own Run")
        .expect("run");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(
            Shard::of(candidate.run_id.as_uuid()),
            std::time::Instant::now(),
        )
        .expect("own shard");
    assert!(
        tx.holds_shard("scoped-worker-test", lease)
            .await
            .expect("current timeout authority")
    );
    let now = tx.now();
    let progress = {
        let (definition, state) = tx.parts();
        execution::time_out_event_wait_instance(definition, state, &candidate.instance, now)
            .expect("actual timeout")
    };
    tx.persist_addressed(
        &progress.events,
        &[],
        Effects::by(Actor::System("scheduler"), None),
    )
    .await
    .expect("held timeout persistence");
    let attempted = async {
        let result = store.begin_path(prepared, Lock::Wait).await;
        assert!(
            matches!(
                result,
                Err(sagat_application::error::StoreError::RevisionConflict { .. })
            ),
            "a prepared ingress snapshot cannot cross a committed Run revision"
        );
    };
    let release = async {
        cancellation::observe_run_waiter(&fixture, holder).await;
        tx.commit()
            .await
            .expect("timeout commits ahead of queued prepared-path entry");
    };
    crate::support::bounded::within(
        "prepared-path event entry is fenced after queued timeout",
        Duration::from_secs(30),
        async { tokio::join!(attempted, release) },
    )
    .await;
    let counts:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.satisfied'),(SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2) FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no stale receipt or outcome");
    assert_eq!(counts, (0, 0));
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("timeout journal replay");
    pool.close().await;
}
