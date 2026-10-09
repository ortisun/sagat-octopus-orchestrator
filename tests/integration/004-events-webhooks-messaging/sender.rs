//! The guarded sender: a subscription reaches only what the egress policy of its namespace allows,
//! and what the receiver answers is mapped to what the worker needs to know (FR-004-006, FR-009-008).

use std::time::Duration;

use sagat_application::outbound::{OutboundRequest, SendFailure, WebhookSender};
use sagat_domain::outbound::TargetUrl;
use sagat_infra_connectors::webhook::{Guarded, GuardedSender};

use crate::support::engine::{World, ctx, world};
use crate::support::receiver::{OpenFactory, Receiver, Reply};

fn request(w: &World, url: &str, timeout: Duration) -> OutboundRequest {
    OutboundRequest {
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        url: TargetUrl::parse(url).expect("url"),
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: b"{}".to_vec(),
        timeout,
    }
}

#[tokio::test]
async fn the_guarded_client_refuses_a_loopback_destination_and_the_refusal_is_audited() {
    let w = world().await;
    let receiver = Receiver::start().await;
    let sender = GuardedSender::new(w.store.clone());

    // With no policy the namespace may reach nothing; with a rule that names the address, a
    // loopback address is still out of reach: no policy can allow it.
    for rules in [vec![], vec!["127.0.0.1/32".to_owned()]] {
        if !rules.is_empty() {
            w.egress
                .set(&w.admin, w.namespace.id(), &rules, &[], true, &ctx())
                .await
                .expect("policy");
        }
        let outcome = sender
            .send(&request(&w, &receiver.url, Duration::from_secs(2)))
            .await;
        assert_eq!(outcome, Err(SendFailure::Denied), "rules: {rules:?}");
    }
    assert!(receiver.seen().is_empty(), "nothing reached the receiver");

    // The refusal is written off the response path.
    let mut audited = 0i64;
    for _ in 0..50 {
        audited =
            sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'egress.denied'")
                .fetch_one(&w.db.pool)
                .await
                .expect("count");
        if audited > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(audited > 0, "the denial is in the trail");
}

#[tokio::test]
async fn the_answer_of_a_receiver_becomes_a_status_and_a_request_for_patience() {
    let w = world().await;
    let receiver = Receiver::start().await;
    receiver.script([
        Reply::Status(200),
        Reply::Status(503),
        Reply::StatusAfter(429, 120),
        Reply::Status(404),
    ]);
    let sender = GuardedSender::with_factory(w.store.clone(), OpenFactory);
    let ask = || request(&w, &receiver.url, Duration::from_secs(2));

    let ok = sender.send(&ask()).await.expect("answered");
    assert_eq!((ok.status, ok.retry_after), (200, None));
    assert_eq!(sender.send(&ask()).await.expect("answered").status, 503);
    let throttled = sender.send(&ask()).await.expect("answered");
    assert_eq!(
        (throttled.status, throttled.retry_after),
        (429, Some(Duration::from_secs(120)))
    );
    assert_eq!(sender.send(&ask()).await.expect("answered").status, 404);
    assert_eq!(receiver.seen().len(), 4);
    assert_eq!(receiver.seen()[0].body, b"{}");
    assert_eq!(
        receiver.seen()[0].headers["content-type"],
        "application/json"
    );
}

#[tokio::test]
async fn a_receiver_that_is_slow_or_gone_is_a_timeout_or_a_network_failure() {
    let w = world().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::Slow(Duration::from_secs(5))]);
    let sender = GuardedSender::with_factory(w.store.clone(), OpenFactory);

    let started = std::time::Instant::now();
    assert_eq!(
        sender
            .send(&request(&w, &receiver.url, Duration::from_secs(1)))
            .await
            .map(|_| ()),
        Err(SendFailure::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(3));

    // A port nobody listens on.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        format!("http://127.0.0.1:{port}/hook")
    };
    assert_eq!(
        sender
            .send(&request(&w, &closed, Duration::from_secs(2)))
            .await
            .map(|_| ()),
        Err(SendFailure::Network)
    );
    let _ = Guarded;
}
