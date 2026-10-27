//! Native collection cancellation must dispose actual event bindings and their retained slots.
use super::*;
use sagat_application::{entropy::SystemEntropy, execution::Engine, runs::RunOperation};
use sagat_scheduler::{CollectionAdvance, Scheduler, SchedulerSettings};

#[tokio::test]
async fn user_cancellation_drains_a_subscribed_member_and_retires_its_reservations() {
    let bounded = cancel_case(false).await;
    let full = cancel_case(true).await;
    assert_eq!(
        bounded, full,
        "full and frontier cancellation retain the same transitions and journal"
    );
}

fn scheduler(
    fixture: &Fixture,
    store: &PgRunStore,
) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(fixture.placement.clone())
}

async fn cancel_case(full: bool) -> (Vec<CollectionAdvance>, Vec<serde_json::Value>) {
    let (fixture, store) = pending_wait(&format!("event-cancel-native-{full}")).await;
    guards::start(&fixture, &store, false)
        .await
        .expect("actual subscribed member");
    sqlx::query("UPDATE steps SET event_scanned_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).execute(&fixture.world.db.pool).await.expect("owner-set scan optimization");
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Cancel,
        "cancel-subscribed-event",
    )
    .await;
    let scheduler = scheduler(&fixture, &store);
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let mut transitions = Vec::new();
    for _ in 0..8 {
        let progress = if full {
            scheduler
                .advance_collection_from_run(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        } else {
            scheduler
                .advance_collection(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        }
        .expect("exact drain transition");
        transitions.push(progress);
        if matches!(
            progress,
            CollectionAdvance::DrainFinished
                | CollectionAdvance::Finished
                | CollectionAdvance::AlreadyFinished
        ) {
            break;
        }
        assert_ne!(
            progress,
            CollectionAdvance::Waiting,
            "no worker owns this suspension"
        );
    }
    for _ in 0..16 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if !report.did_work() {
            break;
        }
    }
    let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("native cancelled status");
    assert_eq!(
        status, "CANCELLED",
        "a collection may not remain stuck on a workerless event suspension"
    );
    let rows:Vec<(String,bool,bool)>=sqlx::query_as("SELECT status,routed,waiting_kind IS NULL AND due_at IS NULL AND event_type IS NULL AND correlation_key IS NULL AND event_since IS NULL AND event_scanned_at IS NULL AND lease_token_hash IS NULL AND reported_lease_token_hash IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' ORDER BY instance_path_indices")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_all(&fixture.world.db.pool).await.expect("closed exact subscriptions");
    assert_eq!(
        rows,
        vec![
            ("CANCELLED".into(), true, true),
            ("CANCELLED".into(), true, true)
        ]
    );
    let usage:(i64,i32,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND slot_held),(SELECT reserved_instances FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2),(SELECT active_iterations::bigint FROM tenant_iteration_usage WHERE tenant_id=$1)")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no retained slot/reservation");
    assert_eq!(usage, (0, 0, 0));
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold cancelled replay");
    let journal=sqlx::query_scalar("SELECT jsonb_build_object('event',event_type,'node',node,'pathNodes',instance_path_nodes,'pathIndices',instance_path_indices,'detail',detail) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type IN ('collection.drain_started','collection.iteration_drained','collection.finished') ORDER BY seq")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_all(&fixture.world.db.pool).await.expect("independent full/frontier drain journal");
    (transitions, journal)
}

