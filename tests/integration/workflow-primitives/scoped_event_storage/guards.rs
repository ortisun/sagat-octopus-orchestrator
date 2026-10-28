//! Current acknowledgement, preparation clock, physical readers and atomic rollback.
use super::*;
use sagat_application::scope_frontier::ScopeFrontierStore;

#[tokio::test]
async fn all_readers_refuse_a_same_tenant_wrong_namespace_or_invented_step_uuid() {
    let (fixture, store) = pending_wait("event-reader-identity").await;
    start(&fixture, &store, false)
        .await
        .expect("actual waiting source");
    let another = fixture.world.add_namespace("another-event-scope").await;
    let original:uuid::Uuid=sqlx::query_scalar("SELECT step_id FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual UUID");
    let mut accepted = Vec::new();
    for wrong_namespace in [false, true] {
        let statement = if wrong_namespace {
            "UPDATE steps SET namespace_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]"
        } else {
            "UPDATE steps SET step_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]"
        };
        let bad = if wrong_namespace {
            another.id().as_uuid()
        } else {
            uuid::Uuid::new_v4()
        };
        sqlx::query(statement)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .bind(bad)
            .execute(&fixture.world.db.pool)
            .await
            .expect("CHECK-valid owner identity corruption");
        let before = snapshot(&fixture).await;
        let full = store
            .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
            .await;
        let full_refused = full.is_err();
        drop(full);
        let path_refused = store
            .load_path(
                fixture.world.admin.tenant_id(),
                fixture.run,
                wait(0).path().clone(),
            )
            .await
            .is_err();
        let frontier_refused = store
            .collection_frontier(
                fixture.world.admin.tenant_id(),
                fixture.run,
                &ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
            )
            .await
            .is_err();
        if (full_refused, path_refused, frontier_refused) != (true, true, true) {
            accepted.push((
                wrong_namespace,
                full_refused,
                path_refused,
                frontier_refused,
            ));
        }
        assert_eq!(snapshot(&fixture).await, before);
        let restored = if wrong_namespace {
            fixture.world.namespace.id().as_uuid()
        } else {
            original
        };
        sqlx::query(statement)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .bind(restored)
            .execute(&fixture.world.db.pool)
            .await
            .expect("restore exact original identity");
    }
    assert!(
        accepted.is_empty(),
        "readers accepted changed physical identities: {accepted:?}"
    );
}

pub(super) async fn snapshot(fixture: &Fixture) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'run',to_jsonb(r), \
          'steps',(SELECT jsonb_agg(to_jsonb(s) ORDER BY step_id) FROM steps s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id), \
          'journal',(SELECT jsonb_agg(to_jsonb(e) ORDER BY seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
          'scopes',(SELECT jsonb_agg(to_jsonb(s) ORDER BY scope_id) FROM iteration_scopes s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id), \
          'members',(SELECT jsonb_agg(to_jsonb(i) ORDER BY scope_id,iteration_index) FROM collection_iterations i WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id), \
          'usage',(SELECT to_jsonb(u) FROM run_instance_usage u WHERE u.tenant_id=r.tenant_id AND u.run_id=r.id), \
          'tenantIterations',(SELECT to_jsonb(u) FROM tenant_iteration_usage u WHERE u.tenant_id=r.tenant_id), \
          'retained',(SELECT to_jsonb(u) FROM tenant_retained_usage u WHERE u.tenant_id=r.tenant_id), \
          'payloads',(SELECT jsonb_agg(to_jsonb(p) ORDER BY digest) FROM payloads p WHERE p.tenant_id=r.tenant_id), \
          'audit',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM audit_records a WHERE a.tenant_id=r.tenant_id), \
          'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id), \
          'telemetry',(SELECT to_jsonb(t) FROM telemetry_pending t WHERE t.tenant_id=r.tenant_id AND t.run_id=r.id)) \
         FROM runs r WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("independent native transaction snapshot")
}

