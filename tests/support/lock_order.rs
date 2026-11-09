//! A harness for the lock order of ADR-0050 decision 7 (`T018-012`, `NFR-018-001`).
//!
//! Three tools:
//!
//! * [`interleave`] runs several lock plans at once, each in a transaction of its own on a
//!   connection of its own, and makes every plan hold its first lock before any of them asks for the
//!   second (a barrier after each step). Two plans that take the same two locks in opposite orders
//!   therefore deadlock for certain, not by luck of timing, and two plans that take them in the same
//!   order never do. The deadlocks are counted by the error PostgreSQL gives the victim (`40P01`).
//! * [`Slot`] and [`assert_ascending`] state the slots of the decision and refuse a plan that returns
//!   to a lower one.
//! * [`RowLockMonitor`] samples the row locks of a table with `pgrowlocks` (`pg_locks` does not list
//!   them) and reports any transaction that holds more than one at the same time.
//!
//! The harness proves nothing by itself about the platform: a test builds the plans from the paths it
//! means to check, or runs the real paths with the monitor beside them.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinHandle;

/// The slots of ADR-0050 decision 7, in the order a transaction may take them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// The requirement epoch of the instance registry (`FR-018-052`): `FOR SHARE` to register, `FOR UPDATE` to record a
    /// requirement. It is the first slot because the activation holds it while the feature's step runs. The step of the
    /// handover is the one that **does lock** (it narrows ADR-0081 decision 2, "the step reads the epoch, it does not lock",
    /// ADR-0082): the two slots below, which no transaction that reads the epoch ever takes, so none of them can wait for the
    /// activation while holding what the activation wants.
    RegistryEpoch,
    /// The table of the cron triggers (`workflow_triggers`), `SHARE ROW EXCLUSIVE`, for at most two seconds (D2070): taken by
    /// the census of the handover and by `schedule_trigger_handover_complete`, in that order of the two and before the handover
    /// row, through `schedule_trigger_handover_lock()`. The test of the order is derived from `pg_locks`
    /// (`trigger_round3::the_function_takes_the_table_of_the_triggers_before_the_handover_row`), not from this list.
    TriggersTable,
    /// The singleton `schedule_trigger_handover` (feature 018): taken by the handover inside the registry epoch's slot and after the
    /// table, `FOR UPDATE`, by the function that moves it (D2046, D2070); read by everything else without a lock.
    HandoverRow,
    /// 026 public-link row or 027 app-session row.
    Entry,
    /// The request receipt.
    Receipt,
    /// The row of a cron trigger (`workflow_triggers`): `FOR SHARE` by the reconciliation, written by a publication, before the row of
    /// the schedule it owns (D2050): the two writers of that pair take them in this order, so neither can abort the other.
    TriggerRow,
    /// Run-creating aggregates: a schedule or pending-reset row, then a business-id row.
    Aggregate,
    /// Existing runs the transaction acts on: chain before run.
    ExistingRuns,
    /// Namespace fences and authority, then catalog heads and pins.
    Authority,
    /// Secret rows.
    Secrets,
    /// New run rows.
    NewRuns,
    /// 022 semaphores.
    Semaphores,
    /// Bookkeeping rows of a transaction whose subject is one existing run (a batch job, then its
    /// item).
    Bookkeeping,
    /// Quota, rate and usage rows, always last.
    Usage,
}

/// # Panics
/// When a plan returns to a lower slot than one it has already taken, naming the two steps.
pub fn assert_ascending(plan: &[(Slot, &str)]) {
    for pair in plan.windows(2) {
        assert!(
            pair[0].0 <= pair[1].0,
            "`{}` ({:?}) is taken after `{}` ({:?}): a transaction never returns to a lower slot",
            pair[1].1,
            pair[1].0,
            pair[0].1,
            pair[0].0
        );
    }
}

/// One lock a plan takes: its name for messages, and the statement that takes it.
#[derive(Debug, Clone)]
pub struct Lock {
    pub name: &'static str,
    pub sql: String,
}

impl Lock {
    /// An advisory lock held to the end of the transaction.
    #[must_use]
    pub fn advisory(name: &'static str, key: i64) -> Self {
        Self {
            name,
            sql: format!("SELECT pg_advisory_xact_lock({key})"),
        }
    }

    /// The row of `table` whose `id` column is `id`, locked `FOR UPDATE`. The table and the column
    /// are those of the test that builds the plan.
    #[must_use]
    pub fn row(name: &'static str, table: &str, id: i64) -> Self {
        Self {
            name,
            sql: format!("SELECT 1 FROM {table} WHERE id = {id} FOR UPDATE"),
        }
    }
}

/// How one plan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Committed,
    /// PostgreSQL chose this transaction to break a deadlock (`40P01`).
    DeadlockVictim,
    Failed(String),
}

