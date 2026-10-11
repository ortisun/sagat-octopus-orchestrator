//! The outbox is divided like the scheduler's passes: a dispatcher publishes the messages of the
//! shards it holds, fairly between tenants, and a message left half-done by a dead one is published
//! by the next owner (FR-010-001, FR-010-002, FR-010-003; edge case "failover with stale outbox").

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_application::entropy::SystemEntropy;
use sagat_application::maintenance::OutboxStore;
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{DispatchSettings, Keeper, OutboxDispatcher, Placement};
use serde_json::json;
use uuid::Uuid;

use crate::passes::{TTL, keeper};
use crate::support::engine::{RecordingBroker, World, chain, world};

fn dispatcher(
    w: &World,
    broker: &RecordingBroker,
    placement: &Placement,
    batch: u32,
    visibility: Duration,
) -> OutboxDispatcher<PgRunStore, RecordingBroker, SystemEntropy> {
    OutboxDispatcher::new(
        w.store.clone(),
        broker.clone(),
        SystemEntropy,
        DispatchSettings {
            batch,
            visibility,
            ..DispatchSettings::default()
        },
    )
    .with_placement(placement.clone())
}

/// Starts `n` runs of the first tenant: one `task.ready` message each.
async fn start_runs(w: &World, name: &str, n: usize) {
    let (wf, _) = w.publish(chain(name)).await;
    let world = w;
    futures_util::future::join_all(
        (0..n).map(|i| async move { world.start(wf, &json!({"i": i})).await }),
    )
    .await;
}

async fn aggregate_of(w: &World, message_id: &str) -> Uuid {
    sqlx::query_scalar("SELECT aggregate_id FROM event_outbox WHERE id = $1")
        .bind(Uuid::parse_str(message_id).expect("a message id"))
        .fetch_one(&w.db.pool)
        .await
        .expect("the message")
}

async fn tenant_of_message(w: &World, message_id: &str) -> Uuid {
    sqlx::query_scalar("SELECT tenant_id FROM event_outbox WHERE id = $1")
        .bind(Uuid::parse_str(message_id).expect("a message id"))
        .fetch_one(&w.db.pool)
        .await
        .expect("the message")
}

async fn drain(dispatcher: &OutboxDispatcher<PgRunStore, RecordingBroker, SystemEntropy>) {
    while dispatcher.run_once().await.busy() {}
}

async fn two_replicas(w: &World) -> (Keeper<PgRunStore>, Keeper<PgRunStore>) {
    let (a, b) = (keeper(w, "alpha", TTL), keeper(w, "beta", TTL));
    a.round().await.expect("alpha");
    b.round().await.expect("beta");
    a.round().await.expect("alpha");
    b.round().await.expect("beta");
    (a, b)
}

#[tokio::test]
async fn each_dispatcher_publishes_the_messages_of_its_own_shards_and_between_them_all_of_them() {
    let w = world().await;
    let (a, b) = two_replicas(&w).await;
    start_runs(&w, "outbox-shards", 60).await;
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert!(total >= 60);

    let (broker_a, broker_b) = (RecordingBroker::default(), RecordingBroker::default());
    let da = dispatcher(&w, &broker_a, a.placement(), 100, Duration::from_secs(30));
    let db = dispatcher(&w, &broker_b, b.placement(), 100, Duration::from_secs(30));
    drain(&da).await;
    let (mine_a, mine_b) = (a.placement().owned(), b.placement().owned());

    let ids_a: BTreeSet<String> = broker_a
        .published()
        .into_iter()
        .map(|p| p.message_id)
        .collect();
    assert!(!ids_a.is_empty(), "alpha held shards with messages in them");
    for id in &ids_a {
        let aggregate = aggregate_of(&w, id).await;
        assert!(
            mine_a.contains(Shard::of(aggregate)),
            "alpha published a message of a shard it does not hold"
        );
    }
    // Nothing of beta's has been published by anybody yet.
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(left, total - i64::try_from(ids_a.len()).expect("fits"));

    drain(&db).await;
    let ids_b: BTreeSet<String> = broker_b
        .published()
        .into_iter()
        .map(|p| p.message_id)
        .collect();
    assert!(!ids_b.is_empty());
    for id in &ids_b {
        assert!(mine_b.contains(Shard::of(aggregate_of(&w, id).await)));
    }
    assert!(
        ids_a.is_disjoint(&ids_b),
        "a message went to both dispatchers"
    );
    assert_eq!(
        i64::try_from(ids_a.len() + ids_b.len()).expect("fits"),
        total,
        "between them they published every message, once"
    );
    assert_eq!(w.backlog().await, 0);
}

