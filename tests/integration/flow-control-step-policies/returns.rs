//! What a granted step that was never leased gives back (T022-100; T29 of the plan; FR-022-040, FR-022-046; D022-028):
//! its permit (`concurrency.released`) and its slot (`rate_limit.returned`) when its run is paused, when an operator
//! holds it for review, when its run is cancelled and when its run ends; a paused step waits again and asks again on
//! resume. At no instant does a key have more granted, unleased steps than its limit (invariant 13).
//!
//! Every run is checked by `verify_run`.

use sagat_application::runs::{ReviewAction, RunOperation};
use sagat_domain::execution::RunId;
use serde_json::{Value, json};

use super::readiness::{granted, queued, queued_after, rows, step};
use crate::support::engine::{World, activity, document, flaky, parallel_node, with};
use crate::support::flow_control::{gated_world, publish};

fn permit(node: Value) -> Value {
    with(
        node,
        "concurrency",
        json!({"group": "acct-r", "key": "input.accountId", "limit": 1}),
    )
}

fn slot(node: Value) -> Value {
    with(
        node,
        "rateLimit",
        json!({"group": "prov-r", "key": "input.accountId", "limit": 1, "per": "hour"}),
    )
}

async fn journal_has(w: &World, run: RunId, event: &str) -> usize {
    w.journal(run).await.iter().filter(|e| *e == event).count()
}

/// Invariant 13 at this instant: the granted, unleased steps of each key of the tests (concurrency holders and rate
/// grants without a lease instant, of steps that are not leased) never number more than its limit (1).
async fn unleased_holders(w: &World) -> i64 {
    let (held, slots): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders h JOIN steps s ON s.run_id = h.run_id AND s.node = h.node \
                  WHERE h.group_name = 'acct-r' AND s.status NOT IN ('LEASED', 'RUNNING')), \
                (SELECT count(*) FROM rate_limit_grants g JOIN steps s ON s.run_id = g.run_id AND s.node = g.node \
                  WHERE g.group_name = 'prov-r' AND g.leased_at IS NULL AND s.status NOT IN ('LEASED', 'RUNNING'))",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("granted, unleased");
    assert!(
        held <= 1 && slots <= 1,
        "more granted, unleased steps than the limit: {held} permits, {slots} slots"
    );
    held
}

/// A pause gives back the permit and the slot of a granted, unleased step and makes it wait again; the key serves the
/// next waiter meanwhile; the resume asks again and queues it behind that waiter.
#[tokio::test]
async fn a_pause_gives_back_a_granted_unleased_step_and_its_resume_asks_again() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("return-pause", "g", vec![slot(permit(activity("g")))]),
    )
    .await;
    let account = json!({"accountId": "acct-1"});
    let paused = w.start(workflow, &account).await;
    granted(&w, paused, "g", (true, true)).await;
    let waiter = w.start(workflow, &account).await;
    queued(&w, waiter, "g", (true, true)).await;
    assert_eq!(unleased_holders(&w).await, 1);

    w.operate_run(paused, RunOperation::Pause).await;
    assert_eq!(journal_has(&w, paused, "concurrency.released").await, 1);
    assert_eq!(journal_has(&w, paused, "rate_limit.returned").await, 1);
    assert_eq!(
        rows(&w, paused, "g").await,
        (0, 0, 0, 0),
        "nothing held, nothing queued"
    );
    let (status, kind, attempt) = step(&w, paused, "g").await;
    assert_eq!(
        (status.as_str(), kind.as_deref(), attempt),
        ("WAITING", Some("RATE_LIMIT"), 0),
        "it waits again, with no attempt spent"
    );
    w.assert_consistent(paused).await;
    assert_eq!(unleased_holders(&w).await, 0);

    // the key serves the next waiter; a pass grants nothing to the paused step
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    granted(&w, waiter, "g", (true, true)).await;
    assert_eq!(unleased_holders(&w).await, 1);
    assert_eq!(rows(&w, paused, "g").await, (0, 0, 0, 0));

    w.operate_run(paused, RunOperation::Resume).await;
    queued(&w, paused, "g", (true, true)).await;
    assert_eq!(unleased_holders(&w).await, 1);
    w.assert_consistent(waiter).await;
}

/// An operator's hold for review takes a granted, unleased step out of `READY`: its permit and slot are given back.
#[tokio::test]
async fn a_hold_for_review_gives_back_a_granted_unleased_step() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("return-review", "g", vec![slot(permit(activity("g")))]),
    )
    .await;
    let account = json!({"accountId": "acct-1"});
    let held = w.start(workflow, &account).await;
    granted(&w, held, "g", (true, true)).await;
    let waiter = w.start(workflow, &account).await;
    w.review(held, "g", ReviewAction::Hold, "check the account first")
        .await
        .expect("held for review");
    assert_eq!(journal_has(&w, held, "concurrency.released").await, 1);
    assert_eq!(journal_has(&w, held, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, held, "g").await, (0, 0, 0, 0));
    w.assert_consistent(held).await;
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    granted(&w, waiter, "g", (true, true)).await;
    // the operator's retry asks again for the same attempt: queued behind the waiter that took the key (correctness
    // review M2 of 2026-11-20)
    w.review(held, "g", ReviewAction::Retry, "the account is fine")
        .await
        .expect("retried");
    queued(&w, held, "g", (true, true)).await;
    assert_eq!(unleased_holders(&w).await, 1);
}

