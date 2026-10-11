//! US03: an effect whose outcome is unknown is reconciled before anything else is decided
//! (FR-003-005, FR-003-006).

use sagat_application::worker::TaskKind;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::support::engine::{
    Session, World, activity, document, effect_node, saga_node, with, world,
};
use crate::{quick, settlement, worker};

/// `reserve` is done and `submit` was leased, but its response is lost: the lease runs out.
async fn lose_the_response(w: &World, session: &Session, run: sagat_domain::execution::RunId) {
    let reserve = w.one_task(session).await;
    w.complete(session, &reserve, &json!({}))
        .await
        .expect("reserve");
    let _submit = w.one_task(session).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;
}

#[tokio::test]
async fn a_lost_response_is_reconciled_before_any_second_attempt() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;

    // No retry, no compensation: the lookup is the only thing that can run.
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(
        w.step_status(run, "submit~r").await,
        Some(StepStatus::Ready)
    );
    assert!(
        w.journal(run)
            .await
            .iter()
            .any(|n| n == "reconciliation.requested")
    );
    let lookup = w.one_task(&s).await;
    assert_eq!(
        (lookup.activity.as_str(), lookup.kind),
        ("lookup.submit", TaskKind::Reconciliation)
    );
    let target = lookup.target.clone().expect("a lookup names its target");
    assert_eq!(target.node.as_str(), "submit");
    // The key the lost attempt carried is what identifies the payment at the provider.
    assert!(
        target.idempotency_key.ends_with(":submit"),
        "{}",
        target.idempotency_key
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_provider_says_the_payment_happened_so_the_step_succeeds_without_a_second_attempt() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;

    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "applied", Some(json!({ "payment": "p-9" })))
        .await
        .expect("applied");
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Succeeded)
    );
    // The step's output is the one the lookup found, and the work went on.
    let notify = w.one_task(&s).await;
    assert_eq!(notify.activity.as_str(), "act.notify");
    let input: serde_json::Value = serde_json::from_slice(&notify.input).expect("json");
    assert_eq!(input["steps"]["submit"]["output"]["payment"], "p-9");
    w.complete(&s, &notify, &json!({})).await.expect("notify");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);

    // The payment was never executed twice: there was exactly one attempt.
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempt FROM steps WHERE run_id = $1 AND node = 'submit'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("attempt");
    assert_eq!(attempts, 1);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_provider_says_it_did_not_happen_so_the_step_is_tried_again() {
    let w = world().await;
    let (workflow, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;

    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "not_applied", None)
        .await
        .expect("not applied");
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::RetryWait)
    );
    w.age_retry(run, "submit").await;
    w.scheduler.tick().await;
    let again = w.one_task(&s).await;
    assert_eq!(again.activity.as_str(), "act.submit");
    assert_eq!(again.fence.attempt, 2);
    // It did not happen, so nothing is recorded as its output.
    let seen: serde_json::Value = serde_json::from_slice(&again.input).expect("json");
    assert!(seen["steps"].get("submit").is_none(), "{seen}");
    // The same key as the lost attempt: the provider can still recognise the request.
    assert_eq!(
        again.idempotency_key,
        lookup.target.expect("target").idempotency_key
    );
    w.complete(&s, &again, &json!({})).await.expect("works now");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn when_it_did_not_happen_and_no_attempt_is_left_the_saga_compensates() {
    let w = world().await;
    let doc = document(
        "settlement",
        "settlement",
        vec![
            saga_node("settlement", "reserve", None),
            effect_node("reserve", Some("submit"), false),
            with(
                effect_node("submit", None, true),
                "retry",
                json!({ "maxAttempts": 1, "initialDelayMs": 100, "maxDelayMs": 100,
                        "strategy": "fixed", "jitter": false }),
            ),
        ],
    );
    let (workflow, _) = w.publish(doc).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "not_applied", None)
        .await
        .expect("not applied");

    assert_eq!(w.step_status(run, "submit").await, Some(StepStatus::Failed));
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_provider_that_cannot_say_is_never_assumed_either_way() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("unknown");

    // Neither retried nor compensated: a human decides.
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Waiting)
    );
    assert!(w.poll(&s, 8).await.is_empty());
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_worker_can_say_it_does_not_know_what_happened() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail_with(&s, &submit, "OUTCOME_UNKNOWN", false)
        .await
        .expect("unknown");

    // The step is reconciled exactly as if its lease had been lost.
    assert_eq!(
        w.step_status(run, "submit~r").await,
        Some(StepStatus::Ready)
    );
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Waiting)
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_ordinary_failure_is_not_reconciled() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("declined");
    assert_eq!(
        w.step_status(run, "submit~r").await,
        None,
        "the worker knows it did not happen"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
}

