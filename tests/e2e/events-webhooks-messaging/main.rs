//! The four user stories of feature 004 through the public interfaces: a signed webhook starts a
//! workflow, an event wakes a suspended run, messages from Kafka, SQS and NATS become events, and a
//! finished run is delivered to a subscriber that is at first unavailable. REST for management, the
//! engine as a process for the loops, real PostgreSQL, real brokers in containers.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod telemetry;
mod us01;
mod us02;
mod us03;
mod us04;

use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};

use support::worker::{Platform, RunningEngine, engine_env};

pub const SECRET: &str = "whsec-0123456789abcdef0123456789abcdef";

/// An engine started the way the binary is, with the key ring of the API (so it can open the secrets
/// the API stored) and loops that look often.
pub async fn engine(p: &Platform, extra: &[(&str, &str)]) -> RunningEngine {
    let ring = format!("k1:{}", p.api.key_k1);
    let mut env = vec![
        ("SAGAT_SECRETS_KEYS", ring.as_str()),
        ("SAGAT_EVENT_POLL_MS", "50"),
        ("SAGAT_DELIVERY_POLL_MS", "50"),
        ("SAGAT_CONNECTOR_REFRESH_SECS", "1"),
    ];
    env.extend_from_slice(extra);
    RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &env)).await
}

/// Stores the signing secret and creates the endpoint `slug`; returns the id of the endpoint.
pub async fn endpoint(p: &Platform, slug: &str, extra: Value) -> String {
    let ns = p.namespace_id().await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/provider-signing"),
            Some(&json!({"value": SECRET})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut request = json!({"slug": slug, "secretName": "provider-signing"});
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            request[key] = value.clone();
        }
    }
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/event-endpoints"),
            Some(&request),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_owned()
}

/// What a provider sends to the public URL of an endpoint.
pub async fn webhook(
    p: &Platform,
    endpoint: &str,
    event_id: &str,
    event_type: &str,
    body: &Value,
    secret: &str,
) -> (u16, Value) {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let bytes = serde_json::to_vec(body).expect("json");
    let response = p
        .api
        .request(Method::POST, &format!("/webhooks/{endpoint}"))
        .header(
            "x-octopus-signature",
            support::signed::inbound(secret, timestamp, event_id, event_type, &bytes),
        )
        .header("x-octopus-timestamp", timestamp.to_string())
        .header("x-octopus-event-id", event_id)
        .header("x-octopus-event-type", event_type)
        .header("content-type", "application/json")
        .body(bytes)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    (status, support::api::json_of(response).await)
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

/// The runs of a workflow, as the API lists them.
pub async fn runs_of(p: &Platform, workflow: &str) -> Vec<Value> {
    let (status, page) = p
        .rest(
            Method::GET,
            &format!("/runs?workflowId={workflow}&limit=100"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    page["items"].as_array().expect("items").clone()
}
