//! A start with a delay (`T018-037`, `FR-018-023`, `SC-018-007`): the run is created `WAITING` with
//! its entry held, counts toward the active-run quota and holds its business identifier; a fenced
//! pass of the run shards releases the entry once, at the instant; a cancellation or a deadline ends
//! it without any step having run; a signal is refused while it waits. Real PostgreSQL, the real
//! run service and scheduler.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::runs::{StartOptions, StartRun, Started};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use serde_json::json;
use time::{Duration, OffsetDateTime};

use crate::business_ids::{business, cancel, key, scheduler_of, start};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{activity, ctx, document, wait_node, with};

fn delayed(by: StdDuration) -> StartOptions {
    StartOptions {
        start_delay: Some(by),
        ..StartOptions::default()
    }
}

/// The clock of the service says it is `ago` ago, so an instant it computes is already past by the
/// database's clock, which is what the pass reads.
fn the_service_clock_is(f: &Fixture, ago: Duration) {
    f.clock.set(OffsetDateTime::now_utc() - ago);
}

async fn journal(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
        .bind(run.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the journal")
}

async fn held_until(f: &Fixture, run: RunId) -> Option<OffsetDateTime> {
    sqlx::query_scalar("SELECT start_delayed_until FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the column")
}

async fn verified(f: &Fixture, run: RunId) {
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

/// Ticks the scheduler until it has nothing more to do (bounded).
async fn settle(passes: &crate::support::engine::Sched) -> u32 {
    let mut released = 0;
    for _ in 0..12 {
        let report = passes.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        released += report.starts_released;
        if !report.did_work() {
            break;
        }
    }
    released
}

#[tokio::test]
async fn a_delayed_start_makes_a_waiting_run_that_holds_its_entry() {
    let f = fixture().await;
    the_service_clock_is(&f, Duration::ZERO);

    let started = start(&f, delayed(StdDuration::from_secs(2 * 86_400)), &key())
        .await
        .expect("the start");
    let run = started.run.info.run_id;

    assert_eq!(started.run.status, RunStatus::Waiting);
    let until = started.run.start_delayed_until.expect("it says until when");
    let expected = f.clock.now() + Duration::days(2);
    assert!(
        (until - expected).abs() < Duration::seconds(5),
        "{until} is two days from the start ({expected})"
    );
    assert_eq!(held_until(&f, run).await, Some(until));
    assert_eq!(journal(&f, run).await, ["run.started", "run.start_delayed"]);
    assert_eq!(
        f.w.step_status(run, "a").await,
        Some(StepStatus::Pending),
        "the entry is held"
    );
    let session = f.w.session(&["act.a", "act.b"]).await;
    assert!(
        f.w.poll(&session, 8).await.is_empty(),
        "no worker is offered the run"
    );
    verified(&f, run).await;
}

#[tokio::test]
async fn the_scheduler_releases_a_delayed_run_once_at_its_instant_and_not_before() {
    let f = fixture().await;
    the_service_clock_is(&f, Duration::seconds(5));
    let due = start(&f, delayed(StdDuration::from_millis(1)), &key())
        .await
        .expect("a delay that has passed")
        .run
        .info
        .run_id;
    the_service_clock_is(&f, Duration::ZERO);
    let later = start(&f, delayed(StdDuration::from_secs(3_600)), &key())
        .await
        .expect("a delay of an hour")
        .run
        .info
        .run_id;
    let passes = scheduler_of(&f).await;

    let released = settle(&passes).await;

    assert_eq!(released, 1, "the run that was due, and only that one");
    assert_eq!(
        journal(&f, due).await,
        [
            "run.started",
            "run.start_delayed",
            "run.start_released",
            "step.released"
        ]
    );
    assert_eq!(f.w.run_status(due).await, RunStatus::Running);
    assert_eq!(held_until(&f, due).await, None);
    assert_eq!(f.w.run_status(later).await, RunStatus::Waiting);
    assert!(held_until(&f, later).await.is_some());

    // a worker is offered the one that was released, and the pass does not release it again
    let session = f.w.session(&["act.a", "act.b"]).await;
    let task = f.w.one_task_of(&session, "act.a").await;
    assert_eq!(task.run_id, due);
    let before = journal(&f, due).await;
    assert_eq!(settle(&passes).await, 0);
    assert_eq!(journal(&f, due).await, before, "released once");
    verified(&f, due).await;
    verified(&f, later).await;
}

#[tokio::test]
async fn a_run_cancelled_while_it_waits_ends_cancelled_without_a_step_having_run_and_is_never_released()
 {
    let f = fixture().await;
    the_service_clock_is(&f, Duration::seconds(5));
    let run = start(&f, delayed(StdDuration::from_millis(1)), &key())
        .await
        .expect("the start")
        .run
        .info
        .run_id;

    cancel(&f, run).await;
    let ended = journal(&f, run).await;

    assert_eq!(f.w.run_status(run).await, RunStatus::Cancelled);
    assert!(
        ended.iter().all(|event| event != "step.leased"),
        "no step ran: {ended:?}"
    );
    let passes = scheduler_of(&f).await;
    assert_eq!(settle(&passes).await, 0, "a run that ended is not released");
    assert_eq!(journal(&f, run).await, ended);
    verified(&f, run).await;
}

#[tokio::test]
async fn a_run_past_its_deadline_while_it_waits_times_out() {
    let f = fixture().await;
    the_service_clock_is(&f, Duration::ZERO);
    let run =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: f.workflow,
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: Some(f.clock.now() + Duration::hours(1)),
                    key: &key(),
                },
                delayed(StdDuration::from_secs(2 * 86_400)),
                &ctx(),
            )
            .await
            .expect("the start")
            .run
            .info
            .run_id;
    f.w.age_run_deadline(run).await;
    let passes = scheduler_of(&f).await;

    settle(&passes).await;

    assert_eq!(f.w.run_status(run).await, RunStatus::TimedOut);
    assert!(
        journal(&f, run)
            .await
            .iter()
            .all(|event| event != "step.leased"),
        "no step ran"
    );
    verified(&f, run).await;
}

