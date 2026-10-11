//! US04 — deliver an outbound webhook: a run finishes, the receiver is unavailable at first, and the
//! platform retries with backoff and records every attempt (US04-AC01).
//!
//! The delivery worker here is the production one over the API's database, with a sender that
//! reaches a receiver on this machine: the receiver stands where `hooks.example.com` would, and the
//! sender checks that is the address it was asked for. (The guarded client itself, which no policy
//! lets reach a loopback address, is tested on its own.)

use reqwest::Method;
use sagat_application::entropy::SystemEntropy;
use sagat_application::outbound::{OutboundRequest, OutboundResponse, SendFailure, WebhookSender};
use sagat_scheduler::{DeliverySettings, DeliveryWorker};
use serde_json::json;

use crate::SECRET;
use crate::support::receiver::{LoopbackSender, Receiver, Reply};
use crate::support::worker::{platform, single_ir};

/// Sends to the local receiver whatever the subscription's address, after checking the address.
struct Standing {
    inner: LoopbackSender,
    receiver: String,
}

impl WebhookSender for Standing {
    async fn send(&self, request: &OutboundRequest) -> Result<OutboundResponse, SendFailure> {
        assert_eq!(
            request.url.as_str(),
            "https://hooks.example.com/in",
            "the address the subscription names"
        );
        let mut redirected = request.clone();
        redirected.url = sagat_domain::outbound::TargetUrl::parse(&self.receiver).expect("url");
        self.inner.send(&redirected).await
    }
}

#[tokio::test]
async fn us04_a_finished_run_is_delivered_after_the_receiver_comes_back_and_every_attempt_is_recorded()
 {
    let p = platform().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::Status(503), Reply::Status(503), Reply::Status(200)]);
    let ns = p.namespace_id().await;
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/provider-signing"),
            json!({"value": SECRET}),
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/egress-policy"),
            json!({"rules": ["hooks.example.com"]}),
        ),
    ] {
        let (status, answer) = p.rest(method, &path, Some(&body)).await;
        assert_eq!(status, 200, "{answer}");
    }
    let (status, subscription) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/webhook-subscriptions"),
            Some(
                &json!({"name": "finance", "url": "https://hooks.example.com/in",
                         "secretName": "provider-signing", "events": ["run.finished"]}),
            ),
        )
        .await;
    assert_eq!(status, 201, "{subscription}");
    let id = subscription["id"].as_str().expect("id").to_owned();

    // A run that finishes.
    let workflow = p
        .publish("close-books", &single_ir("close-books", None))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("wendy").await;
    worker.register(&["act.a"]).await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"closed": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");

    let worker = DeliveryWorker::new(
        p.api.executions.clone(),
        Standing {
            inner: LoopbackSender::new(),
            receiver: receiver.url.clone(),
        },
        SystemEntropy,
        p.api.keys.cipher(),
        DeliverySettings::default(),
    );
    // Attempt one and two meet a receiver that is unavailable; each waits longer than the one before.
    for expected in [1, 2] {
        assert_eq!(worker.tick().await.retried, 1);
        let (_, page) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/webhook-subscriptions/{id}/deliveries"),
                None,
            )
            .await;
        assert_eq!(page["items"][0]["status"], "PENDING");
        assert_eq!(page["items"][0]["attempts"], expected);
        assert_eq!(page["items"][0]["lastHttpStatus"], 503);
        assert!(
            !worker.tick().await.did_work(),
            "and nothing is sent again before its time"
        );
        sqlx::query("UPDATE webhook_deliveries SET next_attempt_at = clock_timestamp()")
            .execute(&p.api.db.pool)
            .await
            .expect("time passes");
        sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity'")
            .execute(&p.api.db.pool)
            .await
            .expect("slots");
    }
    assert_eq!(worker.tick().await.delivered, 1);

    // Every attempt is on record, with what the receiver said, and the receiver saw one delivery id.
    let (status, page) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/webhook-subscriptions/{id}/deliveries?status=DELIVERED"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    let delivery = page["items"][0]["id"]
        .as_str()
        .expect("delivery")
        .to_owned();
    let (status, detail) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/webhook-subscriptions/{id}/deliveries/{delivery}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{detail}");
    let attempts: Vec<(i64, &str, Option<i64>)> = detail["attemptsMade"]
        .as_array()
        .expect("attempts")
        .iter()
        .map(|a| {
            (
                a["attempt"].as_i64().expect("n"),
                a["outcome"].as_str().expect("outcome"),
                a["httpStatus"].as_i64(),
            )
        })
        .collect();
    assert_eq!(
        attempts,
        vec![
            (1, "SERVER_ERROR", Some(503)),
            (2, "SERVER_ERROR", Some(503)),
            (3, "DELIVERED", Some(200))
        ]
    );
    assert_eq!(detail["payload"]["type"], "run.finished");
    assert_eq!(detail["payload"]["data"]["runId"], run);
    let seen = receiver.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|s| s.headers["x-octopus-delivery-id"] == delivery)
    );
    let last = &seen[2];
    assert!(
        crate::support::signed::delivery_verifies(SECRET, &last.headers, &last.body),
        "the receiver can verify what it was sent"
    );
}

#[tokio::test]
async fn a_reader_sees_where_a_subscription_goes_but_not_the_path_that_may_be_the_credential() {
    use sagat_domain::identity::Role;
    let p = platform().await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let ns = p.namespace_id().await;
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/provider-signing"),
            json!({"value": SECRET}),
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/egress-policy"),
            json!({"rules": ["hooks.example.com"]}),
        ),
    ] {
        let (status, answer) = p.rest(method, &path, Some(&body)).await;
        assert_eq!(status, 200, "{answer}");
    }
    // Chat tools hand out incoming-webhook URLs whose path is the secret.
    let url = "https://hooks.example.com/services/T000/B000/xoxb-the-credential?token=also";
    let (status, created) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/webhook-subscriptions"),
            Some(
                &json!({"name": "chat", "url": url, "secretName": "provider-signing",
                         "events": ["run.finished"]}),
            ),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(
        created["url"], url,
        "the administrator who set it up sees it all"
    );
    let id = created["id"].as_str().expect("id").to_owned();

    for path in [
        format!("/namespaces/{ns}/webhook-subscriptions/{id}"),
        format!("/namespaces/{ns}/webhook-subscriptions"),
    ] {
        let (status, seen) = p.rest_as("vera", Method::GET, &path, None).await;
        assert_eq!(status, 200, "{seen}");
        let text = seen.to_string();
        assert!(
            text.contains("https://hooks.example.com/"),
            "a reader still sees where it goes: {text}"
        );
        for secret in ["xoxb-the-credential", "T000", "token=also", "services"] {
            assert!(
                !text.contains(secret),
                "a reader must not see {secret}: {text}"
            );
        }
    }
}
