//! Faults around what the Studio asks the platform to do (feature 007): two people deciding the
//! same step at once, a request sent twice because the answer was lost, a decision made on a view
//! that has gone stale, a compensation that fails again, and a person of another tenant. The
//! Studio's own faults (a stream that drops, a token that expires, a draft saved over) are in the
//! user-story tests (`tests/e2e/007-workflow-studio`, US01 and US03) and in the journeys.
//!
//! What these hold the platform to: a decision is made once, whoever asks and however often; a
//! refusal changes nothing; nothing in the journal is overwritten.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

use std::time::Duration;

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use support::worker::{NAMESPACE, Platform, TENANT, Worker, platform};

const ACTIVITIES: &[&str] = &[
    "payment.validate",
    "limit.reserve",
    "payment.submit",
    "limit.release",
    "payment.return.request",
    "payment.lookup.status",
    "customer.notify",
];

fn saga() -> Value {
    let mut ir: Value =
        serde_json::from_str(include_str!("../../../examples/payment-saga.workflow.json"))
            .expect("the example");
    for node in ir["nodes"].as_array_mut().expect("nodes") {
        if node.get("compensationRetry").is_some() {
            node["compensationRetry"]["maxAttempts"] = json!(1);
        }
    }
    ir
}

/// A run whose release of the limit failed for good: it waits for a person.
async fn stuck(p: &Platform, w: &mut Worker<'_>) -> String {
    let wf = p.publish("corporate-payment", &saga()).await;
    let run = p
        .start(&wf, &json!({"paymentId": "p-1", "amount": 125.5}))
        .await;
    w.register(ACTIVITIES).await;
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task().await;
    w.fail(&submit, false).await.expect("declined");
    let release = w.one_task().await;
    w.fail_with(&release, "LIMIT_SERVICE_DOWN", true)
        .await
        .expect("the limit service is down");
    run
}

fn retry(reason: &str) -> Value {
    json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": reason})
}

async fn decide(p: &Platform, sub: &str, run: &str, body: &Value, key: &str) -> (u16, Value) {
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/operations"),
            sub,
            TENANT,
        )
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn graph(p: &Platform, run: &str) -> Value {
    let (status, body) = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn journal_types(p: &Platform, run: &str) -> Vec<String> {
    let (status, body) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=500"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|e| e["eventType"].as_str().map(str::to_owned))
        .collect()
}

fn count(types: &[String], wanted: &str) -> usize {
    types.iter().filter(|t| *t == wanted).count()
}

async fn ended(p: &Platform, run: &str, status: &str) {
    for _ in 0..100 {
        if graph(p, run).await["status"] == status {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "the run did not reach {status}: {}",
        graph(p, run).await["status"]
    );
}

#[tokio::test]
async fn two_operators_deciding_the_same_step_at_once_make_one_decision() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "oscar", Role::Operator).await;

    let (olivia, oscar) = (
        retry("the limit service is back"),
        retry("it was a blip, trying again"),
    );
    let (first, second) = tokio::join!(
        decide(&p, "olivia", &run, &olivia, "fault-007-olivia-1"),
        decide(&p, "oscar", &run, &oscar, "fault-007-oscar-1"),
    );
    let mut statuses = [first.0, second.0];
    statuses.sort_unstable();
    assert_eq!(statuses, [202, 409], "{first:?} {second:?}");
    let refused = if first.0 == 409 { &first.1 } else { &second.1 };
    assert_eq!(refused["code"], "CONFLICT", "{refused}");
    assert_eq!(refused["details"]["reason"], "STATE_CONFLICT", "{refused}");

    let types = journal_types(&p, &run).await;
    assert_eq!(count(&types, "step.retry_requested"), 1, "{types:?}");

    // The one decision is carried out once: a single new attempt.
    let release = w.one_task().await;
    assert_eq!(release.attempt, 2);
    w.complete(&release, &json!({})).await.expect("released");
    ended(&p, &run, "COMPENSATED").await;
}