#[tokio::test]
async fn a_signal_is_refused_while_the_entry_is_held_and_delivered_once_it_is_released() {
    let f = fixture().await;
    let (waiting, _) =
        f.w.publish(document(
            "delayed-waiter",
            "w",
            vec![
                with(
                    wait_node("w", "payment.confirmed", "input.k", 300_000, None),
                    "next",
                    json!("notify"),
                ),
                activity("notify"),
            ],
        ))
        .await;
    the_service_clock_is(&f, Duration::seconds(5));
    let run =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: waiting,
                    version: None,
                    input: br#"{"k": "s"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                delayed(StdDuration::from_millis(1)),
                &ctx(),
            )
            .await
            .expect("the start")
            .run
            .info
            .run_id;
    let refused = signal(&f, run).await;

    assert!(refused.is_err(), "nothing waits for it yet: {refused:?}");
    assert_eq!(journal(&f, run).await, ["run.started", "run.start_delayed"]);

    settle(&scheduler_of(&f).await).await;
    let woken = signal(&f, run).await.expect("it waits now");
    assert_eq!(woken.iter().map(|n| n.as_str()).collect::<Vec<_>>(), ["w"]);
    verified(&f, run).await;
}

#[tokio::test]
async fn a_delayed_run_counts_toward_the_active_run_quota_and_holds_its_identifier() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 1, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("room for one");
    let held = StartOptions {
        start_delay: Some(StdDuration::from_secs(3_600)),
        business: business(
            "delayed-1",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        )
        .business,
    };

    let run = start(&f, held.clone(), &key())
        .await
        .expect("the delayed run")
        .run
        .info
        .run_id;

    let second = start(&f, StartOptions::default(), &key()).await;
    assert!(
        matches!(second, Err(AppError::QuotaExceeded(_))),
        "the delayed run is an active run: {second:?}"
    );
    let same_id = start(&f, held, &key()).await;
    assert!(
        matches!(same_id, Err(AppError::BusinessIdConflict { holder, .. }) if holder == run),
        "and it holds its identifier: {same_id:?}"
    );
}

