//! The readiness gate (T022-096; T28 of the plan; FR-022-051; D022-020, D022-044) against PostgreSQL: a step of a node
//! that declares `concurrency` or `rateLimit` is granted or queued in the commit that makes it ready, whatever made it
//! ready, and is never offered to a worker before every gate it declares is granted. Every run here is checked by
//! `verify_run`, which replays the journal alone and checks invariant 11.
//!
//! The runs are admitted by the store's test-only admission of step gates (`step-gates-for-tests`): this build lists no
//! flow-control capability (D022-037), and nothing grants a queued step (T022-097) or times it out (T022-098) yet.

use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunId, RunStatus};
use serde_json::{Value, json};

use crate::support::engine::{
    World, activity, child_node, ctx, document, flaky, timer_node, wait_node, with,
};
use crate::support::flow_control::{gated_world, publish};

pub(super) fn serial(node: Value) -> Value {
    with(
        node,
        "concurrency",
        json!({"key": "input.accountId", "limit": 1}),
    )
}

pub(super) fn rated(node: Value) -> Value {
    with(
        node,
        "rateLimit",
        json!({"key": "input.accountId", "limit": 1, "per": "hour"}),
    )
}

/// The flow-control rows of a run's step: (holders, waiters, grants, waiting rate rows).
pub(super) async fn rows(w: &World, run: RunId, node: &str) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND node = $2), \
                (SELECT count(*) FROM concurrency_waiters WHERE run_id = $1 AND node = $2), \
                (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND node = $2), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND node = $2)",
    )
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// What the step's row says: (status, waiting kind, attempt).
pub(super) async fn step(w: &World, run: RunId, node: &str) -> (String, Option<String>, i32) {
    sqlx::query_as(
        "SELECT status, waiting_kind, attempt FROM steps WHERE run_id = $1 AND node = $2",
    )
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("step row")
}

/// How many task-ready hints the step was announced with.
pub(super) async fn hints(w: &World, run: RunId, node: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE event_type = 'task.ready' AND aggregate_id = $1 \
         AND payload->>'node' = $2",
    )
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("hints")
}

/// The step `node` of `run` was granted at once: READY, its rows written, its decision journaled, consistent.
pub(super) async fn granted(w: &World, run: RunId, node: &str, gates: (bool, bool)) {
    assert_eq!(step(w, run, node).await.0, "READY", "granted at once");
    let (holders, waiters, grants, rate_waiters) = rows(w, run, node).await;
    assert_eq!(
        (holders, waiters, grants, rate_waiters),
        (i64::from(gates.0), 0, i64::from(gates.1), 0)
    );
    let journal = w.journal(run).await;
    if gates.0 {
        assert!(
            journal.iter().any(|e| e == "concurrency.acquired"),
            "{journal:?}"
        );
    }
    if gates.1 {
        assert!(
            journal.iter().any(|e| e == "rate_limit.granted"),
            "{journal:?}"
        );
    }
    w.assert_consistent(run).await;
}

/// The step `node` of `run` waits for its gates: WAITING, a waiter row per gate, no attempt, consistent.
pub(super) async fn queued(w: &World, run: RunId, node: &str, gates: (bool, bool)) {
    queued_after(w, run, node, gates, 0).await;
}

/// [`queued`], for a step whose earlier attempts numbered `attempts`.
pub(super) async fn queued_after(
    w: &World,
    run: RunId,
    node: &str,
    gates: (bool, bool),
    attempts: i32,
) {
    let (status, kind, attempt) = step(w, run, node).await;
    assert_eq!(status, "WAITING");
    assert_eq!(
        kind.as_deref(),
        Some(if gates.1 { "RATE_LIMIT" } else { "CONCURRENCY" }),
        "the first gate it waits for, rate first"
    );
    assert_eq!(attempt, attempts, "waiting spends no attempt");
    let (holders, waiters, grants, rate_waiters) = rows(w, run, node).await;
    assert_eq!(
        (holders, waiters, grants, rate_waiters),
        (0, i64::from(gates.0), 0, i64::from(gates.1)),
        "both or neither: queued for every gate it declares"
    );
    w.assert_consistent(run).await;
}

// -------------------------------------------------------------------------------------- one test per source

