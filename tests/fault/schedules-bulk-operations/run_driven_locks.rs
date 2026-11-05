//! Run-driven transitions never touch the tables of schedules and batch jobs (`T018-076`,
//! `SC-018-016`, `FR-018-007`, `FR-018-021`, `FR-018-030`).
//!
//! The aggregates (a schedule, a batch job) learn that a run ended by looking at it, in a
//! transaction of their own; nothing a run does takes one of their rows (ADR-0050 decision 7), so a
//! run-driven transaction cannot be part of a lock cycle with them. The test holds an exclusive lock
//! on every one of those tables, which stops any statement that reads or writes them, and drives
//! every transition a run goes through without a person asking for it (a worker completes or fails
//! an attempt, a timer fires, a lease runs out, a deadline passes) and one a person does ask for (a
//! cancellation): each has to finish while the locks are held. The same locks do stop what belongs to
//! an aggregate, which is the proof that they would have stopped a run-driven transaction that
//! touched one. Real PostgreSQL, the real services and passes.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use sagat_application::runs::RunOperation;
use sagat_application::schedule_firing::Fired;
use sagat_domain::execution::StepStatus;
use sagat_scheduler::SchedulerSettings;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::Duration;

use crate::support::bounded::within;
use crate::support::engine::chain;
use crate::support::scale::{scheduler, sleepers};
use crate::support::schedules::{Fixture, fixture, hourly};

/// The stage limit: a transition that has to wait for a lock never finishes, so this is long enough
/// for a loaded machine and short enough to name the stage that waited.
const STAGE: StdDuration = StdDuration::from_secs(60);

/// Every table of the two aggregates, by name, so that one added later is locked too.
async fn aggregate_tables(f: &Fixture) -> Vec<String> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables \
         WHERE schemaname = current_schema() \
           AND (tablename = 'schedules' OR tablename LIKE 'schedule\\_%' OR tablename LIKE 'batch\\_%') \
         ORDER BY 1",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the tables");
    for known in [
        "schedules",
        "schedule_revisions",
        "schedule_actions",
        "schedule_backfills",
        "schedule_obligations",
        "batch_jobs",
        "batch_items",
    ] {
        assert!(
            names.iter().any(|name| name == known),
            "{known} is among the tables of the aggregates: {names:?}"
        );
    }
    names
}

/// A transaction that holds every aggregate table exclusively until it ends.
async fn lock(f: &Fixture, tables: &[String]) -> Transaction<'static, Postgres> {
    let mut tx = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
        tables.join(", ")
    )))
    .execute(&mut *tx)
    .await
    .expect("lock the aggregates");
    tx
}

#[tokio::test]
async fn no_run_driven_transition_touches_a_table_of_a_schedule_or_a_batch_job() {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    let (workflow, _) = w.publish(chain("run-driven-locks")).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let completes = w.start(workflow, &json!({})).await;
    let complete_a = w.one_task(&session).await;
    let fails = w.start(workflow, &json!({})).await;
    let fail_a = w.one_task(&session).await;
    let lapses = w.start(workflow, &json!({})).await;
    w.one_task(&session).await;
    w.age_lease(lapses, "a").await;
    let late = w.start(workflow, &json!({})).await;
    w.age_run_deadline(late).await;
    let cancelled = w.start(workflow, &json!({})).await;
    let sleeper = sleepers(w, "run-driven-timer", 1).await[0];
    // an aggregate that has work waiting for it, for the control below
    let schedule = f.create("held", &hourly(), &json!({}), None).await;
    f.clock.advance(Duration::hours(2));

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    for table in &tables {
        // a statement of its own that would wait: refused after 300 ms with `lock_not_available`
        let mut probe = f.w.db.pool.begin().await.expect("a transaction");
        sqlx::query("SET LOCAL lock_timeout = '300ms'")
            .execute(&mut *probe)
            .await
            .expect("a short wait");
        let refused = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table}"
        )))
        .fetch_one(&mut *probe)
        .await;
        let code = match &refused {
            Err(sqlx::Error::Database(error)) => error.code().map(|c| c.into_owned()),
            _ => None,
        };
        assert_eq!(
            code.as_deref(),
            Some("55P03"),
            "{table} is locked, so a statement on it waits: {refused:?}"
        );
        drop(probe);
    }

    // a worker completes an attempt, and then the step it unlocks
    within("a worker completes", STAGE, async {
        w.complete(&session, &complete_a, &json!({}))
            .await
            .expect("a completes");
        // other runs have their first step ready too: take the one this run's second step is
        let b = w.one_task_of(&session, "act.b").await;
        w.complete(&session, &b, &json!({}))
            .await
            .expect("b completes");
    })
    .await;
    assert_eq!(
        w.run_status(completes).await,
        sagat_domain::execution::RunStatus::Succeeded
    );

    // a worker fails an attempt for good
    within("a worker fails", STAGE, async {
        w.fail(&session, &fail_a, false).await.expect("a fails");
    })
    .await;
    assert!(
        w.run_status(fails).await.is_terminal(),
        "the failure ended the run"
    );

    // a person cancels
    within("a cancellation", STAGE, async {
        w.operate_run(cancelled, RunOperation::Cancel).await;
    })
    .await;

    // the passes: a timer fires, a lease that ran out is taken back, a deadline passes
    let passes = scheduler(w, SchedulerSettings::default(), None);
    let (mut expired, mut enforced) = (0, 0);
    within("the scheduler's passes", STAGE, async {
        loop {
            let round = passes.tick().await;
            expired += round.leases_expired;
            enforced += round.deadlines_enforced;
            if !round.did_work() {
                break;
            }
        }
    })
    .await;
    assert!(expired >= 1, "the lease that ran out was taken back");
    assert_eq!(
        w.step_status(sleeper, "after").await,
        Some(StepStatus::Ready),
        "the timer fired and the step behind it is ready"
    );
    let ended = w.run_status(late).await;
    assert!(
        enforced >= 1
            && (ended.is_terminal() || ended == sagat_domain::execution::RunStatus::Cancelling),
        "the deadline stopped the run: enforced {enforced} times, the run is {ended:?}"
    );

    // --- the control: what belongs to an aggregate does wait for the same locks -----------------
    let firing = {
        let f = std::sync::Arc::new(f);
        let task = tokio::spawn({
            let f = std::sync::Arc::clone(&f);
            let schedule = schedule.clone();
            async move { f.fire(&schedule).await }
        });
        f.wait_for_a_backend_on_a_lock("the firing of a due schedule waits for the aggregates")
            .await;
        assert!(
            !task.is_finished(),
            "the firing waits while the locks are held"
        );
        holder.rollback().await.expect("release the locks");
        within("the firing after the locks are released", STAGE, task)
            .await
            .expect("the task")
    };
    assert_eq!(firing, Fired::Acted, "and acts once they are released");
}
