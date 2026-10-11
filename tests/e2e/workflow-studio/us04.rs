//! US04 — investigate and act on a failure: the operator sees the failed step wait for a decision,
//! decides with a reason, and the action shows in the live view as it happens, audited and without
//! overwriting the history (US04-AC01). The decisions themselves are feature 003's; what the
//! Studio relies on is that each of them is a change the stream carries.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::{Platform, Worker, platform};
use crate::{Stream, apply, by_id, graph};

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

#[tokio::test]
async fn us04_a_decision_waiting_for_an_operator_and_the_operators_answer_both_arrive_in_the_stream()
 {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;

    // The Studio opens the run: the failed compensation is drawn as waiting for a person.
    let mut stream = Stream::open_as_alice(&p, &run).await;
    let first = stream.next(Duration::from_secs(5)).await.expect("graph");
    let mut model: BTreeMap<String, Value> = BTreeMap::new();
    let mut seq = 0;
    apply(&mut model, &mut seq, &first.data);
    assert_eq!(model["reserve~c"]["waitingOn"], "operator", "{first:?}");
    // The body of the saga hangs from the saga: without that link the picture falls into pieces.
    let edges = first.data["edges"]
        .as_array()
        .expect("a reset carries the edges");
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == "settlement" && e["to"] == "reserve" && e["kind"] == "begin"),
        "{edges:?}"
    );
    assert_eq!(model["reserve~c"]["failureCode"], "LIMIT_SERVICE_DOWN");
    assert_eq!(first.data["status"], "MANUAL_REVIEW");
    let seen = first.data["events"].as_array().expect("events").len();
    assert_eq!(
        seen, 0,
        "a reset carries the graph, not the history; the journal view has that"
    );

    // The operator asks for another attempt, with a reason the audit trail keeps.
    let (status, body) = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({
                "action": "RETRY_COMPENSATION", "stepId": "reserve~c",
                "reason": "the limit service is back"
            })),
        )
        .await;
    assert_eq!(status, 202, "{body}");

    let (changed, _) = stream
        .until("graph", Duration::from_secs(10))
        .await
        .expect("the decision is a change");
    apply(&mut model, &mut seq, &changed.data);
    let kinds: Vec<&str> = changed.data["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|e| e["eventType"].as_str())
        .collect();
    assert!(kinds.contains(&"step.retry_requested"), "{kinds:?}");
    assert_ne!(
        model["reserve~c"]["waitingOn"], "operator",
        "no longer waiting: {}",
        model["reserve~c"]
    );

    // The worker does it; the run ends compensated and the stream ends with it.
    let release = w.one_task().await;
    assert_eq!(release.attempt, 2);
    w.complete(&release, &json!({})).await.expect("released");
    let (_, rest) = stream
        .until("end", Duration::from_secs(15))
        .await
        .expect("the run ended");
    for message in rest {
        apply(&mut model, &mut seq, &message.data);
    }
    assert_eq!(model, by_id(&graph(&p, &run).await));
    assert_eq!(graph(&p, &run).await["status"], "COMPENSATED");

    // The history was not overwritten: the failed attempt is still in the journal.
    let (_, journal) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    let types: Vec<&str> = journal["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|e| e["eventType"].as_str())
        .collect();
    for kept in ["step.needs_review", "step.retry_requested"] {
        assert!(types.contains(&kept), "{kept} in {types:?}");
    }
}

#[tokio::test]
async fn us04_what_an_operator_may_do_follows_their_role_and_a_refusal_changes_nothing() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let before = graph(&p, &run).await;

    let retry = json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "the limit service is back"});
    let (status, body) = p
        .rest_as(
            "vera",
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&retry),
        )
        .await;
    assert_eq!(status, 403, "a viewer watches: {body}");
    let skip = json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "released by hand in the console"});
    let (status, body) = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&skip),
        )
        .await;
    assert_eq!(
        status, 403,
        "giving up on a compensation needs saga:override: {body}"
    );
    assert_eq!(body["details"]["requiredPermission"], "saga:override");
    let short = json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "no"});
    let (status, _) = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&short),
        )
        .await;
    assert!(
        [400, 422].contains(&status),
        "a reason too short to explain anything: {status}"
    );

    // `GET /auth/context` is what the Studio reads to decide which buttons to draw.
    let (status, context) = p
        .rest_as("olivia", Method::GET, "/auth/context", None)
        .await;
    assert_eq!(status, 200, "{context}");
    assert_eq!(context["role"], "operator", "{context}");
    let permissions: Vec<&str> = context["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(permissions.contains(&"run:operate"), "{permissions:?}");
    assert!(!permissions.contains(&"saga:override"), "{permissions:?}");

    assert_eq!(graph(&p, &run).await, before, "nothing moved");
}
