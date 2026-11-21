//! US02 lifecycle of a run's permit (T022-025; FR-022-021): a run deadline dequeues a queued start; a lowered limit never
//! revokes a held permit and applies to the next decision; a queued run counts as an active run of the namespace.

use sagat_application::error::AppError;
use sagat_domain::execution::RunStatus;
use sagat_domain::quota::QuotaResource;
use serde_json::json;

use crate::run_admission::{count, serial_start, set, start_rows};
use crate::support::flow_control::{gated_world, publish};

/// FR-022-021: a queued start whose run deadline passes is dequeued (`concurrency.dequeued`) and the run ends at its
/// deadline without having held a permit; the holder keeps its permit.
#[tokio::test]
async fn a_run_deadline_dequeues_a_queued_start() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("lifecycle-deadline", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.age_run_deadline(waiter).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.deadlines_enforced, 1, "{report:?}");
    assert!(w.run_status(waiter).await.is_terminal());
    assert_eq!(count(&w, waiter, "concurrency.dequeued").await, 1);
    assert_eq!(count(&w, waiter, "step.leased").await, 0);
    assert_eq!(start_rows(&w, waiter).await, (0, 0));
    assert_eq!(start_rows(&w, holder).await, (1, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// ADR-0056 decision 7, FR-022-021: lowering the namespace's `concurrency_limit` below the holders never revokes a
/// permit; the next start waits until the holders fall below the new limit.
#[tokio::test]
async fn a_lowered_limit_revokes_nothing_and_applies_to_the_next_start() {
    let w = gated_world().await;
    // declared 3: room for a third holder until the namespace lowers its ceiling below the two that hold
    let workflow = publish(&w, serial_start("lifecycle-lowered", 3)).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, second).await, (1, 0), "both hold");
    set(&w, QuotaResource::ConcurrencyLimit, 1).await;
    for run in [first, second] {
        assert_eq!(start_rows(&w, run).await, (1, 0), "nothing is revoked");
        assert_eq!(w.run_status(run).await, RunStatus::Running);
    }
    let third = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        start_rows(&w, third).await,
        (0, 1),
        "the lowered limit decides"
    );
    for run in [first, second, third] {
        w.assert_consistent(run).await;
    }
}

/// FR-022-021 (active-run accounting): a queued run is an active run of the namespace, so `active_runs` counts it and
/// refuses a start beyond it whatever the key.
#[tokio::test]
async fn a_queued_run_counts_as_an_active_run() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("lifecycle-active", 1)).await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    set(&w, QuotaResource::ActiveRuns, 2).await;
    let refused = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-2"}),
            "lifecycle-active-3",
            None,
        )
        .await
        .expect_err("two active runs, one of them queued");
    assert!(matches!(refused, AppError::QuotaExceeded(_)), "{refused:?}");
}

/// FR-022-021 ("pausing a holder MUST NOT release its permit", for a run holder; G022-007): a paused run keeps the permit
/// of its start, the run queued behind it is not granted, and a resume goes on with the same permit.
#[tokio::test]
async fn a_paused_run_keeps_the_permit_of_its_start() {
    use sagat_application::runs::RunOperation;
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("lifecycle-pause", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.operate_run(holder, RunOperation::Pause).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0), "the permit is kept");
    assert_eq!(count(&w, holder, "concurrency.released").await, 0);
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        0,
        "nothing was freed"
    );
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    w.operate_run(holder, RunOperation::Resume).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0));
    assert_eq!(
        count(&w, holder, "concurrency.acquired").await,
        1,
        "the same permit"
    );
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}
