//! US04 — resolve a compensation failure: the operator reviews and intervenes, and the state and
//! the reason are audited without erasing previous events (US04-AC01).

use reqwest::Method;
use serde_json::json;

use crate::support::worker::{Platform, Worker, platform};
use crate::{decide, graph, input, journal, node, payment_saga, with_compensation_attempts};
use sagat_domain::identity::Role;

const ACTIVITIES: &[&str] = &[
    "payment.validate",
    "limit.reserve",
    "payment.submit",
    "limit.release",
    "payment.return.request",
    "payment.lookup.status",
    "customer.notify",
];

/// A run whose release of the limit failed for good: it waits for a person.
async fn stuck(p: &Platform, w: &mut Worker<'_>) -> String {
    let wf = p
        .publish(
            "corporate-payment",
            &with_compensation_attempts(payment_saga(), 1),
        )
        .await;
    let run = p.start(&wf, &input()).await;
    w.register(ACTIVITIES).await;
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task().await;
    w.fail(&submit, false).await.expect("declined");
    let release = w.one_task().await;
    assert_eq!(release.activity_name, "limit.release");
    w.fail_with(&release, "LIMIT_SERVICE_DOWN", true)
        .await
        .expect("the limit service is down");
    run
}

#[tokio::test]
async fn us04_a_release_that_failed_after_its_retries_waits_for_a_person_and_nothing_else_moves() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW");
    assert!(w.poll(8).await.is_empty());
    let g = graph(&p, &run).await;
    assert_eq!(node(&g, "reserve~c")["waitingOn"], "operator");
    assert_eq!(node(&g, "reserve~c")["failureCode"], "LIMIT_SERVICE_DOWN");
    assert_eq!(node(&g, "settlement")["saga"]["phase"], "COMPENSATING");
    // Where an operator looks for it.
    let (status, listed) = p
        .rest(Method::GET, "/runs?status=MANUAL_REVIEW", None)
        .await;
    assert_eq!(status, 200, "{listed}");
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["runId"].as_str())
        .collect();
    assert_eq!(ids, [run.as_str()]);
}

#[tokio::test]
async fn us04_an_operator_asks_for_another_attempt_and_the_run_finishes_compensated() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let before = journal(&p, &run).await;

    let (status, body) = decide(
        &p,
        Some("olivia"),
        &run,
        &json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "the limit service is back"}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["runStatus"], "COMPENSATING");
    let release = w.one_task().await;
    assert_eq!(release.attempt, 2, "a new attempt of the same compensation");
    w.complete(&release, &json!({})).await.expect("released");
    assert_eq!(p.status(&run).await, "COMPENSATED");

    // Nothing was erased: what the journal said before it still says, in the same order.
    let after = journal(&p, &run).await;
    assert_eq!(after[..before.len()], before[..]);
    for kept in ["step.needs_review", "step.retry_requested"] {
        assert!(after.iter().any(|(k, _)| k == kept), "{kept} in {after:?}");
    }
}

#[tokio::test]
async fn us04_giving_up_on_a_release_is_an_administrators_decision_and_ends_the_saga_partial() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let skip = json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c",
                      "reason": "limit released by hand in the provider console"});

    // An operator may ask for another attempt, but not give up on one.
    let (status, refused) = decide(&p, Some("olivia"), &run, &skip).await;
    assert_eq!(status, 403, "{refused}");
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW", "nothing changed");

    let (status, body) = decide(&p, None, &run, &skip).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["runStatus"], "PARTIAL");
    let g = graph(&p, &run).await;
    assert_eq!(node(&g, "reserve~c")["status"], "SKIPPED");
    assert_eq!(node(&g, "settlement")["saga"]["outcome"], "PARTIAL");

    // The decision, its author and its reason are in the audit trail.
    let audited: (String, Option<String>) = sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'saga.step_skipped' AND resource_id = $1",
    )
    .bind(format!("{run}/reserve~c"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("audited");
    assert_eq!(audited.0, "alice");
    assert_eq!(
        audited.1.as_deref(),
        Some("limit released by hand in the provider console")
    );
}

#[tokio::test]
async fn us04_declaring_the_release_done_elsewhere_ends_the_saga_compensated() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    let (status, body) = decide(
        &p,
        None,
        &run,
        &json!({"action": "RESOLVE_STEP", "stepId": "reserve~c", "outcome": "SUCCEEDED",
                "reason": "released by hand, ticket OPS-123"}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["runStatus"], "COMPENSATED");
}

#[tokio::test]
async fn us04_a_decision_that_makes_no_sense_is_refused_precisely() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;
    let version = p.run(&run).await["version"].as_u64().expect("version");

    let cases = [
        (
            json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "short"}),
            422,
            "reason",
        ),
        (
            json!({"action": "SKIP_COMPENSATION", "reason": "no step named at all"}),
            422,
            "stepid",
        ),
        (
            json!({"action": "RETRY_COMPENSATION", "stepId": "reserve", "reason": "this is a forward step"}),
            422,
            "needs the id of a compensation",
        ),
        (
            json!({"action": "RESOLVE_STEP", "stepId": "reserve~c", "reason": "no outcome given"}),
            422,
            "outcome",
        ),
        (
            json!({"action": "RESOLVE_STEP", "stepId": "reserve~c", "outcome": "MAYBE", "reason": "not an outcome"}),
            422,
            "succeeded or failed",
        ),
        (
            json!({"action": "RETRY_STEP", "stepId": "reserve~c", "outcome": "FAILED", "reason": "outcome does not go here"}),
            422,
            "resolve_step",
        ),
        (
            json!({"action": "RETRY_STEP", "stepId": "not a node!", "reason": "garbage node id here"}),
            422,
            "step",
        ),
        (
            json!({"action": "RETRY_STEP", "stepId": "ghost", "reason": "there is no such step"}),
            404,
            "",
        ),
        (
            json!({"action": "RETRY_STEP", "stepId": "validate", "reason": "this step is not waiting"}),
            409,
            "",
        ),
        (
            json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "stale view of the run", "expectedRunVersion": version - 1}),
            409,
            "",
        ),
        (
            json!({"action": "RERUN_AS_NEW", "reason": "declared for later features"}),
            422,
            "not supported",
        ),
    ];
    for (body, expected, mention) in cases {
        let (status, answer) = decide(&p, None, &run, &body).await;
        assert_eq!(status, expected, "{body} -> {answer}");
        let text = answer.to_string().to_lowercase();
        assert!(text.contains(&mention.to_lowercase()), "{body} -> {text}");
    }
    assert_eq!(
        p.status(&run).await,
        "MANUAL_REVIEW",
        "none of them changed anything"
    );
}

#[tokio::test]
async fn us04_the_second_of_two_decisions_is_refused_and_another_tenant_sees_nothing() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let run = stuck(&p, &mut w).await;

    let foreign = p
        .api
        .as_user(Method::POST, &format!("/runs/{run}/operations"), "bob", "globex")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "foreign-0000001")
        .json(&json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "not my run at all"}))
        .send()
        .await
        .expect("send");
    assert_eq!(foreign.status(), 404);

    let skip = json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "first decision wins"});
    let (first, _) = decide(&p, None, &run, &skip).await;
    assert_eq!(first, 202);
    let resolve = json!({"action": "RESOLVE_STEP", "stepId": "reserve~c", "outcome": "SUCCEEDED", "reason": "second decision loses"});
    let (second, body) = decide(&p, None, &run, &resolve).await;
    assert_eq!(second, 409, "{body}");
    assert_eq!(p.status(&run).await, "PARTIAL");
}
