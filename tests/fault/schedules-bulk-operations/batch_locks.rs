//! The lock order of batch jobs (`T018-012`, `T018-049`, `FR-018-030`, `NFR-018-001`, `SC-018-016`):
//! the harness of `support/lock_order.rs` reproduces a deadlock when two transactions take the same
//! locks in opposite orders and none when they take them in the same order; a monitor that samples
//! the row locks of `runs` sees a transaction that holds two; and a batch job that works on the same
//! runs as the API, at the top rate, never has a transaction holding two run locks and never
//! deadlocks. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use sagat_application::batch::{BatchService, BatchWrite, DueJob, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest,
};
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::runs::RunOperation;
use sagat_domain::batch::{Action, JobState, Justification, Rate, Selector};
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::bounded::within;
use crate::support::engine::{World, ctx};
use crate::support::lock_order::{
    Lock, Outcome, RowLockMonitor, Slot, assert_ascending, database_deadlocks, deadlocks, describe,
    interleave,
};
use crate::support::schedules::{Fixture, TestClock, fixture};

// --- the harness proves itself --------------------------------------------------------------------

async fn scratch(f: &Fixture, table: &str) {
    for statement in [
        format!("CREATE TABLE {table} (id bigint PRIMARY KEY)"),
        format!("INSERT INTO {table} VALUES (1), (2)"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&f.w.db.pool)
            .await
            .expect("scratch table");
    }
}

#[tokio::test]
async fn two_plans_that_take_the_same_two_rows_in_opposite_orders_deadlock_and_in_one_order_do_not()
{
    let f = fixture().await;
    scratch(&f, "lock_order_rows").await;
    let a = Lock::row("row 1", "lock_order_rows", 1);
    let b = Lock::row("row 2", "lock_order_rows", 2);

    // the same order: one waits for the other and both commit
    let same = interleave(
        &f.w.db.pool,
        &[vec![a.clone(), b.clone()], vec![a.clone(), b.clone()]],
    )
    .await;
    assert_eq!(same, vec![Outcome::Committed, Outcome::Committed]);

    // opposite orders: each holds the lock the other asks for, so PostgreSQL breaks the deadlock by
    // aborting exactly one of them
    let before = database_deadlocks(&f.w.db.pool).await;
    let opposite = interleave(
        &f.w.db.pool,
        &[vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]],
    )
    .await;
    assert_eq!(deadlocks(&opposite), 1, "{opposite:?}");
    assert_eq!(
        opposite
            .iter()
            .filter(|outcome| **outcome == Outcome::Committed)
            .count(),
        1,
        "the other one goes on and commits: {opposite:?}"
    );

    // the counter of the database agrees; it is reported a moment later than the error
    within(
        "the deadlock reaches the statistics of the database",
        StdDuration::from_secs(30),
        async {
            while database_deadlocks(&f.w.db.pool).await < before + 1 {
                tokio::time::sleep(StdDuration::from_millis(250)).await;
            }
        },
    )
    .await;
}

#[tokio::test]
async fn the_same_holds_for_advisory_locks_and_for_three_plans() {
    let f = fixture().await;
    let one = Lock::advisory("one", 70_001);
    let two = Lock::advisory("two", 70_002);
    let three = Lock::advisory("three", 70_003);
    // a ring: each asks for the lock the next one holds
    let ring = interleave(
        &f.w.db.pool,
        &[
            vec![one.clone(), two.clone()],
            vec![two.clone(), three.clone()],
            vec![three.clone(), one.clone()],
        ],
    )
    .await;
    assert!(deadlocks(&ring) >= 1, "{ring:?}");
    let ordered = interleave(
        &f.w.db.pool,
        &[
            vec![one.clone(), two.clone()],
            vec![one.clone(), three.clone()],
            vec![one, two],
        ],
    )
    .await;
    assert_eq!(deadlocks(&ordered), 0, "{ordered:?}");
    assert!(
        ordered.iter().all(|outcome| *outcome == Outcome::Committed),
        "{ordered:?}"
    );
}

#[test]
fn a_plan_in_the_order_of_the_slots_is_accepted() {
    assert_ascending(&[
        (Slot::Receipt, "request receipt"),
        (Slot::Aggregate, "schedule row"),
        (Slot::Aggregate, "business-id row"),
        (Slot::ExistingRuns, "the run it acts on"),
        (Slot::Authority, "memberships"),
        (Slot::NewRuns, "run insert"),
        (Slot::Bookkeeping, "batch job, then item"),
        (Slot::Usage, "quota rows"),
    ]);
}

