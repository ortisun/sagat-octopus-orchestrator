//! Update-with-Start over HTTP (`T018-041`, `FR-018-025`, `FR-018-026`): `POST /runs/update-with-start`
//! applies a declared Update to the open run of a business identifier or starts one and applies it, says
//! which, never lets the result be stored, and refuses in typed words with nothing created. A real API, a
//! real PostgreSQL, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::engine::declared_waiter;
use crate::support::worker::{HUMAN, Platform, TENANT, platform};

struct Rig {
    p: Platform,
    workflow: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let workflow = p.publish("accounts", &declared_waiter("accounts")).await;
    Rig { p, workflow }
}

fn allowed(balance: i64) -> Value {
    json!({"allowed": true, "nextState": {"balance": balance}})
}

impl Rig {
    fn body(&self, id: &str, update_key: &str, args: Value) -> Value {
        json!({
            "workflowId": self.workflow,
            "businessId": id,
            "input": {"account": {"balance": 10}, "k": "s"},
            "update": {"name": "balance.replace", "updateKey": update_key, "args": args}
        })
    }

    async fn go(&self, body: &Value) -> (u16, Value) {
        self.p
            .rest(Method::POST, "/runs/update-with-start", Some(body))
            .await
    }

    async fn runs(&self) -> Vec<Value> {
        let (status, page) = self.p.rest(Method::GET, "/runs", None).await;
        assert_eq!(status, 200, "{page}");
        page["items"].as_array().expect("items").clone()
    }
}

#[tokio::test]
async fn an_absent_identifier_starts_the_run_and_the_update_is_applied_with_a_receipt() {
    let r = rig().await;

    let response =
        r.p.api
            .as_user(Method::POST, "/runs/update-with-start", HUMAN, TENANT)
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "update-start-key-0001")
            .json(&r.body("acct-http-1", "update-key-0001", allowed(25)))
            .send()
            .await
            .expect("send");

    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
        "the result is business data: it is never stored"
    );
    let done: Value = response.json().await.expect("json");
    assert_eq!(done["started"], true);
    assert_eq!(done["update"]["outcome"], "APPLIED");
    let receipt = &done["update"]["receipt"];
    assert_eq!(receipt["status"], "APPLIED");
    assert_eq!(receipt["result"], 25);
    assert_eq!(receipt["beforeStateRevision"], 0);
    assert_eq!(receipt["afterStateRevision"], 1);
    assert_eq!(receipt["replayed"], false);
    assert_eq!(receipt["runId"], done["runId"]);
    let run = done["runId"].as_str().expect("id");
    let (_, read) = r.p.rest(Method::GET, &format!("/runs/{run}"), None).await;
    assert_eq!(read["businessId"], "acct-http-1");
}

#[tokio::test]
async fn the_same_key_answers_again_and_another_update_under_it_is_another_request() {
    let r = rig().await;
    let send = |body: &Value| {
        r.p.api
            .as_user(Method::POST, "/runs/update-with-start", HUMAN, TENANT)
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "update-start-key-0002")
            .json(body)
            .send()
    };
    let body = r.body("acct-http-2", "update-key-0002", allowed(25));

    let first: Value = send(&body).await.expect("send").json().await.expect("json");
    let again = send(&body).await.expect("send");
    assert_eq!(again.status(), 200);
    let again: Value = again.json().await.expect("json");

    assert_eq!(again["runId"], first["runId"]);
    assert_eq!(again["started"], true, "what the first request did");
    assert_eq!(again["update"]["receipt"]["replayed"], true);
    assert_eq!(again["update"]["receipt"]["result"], 25);
    assert_eq!(r.runs().await.len(), 1);

    let other = r.body("acct-http-2", "update-key-0002", allowed(26));
    assert_eq!(send(&other).await.expect("send").status(), 409);
}

#[tokio::test]
async fn a_business_rejection_is_an_answer_and_the_run_stays_started() {
    let r = rig().await;

    let (status, done) = r
        .go(&r.body(
            "acct-http-3",
            "update-key-0003",
            json!({"allowed": false, "nextState": {"balance": 25}}),
        ))
        .await;

    assert_eq!(status, 200, "{done}");
    assert_eq!(done["started"], true);
    assert_eq!(done["update"]["outcome"], "REJECTED");
    assert_eq!(done["update"]["reason"], "UPDATE_REJECTED");
    assert!(done["update"].get("receipt").is_none());
    assert_eq!(r.runs().await.len(), 1, "the run was created and stays");
}