#[tokio::test]
async fn a_decision_sent_again_because_the_answer_was_lost_is_the_same_decision() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let body = retry("the limit service is back");

    let (status, answer) = decide(&p, "olivia", &run, &body, "fault-007-retry-1").await;
    assert_eq!(status, 202, "{answer}");
    // The client never saw the answer and sends the very same request: the platform answers as it did, and does nothing more.
    let (again, replay) = decide(&p, "olivia", &run, &body, "fault-007-retry-1").await;
    assert_eq!(again, 202, "{replay}");
    assert_eq!(replay, answer);
    assert_eq!(
        count(&journal_types(&p, &run).await, "step.retry_requested"),
        1
    );

    // A person who decides again with a new key decides again: the step no longer waits, so it is refused.
    let (other, refusal) = decide(&p, "olivia", &run, &body, "fault-007-retry-2").await;
    assert_eq!(other, 409, "{refusal}");
    assert_eq!(refusal["details"]["reason"], "STATE_CONFLICT", "{refusal}");
    assert_eq!(
        count(&journal_types(&p, &run).await, "step.retry_requested"),
        1
    );

    // The same key with a different request is a mistake of the client, not a replay.
    let different = retry("a different reason for the same key");
    let (mismatch, text) = decide(&p, "olivia", &run, &different, "fault-007-retry-1").await;
    assert!(
        [409, 422].contains(&mismatch),
        "a key reused for another request: {mismatch} {text}"
    );
}

#[tokio::test]
async fn a_decision_made_on_a_view_that_went_stale_is_refused_and_changes_nothing() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let before = graph(&p, &run).await;
    let seen = before["projectionSeq"].as_u64().expect("projectionSeq");

    let mut stale = retry("the limit service is back");
    stale["expectedRunVersion"] = json!(seen - 1);
    let (status, body) = decide(&p, "olivia", &run, &stale, "fault-007-stale-1").await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["details"]["reason"], "REVISION_CONFLICT", "{body}");
    assert_eq!(graph(&p, &run).await, before, "nothing moved");
    assert_eq!(
        count(&journal_types(&p, &run).await, "step.retry_requested"),
        0
    );

    // Looking again and deciding on what is there works.
    let mut fresh = retry("the limit service is back");
    fresh["expectedRunVersion"] = json!(seen);
    let (status, body) = decide(&p, "olivia", &run, &fresh, "fault-007-fresh-1").await;
    assert_eq!(status, 202, "{body}");
}

#[tokio::test]
async fn a_compensation_that_fails_again_waits_for_a_person_again_and_the_journal_keeps_every_decision()
 {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;

    let (status, body) = decide(
        &p,
        "olivia",
        &run,
        &retry("the limit service is back"),
        "fault-007-again-1",
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let second = w.one_task().await;
    assert_eq!(second.attempt, 2);
    w.fail_with(&second, "LIMIT_SERVICE_DOWN", true)
        .await
        .expect("down again");
    ended(&p, &run, "MANUAL_REVIEW").await;
    let waiting = graph(&p, &run).await;
    let node = waiting["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "reserve~c")
        .expect("the compensation")
        .clone();
    assert_eq!(node["waitingOn"], "operator", "{node}");
    assert_eq!(node["attempt"], 2, "{node}");

    // A second decision, a second attempt, and this time it works.
    let (status, body) = decide(
        &p,
        "olivia",
        &run,
        &retry("the provider confirmed it is up"),
        "fault-007-again-2",
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let third = w.one_task().await;
    assert_eq!(third.attempt, 3);
    w.complete(&third, &json!({})).await.expect("released");
    ended(&p, &run, "COMPENSATED").await;

    let types = journal_types(&p, &run).await;
    assert_eq!(count(&types, "step.retry_requested"), 2, "{types:?}");
    assert_eq!(count(&types, "step.needs_review"), 2, "{types:?}");
}

#[tokio::test]
async fn a_person_of_another_tenant_cannot_decide_and_learns_nothing() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    let before = graph(&p, &run).await;

    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/operations"),
            "bob",
            "globex",
        )
        .header("idempotency-key", "fault-007-foreign-1")
        .json(&retry("let me in"))
        .send()
        .await
        .expect("send");
    let foreign = response.status().as_u16();
    let missing = p
        .api
        .as_user(
            Method::POST,
            "/runs/00000000-0000-7000-8000-000000000000/operations",
            "bob",
            "globex",
        )
        .header("idempotency-key", "fault-007-foreign-2")
        .json(&retry("let me in"))
        .send()
        .await
        .expect("send")
        .status()
        .as_u16();
    assert_eq!(
        foreign, missing,
        "a run of acme is as unknown to globex as one that does not exist"
    );
    assert_eq!(foreign, 404, "the run of another tenant does not exist");
    assert_eq!(graph(&p, &run).await, before, "nothing moved");
    assert_eq!(
        count(&journal_types(&p, &run).await, "step.retry_requested"),
        0
    );
}