#[test]
#[should_panic(expected = "never returns to a lower slot")]
fn a_plan_that_goes_back_to_a_lower_slot_is_refused() {
    // a transition on a run that then locks the schedule it came from is the edge ADR-0050 forbids
    assert_ascending(&[
        (Slot::ExistingRuns, "the run"),
        (Slot::Aggregate, "the schedule row"),
    ]);
}

// --- the monitor looks at the row locks of runs ----------------------------------------------------

#[tokio::test]
async fn the_monitor_sees_a_transaction_that_holds_two_run_locks() {
    let f = fixture().await;
    let first = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let second = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let monitor = RowLockMonitor::start(&f.w.db.pool, "runs").await;

    let mut tx = f.w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM runs WHERE id = ANY($1) FOR UPDATE")
        .bind(vec![first, second])
        .execute(&mut *tx)
        .await
        .expect("two run locks in one transaction");
    let seen = monitor.samples();
    within(
        "the monitor looks while the locks are held",
        StdDuration::from_secs(30),
        async {
            while monitor.samples() < seen + 5 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        },
    )
    .await;
    tx.rollback().await.expect("release");
    let report = monitor.stop().await;

    assert!(
        report.violations.iter().any(|(_, held)| *held == 2),
        "the monitor is not blind: {}",
        describe(&report)
    );
}

// --- a batch job beside the API ---------------------------------------------------------------------

#[derive(Clone)]
struct Runs(Arc<World>);

impl ItemActions for Runs {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await
    }
}

type Executor = BatchExecutor<PgBatchStore, Runs, PgIdentityStore, TestClock>;

fn service(f: &Fixture) -> BatchService<PgBatchStore, TestClock> {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), f.clock.clone())
}

async fn run_job(
    f: &Fixture,
    exec: &Executor,
    action: Action,
    runs: &[Uuid],
    key: &str,
) -> JobRecord {
    f.clock
        .set(OffsetDateTime::now_utc() + Duration::seconds(2));
    let job = service(f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            BatchWrite {
                selector: Selector::run_ids(runs.to_vec()).expect("selector"),
                action,
                step_id: None,
                instance_path: Vec::new(),
                signal: None,
                update: None,
                drop_business_id: false,
                repin: false,
                reason: Justification::new("toggle the runs while the API does too")
                    .expect("reason"),
                max_items: None,
                rate: Some(Rate::MAX),
            },
            key,
            &RequestContext::default(),
        )
        .await
        .expect("create the job");
    let due = DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    };
    for _ in 0..2_000 {
        let now = service(f).get(&f.w.admin, job.id).await.expect("job");
        if matches!(
            now.state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return now;
        }
        f.clock.advance(Duration::seconds(1));
        exec.step(&due, None).await.expect("step");
        tokio::time::sleep(StdDuration::from_millis(1)).await;
    }
    panic!("the job never ended");
}

