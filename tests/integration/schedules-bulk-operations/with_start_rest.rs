//! Signal-with-Start over HTTP (`T018-040`, `FR-018-024`, `FR-018-026`): `POST /runs/signal-with-start`
//! signals the open run of a business identifier or starts one and delivers the signal at creation,
//! and says which; the refusals are typed and nothing is stored when it refuses. A real API, a real
//! PostgreSQL, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::engine::{activity, document, wait_node, with};
use crate::support::worker::{Platform, chain_ir, platform};

struct Rig {
    p: Platform,
    waiter: String,
    plain: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let waiter = p
        .publish(
            "approvals",
            &document(
                "approvals",
                "w",
                vec![
                    with(
                        wait_node("w", "payment.confirmed", "input.k", 300_000, None),
                        "next",
                        json!("notify"),
                    ),
                    activity("notify"),
                ],
            ),
        )
        .await;
    let plain = p.publish("orders", &chain_ir("orders")).await;
    Rig { p, waiter, plain }
}

impl Rig {
    async fn go(&self, workflow: &str, extra: Value) -> (u16, Value) {
        let mut body = json!({
            "workflowId": workflow,
            "businessId": "cart-http-1",
            "input": {"k": "s"},
            "signal": {"name": "payment.confirmed", "payload": {"item": "book"}}
        });
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        self.p
            .rest(Method::POST, "/runs/signal-with-start", Some(&body))
            .await
    }

    async fn runs(&self) -> Vec<Value> {
        let (status, page) = self.p.rest(Method::GET, "/runs", None).await;
        assert_eq!(status, 200, "{page}");
        page["items"].as_array().expect("items").clone()
    }
}

fn reason(error: &Value) -> &str {
    error["details"]["reason"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn an_absent_identifier_starts_the_run_and_the_signal_is_delivered_at_creation() {
    let r = rig().await;

    let (status, done) = r.go(&r.waiter, json!({})).await;

    assert_eq!(status, 202, "{done}");
    assert_eq!(done["started"], true);
    assert_eq!(done["deliveredTo"], json!(["w"]));
    let run = done["runId"].as_str().expect("id").to_owned();
    let (_, read) = r.p.rest(Method::GET, &format!("/runs/{run}"), None).await;
    assert_eq!(read["businessId"], "cart-http-1");
}

#[tokio::test]
async fn the_same_key_replays_the_answer_and_another_body_under_it_is_another_request() {
    let r = rig().await;
    let body = json!({
        "workflowId": r.waiter,
        "businessId": "cart-http-2",
        "input": {"k": "s"},
        "signal": {"name": "payment.confirmed", "payload": {"n": 1}}
    });
    let send = |body: &Value| {
        r.p.api
            .as_user(
                Method::POST,
                "/runs/signal-with-start",
                crate::support::worker::HUMAN,
                crate::support::worker::TENANT,
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "with-start-key-0001")
            .json(body)
            .send()
    };

    let first = send(&body).await.expect("send");
    assert_eq!(first.status(), 202);
    let first: Value = first.json().await.expect("json");
    let again = send(&body).await.expect("send");
    assert_eq!(again.status(), 202);
    let again: Value = again.json().await.expect("json");
    assert_eq!(again, first, "the answer of the first request");
    assert_eq!(r.runs().await.len(), 1);

    let mut other = body.clone();
    other["signal"]["payload"] = json!({"n": 2});
    let refused = send(&other).await.expect("send");
    assert_eq!(refused.status(), 409);
}

#[tokio::test]
async fn an_open_run_that_waits_for_nothing_is_a_conflict_with_the_reason_and_nothing_changes() {
    let r = rig().await;
    r.go(&r.waiter, json!({})).await;
    let before = r.runs().await;

    let (status, refused) = r.go(&r.waiter, json!({})).await;

    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["code"], "CONFLICT");
    assert_eq!(reason(&refused), "NOT_WAITING");
    assert_eq!(r.runs().await, before);
}

#[tokio::test]
async fn an_entry_that_does_not_wait_for_the_signal_creates_nothing() {
    let r = rig().await;

    let (status, refused) = r.go(&r.plain, json!({})).await;

    assert_eq!(status, 409, "{refused}");
    assert_eq!(reason(&refused), "SIGNAL_NOT_ACCEPTED_AT_START");
    assert!(r.runs().await.is_empty(), "no run was created");
    // and the identifier is free
    let (status, run) =
        r.p.rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": r.plain, "input": {}, "businessId": "cart-http-1"})),
        )
        .await;
    assert_eq!(status, 202, "{run}");
}

#[tokio::test]
async fn a_retry_after_the_caller_lost_the_right_is_refused_and_not_answered_again() {
    let r = rig().await;
    r.p.grant(r.p.tenant, "sam", sagat_domain::identity::Role::Operator)
        .await;
    let body = json!({
        "workflowId": r.waiter,
        "businessId": "cart-http-9",
        "input": {"k": "s"},
        "signal": {"name": "payment.confirmed", "payload": {"n": 1}}
    });
    let send = || {
        r.p.api
            .as_user(
                Method::POST,
                "/runs/signal-with-start",
                "sam",
                crate::support::worker::TENANT,
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "with-start-key-0011")
            .json(&body)
            .send()
    };

    assert_eq!(send().await.expect("send").status(), 202);
    r.p.demote("sam").await;
    let retried = send().await.expect("send");

    assert_eq!(
        retried.status(),
        403,
        "a replay is judged by the right the caller has now"
    );
    assert_eq!(r.runs().await.len(), 1, "and nothing more was done");
}

#[tokio::test]
async fn a_delay_cannot_go_with_a_signal_and_the_request_is_checked_in_words() {
    let r = rig().await;

    let (status, refused) = r.go(&r.waiter, json!({"startDelayMs": 60_000})).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "START_DELAY_WITH_SIGNAL");

    for body in [
        json!({"businessId": ""}),
        json!({"businessId": "has a space"}),
        json!({"businessIdReusePolicy": "SOMETIMES"}),
        json!({"signal": {"name": "", "payload": {}}}),
    ] {
        let (status, refused) = r.go(&r.waiter, body.clone()).await;
        assert_eq!(status, 422, "{body}: {refused}");
    }
    assert!(r.runs().await.is_empty(), "nothing was started");

    // a business identifier and a signal are both required
    let (status, missing) =
        r.p.rest(
            Method::POST,
            "/runs/signal-with-start",
            Some(&json!({"workflowId": r.waiter, "input": {}})),
        )
        .await;
    assert_eq!(status, 422, "{missing}");
}
