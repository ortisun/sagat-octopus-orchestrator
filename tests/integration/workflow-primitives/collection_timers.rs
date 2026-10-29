//! A member's durable timer owns no worker lease; due discovery retains complete path identity.
use crate::support::scope_flow::{Flow, activity, collection_document};
use serde_json::json;

pub(super) async fn pending_timer(name: &str) -> Flow {
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            1,
        ),
        json!([1]),
    )
    .await;
    let owner = sagat_domain::execution::ExecutionInstanceId::root(
        sagat_domain::workflow::NodeId::parse("lines").expect("owner"),
    );
    assert_eq!(
        flow.scheduler
            .advance_collection(flow.tenant, flow.run, &owner)
            .await
            .expect("enter"),
        sagat_scheduler::CollectionAdvance::Entered
    );
    assert_eq!(
        flow.scheduler
            .advance_collection(flow.tenant, flow.run, &owner)
            .await
            .expect("admit"),
        sagat_scheduler::CollectionAdvance::Admitted { index: 0 }
    );
    flow
}

pub(super) async fn waiting_timer(name: &str) -> Flow {
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            1,
        ),
        json!([1]),
    )
    .await;
    flow.drive().await;
    let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='delay' AND status='WAITING' AND waiting_kind='TIMER' AND due_at>clock_timestamp()")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("one real waiting timer");
    assert_eq!(
        waiting,
        1,
        "{}",
        crate::support::scheduler_evidence(&flow.api.db.pool, flow.tenant, flow.run).await
    );
    let evidence =
        crate::support::scheduler_evidence(&flow.api.db.pool, flow.tenant, flow.run).await;
    assert_eq!(evidence["shardLive"], true, "{evidence}");
    flow
}

pub(super) async fn age(flow: &Flow) {
    sqlx::query("UPDATE steps SET due_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND run_id=$2 AND node='delay' AND status='WAITING' AND waiting_kind='TIMER'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).execute(&flow.api.db.pool).await.expect("explicit timer deadline fixture");
}

async fn finish_due_timers(flow: &Flow, count: i64) {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 \
          AND node='delay' AND status='WAITING' AND waiting_kind='TIMER' \
          AND due_at>clock_timestamp() AND activity IS NULL AND attempt=0 AND generation=0",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("real scoped waiting timers");
    assert_eq!(waiting, count);
    assert!(
        flow.poll().await.is_empty(),
        "a waiting timer is never worker work"
    );
    // This fixture ages only the timers' database deadlines; no race is ordered with a sleep.
    let aged = sqlx::query("UPDATE steps SET due_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND run_id=$2 AND node='delay' AND status='WAITING' AND waiting_kind='TIMER'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).execute(&flow.api.db.pool).await.expect("explicit aged deadline fixture");
    assert_eq!(i64::try_from(aged.rows_affected()).expect("count"), count);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let fired: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='timer.fired' AND instance_path_nodes=ARRAY['lines']::text[]")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("qualified fires");
    assert_eq!(fired, count);
    flow.verify().await;
}

#[tokio::test]
async fn an_activity_routes_each_member_to_its_own_durable_timer_then_return() {
    let name = "collection-activity-timer";
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                activity("reserve", "line.reserve", "delay", json!({})),
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            2,
        ),
        json!([1, 2]),
    )
    .await;
    flow.drive().await;
    let work = flow.poll().await;
    assert_eq!(work.len(), 2);
    for task in &work {
        flow.complete(task, json!({"ok":true})).await;
    }
    flow.drive().await;
    finish_due_timers(&flow, 2).await;
}

#[tokio::test]
async fn a_timer_entry_finishes_without_any_worker_task_or_admission_notification() {
    let name = "collection-timer-entry";
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            2,
        ),
        json!([1, 2]),
    )
    .await;
    flow.drive().await;
    finish_due_timers(&flow, 2).await;
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("no fake task");
    assert_eq!(tasks, 0);
    let usage:(i32,i32,i32)=sqlx::query_as("SELECT admitted_instances,reserved_instances,retired_reservations FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("exact lifetime");
    assert_eq!(usage, (6, 0, 0));
}

#[tokio::test]
async fn timer_start_and_fire_traces_keep_qualified_identity_without_item_data() {
    let marker = "scoped-timer-sensitive-canary-2e61a9";
    let name = "collection-timer-trace";
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            1,
        ),
        json!([{"sensitive":marker}]),
    )
    .await;
    let captured = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    flow.drive().await;
    age(&flow).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let text = captured.text();
    assert!(
        text.contains("scheduler.action")
            && text.contains("collection_local")
            && text.contains("kind=\"timer\""),
        "start and fire spans: {text}"
    );
    assert!(
        text.contains("instance=") && text.contains("delay") && text.contains("lines"),
        "qualified identity"
    );
    assert!(
        text.contains(&flow.run.to_string()) && text.contains(&flow.tenant.to_string()),
        "run/tenant correlation"
    );
    assert!(
        !text.contains(marker) && !text.contains("sensitive"),
        "no captured item in traces"
    );
    flow.verify().await;
}