/// Source: the run's start (the entry step).
#[tokio::test]
async fn the_entry_step_is_decided_by_the_admission_that_releases_it() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("entry-gated", "g", vec![serial(activity("g"))]),
    )
    .await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, first, "g", (true, false)).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, second, "g", (true, false)).await;
    assert_eq!(
        hints(&w, second, "g").await,
        0,
        "no task is announced for a waiting step"
    );
    let other_key = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    granted(&w, other_key, "g", (true, false)).await;
    // the worker gets the two granted steps and never the queued one
    let session = w.session(&["act.g"]).await;
    let tasks = w.poll(&session, 8).await;
    let mut leased: Vec<_> = tasks.iter().map(|task| task.run_id).collect();
    leased.sort();
    let mut expected = vec![first, other_key];
    expected.sort();
    assert_eq!(leased, expected);
    w.assert_consistent(second).await;
}

/// Source: a worker completion releases the gated step.
#[tokio::test]
async fn a_worker_completion_releases_a_gated_step_into_its_gates() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "completion-gated",
            "a",
            vec![
                with(activity("a"), "next", json!("g")),
                rated(serial(activity("g"))),
            ],
        ),
    )
    .await;
    let session_a = w.session(&["act.a"]).await;
    let session = w.session(&["act.g"]).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let mut tasks = w.poll(&session_a, 2).await;
    assert_eq!(tasks.len(), 2);
    tasks.sort_by_key(|task| usize::from(task.run_id != first));
    for task in &tasks {
        w.complete(&session_a, task, &json!({}))
            .await
            .expect("a done");
    }
    granted(&w, first, "g", (true, true)).await;
    queued(&w, second, "g", (true, true)).await;
    // the claim of a gated step is the plain claim (T022-076): its grant's lease instant is recorded by the pass
    // (T23, T022-074), and until then the slot counts as unleased, which never admits more than the limit
    let task = w.one_task_of(&session, "act.g").await;
    assert_eq!(task.run_id, first);
    let leased: Option<time::OffsetDateTime> = sqlx::query_scalar(
        "SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
    )
    .bind(first.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("grant");
    assert!(
        leased.is_none(),
        "the claim does not touch the grant: its lease instant is the pass's (T022-074)"
    );
    // the attempt's end gives the permit back; the run cannot finish holding it
    w.complete(&session, &task, &json!({}))
        .await
        .expect("g done");
    assert_eq!(w.run_status(first).await, RunStatus::Succeeded);
    assert_eq!(rows(&w, first, "g").await.0, 0, "the permit is given back");
    let journal = w.journal(first).await;
    assert!(
        journal.iter().any(|e| e == "concurrency.released"),
        "{journal:?}"
    );
    w.assert_consistent(first).await;
    assert!(
        w.poll(&session, 8).await.is_empty(),
        "the queued step is not offered"
    );
}

/// Source: the retry pass (`step.retry_due`), after the first attempt's permit was given back.
#[tokio::test]
async fn the_retry_pass_decides_the_next_attempt() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("retry-gated", "g", vec![serial(flaky("g", 3))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.fail(&session, &task, true)
        .await
        .expect("retry scheduled");
    assert_eq!(
        rows(&w, run, "g").await.0,
        0,
        "the failed attempt gave its permit back"
    );
    // hold the key: another run takes the only permit before the retry is due
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, holder, "g", (true, false)).await;
    w.age_retry(run, "g").await;
    w.scheduler.tick().await;
    queued_after(&w, run, "g", (true, false), 1).await;
    let journal = w.journal(run).await;
    assert!(journal.iter().any(|e| e == "step.retry_due"), "{journal:?}");
}

/// Source: a timer fires and releases the gated step.
#[tokio::test]
async fn a_fired_timer_releases_a_gated_step_into_its_gates() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "timer-gated",
            "t",
            vec![
                with(timer_node("t", 1), "next", json!("g")),
                serial(activity("g")),
            ],
        ),
    )
    .await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.age_due(run, "t").await;
    w.scheduler.tick().await;
    granted(&w, run, "g", (true, false)).await;
}