#[tokio::test]
async fn a_burst_of_messages_of_one_tenant_does_not_hold_back_another_tenants() {
    let w = world().await;
    let only = keeper(&w, "alpha", TTL);
    only.round().await.expect("alpha");
    // The burst is older than the small tenant's messages.
    start_runs(&w, "outbox-burst", 60).await;
    let other = w.publish_as_other_tenant(chain("outbox-small")).await;
    for _ in 0..5 {
        w.start_as_other_tenant(other, &json!({}))
            .await
            .expect("start");
    }
    let small_tenant = w.other_admin.tenant_id().as_uuid();

    let broker = RecordingBroker::default();
    let dispatcher = dispatcher(&w, &broker, only.placement(), 20, Duration::from_secs(30));
    let round = dispatcher.run_once().await;
    assert_eq!(round.published, 20, "the batch is used in full");
    let mut of_small = 0;
    for published in broker.published() {
        if tenant_of_message(&w, &published.message_id).await == small_tenant {
            of_small += 1;
        }
    }
    assert_eq!(
        of_small, 5,
        "the small tenant's messages went out in the first round, behind a burst of sixty"
    );
    // And the burst drains.
    drain(&dispatcher).await;
    assert_eq!(w.backlog().await, 0);
}

#[tokio::test]
async fn a_dispatcher_that_holds_no_shard_publishes_nothing() {
    let w = world().await;
    start_runs(&w, "outbox-nobody", 5).await;
    let broker = RecordingBroker::default();
    let nobody = Placement::new("lonely");
    let dispatcher = dispatcher(&w, &broker, &nobody, 100, Duration::from_secs(30));
    assert!(!dispatcher.run_once().await.busy());
    assert!(broker.published().is_empty());
    assert!(w.backlog().await >= 5);
}

#[tokio::test]
async fn a_message_claimed_by_a_dispatcher_that_died_is_published_by_the_next_owner_after_the_visibility()
 {
    // Failover with a stale outbox: alpha claimed some messages and died before publishing them.
    let w = world().await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    start_runs(&w, "outbox-failover", 10).await;
    let visibility = Duration::from_millis(600);

    // alpha claims everything that is due and never publishes or confirms (the process is gone).
    let claimed = w.store.claim_outbox(100, visibility).await.expect("claim");
    assert!(claimed.len() >= 10);
    sqlx::query("UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("alpha is gone");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("alpha is gone");

    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    let broker = RecordingBroker::default();
    let dispatcher = dispatcher(&w, &broker, beta.placement(), 100, visibility);
    // The claim is still in force: beta does not publish what alpha may be publishing.
    assert!(
        !dispatcher.run_once().await.busy(),
        "claimed messages are hidden"
    );
    tokio::time::sleep(visibility + Duration::from_millis(300)).await;
    drain(&dispatcher).await;
    let published: BTreeSet<String> = broker
        .published()
        .into_iter()
        .map(|p| p.message_id)
        .collect();
    for message in &claimed {
        assert!(
            published.contains(&message.id.to_string()),
            "a message alpha claimed was never published"
        );
    }
    assert_eq!(w.backlog().await, 0);
}
