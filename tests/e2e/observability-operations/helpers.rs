//! What the stories of this suite share: an engine as a process, an endpoint that receives signed
//! webhooks, a sender that reaches a receiver on this machine, and a poll with a deadline.
#![allow(dead_code)]

use std::time::Duration;

use reqwest::Method;
use sagat_application::outbound::{OutboundRequest, OutboundResponse, SendFailure, WebhookSender};
use serde_json::{Value, json};

use crate::support::receiver::LoopbackSender;
use crate::support::worker::{Platform, RunningEngine, engine_env};

pub const SECRET: &str = "whsec-0123456789abcdef0123456789abcdef";

/// An engine started the way the binary is, with loops that look often.
pub async fn engine(p: &Platform, extra: &[(&str, &str)]) -> RunningEngine {
    let ring = format!("k1:{}", p.api.key_k1);
    let mut env = vec![
        ("SAGAT_SECRETS_KEYS", ring.as_str()),
        ("SAGAT_EVENT_POLL_MS", "50"),
        ("SAGAT_DELIVERY_POLL_MS", "50"),
    ];
    env.extend_from_slice(extra);
    RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &env)).await
}

/// Stores the signing secret and creates the endpoint `slug`; returns its id.
pub async fn endpoint(p: &Platform, slug: &str) -> String {
    let ns = p.namespace_id().await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/provider-signing"),
            Some(&json!({"value": SECRET})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/event-endpoints"),
            Some(&json!({"slug": slug, "secretName": "provider-signing"})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_owned()
}

/// What a provider sends to the public URL of an endpoint, with the headers `extra` adds.
pub async fn webhook(
    p: &Platform,
    endpoint: &str,
    event_id: &str,
    event_type: &str,
    body: &Value,
    extra: &[(&str, &str)],
) -> (u16, Value) {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let bytes = serde_json::to_vec(body).expect("json");
    let mut request = p
        .api
        .request(Method::POST, &format!("/webhooks/{endpoint}"))
        .header(
            "x-octopus-signature",
            crate::support::signed::inbound(SECRET, timestamp, event_id, event_type, &bytes),
        )
        .header("x-octopus-timestamp", timestamp.to_string())
        .header("x-octopus-event-id", event_id)
        .header("x-octopus-event-type", event_type)
        .header("content-type", "application/json");
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    let response = request.body(bytes).send().await.expect("send");
    let status = response.status().as_u16();
    (status, crate::support::api::json_of(response).await)
}

/// Waits for `check` to return something, polling.
pub async fn within<T, F, Fut>(what: &str, seconds: u64, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what} did not happen within {seconds} s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Sends to the local receiver whatever the subscription's address, after checking the address.
pub struct Standing {
    pub inner: LoopbackSender,
    pub receiver: String,
}

impl WebhookSender for Standing {
    async fn send(&self, request: &OutboundRequest) -> Result<OutboundResponse, SendFailure> {
        assert_eq!(request.url.as_str(), "https://hooks.example.com/in");
        let mut redirected = request.clone();
        redirected.url = sagat_domain::outbound::TargetUrl::parse(&self.receiver).expect("url");
        self.inner.send(&redirected).await
    }
}
