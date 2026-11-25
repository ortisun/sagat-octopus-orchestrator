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

/// Decision D022-074 (gap G022-020; FR-022-021; ADR-0056 decision 7): a namespace `concurrency_limit` lowered while starts
/// are queued applies to them at once. Declared 3: three holders and two starts queued with that limit; the namespace then
/// lowers its ceiling to 1. No permit is revoked. When the three holders end, one pass grants one queued start only (a
/// grant against the filed limit of 3 would grant both); the other waits until that holder ends too.
#[tokio::test]
async fn a_lowered_concurrency_ceiling_applies_to_the_queued_starts_and_revokes_no_permit() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("lifecycle-lowered-queued", 3)).await;
    let session = w.session(&["act.g"]).await;
    let input = json!({"accountId": "acct-1"});
    let mut holders = Vec::new();
    for _ in 0..3 {
        holders.push(w.start(workflow, &input).await);
    }
    let first = w.start(workflow, &input).await;
    let second = w.start(workflow, &input).await;
    for run in [first, second] {
        assert_eq!(start_rows(&w, run).await, (0, 1), "the key is full");
    }
    set(&w, QuotaResource::ConcurrencyLimit, 1).await;
    for run in &holders {
        assert_eq!(start_rows(&w, *run).await, (1, 0), "nothing is revoked");
    }
    for task in w.poll(&session, 8).await {
        w.complete(&session, &task, &json!({})).await.expect("done");
    }
    for run in &holders {
        assert_eq!(start_rows(&w, *run).await, (0, 0), "its permit given back");
    }
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 1,
        "the lowered ceiling decides the queued starts: {report:?}"
    );
    assert_eq!(start_rows(&w, first).await, (1, 0), "the oldest is granted");
    assert_eq!(start_rows(&w, second).await, (0, 1), "the other waits");
    assert_eq!(
        holder_limit(&w, first).await,
        1,
        "the holder records the limit its grant was decided with, not the filed 3 (re-review minor 4)"
    );
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, first);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(start_rows(&w, second).await, (1, 0));
    for run in holders.iter().copied().chain([first, second]) {
        w.assert_consistent(run).await;
    }
}

/// The limit the holder row of `run`'s start records.
async fn holder_limit(
    w: &crate::support::engine::World,
    run: sagat_domain::execution::RunId,
) -> i32 {
    sqlx::query_scalar(
        "SELECT declared_limit FROM concurrency_holders WHERE run_id = $1 AND holder_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the holder row")
}

/// Decision D022-076 item 7 (re-review minor 7): a raised namespace `concurrency_limit` applies to the starts already
/// queued, up to their declaration. Declared 3, the namespace's ceiling 1: one holder, and two starts queued under it
/// (filed with 1). The namespace raises its ceiling to 3: one pass grants both queued starts at once (a grant bounded by
/// the filed 1 would grant neither while the holder holds), and each holder records the limit of 3 it was decided with.
#[tokio::test]
async fn a_raised_concurrency_ceiling_applies_to_the_queued_starts_up_to_their_declaration() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("lifecycle-raised-queued", 3)).await;
    set(&w, QuotaResource::ConcurrencyLimit, 1).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let first = w.start(workflow, &input).await;
    let second = w.start(workflow, &input).await;
    for run in [first, second] {
        assert_eq!(start_rows(&w, run).await, (0, 1), "a ceiling of one");
    }
    set(&w, QuotaResource::ConcurrencyLimit, 3).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 2,
        "the raised ceiling applies to the queued starts: {report:?}"
    );
    for run in [first, second] {
        assert_eq!(start_rows(&w, run).await, (1, 0));
        assert_eq!(
            holder_limit(&w, run).await,
            3,
            "decided with the raised ceiling"
        );
    }
    let fourth = w.start(workflow, &input).await;
    assert_eq!(
        start_rows(&w, fourth).await,
        (0, 1),
        "never above the declaration of 3"
    );
    for run in [holder, first, second, fourth] {
        w.assert_consistent(run).await;
    }
}
