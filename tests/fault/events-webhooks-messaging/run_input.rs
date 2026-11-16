//! Design 043, rows 9 and 10 under faults (T043-7, FR-043-007, G043-002): a trigger refusal is recorded whole or not at
//! all, and once, whatever stops the writer or races it. The races are made with locks the test holds, released only once
//! the database shows the writers waiting on them; the crash is the end of the writer's backend while it waits.

use std::time::{Duration, Instant};

use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::workflow::EventType;
use sagat_scheduler::EventDispatchSettings;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, world};
use crate::support::run_input::audits;

/// A schema of the envelope `{event, data}` that requires `data.orderId`: neither the events of these tests nor a cron
/// tick's document carries it.
fn needs_order() -> Value {
    json!({
        "type": "object",
        "required": ["data"],
        "properties": {"data": {"type": "object", "required": ["orderId"]}}
    })
}

fn triggered(name: &str, triggers: Value) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = triggers;
    doc["inputSchema"] = needs_order();
    doc
}

async fn count(w: &World, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

/// Backends of this database that wait on a lock while running a statement that begins with `prefix`.
async fn waiting(w: &World, prefix: &str) -> Vec<i32> {
    sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' \
           AND ltrim(query) LIKE $1 || '%' ORDER BY pid",
    )
    .bind(prefix)
    .fetch_all(&w.db.pool)
    .await
    .expect("pg_stat_activity")
}

/// Returns once `n` backends wait on a lock in a statement that begins with `prefix`; fails after a minute.
async fn until_waiting(w: &World, prefix: &str, n: usize) -> Vec<i32> {
    let give_up = Instant::now() + Duration::from_secs(60);
    loop {
        let pids = waiting(w, prefix).await;
        if pids.len() >= n {
            return pids;
        }
        assert!(
            Instant::now() < give_up,
            "{n} writers never waited on {prefix}"
        );
        tokio::task::yield_now().await;
    }
}

async fn terminate(w: &World, pid: i32) {
    let ended: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&w.db.pool)
        .await
        .expect("terminate");
    assert!(ended, "the writer's backend ended");
}

/// Publishes a cron trigger whose version refuses its tick and makes its first tick due.
async fn a_due_refused_tick(w: &World, name: &str) {
    w.publish(triggered(
        name,
        json!([{"kind": "cron", "cron": "*/5 * * * *", "timezone": "UTC"}]),
    ))
    .await;
    w.dispatcher.tick().await; // plans the first time
    sqlx::query(
        "UPDATE workflow_triggers SET next_fire_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("due");
}

const TRIGGER_WRITE: &str = "UPDATE workflow_triggers";

/// G043-002, two engines on one refused cron tick: both find it due and both refuse its input; the trigger's row is held
/// until both wait to write, then released. The tick is audited once, with its key, and starts nothing.
#[tokio::test]
async fn two_engines_on_one_refused_cron_tick_audit_it_once_with_the_tick_key() {
    let w = world().await;
    a_due_refused_tick(&w, "cron-two-engines").await;
    let other = w.dispatcher_with(EventDispatchSettings::default());
    let mut held = w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT id FROM workflow_triggers FOR UPDATE")
        .fetch_all(&mut *held)
        .await
        .expect("hold the trigger");
    let release = async {
        until_waiting(&w, TRIGGER_WRITE, 2).await;
        held.commit().await.expect("release");
    };
    tokio::join!(w.dispatcher.tick(), other.tick(), release);
    assert_eq!(count(&w, "SELECT count(*) FROM runs").await, 0);
    assert_eq!(
        audits(&w, "run.input_refused", "at tick").await,
        1,
        "one audit for one tick"
    );
}

/// G043-002, a writer that stops between the refusal and the move of the trigger: its backend ends while it waits on the
/// trigger's row. Nothing it wrote stays, and the next round audits the tick once.
#[tokio::test]
async fn a_cron_refusal_whose_writer_stops_before_the_trigger_moves_is_audited_once_by_the_next_round()
 {
    let w = world().await;
    a_due_refused_tick(&w, "cron-stopped-writer").await;
    let mut held = w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT id FROM workflow_triggers FOR UPDATE")
        .fetch_all(&mut *held)
        .await
        .expect("hold the trigger");
    let crash = async {
        let pids = until_waiting(&w, TRIGGER_WRITE, 1).await;
        terminate(&w, pids[0]).await;
        // released at once: the stopped round's own follow-up (its backoff) writes the trigger too
        held.rollback().await.expect("release");
    };
    tokio::join!(w.dispatcher.tick(), crash);
    // a failed round is retried after a backoff; the test does not wait for it
    sqlx::query("UPDATE workflow_triggers SET retry_at = NULL")
        .execute(&w.db.pool)
        .await
        .expect("retry now");
    let after_the_stop = audits(&w, "run.input_refused", "at tick").await;
    w.dispatcher.tick().await;
    assert_eq!(count(&w, "SELECT count(*) FROM runs").await, 0);
    assert_eq!(
        audits(&w, "run.input_refused", "at tick").await,
        1,
        "one audit for one tick (after the stop: {after_the_stop})"
    );
    let moved: bool =
        sqlx::query_scalar("SELECT next_fire_at > clock_timestamp() FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("next");
    assert!(moved, "the tick is acted on");
}

async fn publish_event(w: &World, id: &str) {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse("order.created").expect("type"),
                subject: None,
                correlation_key: None,
                occurred_at: None,
                data: br#"{"amount": 1}"#.to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
}

async fn refused_deliveries(w: &World) -> i64 {
    count(
        w,
        "SELECT count(*) FROM event_deliveries WHERE consumer LIKE 'trigger:%' AND run_id IS NULL",
    )
    .await
}

/// Row 9, a crash between the delivery and the audit: the writer's backend ends after it wrote the delivery without a run
/// and while it waits to write the audit. Neither stays (one transaction); the event is dispatched again and records
/// both, once; a redelivery after that records neither again.
#[tokio::test]
async fn a_crash_between_the_refused_delivery_and_its_audit_keeps_neither_and_the_retry_records_both_once()
 {
    let w = world().await;
    w.publish(triggered(
        "event-crash",
        json!([{"kind": "event", "eventType": "order.created"}]),
    ))
    .await;
    publish_event(&w, "crash-e1").await;
    let mut held = w.db.pool.begin().await.expect("begin");
    sqlx::query("LOCK TABLE audit_records IN SHARE MODE")
        .execute(&mut *held)
        .await
        .expect("hold the audit table");
    let crash = async {
        let pids = until_waiting(&w, "INSERT INTO audit_records", 1).await;
        terminate(&w, pids[0]).await;
    };
    tokio::join!(w.dispatcher.tick(), crash);
    held.rollback().await.expect("release");
    assert_eq!(
        refused_deliveries(&w).await,
        0,
        "the delivery went with the audit"
    );
    assert_eq!(
        audits(&w, "run.input_refused", "RUN_INPUT_INVALID").await,
        0
    );

    for _ in 0..2 {
        sqlx::query(
            "UPDATE inbox_events SET status = 'PENDING', dispatched_at = NULL, next_attempt_at = clock_timestamp() \
             WHERE event_id = 'crash-e1'",
        )
        .execute(&w.db.pool)
        .await
        .expect("dispatch again");
        w.dispatcher.tick().await;
        assert_eq!(refused_deliveries(&w).await, 1);
        assert_eq!(
            audits(&w, "run.input_refused", "RUN_INPUT_INVALID").await,
            1
        );
    }
    assert_eq!(count(&w, "SELECT count(*) FROM runs").await, 0);
}
