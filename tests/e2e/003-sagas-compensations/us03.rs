//! US03 — reconcile an uncertain effect: when the provider accepted the payment but the response
//! was lost, a reconciliation queries the outcome before anything is retried or compensated
//! (US03-AC01).

use std::time::{Duration, Instant};

use reqwest::Method;
use serde_json::json;

use crate::support::worker::{RunningEngine, Worker, engine_env, platform};
use crate::{decide, graph, input, journal, node, payment_saga};

const ACTIVITIES: &[&str] = &[
    "payment.validate",
    "limit.reserve",
    "payment.submit",
    "limit.release",
    "payment.return.request",
    "payment.lookup.status",
    "customer.notify",
];

async fn eventually_task(w: &mut Worker<'_>, within: Duration) -> sagat_api_grpc::pb::TaskLease {
    let deadline = Instant::now() + within;
    loop {
        if let Some(task) = w.poll(8).await.pop() {
            return task;
        }
        assert!(Instant::now() < deadline, "no task within {within:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `validate` and `reserve` done and `submit` leased: the payment is now at the provider.
async fn payment_in_flight(w: &mut Worker<'_>) -> sagat_api_grpc::pb::TaskLease {
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({"reservation": "r-1"}))
        .await
        .expect("reserve");
    let submit = w.one_task().await;
    assert_eq!(submit.activity_name, "payment.submit");
    submit
}

#[tokio::test]
async fn us03_after_an_engine_restart_the_lost_payment_is_looked_up_before_it_is_retried() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;

    let mut first_engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut w = p.worker_at(&first_engine.grpc_url, "wendy").await;
    w.register(ACTIVITIES).await;
    let submit = payment_in_flight(&mut w).await;
    assert_eq!(submit.idempotency_key, "submit:p-1");

    // The provider accepts the payment, but the response never arrives: the engine goes down with
    // the lease still held, and the lease runs out while nothing is watching.
    first_engine.finish().await.expect("a clean stop");
    p.expire_lease(&run, "submit").await;
    assert_eq!(p.status(&run).await, "RUNNING", "nothing has noticed yet");

    let second_engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut rescuer = p.worker_at(&second_engine.grpc_url, "rhea").await;
    rescuer.register(ACTIVITIES).await;

    // The restarted engine asks the provider what happened; it does not submit the payment again.
    let lookup = eventually_task(&mut rescuer, Duration::from_secs(20)).await;
    assert_eq!(lookup.activity_name, "payment.lookup.status");
    assert_eq!(lookup.task_kind, "RECONCILIATION");
    assert_eq!(lookup.target_node, "submit");
    assert_eq!(
        lookup.target_idempotency_key, "submit:p-1",
        "what identifies the payment at the provider"
    );
    let mid = graph(&p, &run).await;
    assert_eq!(node(&mid, "submit")["waitingOn"], "reconciliation");

    rescuer
        .complete(
            &lookup,
            &json!({"effect": "applied", "output": {"paymentRef": "pay-9"}}),
        )
        .await
        .expect("answer");
    let after = graph(&p, &run).await;
    assert_eq!(node(&after, "submit")["status"], "SUCCEEDED");
    assert_eq!(
        node(&after, "submit")["attempt"],
        1,
        "the payment was submitted exactly once"
    );
    assert_eq!(
        p.status(&run).await,
        "WAITING",
        "now it waits for the confirmation event"
    );

    let events = journal(&p, &run).await;
    let at = |kind: &str| {
        events
            .iter()
            .position(|(k, s)| k == kind && s.as_deref() == Some("submit"))
    };
    assert!(
        at("reconciliation.requested") < at("reconciliation.concluded"),
        "{events:?}"
    );
    let leases = events
        .iter()
        .filter(|(k, s)| k == "step.leased" && s.as_deref() == Some("submit"))
        .count();
    assert_eq!(leases, 1, "no second attempt: {events:?}");

    // The run goes on from where the provider says it is.
    let (status, signal) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/signals/payment.confirmed"),
            Some(&json!({"amount": 125.5})),
        )
        .await;
    assert_eq!(status, 202, "{signal}");
    let notify = eventually_task(&mut rescuer, Duration::from_secs(10)).await;
    assert_eq!(notify.activity_name, "customer.notify");
    let data: serde_json::Value = serde_json::from_str(&notify.input_json).expect("json");
    assert_eq!(
        data["steps"]["submit"]["output"]["paymentRef"], "pay-9",
        "later steps see what the lookup found"
    );
    rescuer.complete(&notify, &json!({})).await.expect("notify");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn us03_when_the_provider_says_it_did_not_happen_the_payment_is_retried_with_the_same_key() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let lost = payment_in_flight(&mut w).await;
    p.expire_lease(&run, "submit").await;
    p.tick().await;

    let lookup = w.one_task().await;
    assert_eq!(lookup.task_kind, "RECONCILIATION");
    w.complete(&lookup, &json!({"effect": "not_applied"}))
        .await
        .expect("answer");
    p.elapse_retry_delay(&run, "submit").await;
    p.tick().await;
    let again = w.one_task().await;
    assert_eq!(again.activity_name, "payment.submit");
    assert_eq!(again.attempt, 2);
    assert_eq!(
        again.idempotency_key, lost.idempotency_key,
        "the provider can still recognise the request"
    );
    w.complete(&again, &json!({"paymentRef": "pay-2"}))
        .await
        .expect("submitted");
    assert_eq!(p.status(&run).await, "WAITING");
}

