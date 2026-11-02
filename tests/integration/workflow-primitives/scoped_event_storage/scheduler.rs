//! Actual collection scheduling after worker admission; public wait bodies remain closed.
use super::*;
use sagat_scheduler::CollectionAdvance;

pub(super) async fn advance(
    fixture: &Fixture,
    store: &PgRunStore,
    full: bool,
) -> CollectionAdvance {
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
    .expect("actual collection scheduling")
}

#[tokio::test]
async fn a_failed_wait_recorded_while_paused_keeps_its_local_error_route_before_fail_fast() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let name = format!("scheduler-wait-paused-error-{full}");
        let mut document = wait_document(&name, false, 1);
        document["bodies"]["line"]["nodes"][1]["onError"] = json!("returned");
        let (fixture, store) = pending_wait_document(&name, document).await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress
        );
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-unhandled-timeout",
        )
        .await;
        crate::support::bounded::within("actual scheduled event is due", std::time::Duration::from_secs(30), async {
            loop {
                let due:bool=sqlx::query_scalar("SELECT due_at<=clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("current database deadline");
                if due { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        timeouts::expire(&fixture, &store, full)
            .await
            .expect("actual timeout while paused");
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::Waiting,
            "a paused local error route must not become a collection failure"
        );
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Resume,
            "resume-before-local-error-route",
        )
        .await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress,
            "the declared error route must run before collection fail-fast"
        );
        let state:(String,bool,String,i64)=sqlx::query_as("SELECT s.status,s.routed,r.status,(SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.event_type='collection.drain_started') FROM steps s JOIN runs r ON r.tenant_id=s.tenant_id AND r.id=s.run_id WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("handled local failure, live collection and no drain");
        assert_eq!(state, ("FAILED".into(), true, "RUNNING".into(), 0));
        let returned:String=sqlx::query_scalar("SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='returned' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("declared local error successor");
        assert_eq!(returned, "PENDING");
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("deferred failure and route replay");
    }
}

#[tokio::test]
async fn the_collection_scheduler_starts_each_exact_event_subscription_once_in_both_modes() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("scheduler-event-start-{full}")).await;
        let scheduler = cancellation::scheduler(&fixture, &store);
        let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
        let hints:i64=sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual prior worker notifications");
        for index in 0..2 {
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
            .expect("real collection scheduling");
            assert_eq!(
                progress,
                CollectionAdvance::BodyProgress,
                "the pending wait must subscribe through the scheduler: full={full}, index={index}"
            );
            let row:(String,String,bool)=sqlx::query_as("SELECT status,correlation_key,due_at IS NOT NULL AND event_type='payment.confirmed' AND event_since=due_at-interval '61 seconds' AND waiting_kind='EVENT' AND activity IS NULL AND output_digest IS NULL AND lease_token_hash IS NULL AND reported_lease_token_hash IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_nodes=ARRAY['items']::text[] AND instance_path_indices=ARRAY[$3]::int[]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).bind(index).fetch_one(&fixture.world.db.pool).await.expect("exact computed subscription and pinned window");
            assert_eq!(
                row,
                (
                    "WAITING".into(),
                    if index == 0 { "first" } else { "second" }.into(),
                    true
                )
            );
            store
                .verify_run(fixture.world.admin.tenant_id(), fixture.run)
                .await
                .expect("actual scheduler journal replay");
        }
        let progress = if full {
            scheduler
                .advance_collection_from_run(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        } else {
            scheduler
                .advance_collection(fixture.world.admin.tenant_id(), fixture.run, &owner)
                .await
        }
        .expect("waiting repeat");
        assert_eq!(progress, CollectionAdvance::Waiting);
        let counts:(i64,i64)=sqlx::query_as("SELECT count(*),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready') FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.started'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one subscription per exact member and no new worker notifications");
        assert_eq!(counts, (2, hints));
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    }
}

#[tokio::test]
async fn a_placement_free_scheduler_refuses_scoped_event_clocks_with_a_typed_fence_error() {
    use sagat_application::{entropy::SystemEntropy, execution::Engine};
    use sagat_scheduler::{CollectionAdvanceError, Scheduler, SchedulerSettings};
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-singleton-fence-{full}")).await;
        let before = guards::snapshot(&fixture).await;
        let scheduler = Scheduler::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings::default(),
        );
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
        assert!(
            matches!(result, Err(CollectionAdvanceError::FenceUnavailable)),
            "no-placement event clocks require an explicit authority refusal: {result:?}"
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
    }
}