pub(super) async fn start(
    fixture: &Fixture,
    store: &PgRunStore,
    full: bool,
) -> Result<(), sagat_application::error::StoreError> {
    let member = wait(0);
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    if full {
        let mut tx = store
            .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
            .await?
            .expect("run");
        assert!(tx.holds_shard("scoped-worker-test", lease).await?);
        let now = tx.now();
        let progress = {
            let (definition, state) = tx.parts();
            execution::start_event_wait_instance(
                definition,
                state,
                &member,
                Ok(execution::CorrelationKey::parse("first").expect("key")),
                now,
            )
            .expect("pure full start")
        };
        tx.persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("scheduler"), None),
        )
        .await?;
        tx.commit().await
    } else {
        let prepared = store
            .load_path(
                fixture.world.admin.tenant_id(),
                fixture.run,
                member.path().clone(),
            )
            .await?
            .expect("path");
        let mut tx = store.begin_path(prepared, Lock::Wait).await?.expect("run");
        assert!(tx.holds_shard("scoped-worker-test", lease).await?);
        let now = tx.now();
        let progress = {
            let prepared = tx.snapshot();
            execution::start_event_wait_path_instance(
                &prepared.definition,
                &mut prepared.state,
                &member,
                Ok(execution::CorrelationKey::parse("first").expect("key")),
                now,
            )
            .expect("pure bounded start")
        };
        tx.persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("scheduler"), None),
        )
        .await?;
        tx.commit().await
    }
}

#[tokio::test]
async fn missing_and_later_refused_own_acknowledgements_refuse_both_start_shapes_in_both_writers() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-own-ack-{full}")).await;
        let member = wait(0);
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("own shard");
        for failed in [false, true] {
            for refused_after_success in [false, true] {
                let before = snapshot(&fixture).await;
                let key = if failed {
                    Err(())
                } else {
                    Ok(execution::CorrelationKey::parse("first").expect("key"))
                };
                let rejected = if full {
                    let mut tx = store
                        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                        .await
                        .expect("full entry")
                        .expect("run");
                    if refused_after_success {
                        assert!(
                            tx.holds_shard("scoped-worker-test", lease)
                                .await
                                .expect("first own hold")
                        );
                        assert!(
                            !tx.holds_shard("another-owner", lease)
                                .await
                                .expect("refused later own hold")
                        );
                    } else {
                        assert!(
                            !tx.holds_parent_shard("scoped-worker-test", lease)
                                .await
                                .expect("existing run has no parent creation authority")
                        );
                    }
                    let now = tx.now();
                    let progress = {
                        let (definition, state) = tx.parts();
                        execution::start_event_wait_instance(definition, state, &member, key, now)
                            .expect("full start decision")
                    };
                    let result = tx
                        .persist_addressed(
                            &progress.events,
                            &[],
                            Effects::by(Actor::System("scheduler"), None),
                        )
                        .await;
                    drop(tx);
                    result.is_err()
                } else {
                    let prepared = store
                        .load_path(
                            fixture.world.admin.tenant_id(),
                            fixture.run,
                            member.path().clone(),
                        )
                        .await
                        .expect("prepare")
                        .expect("path");
                    let mut tx = store
                        .begin_path(prepared, Lock::Wait)
                        .await
                        .expect("entry")
                        .expect("path");
                    if refused_after_success {
                        assert!(
                            tx.holds_shard("scoped-worker-test", lease)
                                .await
                                .expect("first own hold")
                        );
                        assert!(
                            !tx.holds_shard("another-owner", lease)
                                .await
                                .expect("refused later own hold")
                        );
                    }
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
                        .expect("bounded start decision")
                    };
                    let result = tx
                        .persist_addressed(
                            &progress.events,
                            &[],
                            Effects::by(Actor::System("scheduler"), None),
                        )
                        .await;
                    drop(tx);
                    result.is_err()
                };
                assert!(
                    rejected,
                    "full={full} failed={failed} refused_after_success={refused_after_success}"
                );
                assert_eq!(snapshot(&fixture).await, before);
            }
        }
    }
}