#[tokio::test]
async fn the_delay_is_checked_in_words() {
    let f = fixture().await;
    for (delay, valid) in [
        (StdDuration::ZERO, false),
        (StdDuration::from_secs(366 * 86_400 + 1), false),
        (StdDuration::from_millis(1), true),
        (StdDuration::from_secs(366 * 86_400), true),
    ] {
        let result = start(&f, delayed(delay), &key()).await;
        if valid {
            result.expect("a delay in range");
        } else {
            assert_eq!(
                result.err(),
                Some(AppError::InvalidStart {
                    reason: "START_DELAY_INVALID",
                    field: Some("/startDelayMs".to_owned())
                }),
                "{delay:?}"
            );
        }
    }
}

#[tokio::test]
async fn the_same_key_with_another_delay_is_another_request_and_the_same_delay_is_a_replay() {
    let f = fixture().await;
    let k = key();
    let first: Started = start(&f, delayed(StdDuration::from_secs(3_600)), &k)
        .await
        .expect("the start");

    let replay = start(&f, delayed(StdDuration::from_secs(3_600)), &k)
        .await
        .expect("a retry of the same request");
    assert_eq!(replay.run.info.run_id, first.run.info.run_id);

    let other = start(&f, delayed(StdDuration::from_secs(7_200)), &k).await;
    assert_eq!(other.err(), Some(AppError::IdempotencyKeyReused));
    let none = start(&f, StartOptions::default(), &k).await;
    assert_eq!(none.err(), Some(AppError::IdempotencyKeyReused));
}

async fn signal(f: &Fixture, run: RunId) -> Result<Vec<sagat_domain::workflow::NodeId>, AppError> {
    f.w.runs
        .signal(&f.w.admin, run, "payment.confirmed", b"{}", &key(), &ctx())
        .await
}

#[tokio::test]
async fn a_run_that_stops_qualifying_between_the_candidate_query_and_the_lock_is_left_alone() {
    let f = std::sync::Arc::new(fixture().await);
    the_service_clock_is(&f, Duration::seconds(5));
    let run = start(&f, delayed(StdDuration::from_millis(1)), &key())
        .await
        .expect("the start")
        .run
        .info
        .run_id;
    let passes = std::sync::Arc::new(scheduler_of(&f).await);
    let before = journal(&f, run).await;

    // somebody holds the row of the run; the pass finds it due, and waits for the lock
    let mut holder = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run.as_uuid())
        .fetch_all(&mut *holder)
        .await
        .expect("the row of the run");
    let ticking = tokio::spawn({
        let passes = std::sync::Arc::clone(&passes);
        async move { passes.tick().await }
    });
    f.wait_for_a_backend_on_a_lock("the release waits for the run")
        .await;
    assert!(!ticking.is_finished(), "it waits while the row is held");

    // and the run is paused meanwhile: it no longer qualifies by the time the lock is released
    sqlx::query("UPDATE runs SET paused = true WHERE id = $1")
        .bind(run.as_uuid())
        .execute(&mut *holder)
        .await
        .expect("paused");
    holder.commit().await.expect("release the row");
    let report = crate::support::bounded::within("the pass", StdDuration::from_secs(60), ticking)
        .await
        .expect("the task");

    assert_eq!(
        report.starts_released, 0,
        "a run that stopped qualifying is not released: {report:?}"
    );
    assert_eq!(
        journal(&f, run).await,
        before,
        "and nothing was written for it"
    );
}

const BEFORE_DELAY: i64 = 20_270_103_000_000;

#[tokio::test]
async fn a_schema_that_predates_the_delay_has_nothing_to_release_and_the_pass_does_not_fail() {
    let f = fixture().await;
    let passes = scheduler_of(&f).await;
    crate::migration::undo_to(&f.w.db, BEFORE_DELAY)
        .await
        .expect("no delayed run exists, so the rollback is allowed");

    // the scheduler keeps going on the schema it finds
    for _ in 0..3 {
        let report = passes.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.starts_released, 0);
    }

    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("forward again");
    assert_eq!(settle(&passes).await, 0);
}