#[tokio::test]
async fn an_answer_that_is_not_a_verdict_fails_the_lookup_for_good_and_a_human_decides() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    lose_the_response(&w, &s, run).await;
    let lookup = w.one_task(&s).await;

    // The worker is told its answer was taken; asking the same lookup again would answer the same.
    w.complete(&s, &lookup, &json!({ "status": "ok" }))
        .await
        .expect("accepted as an answer");
    assert_eq!(
        w.step_status(run, "submit~r").await,
        Some(StepStatus::Failed)
    );
    let code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'submit~r'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("code");
    assert_eq!(code.as_deref(), Some("RECONCILIATION_ANSWER_INVALID"));
    // Nothing is assumed about the payment: the step waits for a person.
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_high_risk_step_without_a_lookup_waits_for_an_operator_instead_of_retrying() {
    let w = world().await;
    let doc = document(
        "pay",
        "pay",
        vec![with(
            with(activity("pay"), "highRisk", json!(true)),
            "retry",
            json!({ "maxAttempts": 5, "initialDelayMs": 100, "maxDelayMs": 100,
                    "strategy": "fixed", "jitter": false }),
        )],
    );
    let (workflow, _) = w.publish(doc).await;
    let s = w.session(&["act.pay"]).await;
    let run = w.start(workflow, &json!({})).await;
    let _pay = w.one_task(&s).await;
    w.age_lease(run, "pay").await;
    w.scheduler.tick().await;

    // Four attempts are left, and not one is used blindly.
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));
    assert!(w.poll(&s, 8).await.is_empty());
    w.assert_consistent(run).await;

    // An ordinary step with the same loss is retried, as before this feature.
    let (plain, _) = w
        .publish(document(
            "plain",
            "pay",
            vec![crate::support::engine::flaky("pay", 5)],
        ))
        .await;
    let plain_run = w.start(plain, &json!({})).await;
    let _ = w.one_task(&s).await;
    w.age_lease(plain_run, "pay").await;
    w.scheduler.tick().await;
    assert_eq!(
        w.step_status(plain_run, "pay").await,
        Some(StepStatus::RetryWait)
    );
}

#[tokio::test]
async fn an_answer_that_would_take_the_run_over_what_it_may_retain_fails_the_lookup_for_good() {
    use sagat_application::execution::MAX_RUN_OUTPUT_BYTES;
    let w = world().await;
    let big = json!({ "blob": "x".repeat(240_000) });
    let len = serde_json::to_vec(&big).expect("json").len();
    let fits = MAX_RUN_OUTPUT_BYTES / len;
    // `fits` steps retain almost everything a run may; the one after them is looked up.
    let mut nodes = vec![saga_node("tx", "s0", None)];
    for i in 0..fits {
        nodes.push(effect_node(
            &format!("s{i}"),
            Some(&if i + 1 < fits {
                format!("s{}", i + 1)
            } else {
                "pay".to_owned()
            }),
            false,
        ));
    }
    nodes.push(effect_node("pay", None, true));
    let (workflow, _) = w.publish(document("hoarder", "tx", nodes)).await;
    let mut names: Vec<String> = (0..fits)
        .flat_map(|i| crate::support::engine::effect_activities(&[&format!("s{i}")]))
        .collect();
    names.extend(crate::support::engine::effect_activities(&["pay"]));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&refs).await;
    let run = w.start(workflow, &json!({})).await;
    for _ in 0..fits {
        let task = w.one_task(&s).await;
        w.complete(&s, &task, &big)
            .await
            .expect("within the budget");
    }
    let _lost = w.one_task(&s).await;
    w.age_lease(run, "pay").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    assert_eq!(lookup.activity.as_str(), "lookup.pay");

    // The provider says it happened and hands back a result too big to keep.
    w.answer_reconciliation(&s, &lookup, "applied", Some(big.clone()))
        .await
        .expect("the report itself is accepted");
    let code: Option<String> =
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'pay~r'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("code");
    assert_eq!(code.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(w.step_status(run, "pay~r").await, Some(StepStatus::Failed));
    // Nothing was assumed about the payment: a person decides, and no output was recorded for it.
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));
    w.assert_consistent(run).await;
}
