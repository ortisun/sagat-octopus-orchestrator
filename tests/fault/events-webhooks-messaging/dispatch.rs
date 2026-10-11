//! Dispatching: however the dispatchers are interrupted, duplicated or raced, each event starts its
//! run once and wakes its wait once.

use std::time::Duration;

use sagat_application::dispatch::EventDispatchStore;
use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::execution::{CorrelationKey, RunId};
use sagat_domain::workflow::EventType;
use sagat_scheduler::EventDispatchSettings;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, document, wait_node, world};

fn on_order() -> Value {
    let mut doc = document("on-order", "work", vec![activity("work")]);
    doc["triggers"] = json!([{"kind": "event", "eventType": "order.created"}]);
    doc
}

async fn publish(w: &World, kind: &str, id: &str, key: Option<&str>) {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse(kind).expect("type"),
                subject: None,
                correlation_key: key.map(|k| CorrelationKey::parse(k).expect("key")),
                occurred_at: None,
                data: br#"{"n":1}"#.to_vec(),
                dedupe_ttl: None,
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("published");
}

async fn count(w: &World, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

/// The invariant of the whole feature, as one question to the database: the runs started by events
/// are exactly the deliveries to triggers, and no event is delivered to one consumer twice.
async fn assert_exactly_once(w: &World, events: i64) {
    assert_eq!(
        count(w, "SELECT count(*) FROM runs").await,
        events,
        "one run per event"
    );
    assert_eq!(
        count(
            w,
            "SELECT count(*) FROM event_deliveries WHERE consumer LIKE 'trigger:%'"
        )
        .await,
        events
    );
    assert_eq!(
        count(
            w,
            "SELECT count(*) FROM (SELECT 1 FROM event_deliveries \
             GROUP BY tenant_id, namespace_id, source, event_id, consumer HAVING count(*) > 1) d"
        )
        .await,
        0
    );
    let runs_of_events: i64 = count(
        w,
        "SELECT count(DISTINCT run_id) FROM event_deliveries WHERE run_id IS NOT NULL",
    )
    .await;
    assert_eq!(
        runs_of_events, events,
        "every run belongs to exactly one delivery"
    );
}

#[tokio::test]
async fn dispatchers_racing_for_the_same_events_start_each_run_once() {
    let w = world().await;
    w.publish(on_order()).await;
    let events = 60;
    for i in 0..events {
        publish(&w, "order.created", &format!("o-{i}"), None).await;
    }
    let dispatchers: Vec<_> = (0..4)
        .map(|_| {
            w.dispatcher_with(EventDispatchSettings {
                batch: 7,
                ..EventDispatchSettings::default()
            })
        })
        .collect();
    // Every round, all of them at once, until nothing is left.
    for _ in 0..40 {
        futures_util::future::join_all(dispatchers.iter().map(|d| d.tick())).await;
        if count(
            &w,
            "SELECT count(*) FROM inbox_events WHERE status = 'PENDING'",
        )
        .await
            == 0
        {
            break;
        }
    }
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM inbox_events WHERE status = 'DISPATCHED'"
        )
        .await,
        events
    );
    assert_exactly_once(&w, events).await;
}

