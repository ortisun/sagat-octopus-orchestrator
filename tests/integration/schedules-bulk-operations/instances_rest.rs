//! What the API does while its process may not serve (`T018-080`, `FR-018-052`): a closed gate refuses every call but the
//! liveness probe with `503 UNAVAILABLE` and `details.reason = BINARY_CAPABILITY_MISSING`, before authentication and before
//! anything is written; an open one serves as it always did; a stream that is open when the gate closes ends with a
//! `reconnect` event. The gate is the one a process's membership in the registry decides; here the test closes it
//! (`instances_process.rs` proves the membership closes it).
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use reqwest::Method;
use sagat_application::instances::ServingGate;
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::worker::{HUMAN, TENANT, chain_ir, platform_with};

fn gated(gate: &Arc<ServingGate>) -> Options {
    Options {
        serving_gate: Some(Arc::clone(gate)),
        ..Options::default()
    }
}

#[tokio::test]
async fn a_closed_gate_refuses_every_call_but_liveness_before_it_authenticates_or_writes_and_an_open_one_serves()
 {
    let gate = Arc::new(ServingGate::closed());
    gate.open(3);
    let p = platform_with(gated(&gate)).await;
    let served = p.api.get_as("/namespaces", HUMAN, TENANT).await;
    assert_eq!(served.status(), 200, "open: it serves as it always did");

    gate.close(4, vec!["schedules-v1".to_owned()]);
    // an authenticated call
    let refused = p.api.get_as("/namespaces", HUMAN, TENANT).await;
    assert_eq!(refused.status(), 503);
    assert_eq!(refused.headers()["retry-after"], "10");
    let body: Value = refused.json().await.expect("an error envelope");
    assert_eq!(body["code"], "UNAVAILABLE", "{body}");
    assert_eq!(body["details"]["reason"], "BINARY_CAPABILITY_MISSING");
    assert_eq!(body["retryable"], true);
    assert!(
        body["remediation"]
            .as_str()
            .is_some_and(|text| text.contains("The service cannot serve this call now")),
        "{body}"
    );
    // an unauthenticated one: the gate answers before the token is looked at
    let anonymous = p
        .api
        .request(Method::GET, "/auth/context")
        .send()
        .await
        .expect("send");
    assert_eq!(anonymous.status(), 503);
    // a write is refused before it writes
    let workspaces_before: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
    let write = p
        .api
        .post_as(
            "/workspaces",
            HUMAN,
            TENANT,
            &json!({"slug": "while-closed"}),
        )
        .await;
    assert_eq!(write.status(), 503);
    let workspaces_after: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
    assert_eq!(workspaces_before, workspaces_after);
    // readiness fails with the reason; liveness does not
    let ready = p
        .api
        .request(Method::GET, "/ready")
        .send()
        .await
        .expect("send");
    assert_eq!(ready.status(), 503);
    let ready: Value = ready.json().await.expect("envelope");
    assert_eq!(ready["details"]["reason"], "BINARY_CAPABILITY_MISSING");
    let health = p
        .api
        .request(Method::GET, "/health")
        .send()
        .await
        .expect("send");
    assert_eq!(
        health.status(),
        200,
        "alive: a restart would only be refused"
    );

    gate.open(5);
    assert_eq!(
        p.api.get_as("/namespaces", HUMAN, TENANT).await.status(),
        200
    );
}

#[tokio::test]
async fn a_stream_that_is_open_when_the_gate_closes_ends_with_a_reconnect() {
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    let p = platform_with(gated(&gate)).await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = p.start(&workflow, &json!({"item": "book"})).await;
    let mut stream = p
        .api
        .as_user(Method::GET, &format!("/runs/{run}/stream"), HUMAN, TENANT)
        .header("accept", "text/event-stream")
        .send()
        .await
        .expect("open the stream");
    assert_eq!(stream.status(), 200);
    let first = stream
        .chunk()
        .await
        .expect("a chunk")
        .expect("the first event");
    assert!(
        !String::from_utf8_lossy(&first).contains("reconnect"),
        "the stream is following the run"
    );

    gate.close(2, vec!["schedules-v1".to_owned()]);
    let ended = crate::support::bounded::within(
        "the stream ends with a reconnect after the gate closed",
        std::time::Duration::from_secs(60),
        async {
            let mut seen = String::new();
            while let Some(chunk) = stream.chunk().await.expect("the stream goes on") {
                seen.push_str(&String::from_utf8_lossy(&chunk));
                if seen.contains("event: reconnect") {
                    return seen;
                }
            }
            seen
        },
    )
    .await;
    assert!(ended.contains("event: reconnect"), "{ended}");
}

#[tokio::test]
async fn the_upload_route_of_run_inputs_obeys_the_gate_as_the_rest_of_the_api_does() {
    use sagat_application::blobs::BlobLimits;
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    let p = platform_with(Options {
        blobs: Some(BlobLimits::default()),
        serving_gate: Some(Arc::clone(&gate)),
        ..Options::default()
    })
    .await;
    let serving = p
        .api
        .request(Method::POST, "/run-input-blobs")
        .send()
        .await
        .expect("send");
    assert_eq!(
        serving.status(),
        401,
        "open: the call meets its authentication, which it lacks"
    );
    gate.close(2, vec!["schedules-v1".to_owned()]);
    let refused = p
        .api
        .request(Method::POST, "/run-input-blobs")
        .send()
        .await
        .expect("send");
    assert_eq!(refused.status(), 503);
    let body: Value = refused.json().await.expect("an error envelope");
    assert_eq!(
        body["details"]["reason"], "BINARY_CAPABILITY_MISSING",
        "{body}"
    );
}