#[tokio::test]
async fn a_start_prepared_before_the_own_guard_cannot_reuse_its_earlier_clock() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-old-start-clock-{full}")).await;
        let member = wait(0);
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("own shard");
        let before = snapshot(&fixture).await;
        let key = Ok(execution::CorrelationKey::parse("first").expect("key"));
        let rejected = if full {
            let mut tx = store
                .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                .await
                .expect("full entry")
                .expect("run");
            let old = tx.now();
            let progress = {
                let (definition, state) = tx.parts();
                execution::start_event_wait_instance(definition, state, &member, key, old)
                    .expect("earlier full decision")
            };
            assert!(
                tx.holds_shard("scoped-worker-test", lease)
                    .await
                    .expect("later guard")
            );
            assert!(tx.now() > old);
            let result = tx
                .persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(Actor::System("scheduler"), None),
                )
                .await;
            drop(tx);
            result.is_err()
        } else {
            let prepared = store
                .load_path(
                    fixture.world.admin.tenant_id(),
                    fixture.run,
                    member.path().clone(),
                )
                .await
                .expect("prepare")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("path");
            let old = tx.now();
            let progress = {
                let prepared = tx.snapshot();
                execution::start_event_wait_path_instance(
                    &prepared.definition,
                    &mut prepared.state,
                    &member,
                    key,
                    old,
                )
                .expect("earlier bounded decision")
            };
            assert!(
                tx.holds_shard("scoped-worker-test", lease)
                    .await
                    .expect("later guard")
            );
            assert!(tx.now() > old);
            let result = tx
                .persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(Actor::System("scheduler"), None),
                )
                .await;
            drop(tx);
            result.is_err()
        };
        assert!(rejected, "full={full}");
        assert_eq!(snapshot(&fixture).await, before);
    }
}

#[tokio::test]
async fn all_readers_reject_orphan_metadata_on_a_real_wait() {
    let (fixture, store) = pending_wait("event-orphan-readers").await;
    start(&fixture, &store, false)
        .await
        .expect("real waiting source");
    // Closed owner-fixture vocabulary, restored exactly after each independent corrupt field.
    let mutations = [
        (
            "UPDATE steps SET reported_lease_token_hash=decode(repeat('07',32),'hex') WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET reported_lease_token_hash=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET event_type='another.event' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET event_type='payment.confirmed' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET event_since=event_since+interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET event_since=event_since-interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET attempt=1 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET attempt=0 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
        (
            "UPDATE steps SET condition=true WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            "UPDATE steps SET condition=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
        ),
    ];
    for (mutate, restore) in mutations {
        sqlx::query(mutate)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .execute(&fixture.world.db.pool)
            .await
            .expect("explicit owned corruption");
        let before = snapshot(&fixture).await;
        let full = store
            .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
            .await;
        let full_refused = full.is_err();
        drop(full);
        let path_refused = store
            .load_path(
                fixture.world.admin.tenant_id(),
                fixture.run,
                wait(0).path().clone(),
            )
            .await
            .is_err();
        let frontier_refused = store
            .collection_frontier(
                fixture.world.admin.tenant_id(),
                fixture.run,
                &ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
            )
            .await
            .is_err();
        assert_eq!(
            (full_refused, path_refused, frontier_refused),
            (true, true, true),
            "{mutate}"
        );
        assert_eq!(snapshot(&fixture).await, before);
        sqlx::query(restore)
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .execute(&fixture.world.db.pool)
            .await
            .expect("restore own single field");
    }
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("unchanged replay after restore");
}

#[tokio::test]
async fn a_journal_abort_rolls_back_start_in_both_native_writers() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-start-rollback-{full}")).await;
        sqlx::raw_sql("CREATE FUNCTION reject_scoped_event_start() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only scoped event start abort' USING ERRCODE='23514'; END $$; CREATE TRIGGER reject_scoped_event_start BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='wait.started') EXECUTE FUNCTION reject_scoped_event_start()")
            .execute(&fixture.world.db.pool).await.expect("owned journal fault");
        let before = snapshot(&fixture).await;
        let error = start(&fixture, &store, full)
            .await
            .expect_err("native start reaches injected journal failure");
        assert!(
            error
                .to_string()
                .contains("test-only scoped event start abort"),
            "{error}"
        );
        assert_eq!(snapshot(&fixture).await, before);
        sqlx::raw_sql("DROP TRIGGER reject_scoped_event_start ON run_events; DROP FUNCTION reject_scoped_event_start()")
            .execute(&fixture.world.db.pool).await.expect("remove own journal fault");
        start(&fixture, &store, full)
            .await
            .expect("unchanged start can commit after fault removal");
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("cold replay");
    }
}