/// Source: a signal satisfies a wait and releases the gated step.
#[tokio::test]
async fn a_signal_releases_a_gated_step_into_its_gates() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "signal-gated",
            "w",
            vec![
                with(
                    wait_node("w", "account.ready", "input.accountId", 300_000, None),
                    "next",
                    json!("g"),
                ),
                rated(activity("g")),
            ],
        ),
    )
    .await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.runs
        .signal(&w.admin, run, "account.ready", b"{}", "signal-1", &ctx())
        .await
        .expect("signal");
    granted(&w, run, "g", (false, true)).await;
}

/// Source: a resume releases what a pause held back.
#[tokio::test]
async fn a_resume_releases_a_gated_step_into_its_gates() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "resume-gated",
            "a",
            vec![
                with(activity("a"), "next", json!("g")),
                serial(activity("g")),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.a", "act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task_of(&session, "act.a").await;
    w.operate_run(run, RunOperation::Pause).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("a done");
    assert_eq!(
        rows(&w, run, "g").await,
        (0, 0, 0, 0),
        "a paused run decides nothing"
    );
    w.operate_run(run, RunOperation::Resume).await;
    granted(&w, run, "g", (true, false)).await;
}

/// Source: a finished child releases the gated step of its parent.
#[tokio::test]
async fn a_finished_child_releases_a_gated_step_of_its_parent() {
    let w = gated_world().await;
    w.publish(document("leaf", "x", vec![activity("x")])).await;
    let workflow = publish(
        &w,
        document(
            "child-gated",
            "c",
            vec![child_node("c", "leaf", Some("g")), serial(activity("g"))],
        ),
    )
    .await;
    let session = w.session(&["act.x", "act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.scheduler.tick().await;
    let task = w.one_task_of(&session, "act.x").await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("child done");
    w.scheduler.tick().await;
    granted(&w, run, "g", (true, false)).await;
}

// -------------------------------------------------------------------------------------- the failure cases

/// FR-022-054: a key that does not evaluate fails the step through its route, with no attempt spent, never granted.
#[tokio::test]
async fn a_key_that_does_not_evaluate_fails_the_step_through_its_route() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "key-invalid",
            "g",
            vec![
                with(
                    with(
                        activity("g"),
                        "concurrency",
                        json!({"key": "input.missing.deeper", "limit": 1}),
                    ),
                    "onError",
                    json!("recover"),
                ),
                activity("recover"),
            ],
        ),
    )
    .await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let (status, failure, attempt): (String, Option<String>, i32) = sqlx::query_as(
        "SELECT status, failure_code, attempt FROM steps WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("step");
    assert_eq!(
        (status.as_str(), failure.as_deref(), attempt),
        ("FAILED", Some("FLOW_CONTROL_KEY_INVALID"), 0)
    );
    assert_eq!(
        rows(&w, run, "g").await,
        (0, 0, 0, 0),
        "never granted nor queued"
    );
    assert_eq!(
        step(&w, run, "recover").await.0,
        "READY",
        "the failure route takes it"
    );
    w.assert_consistent(run).await;
}

/// Invariant 11: a gated step written READY without a grant is found by `verify_run`.
#[tokio::test]
async fn a_seeded_ready_step_without_a_grant_fails_verify_run() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("seeded-breach", "g", vec![serial(activity("g"))]),
    )
    .await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, run, "g", (true, false)).await;
    // a writer that bypassed the gate: the waiting step made claimable without a grant
    sqlx::query(
        "UPDATE steps SET status = 'READY', waiting_kind = NULL, flow_control_gates = NULL \
         WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("seeded");
    let breach = w.store.verify_run(w.admin.tenant_id(), run).await;
    assert!(
        breach
            .as_ref()
            .is_err_and(|why| why.contains("projection differs from the journal")),
        "{breach:?}"
    );
}