#[tokio::test]
async fn a_dispatcher_cut_off_at_any_await_leaves_nothing_started_twice_and_nothing_lost() {
    let w = world().await;
    w.publish(on_order()).await;
    let (waiter, _) = w
        .publish(document(
            "payment-flow",
            "pay",
            vec![
                {
                    let mut n =
                        wait_node("pay", "payment.confirmed", "input.orderId", 300_000, None);
                    n["next"] = json!("work");
                    n
                },
                activity("work"),
            ],
        ))
        .await;
    let waits = 10;
    for i in 0..waits {
        w.start(waiter, &json!({"orderId": format!("ord-{i}")}))
            .await;
    }
    let events = 25;
    for i in 0..events {
        publish(&w, "order.created", &format!("o-{i}"), None).await;
    }
    for i in 0..waits {
        publish(
            &w,
            "payment.confirmed",
            &format!("p-{i}"),
            Some(&format!("ord-{i}")),
        )
        .await;
    }

    // Each round is abandoned after a random few milliseconds, which stops it between two awaits
    // wherever that happens to be: after a claim, after a run was created, inside a transaction.
    let mut microseconds = 700u64;
    for round in 0..400 {
        let dispatcher = w.dispatcher_with(EventDispatchSettings {
            batch: 5,
            ..EventDispatchSettings::default()
        });
        let _ = tokio::time::timeout(Duration::from_micros(microseconds), dispatcher.tick()).await;
        // The lease of whatever it held runs out.
        sqlx::query(
            "UPDATE inbox_events SET next_attempt_at = clock_timestamp() WHERE status = 'PENDING'",
        )
        .execute(&w.db.pool)
        .await
        .expect("leases run out");
        microseconds = 300 + (round * 97) % 4_000;
        if count(
            &w,
            "SELECT count(*) FROM inbox_events WHERE status = 'PENDING'",
        )
        .await
            == 0
        {
            break;
        }
    }
    // Whatever was cut off, one uninterrupted round finishes the job.
    w.dispatcher.tick().await;
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM inbox_events WHERE status = 'PENDING'"
        )
        .await,
        0
    );

    // The runs started by events: one per event, and the waits: each woken once.
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM event_deliveries WHERE consumer LIKE 'trigger:%'"
        )
        .await,
        events
    );
    assert_eq!(
        count(&w, "SELECT count(*) FROM runs r JOIN workflows f ON f.id = r.workflow_id WHERE f.name = 'on-order'").await,
        events
    );
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM event_deliveries WHERE consumer LIKE 'wait:%'"
        )
        .await,
        waits
    );
    assert_eq!(
        count(&w, "SELECT count(*) FROM (SELECT 1 FROM event_deliveries GROUP BY tenant_id, namespace_id, source, event_id, consumer HAVING count(*) > 1) d").await,
        0
    );
    let woken: Vec<RunId> = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT run_id FROM steps WHERE node = 'pay' AND status = 'SUCCEEDED'",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("woken")
    .into_iter()
    .map(RunId::new)
    .collect();
    assert_eq!(woken.len() as i64, waits);
    for run in woken {
        w.assert_consistent(run).await;
        let journal = w.journal(run).await;
        assert_eq!(journal.iter().filter(|e| *e == "wait.satisfied").count(), 1);
    }
}

#[tokio::test]
async fn an_event_claimed_by_a_dispatcher_that_died_is_taken_over_when_its_lease_ends() {
    let w = world().await;
    w.publish(on_order()).await;
    publish(&w, "order.created", "o-1", None).await;

    // A dispatcher claims the event and dies before doing anything with it.
    let held = w
        .store
        .claim_events(10, Duration::from_secs(60))
        .await
        .expect("claim");
    assert_eq!(held.len(), 1);
    assert!(
        !w.dispatcher.tick().await.did_work(),
        "nobody else can touch it while the lease holds"
    );
    assert_eq!(count(&w, "SELECT count(*) FROM runs").await, 0);

    sqlx::query(
        "UPDATE inbox_events SET next_attempt_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the lease ends");
    assert_eq!(w.dispatcher.tick().await.runs_started, 1);
    assert_exactly_once(&w, 1).await;
}

#[tokio::test]
async fn the_event_the_timeout_and_a_second_dispatcher_race_and_exactly_one_outcome_wins_every_time()
 {
    for round in 0..25 {
        let w = world().await;
        let mut wait = wait_node(
            "pay",
            "payment.confirmed",
            "input.orderId",
            300_000,
            Some("late"),
        );
        wait["next"] = json!("work");
        let (wf, _) = w
            .publish(document(
                "race-flow",
                "pay",
                vec![wait, activity("work"), activity("late")],
            ))
            .await;
        let run = w.start(wf, &json!({"orderId": "ord-1"})).await;
        publish(&w, "payment.confirmed", "e", Some("ord-1")).await;
        w.age_due(run, "pay").await;
        let other = w.dispatcher_with(EventDispatchSettings::default());

        let (_, _, _) = tokio::join!(w.scheduler.tick(), w.dispatcher.tick(), other.tick());
        w.scheduler.tick().await;
        w.dispatcher.tick().await;

        let journal = w.journal(run).await;
        let woken = journal.iter().filter(|e| *e == "wait.satisfied").count();
        let timed_out = journal.iter().filter(|e| *e == "wait.timed_out").count();
        assert_eq!(
            woken + timed_out,
            1,
            "round {round}: exactly one of the two happened: {journal:?}"
        );
        let delivered = count(
            &w,
            "SELECT count(*) FROM event_deliveries WHERE consumer LIKE 'wait:%'",
        )
        .await;
        assert_eq!(
            delivered,
            i64::try_from(woken).expect("small"),
            "round {round}: a delivery exists if and only if the event won"
        );
        w.assert_consistent(run).await;
    }
}
