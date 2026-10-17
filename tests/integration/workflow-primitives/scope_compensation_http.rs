//! Compensation of the iterations a collection or loop completed, through the public path: published
//! over HTTP, run by the scheduler, undone by workers over the worker protocol 1.5 (FR-014-005,
//! FR-003-002, D380..D389).
#![allow(clippy::expect_used)]

use reqwest::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::scope_flow::{
    Flow, activity, all_compensations, collection_document, compensating_loop, compensation_order,
    compensation_order_with_carry, forward, loop_until_a_charge_is_declined,
    one_release_that_cannot_be_delivered, reserve_and_charge, reserve_with_a_lost_attempt, retry,
    row, summary,
};

#[tokio::test]
async fn a_failed_collection_undoes_what_its_iterations_completed_in_reverse_order() {
    let name = format!("scope-compensation-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;

    // Forward work: both iterations reserve, then the first charges and the second is declined.
    flow.drive().await;
    let reserves = flow.poll().await;
    assert_eq!(
        summary(&reserves),
        vec![
            row(0, "reserve", "FORWARD", "line.reserve"),
            row(1, "reserve", "FORWARD", "line.reserve")
        ]
    );
    for (index, task) in reserves.iter().enumerate() {
        flow.complete(task, json!({"hold": index})).await;
    }
    flow.drive().await;
    let charges = flow.poll().await;
    assert_eq!(
        summary(&charges),
        vec![
            row(0, "charge", "FORWARD", "line.charge"),
            row(1, "charge", "FORWARD", "line.charge")
        ]
    );
    flow.complete(&charges[0], json!({"charged": 10})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    flow.drive().await;

    // The scope stopped for the failure and compensates: what nothing waits for is ready first, in
    // both iterations at once, and the earlier effect of iteration 0 waits for the later one.
    assert_eq!(flow.run_status().await, "COMPENSATING");
    flow.verify().await;
    let first = flow.poll().await;
    assert_eq!(
        summary(&first),
        vec![
            row(0, "charge~c", "COMPENSATION", "line.refund"),
            row(1, "reserve~c", "COMPENSATION", "line.release")
        ]
    );
    for task in &first {
        // The task names the forward instance it undoes, at the same iteration, and carries the
        // key of the compensation, which is not the key of the effect.
        let target = task.target_instance.as_ref().expect("target instance");
        assert_eq!(
            target.instance_path,
            task.instance.as_ref().expect("instance").instance_path
        );
        assert!(task.idempotency_key.starts_with("compensation:"));
        assert!(!task.target_idempotency_key.is_empty());
        assert_ne!(task.idempotency_key, task.target_idempotency_key);
    }
    assert_ne!(
        first[0].idempotency_key, first[1].idempotency_key,
        "two iterations never share the key of their compensation"
    );
    for task in &first {
        flow.complete(task, json!({"undone": true})).await;
    }
    flow.drive().await;
    let second = flow.poll().await;
    assert_eq!(
        summary(&second),
        vec![row(0, "reserve~c", "COMPENSATION", "line.release")],
        "the reservation of iteration 0 is released only after its charge was refunded"
    );
    flow.complete(&second[0], json!({"undone": true})).await;
    flow.drive().await;

    assert_eq!(flow.run_status().await, "COMPENSATED");
    let scope: (String, String, String) = sqlx::query_as(
        "SELECT lifecycle, drain_kind, drain_failure_code FROM iteration_scopes WHERE run_id = $1",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("scope projection");
    assert_eq!(
        scope,
        ("FAILED".into(), "FAILED".into(), "PAYMENT_DECLINED".into())
    );
    let saga: (String, String, Option<String>, bool) = sqlx::query_as(
        "SELECT phase, cause_code, outcome, during_stop FROM scope_sagas WHERE run_id = $1",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("scope compensation projection");
    assert_eq!(
        saga,
        (
            "DONE".into(),
            "PAYMENT_DECLINED".into(),
            Some("COMPENSATED".into()),
            true
        )
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_cancelled_collection_undoes_what_completed_and_ends_cancelled() {
    let name = format!("scope-cancel-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;
    forward(&flow, &[("reserve", 0), ("reserve", 1)]).await;
    // Iteration 0 charges; the charge of iteration 1 is in flight when the run is cancelled.
    let charges = flow.poll().await;
    assert_eq!(charges.len(), 2);
    flow.complete(&charges[0], json!({"charged": 10})).await;
    flow.drive().await;
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "the operator changed their mind"}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    flow.drive().await;
    // The attempt in flight is asked to stop; its worker acknowledges, and nothing says it happened
    // or not, so it is not compensated (it failed with a known cause).
    flow.fail(&charges[1], "CANCELLED_BY_WORKER", false).await;
    all_compensations(&flow).await;

    assert_eq!(flow.run_status().await, "CANCELLED");
    let scope: (String, String) =
        sqlx::query_as("SELECT lifecycle, drain_kind FROM iteration_scopes WHERE run_id = $1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("scope");
    assert_eq!(scope, ("CANCELLED".into(), "CANCELLED".into()));
    let undone: Vec<(i32, String)> = sqlx::query_as(
        "SELECT instance_path_indices[1], node FROM steps \
         WHERE run_id = $1 AND kind = 'COMPENSATION' AND status = 'SUCCEEDED' \
         ORDER BY instance_path_indices[1], node",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("compensations");
    assert_eq!(
        undone,
        vec![
            (0, "charge~c".into()),
            (0, "reserve~c".into()),
            (1, "reserve~c".into())
        ]
    );
    flow.verify().await;
}

#[tokio::test]
async fn an_irreversible_effect_ends_the_run_partial_and_is_never_called_compensated() {
    let name = format!("scope-partial-{}", Uuid::now_v7());
    let document = collection_document(
        &name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "notify",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            activity(
                "notify",
                "line.notify",
                "charge",
                json!({"irreversible": true}),
            ),
            activity("charge", "line.charge", "returned", json!({})),
            json!({"id": "returned", "type": "return", "result": "steps.charge.output"}),
        ],
        1,
    );
    let flow = Flow::start(&name, document, json!([{"sku": "a"}])).await;
    forward(&flow, &[("reserve", 0)]).await;
    forward(&flow, &[("notify", 0)]).await;
    let charge = flow.poll().await;
    assert_eq!(
        summary(&charge),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    flow.fail(&charge[0], "PAYMENT_DECLINED", false).await;
    all_compensations(&flow).await;

    assert_eq!(flow.run_status().await, "PARTIAL");
    let saga: (String, Option<String>) =
        sqlx::query_as("SELECT phase, outcome FROM scope_sagas WHERE run_id = $1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("saga");
    assert_eq!(saga, ("DONE".into(), Some("PARTIAL".into())));
    let irreversible: Vec<(i32, String)> = sqlx::query_as(
        "SELECT iteration_index, node FROM scope_saga_entries \
         WHERE run_id = $1 AND kind = 'IRREVERSIBLE'",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("irreversible entries");
    assert_eq!(irreversible, vec![(0, "notify".into())]);
    flow.verify().await;
}

#[tokio::test]
async fn a_compensation_that_keeps_failing_waits_for_a_person_who_can_retry_it() {
    let name = format!("scope-review-retry-{}", Uuid::now_v7());
    let (flow, stuck) = one_release_that_cannot_be_delivered(&name).await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let waiting: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, waiting_kind, failure_code FROM steps \
         WHERE run_id = $1 AND node = 'reserve~c' AND instance_path_indices = ARRAY[1]",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("waiting compensation");
    assert_eq!(
        waiting,
        (
            "WAITING".into(),
            Some("OPERATOR".into()),
            Some("PROVIDER_DOWN".into())
        )
    );
    // A run that is not decided is not finished: the scheduler leaves it alone however often it runs.
    assert_eq!(flow.drive().await, 0);
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let key = stuck.idempotency_key.clone();

    let (status, body) = flow
        .operate(json!({
            "action": "RETRY_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "the provider is back, release the reservation"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let retried = flow.poll().await;
    assert_eq!(retried.len(), 1);
    assert_eq!(
        retried[0].idempotency_key, key,
        "the person's retry asks with the same key"
    );
    flow.complete(&retried[0], json!({"undone": true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE resource_id LIKE $1 AND action = 'saga.step_retried'",
    )
    .bind(format!("{}/lines[1]/reserve~c", flow.run))
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("audit");
    assert_eq!(audits, 1);
    flow.verify().await;
}

#[tokio::test]
async fn a_person_can_skip_a_compensation_and_the_run_ends_partial() {
    let name = format!("scope-review-skip-{}", Uuid::now_v7());
    let (flow, _) = one_release_that_cannot_be_delivered(&name).await;
    let (status, body) = flow
        .operate(json!({
            "action": "SKIP_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "released by hand at the provider"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "PARTIAL");
    flow.verify().await;
}

#[tokio::test]
async fn a_person_can_declare_a_compensation_failed_and_the_run_ends_compensation_failed() {
    let name = format!("scope-review-fail-{}", Uuid::now_v7());
    let (flow, _) = one_release_that_cannot_be_delivered(&name).await;
    let (status, body) = flow
        .operate(json!({
            "action": "RESOLVE_STEP", "stepId": "reserve~c", "outcome": "FAILED",
            "errorCode": "PROVIDER_REFUSED",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "the provider confirmed it cannot release"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATION_FAILED");
    flow.verify().await;
}

#[tokio::test]
async fn a_cancellation_never_gives_up_on_a_compensation_that_waits_for_a_person() {
    let name = format!("scope-review-cancel-{}", Uuid::now_v7());
    let (flow, _) = one_release_that_cannot_be_delivered(&name).await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    // The run is already ending for the declined charge: a second stop is not accepted, and the
    // person still has to decide.
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "cancel while it waits for a person"}))
        .await;
    assert!(status.is_client_error(), "{status}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    flow.verify().await;
}

#[tokio::test]
async fn a_cancelled_run_whose_compensation_keeps_failing_ends_partial_not_cancelled() {
    let name = format!("scope-cancel-review-{}", Uuid::now_v7());
    let document = collection_document(
        &name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "returned",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            json!({"id": "returned", "type": "return", "result": "steps.reserve.output"}),
        ],
        2,
    );
    let flow = Flow::start(&name, document, json!([{"sku": "a"}, {"sku": "b"}])).await;
    flow.drive().await;
    let reserves = flow.poll().await;
    flow.complete(&reserves[0], json!({"hold": 1})).await;
    flow.drive().await;
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "stop everything now please"}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    flow.drive().await;
    flow.fail(&reserves[1], "CANCELLED_BY_WORKER", false).await;
    flow.drive().await;
    let mut release = flow.poll().await;
    assert_eq!(release.len(), 1);
    for attempt in 1..=2 {
        flow.fail(&release[0], "PROVIDER_DOWN", true).await;
        if attempt == 1 {
            flow.elapse_backoff().await;
            flow.drive().await;
            release = flow.poll().await;
        }
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let (status, body) = flow
        .operate(json!({
            "action": "SKIP_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 0}],
            "reason": "released by hand at the provider"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(
        flow.run_status().await,
        "PARTIAL",
        "a cancellation whose compensation was not completed is not a clean cancellation"
    );
    flow.verify().await;
}

#[tokio::test]
async fn an_effect_of_unknown_outcome_is_looked_up_and_found_applied_before_anything_repeats() {
    let name = format!("scope-lookup-applied-{}", Uuid::now_v7());
    let (flow, lost) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    assert_eq!(
        summary(&lookups),
        vec![row(0, "reserve~r", "RECONCILIATION", "line.lookup")]
    );
    let lookup = &lookups[0];
    assert_eq!(lookup.target_node, "reserve");
    assert_eq!(
        lookup.target_idempotency_key, lost.idempotency_key,
        "the lookup names the effect by the key its attempts carried"
    );
    assert!(lookup.idempotency_key.starts_with("reconciliation:"));
    flow.complete(lookup, json!({"effect": "applied", "output": {"hold": 7}}))
        .await;
    flow.drive().await;
    // The step happened: the iteration goes on with the output the lookup found, and the effect was
    // not repeated (the forward step has had one attempt).
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempt FROM steps WHERE run_id = $1 AND node = 'reserve'")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("attempts");
    assert_eq!(attempts, 1);
    let next = flow.poll().await;
    assert_eq!(
        summary(&next),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    let input: Value = serde_json::from_str(&next[0].input_json).expect("input");
    assert_eq!(input["steps"]["reserve"]["output"], json!({"hold": 7}));
    // Now the charge fails: what the lookup found is undone like any other effect.
    flow.fail(&next[0], "PAYMENT_DECLINED", false).await;
    all_compensations(&flow).await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}

#[tokio::test]
async fn an_effect_found_not_applied_is_tried_again_with_the_same_key() {
    let name = format!("scope-lookup-not-applied-{}", Uuid::now_v7());
    let (flow, lost) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    flow.complete(&lookups[0], json!({"effect": "not_applied"}))
        .await;
    flow.elapse_backoff().await;
    flow.drive().await;
    let again = flow.poll().await;
    assert_eq!(
        summary(&again),
        vec![row(0, "reserve", "FORWARD", "line.reserve")]
    );
    assert_eq!(again[0].attempt, 2);
    assert_eq!(
        again[0].idempotency_key, lost.idempotency_key,
        "the second attempt carries the key of the first"
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_lookup_that_cannot_say_waits_for_a_person_who_declares_what_happened() {
    let name = format!("scope-lookup-unknown-{}", Uuid::now_v7());
    let (flow, _) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    flow.complete(&lookups[0], json!({"effect": "unknown"}))
        .await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let (status, body) = flow
        .operate(json!({
            "action": "RESOLVE_STEP", "stepId": "reserve", "outcome": "SUCCEEDED",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 0}],
            "reason": "the provider's ledger shows the reservation"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    let next = flow.poll().await;
    assert_eq!(
        summary(&next),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_failed_loop_undoes_its_completed_iterations_last_first() {
    let name = format!("scope-loop-{}", Uuid::now_v7());
    let flow = Flow::start(&name, compensating_loop(&name, false, false), json!([])).await;
    loop_until_a_charge_is_declined(&flow, 2).await;

    assert_eq!(flow.run_status().await, "COMPENSATING");
    let (order, carries) = compensation_order_with_carry(&flow).await;
    assert_eq!(
        carries,
        vec![2, 1, 1, 0, 0],
        "a compensation reads the carry its iteration started from, not the loop's latest"
    );
    assert_eq!(
        order,
        vec![
            // The declined pass reserved but never charged.
            (2, "reserve~c".to_owned()),
            (1, "charge~c".to_owned()),
            (1, "reserve~c".to_owned()),
            (0, "charge~c".to_owned()),
            (0, "reserve~c".to_owned()),
        ],
        "the last iteration is undone first, and inside each one the later effect first"
    );
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let scope: (String, Option<String>) =
        sqlx::query_as("SELECT lifecycle, drain_failure_code FROM loop_scopes WHERE run_id = $1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("loop scope");
    assert_eq!(scope, ("FAILED".into(), Some("PAYMENT_DECLINED".into())));
    flow.verify().await;
}

#[tokio::test]
async fn a_cancelled_loop_undoes_what_completed_and_ends_cancelled() {
    let name = format!("scope-loop-cancel-{}", Uuid::now_v7());
    let flow = Flow::start(&name, compensating_loop(&name, false, false), json!([])).await;
    // Two passes complete; the third has reserved and waits for its charge when the run is cancelled.
    for pass in 0..3 {
        flow.drive().await;
        let reserve = flow.poll().await;
        flow.complete(&reserve[0], json!({"hold": pass})).await;
        flow.drive().await;
        let charge = flow.poll().await;
        if pass < 2 {
            flow.complete(&charge[0], json!({"charged": pass})).await;
        } else {
            let (status, body) = flow
                .operate(json!({"action": "CANCEL", "reason": "the operator changed their mind"}))
                .await;
            assert_eq!(status, StatusCode::ACCEPTED, "{body}");
            flow.drive().await;
            flow.fail(&charge[0], "CANCELLED_BY_WORKER", false).await;
        }
    }
    let order = compensation_order(&flow).await;
    assert_eq!(
        order,
        vec![
            (2, "reserve~c".to_owned()),
            (1, "charge~c".to_owned()),
            (1, "reserve~c".to_owned()),
            (0, "charge~c".to_owned()),
            (0, "reserve~c".to_owned()),
        ]
    );
    assert_eq!(flow.run_status().await, "CANCELLED");
    flow.verify().await;
}

#[tokio::test]
async fn a_loop_error_edge_is_taken_only_after_its_failure_was_undone() {
    let name = format!("scope-loop-edge-{}", Uuid::now_v7());
    let flow = Flow::start(&name, compensating_loop(&name, true, false), json!([])).await;
    loop_until_a_charge_is_declined(&flow, 1).await;
    assert_eq!(flow.run_status().await, "COMPENSATING");
    let order = compensation_order(&flow).await;
    assert_eq!(order.len(), 3, "{order:?}");
    // The failure was undone: the run goes on at the edge and ends there.
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let edge: Vec<(String, bool)> =
        sqlx::query_as("SELECT status, routed FROM steps WHERE run_id = $1 AND node = 'poll'")
            .bind(flow.run.as_uuid())
            .fetch_all(&flow.api.db.pool)
            .await
            .expect("loop owner");
    assert_eq!(edge, vec![("FAILED".to_owned(), true)]);
    flow.verify().await;
}

#[tokio::test]
async fn a_loop_failure_that_was_not_fully_undone_is_not_handled_by_its_error_edge() {
    let name = format!("scope-loop-edge-partial-{}", Uuid::now_v7());
    let flow = Flow::start(&name, compensating_loop(&name, true, true), json!([])).await;
    // `notify` cannot be undone: pass 0 completes it, then pass 1 fails at its charge.
    for pass in 0..2 {
        flow.drive().await;
        let reserve = flow.poll().await;
        flow.complete(&reserve[0], json!({"hold": pass})).await;
        flow.drive().await;
        let notify = flow.poll().await;
        assert_eq!(
            summary(&notify),
            vec![row(pass, "notify", "FORWARD", "line.notify")]
        );
        flow.complete(&notify[0], json!({"sent": pass})).await;
        flow.drive().await;
        let charge = flow.poll().await;
        if pass == 0 {
            flow.complete(&charge[0], json!({"charged": 1})).await;
        } else {
            flow.fail(&charge[0], "PAYMENT_DECLINED", false).await;
        }
    }
    let order = compensation_order(&flow).await;
    assert_eq!(
        order,
        vec![(1, "reserve~c".to_owned()), (0, "reserve~c".to_owned())]
    );
    assert_eq!(
        flow.run_status().await,
        "PARTIAL",
        "an irreversible effect stays, and the failure is not handled as if it had been undone"
    );
    let recovered: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE run_id = $1 AND node = 'recovered'")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("recovery steps");
    assert_eq!(recovered, 0);
    flow.verify().await;
}
