//! The broker gone while a dispatcher changes hands: the outbox is the authority, so nothing is lost
//! and the broker drops what is published twice.

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_application::entropy::SystemEntropy;
use sagat_infra_nats::{NatsConfig, NatsPublisher};
use sagat_scheduler::{DispatchSettings, OutboxDispatcher};
use serde_json::json;

use crate::support::engine::{chain, world};
use crate::support::nats::{self, drain};
use crate::support::scale::{TTL, keeper};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_dispatcher_that_dies_while_the_broker_is_down_is_replaced_and_every_hint_arrives_once() {
    let w = world().await;
    let server = nats::start().await;
    let proxy = nats::proxy_to(server.addr).await;
    let mut config = NatsConfig::new(proxy.url());
    config.ack_timeout = Duration::from_millis(700);
    let publisher = NatsPublisher::connect(config).await.expect("connect");
    let settings = DispatchSettings {
        batch: 50,
        visibility: Duration::from_secs(2),
        base_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_millis(300),
    };

    // alpha is the dispatcher, with every shard.
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let first = OutboxDispatcher::new(w.store.clone(), publisher.clone(), SystemEntropy, settings)
        .with_placement(alpha.placement().clone());

    // The broker disappears and runs keep starting: PostgreSQL is the authority.
    let (wf, _) = w.publish(chain("chaos-broker")).await;
    proxy.block();
    let mut runs = Vec::new();
    for i in 0..30 {
        runs.push(w.start(wf, &json!({ "i": i })).await);
    }
    let during = first.run_once().await;
    assert_eq!(
        during.published, 0,
        "nothing can be published while it is down"
    );
    assert!(during.failed >= 1);
    let owed = w.backlog().await;
    assert!(owed >= 30, "the hints are still owed: {owed}");

    // alpha dies (it never confirms anything); its leases run out; beta takes the shards.
    drop(first);
    sqlx::query("UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    let second = OutboxDispatcher::new(w.store.clone(), publisher.clone(), SystemEntropy, settings)
        .with_placement(beta.placement().clone());

    // The broker returns; the hints go out (the failures of the outage back off first).
    proxy.unblock();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while w.backlog().await > 0 {
        second.run_once().await;
        assert!(
            std::time::Instant::now() < deadline,
            "the outbox did not drain"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Every run's hint is in the stream exactly once, whatever was attempted more than once.
    let tenant = w.admin.tenant_id();
    let stored = drain(&publisher, &format!("sagat.v1.task.ready.{tenant}.act.a")).await;
    let ids: BTreeSet<String> = stored.iter().filter_map(|m| m.2.clone()).collect();
    assert_eq!(stored.len(), ids.len(), "a hint is in the stream twice");
    let hinted: BTreeSet<String> = stored
        .iter()
        .map(|m| {
            let hint: serde_json::Value = serde_json::from_slice(&m.1).expect("json");
            hint["runId"].as_str().expect("a run").to_owned()
        })
        .collect();
    for run in &runs {
        assert!(
            hinted.contains(&run.to_string()),
            "the hint of run {run} never arrived"
        );
    }
}
