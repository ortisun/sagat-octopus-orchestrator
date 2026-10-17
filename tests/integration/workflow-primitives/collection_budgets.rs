//! Budgets of a collection through the public path (T014-014/T014-015): a logical instance is
//! charged once however often it is retried, a derived activation uses the reservation made for
//! it, a reduction of the budget never blocks work already accepted, and a tenant capacity of one
//! still lets every iteration run (no parent-waits-for-slot cycle).
#![allow(clippy::expect_used)]

use reqwest::Method;
use serde_json::json;
use uuid::Uuid;

use crate::support::scope_flow::{
    Flow, all_compensations, forward, iteration, node, reserve_and_charge,
};

async fn usage(flow: &Flow) -> (i32, i32, i32) {
    let row: (i32, i32, i32) = sqlx::query_as(
        "SELECT max_instances, admitted_instances, reserved_instances \
         FROM run_instance_usage WHERE run_id = $1",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("run instance usage");
    row
}

async fn set_iteration_capacity(flow: &Flow, limit: u64) {
    let response = flow
        .api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": limit}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn a_retried_iteration_is_charged_once_and_a_derived_activation_uses_its_reservation() {
    let name = format!("collection-budget-once-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;
    flow.drive().await;
    let reserves = flow.poll().await;
    assert_eq!(reserves.len(), 2);
    let (_, admitted, reserved) = usage(&flow).await;
    let accepted = admitted + reserved;
    // The owner, then per iteration the reserve (+ its release), the charge (+ its refund) and the
    // return: five instances each, all charged up front.
    assert_eq!(accepted, 1 + 2 * 5, "owner plus five per iteration");

    // The first attempt of iteration 0 fails and is retried: same logical instance, same charge.
    flow.fail(&reserves[0], "TEMPORARILY_UNAVAILABLE", true)
        .await;
    flow.elapse_backoff().await;
    flow.drive().await;
    let retried = flow.poll().await;
    assert!(
        retried
            .iter()
            .any(|task| iteration(task) == 0 && node(task) == "reserve" && task.attempt == 2),
        "iteration 0 is leased again as attempt 2"
    );
    let (_, admitted_after, reserved_after) = usage(&flow).await;
    assert_eq!(
        (admitted_after, reserved_after),
        (admitted, reserved),
        "a retry allocates no instance"
    );

    // Everything succeeds but the second charge, which is declined: the saga undoes the rest with
    // the activations that were reserved, so nothing is charged beyond what was accepted.
    for task in retried
        .iter()
        .chain(reserves.iter().filter(|t| iteration(t) == 1))
    {
        flow.complete(task, json!({"hold": iteration(task)})).await;
    }
    flow.drive().await;
    let charges = flow.poll().await;
    flow.complete(&charges[0], json!({"charged": 1})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    all_compensations(&flow).await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let (_, admitted_end, reserved_end) = usage(&flow).await;
    // Derived activations consumed reservations (reserved fell, admitted rose); a reservation the
    // drained iterations no longer needed was handed back, so the sum can only shrink.
    assert!(
        admitted_end + reserved_end <= accepted,
        "nothing was charged beyond the {accepted} accepted up front: {admitted_end}+{reserved_end}"
    );
    assert!(
        admitted_end > admitted && reserved_end < reserved,
        "the compensations moved instances from reserved to admitted"
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_budget_reduced_to_what_is_accepted_still_lets_the_accepted_failure_be_undone() {
    let name = format!("collection-budget-reduced-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;
    forward(&flow, &[("reserve", 0), ("reserve", 1)]).await;
    let (_, admitted, reserved) = usage(&flow).await;
    // Tenant policy lowers the run to exactly what it has accepted, reservations included.
    sqlx::query("UPDATE run_instance_usage SET max_instances = $2 WHERE run_id = $1")
        .bind(flow.run.as_uuid())
        .bind(admitted + reserved)
        .execute(&flow.api.db.pool)
        .await
        .expect("reduce the budget");

    flow.drive().await;
    let charges = flow.poll().await;
    flow.complete(&charges[0], json!({"charged": 1})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    all_compensations(&flow).await;
    assert_eq!(
        flow.run_status().await,
        "COMPENSATED",
        "the reservations made before the reduction pay for the compensation"
    );
    let (max, admitted_end, reserved_end) = usage(&flow).await;
    assert_eq!(max, admitted + reserved, "the reduction was kept");
    assert!(
        admitted_end + reserved_end <= admitted + reserved,
        "the compensation stayed inside what was accepted before the reduction"
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_tenant_capacity_of_one_runs_every_iteration_to_the_end() {
    let name = format!("collection-budget-low-{}", Uuid::now_v7());
    let flow = Flow::start(
        &name,
        reserve_and_charge(&name, 4),
        json!([{"sku": "a"}, {"sku": "b"}, {"sku": "c"}, {"sku": "d"}]),
    )
    .await;
    set_iteration_capacity(&flow, 1).await;
    let mut peak_active = 0_i64;
    let mut order = Vec::new();
    for _ in 0..40 {
        flow.drive().await;
        let tasks = flow.poll().await;
        let active: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(active_count), 0)::bigint FROM iteration_scopes WHERE run_id = $1",
        )
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("active iterations");
        peak_active = peak_active.max(active);
        assert!(
            tasks.len() <= 1,
            "one slot, one iteration at a time: {tasks:?}"
        );
        if tasks.is_empty() {
            if flow.run_status().await == "SUCCEEDED" {
                break;
            }
            continue;
        }
        order.push((iteration(&tasks[0]), node(&tasks[0])));
        flow.complete(&tasks[0], json!({"ok": true})).await;
    }
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    assert_eq!(
        peak_active, 1,
        "the capacity of one was used and never exceeded"
    );
    assert_eq!(
        order.len(),
        8,
        "two activities in each of four iterations: {order:?}"
    );
    let mut iterations: Vec<u32> = order.iter().map(|(index, _)| *index).collect();
    iterations.dedup();
    assert_eq!(
        iterations,
        vec![0, 1, 2, 3],
        "each iteration ran to its end in turn"
    );
    flow.verify().await;
}