/// A cancellation gives back the permit and the slot of a granted, unleased step; the next waiter is granted.
#[tokio::test]
async fn a_cancellation_gives_back_a_granted_unleased_step() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("return-cancel", "g", vec![slot(permit(activity("g")))]),
    )
    .await;
    let account = json!({"accountId": "acct-1"});
    let cancelled = w.start(workflow, &account).await;
    granted(&w, cancelled, "g", (true, true)).await;
    let waiter = w.start(workflow, &account).await;
    w.operate_run(cancelled, RunOperation::Cancel).await;
    assert_eq!(journal_has(&w, cancelled, "concurrency.released").await, 1);
    assert_eq!(journal_has(&w, cancelled, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, cancelled, "g").await, (0, 0, 0, 0));
    w.assert_consistent(cancelled).await;
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    granted(&w, waiter, "g", (true, true)).await;
}

/// A run that ends (another branch fails it) gives back the permit and the slot of its granted, unleased step.
#[tokio::test]
async fn a_run_that_ends_gives_back_a_granted_unleased_step() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "return-end",
            "p",
            vec![
                parallel_node("p", &["x", "g"], "all"),
                activity("x"),
                slot(permit(activity("g"))),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.x"]).await;
    let account = json!({"accountId": "acct-1"});
    let ended = w.start(workflow, &account).await;
    granted(&w, ended, "g", (true, true)).await;
    let task = w.one_task_of(&session, "act.x").await;
    assert_eq!(task.run_id, ended);
    w.fail(&session, &task, false)
        .await
        .expect("x fails the run");
    assert_eq!(journal_has(&w, ended, "concurrency.released").await, 1);
    assert_eq!(journal_has(&w, ended, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, ended, "g").await, (0, 0, 0, 0));
    w.assert_consistent(ended).await;
}

/// A step that the retry pass makes ready while its run is paused is decided in that commit; granted, it gives the grant
/// back in the same commit and waits for the resume, with no row left (architecture review M2 of 2026-11-20).
#[tokio::test]
async fn a_retry_due_while_paused_is_granted_and_given_back_in_one_commit() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("return-retry-paused", "g", vec![permit(flaky("g", 3))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.fail(&session, &task, true)
        .await
        .expect("retry scheduled");
    w.operate_run(run, RunOperation::Pause).await;
    w.age_retry(run, "g").await;
    w.scheduler.tick().await;
    let journal = w.journal(run).await;
    let tail: Vec<&str> = journal
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        [
            "step.retry_due",
            "concurrency.acquired",
            "concurrency.released"
        ]
    );
    assert_eq!(rows(&w, run, "g").await, (0, 0, 0, 0), "nothing held");
    assert_eq!(step(&w, run, "g").await.0, "WAITING");
    w.assert_consistent(run).await;
    w.operate_run(run, RunOperation::Resume).await;
    granted(&w, run, "g", (true, false)).await;
}

/// A paused run whose step a pause gave back is cancelled: the step is cancelled with it and nothing is held.
#[tokio::test]
async fn a_cancellation_of_a_paused_run_ends_the_step_it_gave_back() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "return-cancel-paused",
            "g",
            vec![slot(permit(activity("g")))],
        ),
    )
    .await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, run, "g", (true, true)).await;
    w.operate_run(run, RunOperation::Pause).await;
    w.operate_run(run, RunOperation::Cancel).await;
    assert_eq!(step(&w, run, "g").await.0, "CANCELLED");
    assert_eq!(rows(&w, run, "g").await, (0, 0, 0, 0));
    w.assert_consistent(run).await;
}

/// D022-038 on pause (correctness review M1 of 2026-11-20): a pause gives back a granted step's slot (a window) and
/// makes the key of its queued step (a semaphore) due; it locks the semaphore before the window. With the semaphore
/// row held by another transaction, the pause waits for it and has not locked the window meanwhile.
#[tokio::test]
async fn a_pause_locks_its_keys_in_the_one_sorted_order() {
    let w = gated_world().await;
    let holder_workflow = publish(
        &w,
        document(
            "return-lock-holder",
            "h",
            vec![with(
                activity("h"),
                "concurrency",
                json!({"group": "s-wait", "key": "input.accountId", "limit": 1}),
            )],
        ),
    )
    .await;
    let workflow = publish(
        &w,
        document(
            "return-lock-order",
            "p",
            vec![
                parallel_node("p", &["g1", "g2"], "all"),
                with(
                    activity("g1"),
                    "rateLimit",
                    json!({"group": "w-ret", "key": "input.accountId", "limit": 5, "per": "hour"}),
                ),
                with(
                    activity("g2"),
                    "concurrency",
                    json!({"group": "s-wait", "key": "input.accountId", "limit": 1}),
                ),
            ],
        ),
    )
    .await;
    let account = json!({"accountId": "acct-1"});
    let other = w.start(holder_workflow, &account).await;
    granted(&w, other, "h", (true, false)).await;
    let run = w.start(workflow, &account).await;
    granted(&w, run, "g1", (false, true)).await;
    queued_after(&w, run, "g2", (true, false), 0).await;

    let mut holder = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE group_name = 's-wait' FOR UPDATE")
        .execute(&mut *holder)
        .await
        .expect("the semaphore held");
    let pausing = w.operate_run(run, RunOperation::Pause);
    let watch = async {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock' AND state = 'active' AND pid <> pg_backend_pid()",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the pause waits for the held semaphore");
        let mut probe = w.db.pool.begin().await.expect("a probe");
        let free = sqlx::query(
            "SELECT 1 FROM rate_limit_windows WHERE group_name = 'w-ret' FOR UPDATE NOWAIT",
        )
        .execute(&mut *probe)
        .await;
        probe.rollback().await.expect("probe ends");
        holder.rollback().await.expect("the semaphore freed");
        free.is_ok()
    };
    let ((), window_was_free) = tokio::join!(pausing, watch);
    assert!(
        window_was_free,
        "the pause locked the window before the semaphore: out of the one order"
    );
    assert_eq!(journal_has(&w, run, "rate_limit.returned").await, 1);
    w.assert_consistent(run).await;
}
