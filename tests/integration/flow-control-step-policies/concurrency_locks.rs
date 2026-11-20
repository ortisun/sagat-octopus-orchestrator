//! The lock order of a run's permit against PostgreSQL (T022-029; FR-022-019, FR-022-020; ADR-0056 decision 2,
//! ADR-0050 decision 7): each real path that takes a key's row is run against a reference transaction that takes the
//! same rows in the order of the slots (the run, then its active-run counter row, then the key: slots 3, 6 and 7), from
//! every start (`support::lock_order::against_at`). A path that held a later row while it waited for an earlier one
//! would deadlock with the reference; each path lets the reference through and completes. A control plan in the reversed
//! order is a deadlock, so the harness is not vacuous. Waits are observed with `pg_blocking_pids`, never with sleeps.
//!
//! An admission takes its new run's counter row (through the `runs` insert) before it files its key. The grant routine
//! takes a waiter's run with `SKIP LOCKED` (it never waits for a run while it holds a key), and the readiness round and
//! the returns of a pause have their own held-lock tests (`readiness`, `returns`). `flow_control_counts` (slot 8b) does
//! not exist yet (T022-105).

use sagat_application::runs::RunOperation;
use sagat_domain::execution::RunId;
use serde_json::json;

use crate::run_admission::{serial_start, start_rows};
use crate::support::engine::World;
use crate::support::flow_control::{gated_world, publish};
use crate::support::lock_order::{Lock, Outcome, Slot, against_at, deadlocks, run_plan};

fn run_row(run: RunId) -> Lock {
    Lock::statement(
        "the run",
        format!(
            "SELECT 1 FROM runs WHERE id = '{}' FOR UPDATE",
            run.as_uuid()
        ),
    )
}

/// The run's active-run counter row (ADR-0050 slot 6), as `run_active_counts_lock_slot` takes it.
fn counter_row(run: RunId) -> Lock {
    Lock::statement(
        "the run's active-run counter row",
        format!(
            "SELECT 1 FROM run_active_counts c JOIN runs r ON r.tenant_id = c.tenant_id AND r.namespace_id = c.namespace_id \
              WHERE r.id = '{0}' AND c.slot = (get_byte(uuid_send('{0}'::uuid), 15) & 15)::smallint FOR UPDATE OF c",
            run.as_uuid()
        ),
    )
}

/// Every run-scope key row of the tenant (slot 7).
fn key_rows(w: &World) -> Lock {
    Lock::statement(
        "the key",
        format!(
            "SELECT 1 FROM concurrency_semaphores WHERE tenant_id = '{}' AND permit_scope = 'RUN' FOR UPDATE",
            w.admin.tenant_id().as_uuid()
        ),
    )
}

/// A holder run of `acct-1` whose step is leased by `session`, and a run queued behind it.
async fn holder_and_waiter() -> (World, crate::support::engine::Session, RunId, RunId) {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("permit-locks", 1)).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    (w, session, holder, waiter)
}

