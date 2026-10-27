//! Scheduler-owned WaitEvent entry; internal immutable definitions, no public activation.
use super::*;
use sagat_scheduler::CollectionAdvance;

fn document(name: &str) -> serde_json::Value {
    let mut document = wait_document(name, true, 60000);
    document["bodies"]["line"]["entrypoint"] = json!("b");
    document["bodies"]["line"]["nodes"]
        .as_array_mut()
        .expect("body nodes")
        .retain(|node| node["id"] != "a");
    document["nodes"][0]["next"] = json!("done");
    document["nodes"]
        .as_array_mut()
        .expect("root nodes")
        .retain(|node| node["id"] != "a");
    document
}

async fn fixture(name: &str) -> (Fixture, PgRunStore) {
    let fixture = scoped_worker::fixture_with_unadmitted_document(name, document(name)).await;
    let runtime = fixture.world.db.runtime_pool().await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("actual execution role");
    assert_eq!(role, "sagat_app");
    let store = PgRunStore::new(
        runtime,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    (fixture, store)
}

async fn admission_and_delivery(full: bool) {
    let (fixture, store) = fixture(&format!("event-entry-admission-{full}")).await;
    assert_eq!(
        scheduler::advance(&fixture, &store, full).await,
        CollectionAdvance::Entered
    );
    assert_eq!(
        scheduler::advance(&fixture, &store, full).await,
        CollectionAdvance::Admitted { index: 0 },
        "WaitEvent entry is a scheduler-owned pending member"
    );
    let pending:(String,String,bool,bool,i64)=sqlx::query_as("SELECT s.status,i.entry_dispatch,i.activity IS NULL AND i.outbox_id IS NULL,s.activity IS NULL AND s.ready_at IS NULL AND s.waiting_kind IS NULL AND s.due_at IS NULL AND s.event_type IS NULL AND s.correlation_key IS NULL AND s.event_since IS NULL AND s.lease_token_hash IS NULL,(SELECT count(*) FROM event_outbox WHERE tenant_id=s.tenant_id AND aggregate_id=s.run_id AND event_type='task.ready') FROM steps s JOIN collection_iterations i ON i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.entry_instance_id=s.step_id WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("pending admission has no worker or clock binding");
    assert_eq!(
        pending,
        ("PENDING".into(), "SCHEDULER".into(), true, true, 0)
    );
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold admitted event entry replay");
    let bounded = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            wait(0).path().clone(),
        )
        .await
        .expect("bounded cold entry reader")
        .expect("path");
    assert_eq!(
        bounded
            .state
            .local_instance(wait(0).node())
            .expect("admitted event")
            .status(),
        StepStatus::Pending
    );
    for _ in 0..8 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND waiting_kind='EVENT'",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("recorded subscriptions");
        if waiting == 2 {
            break;
        }
        scheduler::advance(&fixture, &store, full).await;
    }
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND waiting_kind='EVENT'",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("both event entries subscribed");
    assert_eq!(waiting, 2);
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    receiver::publish_id(&fixture, "first", "event-entry-first").await;
    receiver::publish_id(&fixture, "second", "event-entry-second").await;
    let delivered = receiver::dispatcher(&store, 0, 8).tick().await;
    assert_eq!(
        (
            delivered.steps_woken,
            delivered.dispatched,
            delivered.retried
        ),
        (2, 2, 0)
    );
    let scheduler = cancellation::scheduler(&fixture, &store);
    for _ in 0..16 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("whole run status");
        if status == "SUCCEEDED" {
            break;
        }
    }
    let outcome:(String,i64,i64,i64)=sqlx::query_as("SELECT r.status,(SELECT count(*) FROM event_deliveries WHERE tenant_id=r.tenant_id AND run_id=r.id),(SELECT count(*) FROM event_outbox WHERE tenant_id=r.tenant_id AND aggregate_id=r.id AND event_type='task.ready'),(SELECT count(*) FROM collection_iterations WHERE tenant_id=r.tenant_id AND run_id=r.id AND slot_held) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("terminal worker-free event-entry collection");
    assert_eq!(outcome, ("SUCCEEDED".into(), 2, 0, 0));
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("whole event entry workflow replay");
}

#[tokio::test]
async fn frontier_admission_runs_event_entries_without_creating_worker_tasks() {
    admission_and_delivery(false).await;
}

#[tokio::test]
async fn full_oracle_admission_runs_event_entries_without_creating_worker_tasks() {
    admission_and_delivery(true).await;
}

