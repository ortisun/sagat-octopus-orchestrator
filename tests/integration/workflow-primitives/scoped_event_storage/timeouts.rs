//! Clocked native timeout closure, not an external event receiver or public activation.
use super::*;

pub(super) async fn waiting(name: &str, handled: bool) -> (Fixture, PgRunStore) {
    let (fixture, store) = pending_wait_with_timeout(name, handled, 1).await;
    guards::start(&fixture, &store, false)
        .await
        .expect("actual wait start");
    crate::support::eventually("recorded event deadline is due on PostgreSQL clock",|| async {
        sqlx::query_scalar::<_,bool>("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
            .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("recorded deadline clock barrier").then_some(())
    }).await;
    (fixture, store)
}

async fn expire(
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
            execution::time_out_event_wait_instance(definition, state, &member, now)
                .expect("pure full due timeout")
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
            execution::time_out_event_wait_path_instance(
                &prepared.definition,
                &mut prepared.state,
                &member,
                now,
            )
            .expect("pure bounded due timeout")
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

async fn check_timeout(full: bool, handled: bool) {
    let (fixture, store) = waiting(&format!("event-timeout-{full}-{handled}"), handled).await;
    // A scanner marker is only an optimization, and must disappear with the subscription.
    sqlx::query("UPDATE steps SET event_scanned_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).execute(&fixture.world.db.pool).await.expect("owner-set scan optimization");
    expire(&fixture, &store, full)
        .await
        .expect("native timeout storage");
    let actual:(String,bool,bool,Option<String>,bool)=sqlx::query_as("SELECT status,routed,timed_out,failure_code,waiting_kind IS NULL AND due_at IS NULL AND event_type IS NULL AND correlation_key IS NULL AND event_since IS NULL AND event_scanned_at IS NULL AND activity IS NULL AND output_digest IS NULL AND lease_token_hash IS NULL AND reported_lease_token_hash IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("independent closed binding");
    assert_eq!(
        actual,
        (
            if handled { "SUCCEEDED" } else { "FAILED" }.into(),
            true,
            handled,
            if handled {
                None
            } else {
                Some("WAIT_TIMED_OUT".into())
            },
            true
        )
    );
    let sibling:(String,Option<String>)=sqlx::query_as("SELECT status,correlation_key FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[1]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("untouched same-name sibling");
    assert_eq!(sibling, ("PENDING".into(), None));
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    let cold = PgRunStore::new(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold timeout replay with structured test codec");
}

#[tokio::test]
async fn an_unhandled_native_timeout_uses_the_recorded_deadline_in_the_bounded_writer() {
    check_timeout(false, false).await;
}

#[tokio::test]
async fn a_handled_native_timeout_clears_the_binding_in_the_full_writer() {
    check_timeout(true, true).await;
}

#[tokio::test]
async fn a_handled_native_timeout_clears_the_binding_in_the_bounded_writer() {
    check_timeout(false, true).await;
}

#[tokio::test]
async fn an_unhandled_native_timeout_uses_the_recorded_deadline_in_the_full_writer() {
    check_timeout(true, false).await;
}

#[tokio::test]
async fn an_unhandled_timeout_keeps_its_local_error_edge_in_both_writers() {
    for full in [false, true] {
        let name = format!("event-timeout-on-error-{full}");
        let mut document = wait_document(&name, false, 1);
        document["bodies"]["line"]["nodes"][1]["onError"] = json!("returned");
        let (fixture, store) = pending_wait_document(&name, document).await;
        guards::start(&fixture, &store, false)
            .await
            .expect("actual start");
        due(&fixture).await;
        expire(&fixture, &store, full)
            .await
            .expect("native unhandled timeout with local recovery");
        let row:(String,bool,String,i64)=sqlx::query_as("SELECT status,routed,failure_code,(SELECT count(*) FROM steps target WHERE target.tenant_id=s.tenant_id AND target.run_id=s.run_id AND target.node='returned' AND target.instance_path_indices=ARRAY[0]) FROM steps s WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
            .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("native local error route");
        assert_eq!(row, ("FAILED".into(), true, "WAIT_TIMED_OUT".into(), 1));
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("local error route replay");
    }
}

async fn due(fixture: &Fixture) {
    crate::support::eventually("PostgreSQL observes the recorded event deadline",|| async {
        sqlx::query_scalar::<_,bool>("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
            .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("due clock barrier").then_some(())
    }).await;
}

#[tokio::test]
async fn native_timeouts_refuse_missing_authority_early_clocks_and_the_wrong_pinned_outcome() {
    for due_source in [false, true] {
        let (fixture, store) = if due_source {
            waiting("event-timeout-reject-due", true).await
        } else {
            let (fixture, store) = pending_wait("event-timeout-reject-early").await;
            guards::start(&fixture, &store, false)
                .await
                .expect("future wait");
            (fixture, store)
        };
        let member = wait(0);
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("own shard");
        for full in [false, true] {
            for own in [false, true] {
                let before = guards::snapshot(&fixture).await;
                let event = execution::AddressedRunEvent::in_path(
                    member.path().clone(),
                    execution::RunEvent::WaitTimedOut {
                        node: member.node().clone(),
                        // A due write without authority otherwise has a valid pinned outcome;
                        // the authorized due case isolates the wrong-outcome refusal instead.
                        handled: !due_source || !own,
                    },
                )
                .expect("adversarial timeout outcome");
                let rejected = if full {
                    let mut tx = store
                        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                        .await
                        .expect("full entry")
                        .expect("run");
                    if own {
                        assert!(
                            tx.holds_shard("scoped-worker-test", lease)
                                .await
                                .expect("current own authority")
                        );
                    }
                    tx.parts()
                        .1
                        .apply_addressed(&event)
                        .expect("generic reducer accepts supplied outcome");
                    let result = tx
                        .persist_addressed(
                            &[event],
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
                        .expect("snapshot")
                        .expect("path");
                    let mut tx = store
                        .begin_path(prepared, Lock::Wait)
                        .await
                        .expect("entry")
                        .expect("run");
                    if own {
                        assert!(
                            tx.holds_shard("scoped-worker-test", lease)
                                .await
                                .expect("current own authority")
                        );
                    }
                    tx.snapshot()
                        .state
                        .apply_addressed(&event)
                        .expect("generic reducer accepts supplied outcome");
                    let result = tx
                        .persist_addressed(
                            &[event],
                            &[],
                            Effects::by(Actor::System("scheduler"), None),
                        )
                        .await;
                    drop(tx);
                    result.is_err()
                };
                assert!(rejected, "due={due_source} full={full} own={own}");
                assert_eq!(guards::snapshot(&fixture).await, before);
            }
        }
    }
}

pub(super) async fn operate(
    fixture: &Fixture,
    store: &PgRunStore,
    operation: sagat_application::runs::RunOperation,
    key: &str,
) {
    let cache = DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8);
    sagat_application::runs::RunService::new(
        store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(JsonEvaluator),
        cache,
    )
    .operate(
        &fixture.world.admin,
        fixture.run,
        operation,
        "native event wait lifecycle proof",
        None,
        key,
        &crate::support::engine::ctx(),
    )
    .await
    .expect("real audited run operation");
}

#[tokio::test]
async fn paused_timeouts_persist_outcomes_and_cold_explicit_resume_routes_once() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        for handled in [false, true] {
            let (fixture, store) =
                waiting(&format!("event-timeout-pause-{full}-{handled}"), handled).await;
            operate(&fixture, &store, RunOperation::Pause, "pause-event-wait").await;
            expire(&fixture, &store, full)
                .await
                .expect("due timeout while paused");
            let row:(String,bool,bool)=sqlx::query_as("SELECT r.status,s.routed,s.waiting_kind IS NULL AND s.due_at IS NULL AND s.event_type IS NULL AND s.correlation_key IS NULL AND s.event_since IS NULL AND s.event_scanned_at IS NULL FROM runs r JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]")
                .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("deferred paused route and closed subscription");
            assert_eq!(row, ("PAUSED".into(), false, true));
            assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
            let cold = PgRunStore::new(
                fixture.world.db.runtime_pool().await,
                DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
            )
            .with_scoped_worker_protocol();
            cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
                .await
                .expect("paused cold replay");
            operate(&fixture, &cold, RunOperation::Resume, "resume-event-wait").await;
            let member = wait(0);
            let prepared = cold
                .load_path(
                    fixture.world.admin.tenant_id(),
                    fixture.run,
                    member.path().clone(),
                )
                .await
                .expect("cold read")
                .expect("path");
            assert!(
                !prepared
                    .state
                    .local_instance(member.node())
                    .expect("wait")
                    .routed()
            );
            let mut tx = cold
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("cold entry")
                .expect("run");
            let lease = fixture
                .placement
                .snapshot()
                .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
                .expect("own shard");
            assert!(
                tx.holds_shard("scoped-worker-test", lease)
                    .await
                    .expect("own guard")
            );
            let events = {
                let prepared = tx.snapshot();
                execution::release_pending_path(&prepared.definition, &mut prepared.state)
                    .expect("explicit cold deferred route")
                    .events
            };
            tx.persist_addressed(&events, &[], Effects::by(Actor::System("scheduler"), None))
                .await
                .expect("cold route storage");
            tx.commit().await.expect("cold route commit");
            let counts:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.timed_out'),count(*) FILTER(WHERE event_type='step.routed' AND node='b' AND instance_path_indices=ARRAY[0]) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
                .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one timeout and one qualified route");
            assert_eq!(counts, (1, 1));
            cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
                .await
                .expect("resumed replay");
        }
    }
}

#[tokio::test]
async fn a_late_projection_fault_rolls_back_timeout_and_its_preceding_journal_in_both_writers() {
    for full in [false, true] {
        let (fixture, store) = waiting(&format!("event-timeout-late-abort-{full}"), true).await;
        sqlx::raw_sql("CREATE FUNCTION reject_event_timeout_projection() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only event timeout projection abort' USING ERRCODE='23514'; END $$; CREATE TRIGGER reject_event_timeout_projection BEFORE UPDATE ON steps FOR EACH ROW WHEN (OLD.waiting_kind='EVENT' AND NEW.status='SUCCEEDED' AND NEW.timed_out) EXECUTE FUNCTION reject_event_timeout_projection()")
            .execute(&fixture.world.db.pool).await.expect("owned late timeout fault");
        let before = guards::snapshot(&fixture).await;
        let error = expire(&fixture, &store, full)
            .await
            .expect_err("timeout reaches projection after writing its journal");
        assert!(
            error
                .to_string()
                .contains("test-only event timeout projection abort"),
            "{error}"
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
        sqlx::raw_sql("DROP TRIGGER reject_event_timeout_projection ON steps; DROP FUNCTION reject_event_timeout_projection()")
            .execute(&fixture.world.db.pool).await.expect("remove own late fault");
        expire(&fixture, &store, full)
            .await
            .expect("same timeout can commit after fault removal");
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("replay after rollback/retry");
    }
}

#[tokio::test]
async fn prepared_timeouts_refuse_changed_keys_and_windows_without_a_run_revision() {
    for full in [false, true] {
        let (fixture, store) = waiting(&format!("event-timeout-binding-cas-{full}"), true).await;
        let member = wait(0);
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
            .expect("own shard");
        let changes = [
            (
                "UPDATE steps SET correlation_key='another-key' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET correlation_key='first' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            (
                "UPDATE steps SET event_type='another.event' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET event_type='payment.confirmed' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
            ),
            // Shift both columns: the new window remains internally valid, still due, and differs
            // only from the exact binding that was prepared before the owner fixture changed it.
            (
                "UPDATE steps SET due_at=due_at-interval '1 microsecond',event_since=event_since-interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
                "UPDATE steps SET due_at=due_at+interval '1 microsecond',event_since=event_since+interval '1 microsecond' WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]",
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
                    .expect("prepared binding")
            };
            let mut full_tx = if full {
                store
                    .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
                    .await
                    .expect("full prepared binding")
            } else {
                None
            };
            let revision: i64 =
                sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                    .bind(fixture.world.admin.tenant_id().as_uuid())
                    .bind(fixture.run.as_uuid())
                    .fetch_one(&fixture.world.db.pool)
                    .await
                    .expect("prepared revision");
            sqlx::query(mutate)
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .execute(&fixture.world.db.pool)
                .await
                .expect("owner changes exact binding");
            let current: i64 =
                sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                    .bind(fixture.world.admin.tenant_id().as_uuid())
                    .bind(fixture.run.as_uuid())
                    .fetch_one(&fixture.world.db.pool)
                    .await
                    .expect("same revision");
            assert_eq!(revision, current);
            let before = guards::snapshot(&fixture).await;
            let rejected = if let Some(mut tx) = full_tx.take() {
                assert!(
                    tx.holds_shard("scoped-worker-test", lease)
                        .await
                        .expect("own authority")
                );
                let now = tx.now();
                let progress = {
                    let (definition, state) = tx.parts();
                    execution::time_out_event_wait_instance(definition, state, &member, now)
                        .expect("prepared full due decision")
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
                        .expect("own authority")
                );
                let now = tx.now();
                let progress = {
                    let prepared = tx.snapshot();
                    execution::time_out_event_wait_path_instance(
                        &prepared.definition,
                        &mut prepared.state,
                        &member,
                        now,
                    )
                    .expect("prepared bounded due decision")
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
            assert_eq!(guards::snapshot(&fixture).await, before);
            sqlx::query(restore)
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .execute(&fixture.world.db.pool)
                .await
                .expect("restore owner binding");
        }
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("restored binding equals original journal");
    }
}