#[tokio::test]
async fn a_gate_that_is_closed_for_want_of_a_registration_says_nothing_of_a_capability() {
    // the process has not registered yet (the registry cannot be asked, the migration was not run): the plain 503
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    let p = platform_with(gated(&gate)).await;
    gate.close(2, Vec::new());
    let refused = p.api.get_as("/namespaces", HUMAN, TENANT).await;
    assert_eq!(refused.status(), 503);
    let body: Value = refused.json().await.expect("an error envelope");
    assert_eq!(body["code"], "UNAVAILABLE", "{body}");
    assert!(
        body["details"]["reason"].is_null(),
        "no capability is missing, so none is named: {body}"
    );
}

#[tokio::test]
async fn a_registry_that_has_been_silent_for_too_long_fails_readiness_and_leaves_the_calls_to_the_gate()
 {
    use std::time::{Duration, Instant};
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    gate.heard();
    let p = platform_with(gated(&gate)).await;
    assert_eq!(
        p.api
            .request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200,
        "the registry answered a moment ago"
    );
    // thirty-one seconds without an answer: not ready, and the plain 503 of every other cause
    gate.heard_at(Instant::now() - Duration::from_secs(31));
    let ready = p
        .api
        .request(Method::GET, "/ready")
        .send()
        .await
        .expect("send");
    assert_eq!(ready.status(), 503);
    let body: Value = ready.json().await.expect("an error envelope");
    assert!(body["details"]["reason"].is_null(), "{body}");
    // the gate is its own rule: the calls are still served
    assert_eq!(
        p.api.get_as("/namespaces", HUMAN, TENANT).await.status(),
        200
    );
    // an answer ends it
    gate.heard();
    assert_eq!(
        p.api
            .request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );
}

#[tokio::test]
async fn every_operation_of_the_contract_that_this_server_mounts_is_refused_by_a_closed_gate_but_liveness()
 {
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    let p = platform_with(gated(&gate)).await;
    gate.close(2, vec!["schedules-v1".to_owned()]);
    let contract: Value = serde_saphyr::from_str(include_str!(
        "../../../specs/001-platform-foundation/contracts/openapi.yaml"
    ))
    .expect("the contract");
    let mut refused = 0;
    let mut unmounted = Vec::new();
    for (path, item) in contract["paths"].as_object().expect("paths") {
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            if operation.get("x-planned").is_some() {
                continue;
            }
            // every parameter of the path is a value that names nothing: the gate answers before anything is looked up
            let mut concrete = path.clone();
            while let (Some(open), Some(close)) = (concrete.find('{'), concrete.find('}')) {
                concrete.replace_range(open..=close, "00000000-0000-0000-0000-000000000000");
            }
            let verb =
                reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).expect("a method");
            let response = p.api.request(verb, &concrete).send().await.expect("send");
            let status = response.status().as_u16();
            let body: Value = response.json().await.unwrap_or(Value::Null);
            if path == "/health" {
                assert_eq!(status, 200, "liveness is never refused");
            } else if status == 404 && body["details"]["reason"] == "ROUTE_NOT_FOUND" {
                // a route this harness does not mount (its feature is off here): not served, so not gated either
                unmounted.push(format!("{method} {path}"));
            } else {
                assert_eq!(
                    status, 503,
                    "{method} {path} answered {status} with the gate closed: a route outside the gated layer? {body}"
                );
                refused += 1;
            }
        }
    }
    assert!(
        refused >= 100,
        "only {refused} operations were refused; unmounted: {unmounted:?}"
    );
}

#[tokio::test]
async fn a_task_stream_that_is_open_when_the_gate_closes_leases_nothing_after_it() {
    use tonic::Code;
    let gate = Arc::new(ServingGate::closed());
    gate.open(1);
    let p = platform_with(gated(&gate)).await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let mut worker = p.worker("gated-stream-worker").await;
    worker.register_version(&["act.a", "act.b"], "1.4").await;
    // a run exists whose start is delayed: its first task is not ready until the scheduler releases it
    let (status, started) = p
        .rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": workflow, "input": {"item": "book"}, "startDelayMs": 1200})),
        )
        .await;
    assert_eq!(status, 202, "{started}");
    let run = started["runId"].as_str().expect("run id").to_owned();
    // a stream opened while the replica may serve, with nothing to lease yet
    let mut stream = worker
        .client
        .stream_tasks(worker.req(worker.poll_message(1)))
        .await
        .expect("open the stream")
        .into_inner();
    gate.close(2, vec!["schedules-v1".to_owned()]);
    // the delay ends and the scheduler releases the run: its first task becomes ready after the gate closed, and the stream,
    // still inside its window, must not lease it
    crate::support::eventually("the scheduler releases the run", || async {
        p.tick().await;
        let released: bool =
            sqlx::query_scalar("SELECT status <> 'WAITING' FROM runs WHERE id = $1::uuid")
                .bind(&run)
                .fetch_one(&p.api.db.pool)
                .await
                .expect("status");
        released.then_some(())
    })
    .await;
    let refused = crate::support::bounded::within(
        "the stream answers",
        std::time::Duration::from_secs(30),
        stream.message(),
    )
    .await
    .expect_err("a replica that may not serve leases no task");
    assert_eq!(refused.code(), Code::Unavailable, "{refused:?}");
    assert_eq!(
        refused
            .metadata()
            .get("x-sagat-error")
            .and_then(|value| value.to_str().ok()),
        Some("BINARY_CAPABILITY_MISSING")
    );
    let leased: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE run_id = $1::uuid AND status IN ('LEASED', 'RUNNING')",
    )
    .bind(&run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("steps");
    assert_eq!(leased, 0, "nothing was leased");
}
