//! The lock order of the rate gate against PostgreSQL (task T022-081; FR-022-044, SC-022-019; ADR-0088 decision 4,
//! ADR-0050 decision 7): each real path that takes a window row is run against a reference transaction that takes the
//! same rows in the order of the slots (the run, then its active-run counter row, then the semaphore when one is
//! involved, then the windows: slots 3, 6, 7 and 8b), from every start (`support::lock_order::against_at`). A path that
//! held a later row while it waited for an earlier one would deadlock with the reference; each path lets the reference
//! through and completes. The two paths that hold no run lock (lease recording and the purge) let a reference that holds
//! the run first through, since they never wait for it. A control plan in the reversed order is a deadlock, so the
//! harness is not vacuous. Waits are observed with `pg_blocking_pids`, never with sleeps; a window that must pass is
//! waited for on the database clock with a bound (`until_past`).

use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunId, RunStatus};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::concurrency_locks::{counter_row, every_counter_row, key_rows, run_row};
use crate::run_admission::{age_start_deadline, start_rows};
use crate::support::engine::{World, activity, document, with};
use crate::support::flow_control::{gated_world, publish, until_past};
use crate::support::lock_order::{Lock, Outcome, Slot, against_at, deadlocks, run_plan};

const SECOND_US: i64 = 1_000_000;
const HOUR_US: i64 = 3_600 * SECOND_US;

/// Every window row of the tenant, in the order of ADR-0088 decision 4 (slot 8b).
fn window_rows(w: &World) -> Lock {
    Lock::statement(
        "the windows",
        format!(
            "SELECT 1 FROM rate_limit_windows WHERE tenant_id = '{}' \
              ORDER BY pool, limit_scope, group_name, key_digest FOR UPDATE",
            w.admin.tenant_id().as_uuid()
        ),
    )
}

/// A one-step workflow whose start takes a slot of `input.accountId`, `limit: 1` per `per`, and, when `permit`, a
/// permit of the same key with `limit: 1` too.
fn rated_start(name: &str, per: &str, permit: bool) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": 1, "per": per});
    if permit {
        doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    }
    doc
}

/// The run's own run-level grant and waiter rows: (grants, waiters).
async fn slot_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

async fn granted_us(w: &World, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM granted_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the run's grant")
}

/// A holder run of `acct-1` and a run buffered behind it, of `limit: 1` per `per`, with a permit too when `permit`.
async fn holder_and_waiter(name: &str, per: &str, permit: bool) -> (World, RunId, RunId) {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start(name, per, permit)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(slot_rows(&w, waiter).await, (0, 1), "buffered");
    (w, holder, waiter)
}

/// The slots a run's path takes, alone or with its permit: the run, its active-run counter row, the semaphore, the
/// windows.
fn run_path_reference(w: &World, run: RunId, permit: bool) -> Vec<(Slot, Lock)> {
    let mut reference = vec![
        (Slot::ExistingRuns, run_row(run)),
        (Slot::NewRuns, counter_row(run)),
    ];
    if permit {
        reference.push((Slot::Semaphores, key_rows(w)));
    }
    reference.push((Slot::RateWindows, window_rows(w)));
    reference
}

/// Transaction T19 (an admission): a start takes its active-run counter row (through the `runs` insert's trigger), then
/// its semaphore when it declares a permit, then its window, from every start; buffered behind the holder.
#[tokio::test]
async fn an_admission_takes_its_counter_row_then_the_semaphore_then_the_window() {
    for permit in [false, true] {
        let reference_len = if permit { 3 } else { 2 };
        for start in 0..reference_len - 1 {
            let (w, holder, waiter) =
                holder_and_waiter("rate-locks-admission", "hour", permit).await;
            let workflow = publish(&w, rated_start("rate-locks-admission-2", "hour", permit)).await;
            let mut reference = vec![(Slot::NewRuns, every_counter_row(&w, holder).await)];
            if permit {
                reference.push((Slot::Semaphores, key_rows(&w)));
            }
            reference.push((Slot::RateWindows, window_rows(&w)));
            let account = json!({"accountId": "acct-1"});
            let seen = against_at(&w.db.pool, &reference, start, w.start(workflow, &account)).await;
            assert_eq!(
                seen.reference,
                Outcome::Committed,
                "permit {permit}, start {start}: {:?}",
                seen.reference
            );
            let started = seen.path;
            assert_eq!(
                slot_rows(&w, started).await,
                (0, 1),
                "buffered behind the holder"
            );
            for run in [holder, waiter, started] {
                w.assert_consistent(run).await;
            }
        }
    }
}

