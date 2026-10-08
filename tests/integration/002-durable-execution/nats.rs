//! The outbox over a real NATS JetStream server: hints arrive, duplicates by id are dropped, and a
//! broker outage loses nothing (T002-031, FR-002-003, NFR-002-DUR).

use std::time::Duration;

use async_nats::jetstream::consumer::{AckPolicy, pull};
use futures_util::StreamExt;
use sagat_application::entropy::SystemEntropy;
use sagat_application::maintenance::{EventPublisher, PublishError};
use sagat_infra_nats::{NatsConfig, NatsPublisher};
use sagat_scheduler::{DispatchSettings, OutboxDispatcher};
use serde_json::{Value, json};

use crate::support::engine::{chain, world};
use crate::support::nats;

fn config(url: String) -> NatsConfig {
    let mut config = NatsConfig::new(url);
    config.ack_timeout = Duration::from_secs(2);
    config
}

/// Everything currently in the stream on `filter`, without acknowledging a thing.
async fn drain(publisher: &NatsPublisher, filter: &str) -> Vec<(String, Vec<u8>, Option<String>)> {
    let stream = publisher.stream().await.expect("stream");
    let consumer = stream
        .create_consumer(pull::Config {
            filter_subject: filter.to_owned(),
            ack_policy: AckPolicy::None,
            ..Default::default()
        })
        .await
        .expect("consumer");
    let mut batch = consumer
        .fetch()
        .max_messages(100)
        .expires(Duration::from_millis(800))
        .messages()
        .await
        .expect("fetch");
    let mut out = Vec::new();
    while let Some(message) = batch.next().await {
        let message = message.expect("message");
        let id = message
            .headers
            .as_ref()
            .and_then(|h| h.get("Nats-Msg-Id"))
            .map(ToString::to_string);
        out.push((message.subject.to_string(), message.payload.to_vec(), id));
    }
    out
}

#[tokio::test]
async fn a_message_published_twice_with_the_same_id_is_stored_once() {
    let server = nats::start().await;
    let publisher = NatsPublisher::connect(config(server.url()))
        .await
        .expect("connect");
    publisher
        .publish("sagat.v1.task.ready.t1", "msg-1", b"{\"n\":1}")
        .await
        .expect("first");
    publisher
        .publish("sagat.v1.task.ready.t1", "msg-1", b"{\"n\":1}")
        .await
        .expect("a duplicate is acknowledged too");
    publisher
        .publish("sagat.v1.task.ready.t1", "msg-2", b"{\"n\":2}")
        .await
        .expect("second");
    let stored = drain(&publisher, "sagat.v1.>").await;
    assert_eq!(stored.len(), 2, "the duplicate was dropped by its id");
    assert_eq!(stored[0].2.as_deref(), Some("msg-1"));
}

#[tokio::test]
async fn subjects_outside_the_platform_prefix_are_rejected_not_published() {
    let server = nats::start().await;
    let publisher = NatsPublisher::connect(config(server.url()))
        .await
        .expect("connect");
    for subject in [
        "other.thing",
        "sagat.v2.x",
        "_INBOX.x",
        "$JS.API.STREAM.DELETE.SAGAT",
    ] {
        assert_eq!(
            publisher.publish(subject, "id", b"{}").await,
            Err(PublishError::Rejected(
                "the subject is outside the platform's"
            )),
            "{subject}"
        );
    }
    assert!(drain(&publisher, "sagat.v1.>").await.is_empty());
}

#[tokio::test]
async fn an_unreachable_broker_does_not_stop_the_process_and_reads_as_unavailable() {
    // A port nobody listens on: connecting must not fail (the engine starts and its outbox
    // accumulates), publishing must report "unavailable" within the acknowledgement timeout.
    let free = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr")
    };
    let mut cfg = config(format!("nats://{free}"));
    cfg.ack_timeout = Duration::from_millis(500);
    let publisher = NatsPublisher::connect(cfg)
        .await
        .expect("connect never fails for an outage");
    let started = std::time::Instant::now();
    assert_eq!(
        publisher
            .publish("sagat.v1.task.ready.t", "id", b"{}")
            .await,
        Err(PublishError::Unavailable)
    );
    assert!(started.elapsed() < Duration::from_secs(10), "bounded wait");
}

#[tokio::test]
async fn hints_flow_from_the_outbox_to_jetstream_and_survive_a_broker_outage() {
    let w = world().await;
    let server = nats::start().await;
    let proxy = nats::proxy_to(server.addr).await;
    let mut cfg = config(proxy.url());
    cfg.ack_timeout = Duration::from_millis(700);
    let publisher = NatsPublisher::connect(cfg).await.expect("connect");
    let dispatcher = OutboxDispatcher::new(
        w.store.clone(),
        publisher.clone(),
        SystemEntropy,
        DispatchSettings {
            visibility: Duration::from_secs(5),
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(300),
            ..DispatchSettings::default()
        },
    );
    let (wf, _) = w.publish(chain("through-nats")).await;

    // Healthy: the hint of the first step arrives on the tenant's subject.
    let first = w.start(wf, &json!({})).await;
    let round = dispatcher.run_once().await;
    assert_eq!((round.published, round.failed), (1, 0));
    let tenant = w.admin.tenant_id();
    let filter = format!("sagat.v1.task.ready.{tenant}");
    let seen = drain(&publisher, &filter).await;
    assert_eq!(seen.len(), 1);
    let hint: Value = serde_json::from_slice(&seen[0].1).expect("json");
    assert_eq!(hint["runId"], first.to_string());
    assert_eq!(hint["node"], "a");
    assert!(
        hint.get("input").is_none(),
        "a hint is an identifier, not a payload"
    );

    // The broker disappears. Runs keep starting (PostgreSQL is the authority); nothing is lost.
    proxy.block();
    let second = w.start(wf, &json!({})).await;
    let third = w.start(wf, &json!({})).await;
    let during = dispatcher.run_once().await;
    assert_eq!(
        during.published, 0,
        "nothing can be published while it is down"
    );
    assert!(during.failed >= 1);
    assert_eq!(w.backlog().await, 2, "both hints are still owed");

    // It comes back: the client reconnects by itself and the owed hints go out.
    proxy.unblock();
    let mut delivered = 0;
    for _ in 0..60 {
        delivered += dispatcher.run_once().await.published;
        if w.backlog().await == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(w.backlog().await, 0, "the backlog drained after the outage");
    assert!(delivered >= 2);
    let after: Vec<Value> = drain(&publisher, &filter)
        .await
        .into_iter()
        .map(|m| serde_json::from_slice(&m.1).expect("json"))
        .collect();
    let runs: Vec<String> = after
        .iter()
        .map(|h| h["runId"].as_str().expect("run").to_owned())
        .collect();
    for expected in [first, second, third] {
        assert!(
            runs.contains(&expected.to_string()),
            "the hint of {expected} was lost: {runs:?}"
        );
    }
}