async fn command(
    fixture: &Fixture,
    store: &PgRunStore,
    full: bool,
) -> Result<CollectionAdvance, sagat_scheduler::CollectionAdvanceError> {
    let scheduler = cancellation::scheduler(fixture, store);
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    if full {
        scheduler
            .advance_collection_from_run(fixture.world.admin.tenant_id(), fixture.run, &owner)
            .await
    } else {
        scheduler
            .advance_collection(fixture.world.admin.tenant_id(), fixture.run, &owner)
            .await
    }
}

async fn entered(name: &str, full: bool) -> (Fixture, PgRunStore) {
    let (fixture, store) = fixture(name).await;
    assert_eq!(
        scheduler::advance(&fixture, &store, full).await,
        CollectionAdvance::Entered
    );
    (fixture, store)
}

async fn admitted(name: &str, full: bool) -> (Fixture, PgRunStore) {
    let (fixture, store) = entered(name, full).await;
    assert_eq!(
        scheduler::advance(&fixture, &store, full).await,
        CollectionAdvance::Admitted { index: 0 }
    );
    (fixture, store)
}

#[tokio::test]
async fn pause_prevents_event_entry_admission_and_resume_preserves_scheduler_dispatch() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store) = entered(&format!("event-entry-pause-{full}"), full).await;
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-event-entry-admission",
        )
        .await;
        let before = guards::snapshot(&fixture).await;
        assert!(matches!(
            command(&fixture, &store, full).await,
            Err(sagat_scheduler::CollectionAdvanceError::Primitive(
                sagat_application::primitives::PrimitiveStoreError::InvalidTransition
            ))
        ));
        assert_eq!(guards::snapshot(&fixture).await, before);
        let report = cancellation::scheduler(&fixture, &store).tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(guards::snapshot(&fixture).await, before);
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Resume,
            "resume-before-event-entry-admission",
        )
        .await;
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::Admitted { index: 0 }
        );
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress
        );
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("paused admission and resumed clock replay");
    }
}

#[tokio::test]
async fn cancellation_before_an_event_entry_starts_retires_its_return_reservation() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store) = admitted(&format!("event-entry-cancel-pending-{full}"), full).await;
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Cancel,
            "cancel-pending-event-entry",
        )
        .await;
        let scheduler = cancellation::scheduler(&fixture, &store);
        for _ in 0..16 {
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            if !report.did_work() {
                break;
            }
        }
        let outcome:(String,i64,i32,i32,i64)=sqlx::query_as("SELECT r.status,(SELECT count(*) FROM run_events WHERE tenant_id=r.tenant_id AND run_id=r.id AND event_type='wait.started'),u.reserved_instances,u.retired_reservations,(SELECT count(*) FROM collection_iterations WHERE tenant_id=r.tenant_id AND run_id=r.id AND slot_held) FROM runs r JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2")
            .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("pending cancellation finality and reservations");
        assert_eq!(outcome, ("CANCELLED".into(), 0, 0, 1, 0));
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("pending event entry cancellation replay");
    }
}