#[tokio::test]
async fn an_open_holder_gets_the_update_and_a_fail_policy_makes_it_a_conflict() {
    let r = rig().await;
    let (_, first) = r
        .go(&r.body("acct-http-4", "update-key-0004", allowed(25)))
        .await;

    let (status, again) = r
        .go(&r.body("acct-http-4", "update-key-0005", allowed(40)))
        .await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["started"], false);
    assert_eq!(again["runId"], first["runId"]);
    assert_eq!(again["update"]["receipt"]["afterStateRevision"], 2);
    assert_eq!(r.runs().await.len(), 1);

    let mut failing = r.body("acct-http-4", "update-key-0006", allowed(50));
    failing["businessIdConflictPolicy"] = json!("FAIL");
    let (status, refused) = r.go(&failing).await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["details"]["reason"], "BUSINESS_ID_IN_USE");
}

#[tokio::test]
async fn what_the_declaration_refuses_creates_nothing_and_says_what_updaterun_says() {
    let r = rig().await;

    let mut undeclared = r.body("acct-http-5", "update-key-0007", allowed(25));
    undeclared["update"]["name"] = json!("balance.unknown");
    let (status, refused) = r.go(&undeclared).await;
    assert_eq!(status, 404, "{refused}");

    let (status, refused) = r
        .go(&r.body("acct-http-5", "update-key-0008", json!({"allowed": "yes"})))
        .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(refused["details"]["reason"], "INVALID_ARGUMENTS");

    let mut key = r.body("acct-http-5", "has a space", allowed(25));
    key["update"]["updateKey"] = json!("has a space");
    let (status, refused) = r.go(&key).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(refused["details"]["reason"], "INVALID_UPDATE_KEY");

    assert!(r.runs().await.is_empty(), "nothing was started");
}

#[tokio::test]
async fn the_request_is_checked_in_words() {
    let r = rig().await;

    for body in [
        json!({"businessId": ""}),
        json!({"businessId": "has a space"}),
        json!({"businessIdReusePolicy": "SOMETIMES"}),
        json!({"businessIdConflictPolicy": "SOMETIMES"}),
    ] {
        let mut sent = r.body("acct-http-6", "update-key-0009", allowed(25));
        for (key, value) in body.as_object().expect("an object") {
            sent[key] = value.clone();
        }
        let (status, refused) = r.go(&sent).await;
        assert_eq!(status, 422, "{body}: {refused}");
    }
    // a business identifier and an update are both required
    let (status, missing) = r
        .go(&json!({"workflowId": r.workflow, "input": {}, "update": {"name": "x", "updateKey": "y", "args": {}}}))
        .await;
    assert_eq!(status, 422, "{missing}");
    assert!(r.runs().await.is_empty(), "nothing was started");
}

#[tokio::test]
async fn a_start_delay_is_accepted_and_the_run_waits_with_its_update_applied() {
    let r = rig().await;
    let mut body = r.body("acct-http-7", "update-key-0010", allowed(25));
    body["startDelayMs"] = json!(3_600_000);

    let (status, done) = r.go(&body).await;

    assert_eq!(status, 200, "{done}");
    assert_eq!(done["update"]["outcome"], "APPLIED");
    let run = done["runId"].as_str().expect("id");
    let (_, read) = r.p.rest(Method::GET, &format!("/runs/{run}"), None).await;
    assert_eq!(read["status"], "WAITING");
    assert!(read["startDelayedUntil"].is_string(), "{read}");
}

#[tokio::test]
async fn a_retry_after_the_caller_lost_the_right_is_refused_and_not_answered_again() {
    let r = rig().await;
    r.p.grant(r.p.tenant, "sam", sagat_domain::identity::Role::Operator)
        .await;
    let body = r.body("acct-http-9", "update-key-0011", allowed(25));
    let send = || {
        r.p.api
            .as_user(Method::POST, "/runs/update-with-start", "sam", TENANT)
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "update-start-key-0011")
            .json(&body)
            .send()
    };

    assert_eq!(send().await.expect("send").status(), 200);
    r.p.demote("sam").await;
    let retried = send().await.expect("send");

    assert_eq!(
        retried.status(),
        403,
        "a replay is judged by the right the caller has now"
    );
    assert_eq!(r.runs().await.len(), 1, "and nothing more was done");
}