#[tokio::test]
async fn paused_pending_waits_create_no_subscription_and_resume_starts_normally() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-paused-start-{full}")).await;
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-scheduled-start",
        )
        .await;
        let before = guards::snapshot(&fixture).await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::Waiting
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Resume,
            "resume-before-scheduled-start",
        )
        .await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress
        );
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("resumed start replay");
    }
}

#[tokio::test]
async fn committed_cancellation_drains_before_any_pending_event_subscription_can_start() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-stopped-start-{full}")).await;
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Cancel,
            "cancel-before-scheduled-start",
        )
        .await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::DrainStarted
        );
        let started:i64=sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.started'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no start after committed stop");
        assert_eq!(started, 0);
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("stop precedence replay");
    }
}

#[tokio::test]
async fn scheduled_correlation_errors_take_the_local_error_route_without_worker_capabilities() {
    for full in [false, true] {
        let name = format!("event-scheduled-key-error-{full}");
        let mut document = wait_document(&name, true, 60000);
        document["bodies"]["line"]["nodes"][1]["correlationExpr"] = json!("item.missing");
        document["bodies"]["line"]["nodes"][1]["onError"] = json!("returned");
        let (fixture, store) = pending_wait_document(&name, document).await;
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::BodyProgress
        );
        let actual:(String,bool,String,bool)=sqlx::query_as("SELECT status,routed,failure_code,waiting_kind IS NULL AND due_at IS NULL AND correlation_key IS NULL AND event_type IS NULL AND event_since IS NULL AND activity IS NULL AND lease_token_hash IS NULL AND output_digest IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("typed expression failure and no subscription/worker capability");
        assert_eq!(
            actual,
            ("FAILED".into(), true, "EXPRESSION_ERROR".into(), true)
        );
        let detail:String=sqlx::query_scalar("SELECT detail::text FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0] AND event_type='step.failed'").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("recorded typed failure");
        assert!(!detail.contains("item.missing") && !detail.contains("first"));
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("computed failure and local route replay");
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    }
}

#[tokio::test]
async fn a_scheduled_subscription_refused_at_commit_rolls_back_before_current_quota_drain() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-scheduled-quota-{full}")).await;
        sqlx::query("INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) SELECT tenant_id,NULL,'tenant_content_bytes',content_bytes+content_reserved_bytes,'test' FROM tenant_retained_usage WHERE tenant_id=$1").bind(fixture.world.admin.tenant_id().as_uuid()).execute(&fixture.world.db.pool).await.expect("close current subscription content quota");
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::QuotaStopped
        );
        let actual:(String,bool)=sqlx::query_as("SELECT status,waiting_kind IS NULL AND due_at IS NULL AND correlation_key IS NULL AND event_type IS NULL AND event_since IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("failed binding transaction rolled back before drain");
        assert_eq!(actual, ("PENDING".into(), true));
        let counts:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.started'),count(*) FILTER(WHERE event_type='collection.drain_started') FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no rejected start and one quota drain");
        assert_eq!(counts, (0, 1));
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("quota refusal and drain replay");
    }
}

#[tokio::test]
async fn a_persist_time_quota_fault_rolls_back_before_reopening_the_scope_for_drain() {
    for full in [false, true] {
        let (fixture, store) = pending_wait(&format!("event-persist-quota-fault-{full}")).await;
        sqlx::raw_sql("CREATE FUNCTION reject_scheduled_wait_quota() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only event start persistence quota refusal' USING ERRCODE='54000', CONSTRAINT='tenant_content_quota_exceeded'; END $$; CREATE TRIGGER reject_scheduled_wait_quota BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='wait.started') EXECUTE FUNCTION reject_scheduled_wait_quota()")
            .execute(&fixture.world.db.pool).await.expect("owned early-persistence quota injection; no actual quota is closed");
        assert_eq!(
            advance(&fixture, &store, full).await,
            CollectionAdvance::QuotaStopped
        );
        let counts:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='wait.started'),count(*) FILTER(WHERE event_type='collection.drain_started') FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("no rejected start and exactly one recovery drain");
        assert_eq!(counts, (0, 1));
        let clean:bool=sqlx::query_scalar("SELECT status='PENDING' AND waiting_kind IS NULL AND due_at IS NULL AND correlation_key IS NULL AND event_type IS NULL AND event_since IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]").bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("aborted first transaction has no retained binding");
        assert!(clean);
        sqlx::raw_sql("DROP TRIGGER reject_scheduled_wait_quota ON run_events; DROP FUNCTION reject_scheduled_wait_quota()")
            .execute(&fixture.world.db.pool).await.expect("remove own injected fault");
        store
            .verify_run(fixture.world.admin.tenant_id(), fixture.run)
            .await
            .expect("fresh quota drain owns a consistent Run revision and scope");
    }
}