/// Runs the plans at once and returns how each ended. Every plan takes its locks in the order given;
/// after each lock but the last it waits until every other plan has either taken its lock of that
/// step or is waiting on a lock somebody holds (seen in `pg_stat_activity` from a connection of its
/// own, so a plan queued behind another one does not keep the holder at the barrier for ever).
///
/// # Panics
/// When the plans do not all have the same number of locks.
pub async fn interleave(pool: &PgPool, plans: &[Vec<Lock>]) -> Vec<Outcome> {
    let steps = plans.first().map_or(0, Vec::len);
    assert!(
        plans.iter().all(|plan| plan.len() == steps),
        "every plan takes the same number of locks"
    );
    let count = plans.len();
    let arrived: Arc<Vec<AtomicUsize>> =
        Arc::new((0..steps).map(|_| AtomicUsize::new(0)).collect());
    let tasks: Vec<JoinHandle<Outcome>> = plans
        .iter()
        .cloned()
        .map(|plan| {
            let pool = pool.clone();
            let arrived = Arc::clone(&arrived);
            tokio::spawn(async move {
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(error) => {
                        // it will take no lock at all: the others must not wait for it
                        for step in arrived.iter() {
                            step.fetch_add(1, Ordering::SeqCst);
                        }
                        return Outcome::Failed(error.to_string());
                    }
                };
                let last = plan.len().saturating_sub(1);
                for (n, lock) in plan.iter().enumerate() {
                    if let Err(error) = sqlx::query(sqlx::AssertSqlSafe(lock.sql.clone()))
                        .execute(&mut *tx)
                        .await
                    {
                        // this plan will take no more locks: the others must not wait for it
                        for step in &arrived[n..last.max(n)] {
                            step.fetch_add(1, Ordering::SeqCst);
                        }
                        let deadlock = error
                            .as_database_error()
                            .and_then(|e| e.code())
                            .is_some_and(|code| code == "40P01");
                        return if deadlock {
                            Outcome::DeadlockVictim
                        } else {
                            Outcome::Failed(format!("{}: {error}", lock.name))
                        };
                    }
                    if n < last {
                        arrived[n].fetch_add(1, Ordering::SeqCst);
                        loop {
                            let blocked = waiting_on_locks(&pool).await;
                            if arrived[n].load(Ordering::SeqCst) + blocked >= count {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                    }
                }
                match tx.commit().await {
                    Ok(()) => Outcome::Committed,
                    Err(error) => Outcome::Failed(error.to_string()),
                }
            })
        })
        .collect();
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("a plan"));
    }
    outcomes
}

/// How many backends of this database wait on a lock another one holds.
async fn waiting_on_locks(pool: &PgPool) -> usize {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event_type = 'Lock'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    usize::try_from(waiting).unwrap_or(0)
}

/// How many of `outcomes` were chosen to break a deadlock.
#[must_use]
pub fn deadlocks(outcomes: &[Outcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| **outcome == Outcome::DeadlockVictim)
        .count()
}

/// The deadlocks PostgreSQL has counted for this database. The counter is reported lazily, so a test
/// reads it before and after, a while apart.
pub async fn database_deadlocks(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT deadlocks FROM pg_stat_database WHERE datname = current_database()")
        .fetch_one(pool)
        .await
        .expect("deadlocks")
}

/// What a [`RowLockMonitor`] saw.
#[derive(Debug, Default, Clone)]
pub struct LockReport {
    /// How many times it looked.
    pub samples: u64,
    /// The most rows of the table one transaction held at the same time in any sample.
    pub most_held: usize,
    /// Every sample in which a transaction held more than one: its id and what it held.
    pub violations: Vec<(String, usize)>,
}

/// Samples the row locks of one table, as often as it can, until stopped.
pub struct RowLockMonitor {
    stop: Arc<AtomicBool>,
    samples: Arc<AtomicU64>,
    task: JoinHandle<LockReport>,
}

impl RowLockMonitor {
    /// Starts looking at `table`. The `pgrowlocks` extension is installed first.
    pub async fn start(pool: &PgPool, table: &'static str) -> Self {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pgrowlocks")
            .execute(pool)
            .await
            .expect("the pgrowlocks extension");
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn({
            let pool = pool.clone();
            let stop = Arc::clone(&stop);
            let samples = Arc::clone(&samples);
            async move {
                let mut report = LockReport::default();
                let sql = format!(
                    "SELECT x::text, count(*)::bigint FROM pgrowlocks('{table}') r, \
                     LATERAL unnest(r.xids) AS x GROUP BY x"
                );
                while !stop.load(Ordering::Relaxed) {
                    let held: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(sql.clone()))
                        .fetch_all(&pool)
                        .await
                        .expect("sample the row locks");
                    report.samples += 1;
                    samples.store(report.samples, Ordering::Relaxed);
                    for (xid, count) in held {
                        let count = usize::try_from(count).unwrap_or(usize::MAX);
                        report.most_held = report.most_held.max(count);
                        if count > 1 {
                            report.violations.push((xid, count));
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                report
            }
        });
        Self {
            stop,
            samples,
            task,
        }
    }

    /// How many samples it has taken so far.
    #[must_use]
    pub fn samples(&self) -> u64 {
        self.samples.load(Ordering::Relaxed)
    }

    /// Stops looking and returns what it saw.
    pub async fn stop(self) -> LockReport {
        self.stop.store(true, Ordering::Relaxed);
        self.task.await.expect("the monitor")
    }
}

/// The transactions that held locks, by how many: for a message when a test fails.
#[must_use]
pub fn describe(report: &LockReport) -> String {
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for (xid, count) in &report.violations {
        let most = by.entry(xid.as_str()).or_default();
        *most = (*most).max(*count);
    }
    format!(
        "{} samples, most held by one transaction {}, violations {by:?}",
        report.samples, report.most_held
    )
}