#[tokio::test]
async fn us03_a_worker_that_does_not_know_what_happened_says_so_and_the_lookup_follows() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let submit = payment_in_flight(&mut w).await;
    w.fail_with(&submit, "OUTCOME_UNKNOWN", false)
        .await
        .expect("reported");

    let lookup = w.one_task().await;
    assert_eq!(
        (lookup.activity_name.as_str(), lookup.task_kind.as_str()),
        ("payment.lookup.status", "RECONCILIATION")
    );
    assert_eq!(p.status(&run).await, "RUNNING");
    let g = graph(&p, &run).await;
    assert_eq!(node(&g, "submit")["waitingOn"], "reconciliation");
}

#[tokio::test]
async fn us03_a_provider_that_cannot_say_leaves_the_decision_to_a_person_who_records_why() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    payment_in_flight(&mut w).await;
    p.expire_lease(&run, "submit").await;
    p.tick().await;
    let lookup = w.one_task().await;
    w.complete(&lookup, &json!({"effect": "unknown"}))
        .await
        .expect("answer");

    // Neither retried nor compensated.
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW");
    assert!(w.poll(8).await.is_empty());
    let g = graph(&p, &run).await;
    assert_eq!(node(&g, "submit")["waitingOn"], "operator");

    // The person checks with the provider and says the payment exists.
    let (status, body) = decide(
        &p,
        None,
        &run,
        &json!({"action": "RESOLVE_STEP", "stepId": "submit", "outcome": "SUCCEEDED",
                "reason": "payment pay-9 confirmed in the provider console"}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["runStatus"], "WAITING");
    let audited: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'saga.step_resolved' AND resource_id = $1",
    )
    .bind(format!("{run}/submit"))
    .fetch_optional(&p.api.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audited.as_deref(),
        Some("declared SUCCEEDED: payment pay-9 confirmed in the provider console"),
        "the audit trail says what was declared, not only that something was"
    );
}

#[tokio::test]
async fn us03_an_answer_that_is_not_a_verdict_never_counts_as_one() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    payment_in_flight(&mut w).await;
    p.expire_lease(&run, "submit").await;
    p.tick().await;
    let lookup = w.one_task().await;
    w.complete(&lookup, &json!({"status": "PAID"}))
        .await
        .expect("the answer is taken");
    assert_eq!(
        p.status(&run).await,
        "MANUAL_REVIEW",
        "a wrong shape is not a yes"
    );
    let g = graph(&p, &run).await;
    assert_eq!(
        node(&g, "submit~r")["failureCode"],
        "RECONCILIATION_ANSWER_INVALID"
    );
}