/// Transaction T21 (a queue timeout): the timeout of a buffered start takes the run, then its active-run counter row
/// (the run ends), then the semaphore it dequeues from when it waits for a permit too, then the window, from every start.
#[tokio::test]
async fn the_timeout_of_a_buffered_start_takes_the_run_then_its_counter_row_then_the_window() {
    for permit in [false, true] {
        let reference_len = if permit { 4 } else { 3 };
        for start in 0..reference_len - 1 {
            let (w, holder, waiter) = holder_and_waiter("rate-locks-timeout", "hour", permit).await;
            age_start_deadline(&w, waiter).await;
            let reference = run_path_reference(&w, waiter, permit);
            let seen =
                against_at(&w.db.pool, &reference, start, w.scheduler.time_out_gates()).await;
            assert_eq!(
                seen.reference,
                Outcome::Committed,
                "permit {permit}, start {start}: {:?}",
                seen.reference
            );
            assert_eq!(seen.path.gate_waits_timed_out, 1, "{:?}", seen.path);
            assert_eq!(slot_rows(&w, waiter).await, (0, 0));
            for run in [holder, waiter] {
                w.assert_consistent(run).await;
            }
        }
    }
}

/// Transaction T24 (a cancellation): the cancellation of a buffered start takes the run, then its active-run counter row
/// (the run ends), then the semaphore and the window it dequeues from, from every start.
#[tokio::test]
async fn a_cancellation_of_a_buffered_start_takes_the_run_then_its_counter_row_then_the_window() {
    for permit in [false, true] {
        let reference_len = if permit { 4 } else { 3 };
        for start in 0..reference_len - 1 {
            let (w, holder, waiter) = holder_and_waiter("rate-locks-cancel", "hour", permit).await;
            let reference = run_path_reference(&w, waiter, permit);
            let seen = against_at(
                &w.db.pool,
                &reference,
                start,
                w.operate(waiter, RunOperation::Cancel),
            )
            .await;
            assert_eq!(
                seen.reference,
                Outcome::Committed,
                "permit {permit}, start {start}: {:?}",
                seen.reference
            );
            seen.path
                .expect("the cancellation commits after the reference");
            assert_eq!(slot_rows(&w, waiter).await, (0, 0), "dequeued");
            for run in [holder, waiter] {
                w.assert_consistent(run).await;
            }
        }
    }
}

/// Holds `lock` in a transaction of its own while `path` runs, and returns what `path` returned: the path must complete
/// within a bound while the lock is held (it never waits for that row), and the lock is let go after it.
async fn while_held<T>(w: &World, lock: &Lock, path: impl std::future::Future<Output = T>) -> T {
    let mut held = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(sqlx::AssertSqlSafe(lock.sql.clone()))
        .execute(&mut *held)
        .await
        .unwrap_or_else(|error| panic!("`{}`: {error}", lock.name));
    let done = crate::support::bounded::within(
        format!("the path while `{}` is held", lock.name),
        std::time::Duration::from_secs(30),
        path,
    )
    .await;
    held.commit().await.expect("let go");
    done
}