/// T8/T14: the completion that ends a gated run takes the run, then its active-run counter row, then the key, so it
/// lets a reference in that order through from every start.
#[tokio::test]
async fn the_end_of_a_gated_run_takes_the_run_then_its_counter_row_then_the_key() {
    for start in 0..2 {
        let (w, session, holder, waiter) = holder_and_waiter().await;
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, holder);
        let reference = [
            (Slot::ExistingRuns, run_row(holder)),
            (Slot::NewRuns, counter_row(holder)),
            (Slot::Semaphores, key_rows(&w)),
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
        assert!(w.run_status(holder).await.is_terminal());
        assert_eq!(start_rows(&w, holder).await, (0, 0), "released");
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
}

/// FR-022-021: a cancellation of a queued start takes the run, then its active-run counter row (the run ends), then the
/// key it dequeues from, from every start (keys review, correctness m1).
#[tokio::test]
async fn a_cancellation_of_a_queued_start_takes_the_run_then_its_counter_row_then_the_key() {
    for start in 0..2 {
        let (w, _, holder, waiter) = holder_and_waiter().await;
        let reference = [
            (Slot::ExistingRuns, run_row(waiter)),
            (Slot::NewRuns, counter_row(waiter)),
            (Slot::Semaphores, key_rows(&w)),
        ];
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
            "start {start}: {:?}",
            seen.reference
        );
        seen.path
            .expect("the cancellation commits after the reference");
        assert_eq!(start_rows(&w, waiter).await, (0, 0), "dequeued");
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
}

/// T12: the timeout of a queued start takes the run, then its active-run counter row (the run ends), then the key, from
/// every start (keys review, correctness m1).
#[tokio::test]
async fn the_timeout_of_a_queued_start_takes_the_run_then_its_counter_row_then_the_key() {
    for start in 0..2 {
        let (w, _, holder, waiter) = holder_and_waiter().await;
        crate::run_admission::age_start_deadline(&w, waiter).await;
        let reference = [
            (Slot::ExistingRuns, run_row(waiter)),
            (Slot::NewRuns, counter_row(waiter)),
            (Slot::Semaphores, key_rows(&w)),
        ];
        let seen = against_at(&w.db.pool, &reference, start, w.scheduler.time_out_gates()).await;
        assert_eq!(
            seen.reference,
            Outcome::Committed,
            "start {start}: {:?}",
            seen.reference
        );
        assert_eq!(seen.path.gate_waits_timed_out, 1, "{:?}", seen.path);
        assert_eq!(start_rows(&w, waiter).await, (0, 0));
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
}

/// Every active-run counter row of the namespace of `run` (ADR-0050 slot 6), all 16 slots made to exist first: a new
/// run's slot is not known before its id is drawn, so the reference holds every slot the admission could take.
async fn every_counter_row(w: &World, run: RunId) -> Lock {
    sqlx::query(
        "INSERT INTO run_active_counts (tenant_id, namespace_id, slot, active) \
         SELECT r.tenant_id, r.namespace_id, s::smallint, 0 FROM runs r, generate_series(0, 15) s WHERE r.id = $1 \
         ON CONFLICT (tenant_id, namespace_id, slot) DO NOTHING",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("every slot");
    Lock::statement(
        "every active-run counter row of the namespace",
        format!(
            "SELECT 1 FROM run_active_counts c JOIN runs r ON r.tenant_id = c.tenant_id AND r.namespace_id = c.namespace_id \
              WHERE r.id = '{}' FOR UPDATE OF c",
            run.as_uuid()
        ),
    )
}

/// Keys review (2026-11-20), correctness M3; T022-029: a start's admission takes its active-run counter row (slot 6,
/// through the `runs` insert's trigger) before it files its key (slot 7), the order a run's end takes them in
/// (counter row, then key), so an admission and an end never deadlock. The start is queued behind the holder.
#[tokio::test]
async fn an_admission_takes_its_counter_row_before_its_key() {
    let (w, _, holder, waiter) = holder_and_waiter().await;
    // another workflow of the same group: the same key
    let workflow = publish(&w, serial_start("permit-locks-admission", 1)).await;
    let reference = [
        (Slot::NewRuns, every_counter_row(&w, holder).await),
        (Slot::Semaphores, key_rows(&w)),
    ];
    let account = json!({"accountId": "acct-1"});
    let seen = against_at(&w.db.pool, &reference, 0, w.start(workflow, &account)).await;
    assert_eq!(seen.reference, Outcome::Committed, "{:?}", seen.reference);
    let started = seen.path;
    assert_eq!(
        start_rows(&w, started).await,
        (0, 1),
        "queued behind the holder"
    );
    for run in [holder, waiter, started] {
        w.assert_consistent(run).await;
    }
}

/// The control: a plan that takes the key and then the run, against the reference in the order of the slots, is a
/// deadlock (one victim), so a path in the wrong order could not pass the tests above.
#[tokio::test]
async fn a_plan_in_the_reversed_order_deadlocks_with_the_reference() {
    let (w, _, holder, _) = holder_and_waiter().await;
    let reference = [
        (Slot::ExistingRuns, run_row(holder)),
        (Slot::Semaphores, key_rows(&w)),
    ];
    let reversed = [key_rows(&w), run_row(holder)];
    let seen = against_at(&w.db.pool, &reference, 0, run_plan(&w.db.pool, &reversed)).await;
    assert_eq!(
        deadlocks(&[seen.reference.clone(), seen.path.clone()]),
        1,
        "{seen:?}"
    );
}