/// A cancelled run's queued step leaves its queues (rate first) before it is cancelled, and nothing of it is left.
#[tokio::test]
async fn a_cancelled_run_takes_its_queued_step_out_of_every_queue() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("cancel-gated", "g", vec![rated(serial(activity("g")))]),
    )
    .await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, holder, "g", (true, true)).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, run, "g", (true, true)).await;
    w.operate_run(run, RunOperation::Cancel).await;
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    assert_eq!(
        rows(&w, run, "g").await,
        (0, 0, 0, 0),
        "no waiter row is left"
    );
    let journal = w.journal(run).await;
    let at = |name: &str| {
        journal
            .iter()
            .position(|e| e == name)
            .unwrap_or_else(|| panic!("{name}: {journal:?}"))
    };
    assert!(at("rate_limit.dequeued") < at("concurrency.dequeued"));
    assert!(at("concurrency.dequeued") < at("step.cancelled"));
    let hint: Option<time::OffsetDateTime> = sqlx::query_scalar(
        "SELECT next_eligible_at FROM concurrency_semaphores WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("semaphore");
    assert!(hint.is_none(), "no waiter is left, so no hint is set");
    w.assert_consistent(run).await;
}

/// The correctness and architecture reviews of 2026-11-19 (finding 1 / M2): a commit that releases a permit and decides
/// a successor on the same key gives the permit back before it counts, so a `limit: 1` successor is granted, not queued
/// behind a permit that frees in that very commit.
#[tokio::test]
async fn a_permit_released_and_a_successor_decided_in_one_commit_is_counted_once() {
    let w = gated_world().await;
    let shared = json!({"group": "account-serial", "key": "input.accountId", "limit": 1});
    let workflow = publish(
        &w,
        document(
            "same-key-chain",
            "x",
            vec![
                with(
                    with(activity("x"), "concurrency", shared.clone()),
                    "next",
                    json!("y"),
                ),
                with(activity("y"), "concurrency", shared),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.x", "act.y"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, run, "x", (true, false)).await;
    let task = w.one_task_of(&session, "act.x").await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("x done");
    granted(&w, run, "y", (true, false)).await;
    assert_eq!(rows(&w, run, "x").await.0, 0, "x gave its permit back");
}

/// D022-038 with a held lock: a commit that releases a permit of group `a-…` and decides a successor of group `b-…`
/// takes the semaphore of `a` before the one of `b` (sorted), so while another transaction holds `a`'s row it holds
/// nothing of `b`'s; the decision round took them in decision order first (the reviews of 2026-11-19).
#[tokio::test]
async fn a_release_and_a_decision_lock_their_keys_in_one_sorted_order() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "lock-order",
            "x",
            vec![
                with(
                    with(
                        activity("x"),
                        "concurrency",
                        json!({"group": "a-release", "limit": 5}),
                    ),
                    "next",
                    json!("y"),
                ),
                with(
                    activity("y"),
                    "concurrency",
                    json!({"group": "b-decide", "limit": 5}),
                ),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.x", "act.y"]).await;
    let run = w.start(workflow, &json!({})).await;
    let task = w.one_task_of(&session, "act.x").await;
    // `b-decide` exists before the commit, so a lock taken on it would be seen
    let other = w.start(workflow, &json!({})).await;
    let other_task = w.one_task_of(&session, "act.x").await;
    assert_eq!(other_task.run_id, other);
    w.complete(&session, &other_task, &json!({}))
        .await
        .expect("the other run reaches y");
    let mut holder = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE group_name = 'a-release' FOR UPDATE")
        .execute(&mut *holder)
        .await
        .expect("a's row held");
    let completing = {
        let w = &w;
        let session = &session;
        let task = &task;
        async move { w.complete(session, task, &json!({})).await }
    };
    let watch = async {
        // the barrier: wait until the completion waits for a lock, then look at `b`'s row
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND wait_event_type = 'Lock' AND state = 'active'",
            )
            .fetch_one(&w.db.pool)
            .await
            .expect("activity");
            if waiting > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let mut probe = w.db.pool.begin().await.expect("a probe");
        let free = sqlx::query(
            "SELECT 1 FROM concurrency_semaphores WHERE group_name = 'b-decide' FOR UPDATE NOWAIT",
        )
        .execute(&mut *probe)
        .await;
        probe.rollback().await.expect("probe ends");
        holder.rollback().await.expect("a's row freed");
        free.is_ok()
    };
    let (done, b_was_free) = tokio::join!(completing, watch);
    done.expect("x done");
    assert!(
        b_was_free,
        "the commit locked b before a: out of the one order"
    );
    granted(&w, run, "y", (true, false)).await;
}
