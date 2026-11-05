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

use sagat_application::runs::{BusinessStart, RunOperation, StartOptions, StartRun};
use sagat_application::schedule_firing::Fired;
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::execution::{RunId, StepStatus};
use sagat_scheduler::SchedulerSettings;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::Duration;

use crate::support::bounded::within;
use crate::support::engine::{World, activity, chain, ctx, document, wait_node, with};
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
           AND (tablename = 'schedules' OR tablename = 'business_ids' OR tablename LIKE 'schedule\\_%' OR tablename LIKE 'batch\\_%') \
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
        "business_ids",
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

/// Every one of the tables is locked: a statement of its own that would wait is refused after 300 ms.
async fn assert_locked(f: &Fixture, tables: &[String]) {
    for table in tables {
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
}

/// A run that holds a business identifier: what it goes through must not touch the binding either.
async fn bound(w: &World, workflow: sagat_domain::definition::WorkflowId, id: &str) -> RunId {
    bound_input(w, workflow, id, b"{}").await
}

async fn bound_input(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    id: &str,
    input: &[u8],
) -> RunId {
    w.runs
        .start_with_options(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input,
                correlation_id: None,
                deadline: None,
                key: &format!("{id}-start-key"),
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(id).expect("an identifier"),
                    conflict: ConflictPolicy::Fail,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
            },
            &ctx(),
        )
        .await
        .expect("start a run under an identifier")
        .run
        .info
        .run_id
}

#[tokio::test]
async fn no_run_driven_transition_touches_a_table_of_a_schedule_or_a_batch_job() {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    let (workflow, _) = w.publish(chain("run-driven-locks")).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let completes = bound(w, workflow, "locks-completes").await;
    let complete_a = w.one_task(&session).await;
    let fails = bound(w, workflow, "locks-fails").await;
    let fail_a = w.one_task(&session).await;
    let lapses = bound(w, workflow, "locks-lapses").await;
    w.one_task(&session).await;
    w.age_lease(lapses, "a").await;
    let late = bound(w, workflow, "locks-late").await;
    w.age_run_deadline(late).await;
    let cancelled = bound(w, workflow, "locks-cancelled").await;
    let sleeper = sleepers(w, "run-driven-timer", 1).await[0];
    // an aggregate that has work waiting for it, for the control below
    let schedule = f.create("held", &hourly(), &json!({}), None).await;
    f.clock.advance(Duration::hours(2));

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;

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

/// A scheduler that owns the shards, as the hand-off of a chain needs; reused across ticks, since a
/// new placement per tick would starve the later ones.
async fn shard_owner(f: &Fixture) -> crate::support::engine::Sched {
    let placement = sagat_scheduler::Placement::new(format!("run-driven-{}", uuid::Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire the shards");
    scheduler(&f.w, SchedulerSettings::default(), Some(placement))
}

/// A workflow that waits for a signal and, if it does not come, takes the other way.
fn waiter(name: &str) -> serde_json::Value {
    document(
        name,
        "w",
        vec![
            with(
                wait_node("w", "payment.confirmed", "input.k", 300_000, Some("review")),
                "next",
                json!("notify"),
            ),
            activity("notify"),
            activity("review"),
        ],
    )
}

/// A workflow of two segments: one activity, then a hand-off to the next segment.
fn rollover(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 2, "timeoutMs": 3_600_000}
    })
}

#[tokio::test]
async fn a_signal_a_wait_that_times_out_and_a_handoff_take_no_lock_of_an_aggregate() {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    let (waiting, _) = w.publish(waiter("run-driven-signals")).await;
    let (rolling, _) = w.publish(rollover("run-driven-handoff")).await;
    let signalled = bound_input(w, waiting, "locks-signalled", br#"{"k": "s"}"#).await;
    let timed_out = bound_input(w, waiting, "locks-timed-out", br#"{"k": "t"}"#).await;
    w.age_due(timed_out, "w").await;
    let chained = bound_input(w, rolling, "locks-chained", br#"{"next": {"segment": 2}}"#).await;
    let session = w
        .session(&["act.notify", "act.review", "segment.work"])
        .await;
    let segment = w.one_task_of(&session, "segment.work").await;
    let owner = shard_owner(&f).await;

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;

    // a signal wakes the run that waits for it
    let woken = within("a signal", STAGE, async {
        w.runs
            .signal(
                &w.admin,
                signalled,
                "payment.confirmed",
                br#"{"ok": true}"#,
                "signal-under-locks",
                &ctx(),
            )
            .await
            .expect("the signal")
    })
    .await;
    assert_eq!(woken.iter().map(|n| n.as_str()).collect::<Vec<_>>(), ["w"]);

    // the wait of the other one runs out, and it takes the way for that
    let passes = scheduler(w, SchedulerSettings::default(), None);
    within("the wait that times out", STAGE, async {
        for _ in 0..8 {
            if !passes.tick().await.did_work() {
                break;
            }
        }
    })
    .await;
    assert_eq!(
        w.step_status(timed_out, "review").await,
        Some(StepStatus::Ready),
        "the wait ran out and the way for that is ready"
    );

    // a segment ends and the next one starts, under the same identifier
    within("the hand-off", STAGE, async {
        w.complete(&session, &segment, &json!({}))
            .await
            .expect("the segment works");
        for _ in 0..12 {
            let report = owner.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            let count: i32 = sqlx::query_scalar(
                "SELECT segment_count FROM execution_chains WHERE chain_id = $1",
            )
            .bind(chained.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the chain");
            if count >= 2 {
                break;
            }
        }
    })
    .await;
    let segments: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE business_id = $1")
        .bind("locks-chained")
        .fetch_one(&w.db.pool)
        .await
        .expect("segments");
    assert_eq!(segments, 2, "the successor carries the identifier");

    holder.rollback().await.expect("release the locks");
}