#[tokio::test]
async fn prepared_starts_refuse_physical_changes_without_a_run_revision() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-start-source-cas-{full}")).await;
        let member = wait(0);
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("own shard");
        // Fixed SQL vocabulary; each owner mutation is restored after the writer refuses it.
        let changes = [
            (
                "UPDATE steps SET correlation_key='forged' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET correlation_key=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET event_type='forged.event' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET event_type=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET event_since=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET event_since=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET routed=true WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET routed=false WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET node_kind='activity' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET node_kind='waitEvent' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET attempt=1 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET attempt=0 WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
        ];
        for (mutate, restore) in changes {
            let prepared = if full {
                None
            } else {
                store
                    .load_path(
                        fixture.world.admin.tenant_id(),
                        fixture.run,
                        member.path().clone(),
                    )
                    .await
                    .expect("bounded source")
            };
            let mut full_tx = if full {
                store
                    .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                    .await
                    .expect("full source")
            } else {
                None
            };
            let version: i64 =
                sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                    .bind(fixture.world.admin.tenant_id().as_uuid())
                    .bind(fixture.run.as_uuid())
                    .fetch_one(&fixture.world.db.pool)
                    .await
                    .expect("source revision");
            sqlx::query(mutate)
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .execute(&fixture.world.db.pool)
                .await
                .expect("CHECK-valid source change");
            let after_version: i64 =
                sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                    .bind(fixture.world.admin.tenant_id().as_uuid())
                    .bind(fixture.run.as_uuid())
                    .fetch_one(&fixture.world.db.pool)
                    .await
                    .expect("unchanged revision");
            assert_eq!(version, after_version);
            let before = snapshot(&fixture).await;
            let key = Ok(execution::CorrelationKey::parse("first").expect("key"));
            let rejected = if let Some(mut tx) = full_tx.take() {
                assert!(
                    tx.holds_shard("scoped-worker-test", lease)
                        .await
                        .expect("own guard")
                );
                let now = tx.now();
                let progress = {
                    let (definition, state) = tx.parts();
                    execution::start_event_wait_instance(definition, state, &member, key, now)
                        .expect("prepared full decision")
                };
                let result = tx
                    .persist_addressed(
                        &progress.events,
                        &[],
                        Effects::by(Actor::System("scheduler"), None),
                    )
                    .await;
                drop(tx);
                result.is_err()
            } else {
                let mut tx = store
                    .begin_path(prepared.expect("bounded source"), Lock::Wait)
                    .await
                    .expect("same revision entry")
                    .expect("run");
                assert!(
                    tx.holds_shard("scoped-worker-test", lease)
                        .await
                        .expect("own guard")
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
                    .expect("prepared bounded decision")
                };
                let result = tx
                    .persist_addressed(
                        &progress.events,
                        &[],
                        Effects::by(Actor::System("scheduler"), None),
                    )
                    .await;
                drop(tx);
                result.is_err()
            };
            assert!(rejected, "full={full}: {mutate}");
            assert_eq!(snapshot(&fixture).await, before);
            sqlx::query(restore)
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .execute(&fixture.world.db.pool)
                .await
                .expect("restore owned source field");
        }
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("restored exact source replays");
    }
}