#[tokio::test]
async fn a_batch_job_and_the_api_on_the_same_runs_never_hold_two_run_locks_or_deadlock() {
    let f = fixture().await;
    let mut runs = Vec::new();
    for _ in 0..24 {
        runs.push(f.w.start(f.workflow, &json!({})).await);
    }
    let ids: Vec<Uuid> = runs.iter().map(|run| run.as_uuid()).collect();
    let exec: Executor = BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings::default(),
    );
    let deadlocks_before = database_deadlocks(&f.w.db.pool).await;
    let monitor = RowLockMonitor::start(&f.w.db.pool, "runs").await;

    // the API pauses, resumes and signals the same runs for as long as the jobs run
    let stop = Arc::new(AtomicBool::new(false));
    let unexpected = Arc::new(Mutex::new(Vec::<String>::new()));
    let calls = Arc::new(AtomicU64::new(0));
    let hammers: Vec<_> = (0..2)
        .map(|n| {
            let (w, stop, unexpected, calls) = (
                Arc::clone(&f.w),
                Arc::clone(&stop),
                Arc::clone(&unexpected),
                Arc::clone(&calls),
            );
            let runs = runs.clone();
            tokio::spawn(async move {
                let mut turn = n;
                while !stop.load(Ordering::Relaxed) {
                    for run in &runs {
                        turn += 1;
                        let op = if turn % 2 == 0 {
                            RunOperation::Pause
                        } else {
                            RunOperation::Resume
                        };
                        let result = match w.operate(*run, op).await {
                            // already paused, or not paused: the other side got there first
                            Ok(_) | Err(AppError::StateConflict(_)) => Ok(()),
                            Err(other) => Err(other),
                        };
                        let signalled = w
                            .runs
                            .signal(
                                &w.admin,
                                *run,
                                "nothing.waits.for.this",
                                b"{}",
                                &format!("hammer-{}", Uuid::now_v7()),
                                &ctx(),
                            )
                            .await;
                        let signalled = match signalled {
                            Ok(_) | Err(AppError::StateConflict(_)) => Ok(()),
                            Err(other) => Err(other),
                        };
                        calls.fetch_add(2, Ordering::Relaxed);
                        for failure in [result.err(), signalled.err()].into_iter().flatten() {
                            unexpected.lock().expect("lock").push(failure.to_string());
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();

    // the jobs: pause everything, resume everything, three times
    let mut ended = Vec::new();
    for round in 0..3 {
        for (n, action) in [Action::Pause, Action::Resume].into_iter().enumerate() {
            ended.push(run_job(&f, &exec, action, &ids, &format!("locks-{round}-{n}")).await);
        }
    }
    // then the API cancels the same runs from the other end while a cancel job works through them
    let canceller = tokio::spawn({
        let w = Arc::clone(&f.w);
        let runs = runs.clone();
        async move {
            for run in runs.iter().rev() {
                match w.operate(*run, RunOperation::Cancel).await {
                    Ok(_) | Err(AppError::StateConflict(_)) => {}
                    Err(other) => panic!("the API cancel met {other}"),
                }
            }
        }
    });
    let cancel = run_job(&f, &exec, Action::Cancel, &ids, "locks-cancel").await;
    canceller.await.expect("canceller");
    assert_eq!(cancel.state, JobState::Completed);
    assert_eq!(cancel.counts.failed, 0);
    assert_eq!(
        cancel.counts.succeeded + cancel.counts.skipped,
        24,
        "each run was cancelled by the job or found already cancelled by the API"
    );
    for run in &runs {
        assert_eq!(
            f.w.run_status(*run).await,
            sagat_domain::execution::RunStatus::Cancelled
        );
    }
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM batch_items WHERE job_id = $1 AND state = 'SKIPPED'",
    )
    .bind(cancel.id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("reasons");
    assert!(
        reasons
            .iter()
            .all(|reason| reason.as_deref() == Some("ALREADY_TERMINAL")),
        "{reasons:?}"
    );
    ended.push(cancel);

    stop.store(true, Ordering::Relaxed);
    for hammer in hammers {
        hammer.await.expect("hammer");
    }
    let report = monitor.stop().await;
    println!(
        "{} ({} calls of the API, {} jobs)",
        describe(&report),
        calls.load(Ordering::Relaxed),
        ended.len()
    );

    assert!(
        report.most_held >= 1,
        "the monitor saw run locks held while the work went on, or it proves nothing: {}",
        describe(&report)
    );
    assert!(
        report.samples >= 20,
        "the monitor looked while the work went on: {}",
        describe(&report)
    );
    assert!(
        calls.load(Ordering::Relaxed) >= 50,
        "the API really worked on the same runs ({} calls)",
        calls.load(Ordering::Relaxed)
    );
    assert!(
        report.violations.is_empty(),
        "no transaction holds two run locks: {}",
        describe(&report)
    );
    assert_eq!(
        *unexpected.lock().expect("lock"),
        Vec::<String>::new(),
        "the API and the job only ever meet as a state that is not the one asked for"
    );
    for job in &ended {
        assert_eq!(job.state, JobState::Completed);
        let c = &job.counts;
        assert_eq!(c.selected, 24);
        assert_eq!(
            c.succeeded + c.skipped + c.failed + c.cancelled,
            c.selected,
            "the counters equal the outcomes of the items"
        );
        assert_eq!(
            c.failed, 0,
            "nothing failed, an item only succeeds or is skipped"
        );
    }
    // the statistics of the database report a deadlock a moment after it; give them that moment
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    assert_eq!(
        database_deadlocks(&f.w.db.pool).await,
        deadlocks_before,
        "no deadlock"
    );
}
