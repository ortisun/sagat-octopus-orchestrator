//! A duplicate event around a rollback starts nothing twice, and an event that arrives while the
//! pointer moves starts one run on a version that was active.

use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::execution::RunId;
use sagat_domain::workflow::EventType;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, world};

fn triggered(marker: &str) -> Value {
    let mut doc = document("on-order", "work", vec![activity("work")]);
    doc["triggers"] = json!([{"kind": "event", "eventType": "order.created"}]);
    doc["metadata"]["description"] = json!(marker);
    doc
}

async fn event(w: &World, id: &str) -> bool {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse("order.created").expect("type"),
                subject: Some(format!("subject/{id}")),
                correlation_key: None,
                occurred_at: None,
                data: serde_json::to_vec(&json!({"order": id})).expect("json"),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published")
        .duplicate
}

async fn runs(w: &World) -> Vec<(RunId, uuid::Uuid)> {
    sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        "SELECT r.id, r.workflow_version_id FROM runs r JOIN workflows f ON f.id = r.workflow_id \
         WHERE f.name = 'on-order' ORDER BY r.id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("runs")
    .into_iter()
    .map(|(run, version)| (RunId::new(run), version))
    .collect()
}

#[tokio::test]
async fn a_duplicate_event_around_a_rollback_starts_one_run_and_new_events_follow_the_pointer() {
    let w = world().await;
    let (id, v1) = w.publish(triggered("one")).await;
    let v2 = w.publish_new_version(id, triggered("two")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;

    assert!(!event(&w, "o-1").await);
    w.dispatcher.tick().await;
    let first = runs(&w).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].1, v2.as_uuid(), "started on the active version");

    // The pointer moves back and the sender, not sure its first call arrived, sends it again.
    w.publishing
        .activate(&ops, id, 1, "two misbehaves", Some(2), false, &ctx())
        .await
        .expect("rollback");
    assert!(
        event(&w, "o-1").await,
        "the second send is recognised as the same event"
    );
    w.dispatcher.tick().await;
    assert_eq!(runs(&w).await.len(), 1, "nothing was started twice");

    // A new event follows the pointer.
    assert!(!event(&w, "o-2").await);
    w.dispatcher.tick().await;
    let all = runs(&w).await;
    assert_eq!(all.len(), 2);
    assert_eq!(
        all[1].1,
        v1.as_uuid(),
        "started on the version the pointer was moved to"
    );
}

#[tokio::test]
async fn events_that_arrive_while_the_pointer_moves_start_one_run_each_on_a_version_that_was_active()
 {
    let w = world().await;
    let (id, v1) = w.publish(triggered("one")).await;
    let v2 = w.publish_new_version(id, triggered("two")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;

    for n in 0..6 {
        event(&w, &format!("burst-{n}")).await;
    }
    let mover = async {
        for round in 0..10u32 {
            let _ = w
                .publishing
                .activate(
                    &ops,
                    id,
                    1 + round % 2,
                    "moving the pointer while events arrive",
                    None,
                    true,
                    &ctx(),
                )
                .await;
        }
    };
    let dispatchers =
        futures_util::future::join_all((0..4).map(|_| async { w.dispatcher.tick().await }));
    futures_util::future::join(mover, dispatchers).await;
    while w.dispatcher.tick().await.did_work() {}

    let started = runs(&w).await;
    assert_eq!(
        started.len(),
        6,
        "one run per event, however the dispatchers and the pointer raced"
    );
    for (_, version) in &started {
        assert!(*version == v1.as_uuid() || *version == v2.as_uuid());
    }
}