#[tokio::test]
async fn zero_iteration_policy_blocks_new_event_entries_but_keeps_admitted_waits_progressing() {
    for full in [false, true] {
        let (fixture, store) = admitted(&format!("event-entry-zero-policy-{full}"), full).await;
        sqlx::query(
            "UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .execute(&fixture.world.db.pool)
        .await
        .expect("current explicit zero iteration policy");
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress,
            "admitted entry may subscribe under zero"
        );
        let before = guards::snapshot(&fixture).await;
        assert!(
            matches!(
                command(&fixture, &store, full).await,
                Err(sagat_scheduler::CollectionAdvanceError::Primitive(
                    sagat_application::primitives::PrimitiveStoreError::AtCapacity
                ))
            ),
            "another iteration may not enter"
        );
        let report = cancellation::scheduler(&fixture, &store).tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(guards::snapshot(&fixture).await, before);
        receiver::publish_id(&fixture, "first", "event-entry-zero-first").await;
        assert_eq!(
            receiver::dispatcher(&store, 0, 8).tick().await.steps_woken,
            1
        );
        for _ in 0..6 {
            match command(&fixture, &store, full).await {
                Ok(CollectionAdvance::Waiting)
                | Err(sagat_scheduler::CollectionAdvanceError::Primitive(
                    sagat_application::primitives::PrimitiveStoreError::AtCapacity,
                )) => break,
                Ok(_) => {}
                Err(error) => panic!("accepted member progress under zero: {error:?}"),
            }
        }
        let scope:(i32,i32,i32)=sqlx::query_as("SELECT next_admission_index,admitted_count,active_count FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("accepted member finishes while new admission waits");
        assert_eq!(scope, (1, 1, 0));
        sqlx::query(
            "UPDATE tenant_collection_policies SET max_active_iterations=2 WHERE tenant_id=$1",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .execute(&fixture.world.db.pool)
        .await
        .expect("restore owned fixture admission capacity");
        for _ in 0..6 {
            scheduler::advance(&fixture, &store, full).await;
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[1] AND waiting_kind='EVENT')").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("second entry current subscription");
            if waiting {
                break;
            }
        }
        receiver::publish_id(&fixture, "second", "event-entry-zero-second").await;
        assert_eq!(
            receiver::dispatcher(&store, 0, 8).tick().await.steps_woken,
            1
        );
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
                .bind(fixture.world.admin.tenant_id().as_uuid())
                .bind(fixture.run.as_uuid())
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("completed admitted waits after policy restore");
        assert_eq!(status, "SUCCEEDED");
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("zero policy accepted progress and restored admission replay");
    }
}

#[tokio::test]
async fn an_event_entry_admission_fault_rolls_back_membership_step_budget_and_captured_item() {
    for full in [false, true] {
        let (fixture, store) =
            entered(&format!("event-entry-admission-rollback-{full}"), full).await;
        sqlx::raw_sql("CREATE FUNCTION reject_event_entry_admission() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only event entry admission abort' USING ERRCODE='23514', CONSTRAINT='event_entry_admission_forced_abort'; END $$; CREATE TRIGGER reject_event_entry_admission BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='collection.iteration_admitted') EXECUTE FUNCTION reject_event_entry_admission()")
            .execute(&fixture.world.db.pool).await.expect("owned admission journal fault");
        let before = guards::snapshot(&fixture).await;
        let scheduler = cancellation::scheduler(&fixture, &store);
        let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
        let result = if full {
            scheduler
                .advance_collection_from_run(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        } else {
            scheduler
                .advance_collection(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        };
        let error = result.expect_err("the actual admission reaches its injected journal fault");
        assert!(
            error
                .to_string()
                .contains("test-only event entry admission abort"),
            "{error:?}"
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
        sqlx::raw_sql("DROP TRIGGER reject_event_entry_admission ON run_events; DROP FUNCTION reject_event_entry_admission()")
            .execute(&fixture.world.db.pool).await.expect("remove owned admission fault");
        assert_eq!(
            scheduler::advance(&fixture, &store, full).await,
            CollectionAdvance::Admitted { index: 0 }
        );
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("atomic rollback and exact admission retry replay");
    }
}

#[tokio::test]
async fn all_cold_readers_refuse_worker_dispatch_metadata_on_an_event_entry() {
    use sagat_application::scope_frontier::ScopeFrontierStore;
    let (fixture, store) = admitted("event-entry-wrong-dispatch", false).await;
    sqlx::query("UPDATE collection_iterations SET entry_dispatch='ACTIVITY',activity='scoped.impostor',outbox_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND iteration_index=0")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).bind(uuid::Uuid::new_v4()).execute(&fixture.world.db.pool).await.expect("CHECK-valid deliberate dispatch corruption");
    let before = guards::snapshot(&fixture).await;
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
        (true, true, true)
    );
    assert_eq!(guards::snapshot(&fixture).await, before);
    sqlx::query("UPDATE collection_iterations SET entry_dispatch='SCHEDULER',activity=NULL,outbox_id=NULL WHERE tenant_id=$1 AND run_id=$2 AND iteration_index=0")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).execute(&fixture.world.db.pool).await.expect("restore exact owned admission metadata");
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("restored correct scheduler entry replay");
}

#[tokio::test]
async fn a_cold_single_connection_scheduler_discovers_pending_event_entries_without_hints() {
    use sagat_application::maintenance::{MaintenanceStore, Window};
    let (fixture, original) = admitted("event-entry-cold-discovery", false).await;
    drop(original);
    let pool = fixture.world.db.runtime_pool_with(1).await;
    let cold = PgRunStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let candidates = cold
        .collection_candidates(&Window::everything(8))
        .await
        .expect("cold durable collection candidate");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.run_id == fixture.run
                && candidate.instance()
                    == ExecutionInstanceId::root(NodeId::parse("items").expect("owner")))
    );
    let scheduler = cancellation::scheduler(&fixture, &cold);
    crate::support::bounded::within("cold scheduler starts event entries using one connection and no worker hint", std::time::Duration::from_secs(30), async {
        for _ in 0..8 {
            let report=scheduler.tick().await;
            assert_eq!(report.failed,0,"{report:?}");
            let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND waiting_kind='EVENT'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("cold recorded subscriptions");
            if waiting==2 {break;}
        }
    }).await;
    let evidence:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND waiting_kind='EVENT'),(SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.started'),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready')")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("cold subscriptions and zero worker notifications");
    assert_eq!(evidence, (2, 2, 0));
    cold.verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("cold discovery and subscription replay");
    pool.close().await;
}