/// Transaction T20 (a grant from the queue): once the holder's slot has aged out (`per: second`), the pass grants the
/// waiter under its run, then the semaphore when it waits for a permit too, then the window. The pass takes a waiter's
/// run with `SKIP LOCKED` and holds nothing it would wait on while it does, so a run held elsewhere is passed by (the
/// pass completes, grants nothing, and the next one grants); with a permit, a reference that takes the semaphore and
/// then the windows lets the pass through, from either start.
#[tokio::test]
async fn a_grant_from_the_queue_takes_the_run_then_the_semaphore_then_the_window() {
    for permit in [false, true] {
        let (w, holder, waiter) = holder_and_waiter("rate-locks-grant", "second", permit).await;
        if permit {
            let session = w.session(&["act.g"]).await;
            let task = w.one_task(&session).await;
            assert_eq!(task.run_id, holder);
            w.complete(&session, &task, &json!({})).await.expect("done");
            assert_eq!(w.run_status(holder).await, RunStatus::Succeeded);
        }
        until_past(&w.db.pool, granted_us(&w, holder).await + SECOND_US).await;
        let passed = while_held(&w, &run_row(waiter), w.scheduler.grant_gates()).await;
        assert_eq!(
            passed.gates_granted, 0,
            "a held run is passed by: {passed:?}"
        );
        if permit {
            for start in 0..1 {
                let reference = [
                    (Slot::Semaphores, key_rows(&w)),
                    (Slot::RateWindows, window_rows(&w)),
                ];
                let seen =
                    against_at(&w.db.pool, &reference, start, w.scheduler.grant_gates()).await;
                assert_eq!(
                    seen.reference,
                    Outcome::Committed,
                    "start {start}: {:?}",
                    seen.reference
                );
                assert_eq!(seen.path.gates_granted, 1, "{:?}", seen.path);
            }
        } else {
            assert_eq!(
                w.scheduler.grant_gates().await.gates_granted,
                1,
                "the next pass grants it"
            );
        }
        assert_eq!(
            slot_rows(&w, waiter).await,
            (1, 0),
            "permit {permit}: granted"
        );
        if permit {
            assert_eq!(start_rows(&w, waiter).await, (1, 0), "with its permit");
        }
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
}

/// Transaction T22 (readiness): a completion whose next step declares `rateLimit` takes the run, then the window of the
/// next step's key, from every start; its slot is free, so the step is granted.
#[tokio::test]
async fn a_readiness_round_takes_the_run_then_the_window() {
    // two slots: the reference starts at the run (the round's first lock), so only start 0 orders anything
    for start in 0..1 {
        let w = gated_world().await;
        let doc = document(
            "rate-locks-readiness",
            "a",
            vec![
                with(activity("a"), "next", json!("g")),
                with(
                    activity("g"),
                    "rateLimit",
                    json!({"group": "provider", "key": "input.accountId", "limit": 2, "per": "hour"}),
                ),
            ],
        );
        let workflow = publish(&w, doc).await;
        // a window of the key exists before the round (an earlier run holds its slot), so the reference can hold it
        let earlier = publish(
            &w,
            document(
                "rate-locks-readiness-earlier",
                "g",
                vec![with(
                    activity("g"),
                    "rateLimit",
                    json!({"group": "provider", "key": "input.accountId", "limit": 2, "per": "hour"}),
                )],
            ),
        )
        .await;
        let _holding = w.start(earlier, &json!({"accountId": "acct-1"})).await;
        let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
        let session = w.session(&["act.a"]).await;
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, run);
        let reference = [
            (Slot::ExistingRuns, run_row(run)),
            (Slot::RateWindows, window_rows(&w)),
        ];
        let output = json!({});
        let seen = against_at(
            &w.db.pool,
            &reference,
            start,
            w.complete(&session, &task, &output),
        )
        .await;
        assert_eq!(
            seen.reference,
            Outcome::Committed,
            "start {start}: {:?}",
            seen.reference
        );
        seen.path
            .expect("the completion commits after the reference");
        let granted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("grants");
        assert_eq!(granted, 1, "the next step is granted its slot");
        w.assert_consistent(run).await;
    }
}

/// ADR-0088 decision 6 (lease recording, transaction T23) and transaction T25 (the purge): the two paths that hold no
/// run lock write only window and grant rows, so each completes while another transaction holds the run's row: neither
/// waits for it.
#[tokio::test]
async fn lease_recording_and_the_purge_take_no_run() {
    let w = gated_world().await;
    let doc = document(
        "rate-locks-lease",
        "g",
        vec![with(
            activity("g"),
            "rateLimit",
            json!({"group": "provider", "key": "input.accountId", "limit": 1, "per": "hour"}),
        )],
    );
    let workflow = publish(&w, doc).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let session = w.session(&["act.g"]).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, run);
    let recorded = while_held(&w, &run_row(run), w.scheduler.record_rate_leases()).await;
    assert_eq!(recorded.rate_leases_recorded, 1, "{recorded:?}");
    w.complete(&session, &task, &json!({})).await.expect("done");
    let leased: OffsetDateTime = sqlx::query_scalar(
        "SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("a lease");
    // the purge as of the first instant the grant may go (its window and a minute, and a microsecond more)
    let purge_at = leased + time::Duration::microseconds(HOUR_US + 60 * SECOND_US + 1);
    let purged = while_held(
        &w,
        &run_row(run),
        w.store.purge_rate_grants_as_of(purge_at, 64, 100),
    )
    .await;
    assert_eq!(purged.expect("purge"), 1, "the grant goes");
    w.assert_consistent(run).await;
}

/// The control: a plan that takes the windows and then the run, against the reference in the order of the slots, is a
/// deadlock (one victim), so a path in the wrong order could not pass the tests above.
#[tokio::test]
async fn a_plan_that_takes_a_window_before_its_run_deadlocks_with_the_reference() {
    let (w, holder, _) = holder_and_waiter("rate-locks-control", "hour", false).await;
    let reference = [
        (Slot::ExistingRuns, run_row(holder)),
        (Slot::RateWindows, window_rows(&w)),
    ];
    let reversed = [window_rows(&w), run_row(holder)];
    let seen = against_at(&w.db.pool, &reference, 0, run_plan(&w.db.pool, &reversed)).await;
    assert_eq!(
        deadlocks(&[seen.reference.clone(), seen.path.clone()]),
        1,
        "{seen:?}"
    );
}