async fn direct_drain(
    fixture: &Fixture,
    store: &PgRunStore,
) -> sagat_application::primitives::CollectionIterationDrain {
    use sagat_application::primitives::{
        CollectionIterationDrain, CollectionScopeId, CollectionTransition, SchedulerAuthority,
    };
    assert_eq!(
        scheduler(fixture, store)
            .advance_collection(
                fixture.world.admin.tenant_id(),
                fixture.run,
                &ExecutionInstanceId::root(NodeId::parse("items").expect("owner"))
            )
            .await
            .expect("actual coordinator begins drain"),
        CollectionAdvance::DrainStarted
    );
    let (revision,scope):(i64,uuid::Uuid)=sqlx::query_as("SELECT r.run_version,s.scope_id FROM runs r JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2 AND s.owner_node='items'")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("current native drain source");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    CollectionIterationDrain::new(
        CollectionTransition::new(
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
            fixture.run,
            u64::try_from(revision).expect("revision"),
            ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
            SchedulerAuthority::new("scoped-worker-test", lease),
            sagat_domain::identity::Subject::engine(),
            None,
            crate::support::now(),
            None,
        ),
        CollectionScopeId::new(scope),
        0,
        uuid::Uuid::now_v7(),
    )
}

#[tokio::test]
async fn the_independent_native_drain_writer_refuses_corrupted_wait_capabilities_and_identity() {
    use sagat_application::primitives::CollectionStore;
    let (fixture, store) = pending_wait("event-drain-native-source").await;
    guards::start(&fixture, &store, false)
        .await
        .expect("actual subscribed source");
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Cancel,
        "cancel-before-source-guard",
    )
    .await;
    let drain = direct_drain(&fixture, &store).await;
    let native = sagat_infra_postgres::PgCollectionStore::with_definition_cache(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    let changes = [
        (
            "UPDATE steps SET reported_lease_token_hash=decode(repeat('07',32),'hex') WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET reported_lease_token_hash=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET event_since=event_since+interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET event_since=event_since-interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET node_kind='activity' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET node_kind='waitEvent' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
    ];
    for (mutate, restore) in changes {
        sqlx::query(mutate)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .execute(&fixture.world.db.pool)
            .await
            .expect("explicit current locked source corruption");
        let before = guards::snapshot(&fixture).await;
        assert!(
            native.drain_collection_iteration(&drain).await.is_err(),
            "{mutate}"
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
        sqlx::query(restore)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .execute(&fixture.world.db.pool)
            .await
            .expect("restore exact source field");
    }
    let old_id:uuid::Uuid=sqlx::query_scalar("SELECT step_id FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("canonical physical UUID");
    let other_namespace = fixture.world.add_namespace("event-drain-other-scope").await;
    for wrong_namespace in [false, true] {
        let statement = if wrong_namespace {
            "UPDATE steps SET namespace_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]"
        } else {
            "UPDATE steps SET step_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]"
        };
        sqlx::query(statement)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .bind(if wrong_namespace {
                other_namespace.id().as_uuid()
            } else {
                uuid::Uuid::new_v4()
            })
            .execute(&fixture.world.db.pool)
            .await
            .expect("CHECK-valid identity corruption");
        let before = guards::snapshot(&fixture).await;
        assert!(native.drain_collection_iteration(&drain).await.is_err());
        assert_eq!(guards::snapshot(&fixture).await, before);
        sqlx::query(statement)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .bind(if wrong_namespace {
                fixture.world.namespace.id().as_uuid()
            } else {
                old_id
            })
            .execute(&fixture.world.db.pool)
            .await
            .expect("restore exact physical identity");
    }
    native
        .drain_collection_iteration(&drain)
        .await
        .expect("restored exact source drains");
    let before = guards::snapshot(&fixture).await;
    assert!(
        !native
            .drain_collection_iteration(&drain)
            .await
            .expect("same durable drain receipt")
            .applied
    );
    assert_eq!(guards::snapshot(&fixture).await, before);
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("native independent writer replay");
}

#[tokio::test]
async fn a_late_native_drain_journal_failure_rolls_back_binding_usage_and_outbox() {
    use sagat_application::primitives::CollectionStore;
    let (fixture, store) = pending_wait("event-drain-rollback").await;
    guards::start(&fixture, &store, false)
        .await
        .expect("actual source");
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Cancel,
        "cancel-before-drain-fault",
    )
    .await;
    let drain = direct_drain(&fixture, &store).await;
    let native = sagat_infra_postgres::PgCollectionStore::with_definition_cache(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    sqlx::raw_sql("CREATE FUNCTION reject_event_drain_journal() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NOT EXISTS (SELECT 1 FROM steps s WHERE s.tenant_id=NEW.tenant_id AND s.run_id=NEW.run_id AND s.node='b' AND s.instance_path_indices=ARRAY[0] AND s.status='CANCELLED' AND s.event_since IS NULL AND s.event_scanned_at IS NULL) THEN RAISE EXCEPTION 'drain fault fired before binding cleanup'; END IF; RAISE EXCEPTION 'test-only event drain journal abort' USING ERRCODE='23514'; END $$; CREATE TRIGGER reject_event_drain_journal BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='collection.iteration_drained') EXECUTE FUNCTION reject_event_drain_journal()")
        .execute(&fixture.world.db.pool).await.expect("owned post-cleanup journal fault");
    let before = guards::snapshot(&fixture).await;
    let error = native
        .drain_collection_iteration(&drain)
        .await
        .expect_err("real transaction reaches late fault");
    assert!(
        error
            .to_string()
            .contains("test-only event drain journal abort"),
        "{error}"
    );
    assert_eq!(guards::snapshot(&fixture).await, before);
    sqlx::raw_sql("DROP TRIGGER reject_event_drain_journal ON run_events; DROP FUNCTION reject_event_drain_journal()")
        .execute(&fixture.world.db.pool).await.expect("remove own fault");
    native
        .drain_collection_iteration(&drain)
        .await
        .expect("same receipt/source commits after fault removal");
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold replay after native rollback/retry");
}

async fn observe_run_waiter(fixture: &Fixture, holder: (i32, time::OffsetDateTime)) {
    loop {
        let observed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity waiter JOIN pg_stat_activity holder ON holder.pid=$1 AND holder.backend_start=$2 WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' AND waiter.wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(waiter.pid)) AND waiter.query LIKE '%FROM runs%' AND waiter.query LIKE '%FOR UPDATE%')")
            .bind(holder.0).bind(holder.1).fetch_one(&fixture.world.db.pool).await.expect("exact current native Run lock wait edge");
        if observed {
            break;
        }
        tokio::task::yield_now().await;
    }
}

async fn timeout_and_cancel_order(cancel_first: bool) {
    let (fixture, store) =
        timeouts::waiting(&format!("event-cancel-timeout-order-{cancel_first}"), true).await;
    let holder_pool = fixture.world.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&holder_pool)
    .await
    .expect("single native holder backend");
    let holder_store = PgRunStore::new(
        holder_pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let mut tx = holder_store
        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
        .await
        .expect("held Run")
        .expect("run");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    assert!(
        tx.holds_shard("scoped-worker-test", lease)
            .await
            .expect("holder own authority")
    );
    if cancel_first {
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
        let attempted = async {
            let mut contender = store
                .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                .await
                .expect("queued timeout entry")
                .expect("run");
            assert!(
                contender
                    .holds_shard("scoped-worker-test", lease)
                    .await
                    .expect("queued current authority")
            );
            let now = contender.now();
            let progress = {
                let (definition, state) = contender.parts();
                execution::time_out_event_wait_instance(definition, state, &wait(0), now)
            };
            if let Ok(progress) = progress {
                contender
                    .persist_addressed(
                        &progress.events,
                        &[],
                        Effects::by(Actor::System("scheduler"), None),
                    )
                    .await
                    .expect("observe an incorrectly accepted timeout");
                contender
                    .commit()
                    .await
                    .expect("observe wrong timeout commit");
                false
            } else {
                drop(contender);
                true
            }
        };
        let release = async {
            observe_run_waiter(&fixture, holder).await;
            tx.commit()
                .await
                .expect("cancellation commits ahead of the actual queued timeout");
        };
        let (refused, ()) = crate::support::bounded::within(
            "cancellation-first event timeout Run-lock proof",
            std::time::Duration::from_secs(30),
            async { tokio::join!(attempted, release) },
        )
        .await;
        assert!(
            refused,
            "committed cancellation prevents the queued due timeout"
        );
    } else {
        let now = tx.now();
        let progress = {
            let (definition, state) = tx.parts();
            execution::time_out_event_wait_instance(definition, state, &wait(0), now)
                .expect("real holder timeout")
        };
        tx.persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("scheduler"), None),
        )
        .await
        .expect("uncommitted native timeout outcome");
        let cancel = timeouts::operate(
            &fixture,
            &store,
            RunOperation::Cancel,
            "cancel-after-held-timeout",
        );
        let release = async {
            observe_run_waiter(&fixture, holder).await;
            tx.commit()
                .await
                .expect("timeout commits ahead of actual queued public cancellation");
        };
        crate::support::bounded::within(
            "timeout-first public cancellation Run-lock proof",
            std::time::Duration::from_secs(30),
            async { tokio::join!(cancel, release) },
        )
        .await;
    }
    let scheduler = scheduler(&fixture, &store);
    for _ in 0..16 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if !report.did_work() {
            break;
        }
    }
    let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("cancelled final run");
    assert_eq!(status, "CANCELLED");
    let outcome:(String,bool,bool)=sqlx::query_as("SELECT status,timed_out,waiting_kind IS NULL AND due_at IS NULL AND event_type IS NULL AND correlation_key IS NULL AND event_since IS NULL AND event_scanned_at IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("winning exact outcome");
    assert_eq!(
        outcome,
        (
            if cancel_first {
                "CANCELLED"
            } else {
                "SUCCEEDED"
            }
            .into(),
            !cancel_first,
            true
        )
    );
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.timed_out'),(SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND slot_held)")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one winning decision and no held slot");
    assert_eq!(counts, (i64::from(!cancel_first), 0));
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold ordered race replay");
    holder_pool.close().await;
}

#[tokio::test]
async fn committed_cancellation_prevents_a_due_timeout_queued_on_the_run_lock() {
    timeout_and_cancel_order(true).await;
}

#[tokio::test]
async fn a_timeout_committed_ahead_of_queued_public_cancellation_keeps_its_outcome() {
    timeout_and_cancel_order(false).await;
}

#[tokio::test]
async fn a_real_sibling_correlation_failure_drains_the_subscribed_member_in_both_modes() {
    for full in [false, true] {
        let name = format!("event-failfast-wait-{full}");
        let mut document = wait_document(&name, true, 60000);
        document["bodies"]["line"]["nodes"][1]["correlationExpr"] =
            json!("item.sku == 'first' ? item.sku : input.missing");
        let (fixture, store) = pending_wait_document(&name, document).await;
        for index in 0..2 {
            let member = wait(index);
            let prepared = store
                .load_path(
                    fixture.world.admin.tenant_id(),
                    fixture.run,
                    member.path().clone(),
                )
                .await
                .expect("exact evaluation snapshot")
                .expect("path");
            let sagat_domain::workflow::NodeKind::WaitEvent { correlation, .. } = prepared
                .definition
                .node_for_instance(&member)
                .expect("pinned wait")
                .kind()
            else {
                panic!("fixture wait declaration")
            };
            let key = JsonEvaluator
                .correlation_key_instance(correlation, &prepared.data, &member)
                .map_err(|_| ());
            assert_eq!(
                key.is_ok(),
                index == 0,
                "real expression has only one valid member"
            );
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("revision-fenced entry")
                .expect("run");
            let lease = fixture
                .placement
                .snapshot()
                .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
                .expect("own shard");
            assert!(
                tx.holds_shard("scoped-worker-test", lease)
                    .await
                    .expect("own authority")
            );
            let now = tx.now();
            let progress = {
                let prepared = tx.snapshot();
                execution::start_event_wait_path_instance(
                    &prepared.definition,
                    &mut prepared.state,
                    &member,
                    key,
                    now,
                )
                .expect("actual evaluated start/failure")
            };
            tx.persist_addressed(
                &progress.events,
                &[],
                Effects::by(Actor::System("scheduler"), None),
            )
            .await
            .expect("native source outcome");
            tx.commit().await.expect("source outcome commit");
        }
        let scheduler = scheduler(&fixture, &store);
        let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
        for _ in 0..8 {
            let progress = if full {
                scheduler
                    .advance_collection_from_run(
                        fixture.world.admin.tenant_id(),
                        fixture.run,
                        &owner,
                    )
                    .await
            } else {
                scheduler
                    .advance_collection(fixture.world.admin.tenant_id(), fixture.run, &owner)
                    .await
            }
            .expect("real fail-fast collection transition");
            if matches!(
                progress,
                CollectionAdvance::DrainFinished
                    | CollectionAdvance::Finished
                    | CollectionAdvance::AlreadyFinished
            ) {
                break;
            }
            assert_ne!(
                progress,
                CollectionAdvance::Waiting,
                "the sibling event suspension owns no worker"
            );
        }
        let outcomes:Vec<(String,Option<String>)>=sqlx::query_as("SELECT status,failure_code FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' ORDER BY instance_path_indices")
            .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_all(&fixture.world.db.pool).await.expect("failure/cancellation isolation");
        assert_eq!(
            outcomes,
            vec![
                ("CANCELLED".into(), None),
                ("FAILED".into(), Some("EXPRESSION_ERROR".into()))
            ]
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("failed final run");
        assert_eq!(status, "FAILED");
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("cold actual sibling-failure replay");
    }
}

#[tokio::test]
async fn the_independent_event_drain_requires_current_own_authority_and_run_revision() {
    use sagat_application::primitives::{
        CollectionStore, CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
    };
    let (fixture, store) = pending_wait("event-drain-current-authority").await;
    guards::start(&fixture, &store, false)
        .await
        .expect("real subscribed source");
    timeouts::operate(
        &fixture,
        &store,
        RunOperation::Cancel,
        "cancel-before-stale-drain",
    )
    .await;
    let drain = direct_drain(&fixture, &store).await;
    let native = sagat_infra_postgres::PgCollectionStore::with_definition_cache(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    for stale_authority in [true, false] {
        let mut refused = drain.clone();
        refused.transition = CollectionTransition::new(
            drain.transition.tenant_id(),
            drain.transition.namespace_id(),
            drain.transition.run_id(),
            if stale_authority {
                drain.transition.expected_run_version()
            } else {
                drain
                    .transition
                    .expected_run_version()
                    .checked_sub(1)
                    .expect("source has recorded history")
            },
            drain.transition.owner().clone(),
            SchedulerAuthority::new(
                if stale_authority {
                    "another-scheduler"
                } else {
                    drain.transition.scheduler().owner()
                },
                drain.transition.scheduler().lease(),
            ),
            sagat_domain::identity::Subject::engine(),
            None,
            crate::support::now(),
            None,
        );
        let before = guards::snapshot(&fixture).await;
        let error = native
            .drain_collection_iteration(&refused)
            .await
            .expect_err("stale authority or preparation cannot dispose a subscription");
        if stale_authority {
            assert!(
                matches!(error, PrimitiveStoreError::SchedulerFenceLost),
                "{error:?}"
            );
        } else {
            assert!(
                matches!(error, PrimitiveStoreError::RevisionConflict { .. }),
                "{error:?}"
            );
        }
        assert_eq!(guards::snapshot(&fixture).await, before);
    }
    native
        .drain_collection_iteration(&drain)
        .await
        .expect("current exact source still drains");
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("current-authority native replay");
}
