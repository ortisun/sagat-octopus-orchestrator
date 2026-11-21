//! Faults of the rate gate (task T022-080; SC-022-017, SC-022-018, SC-022-022, SC-022-023; NFR-022-001, NFR-022-008;
//! ADR-0088 evidence). A process that dies inside the admission (T19), the grant from the buffer (T20), the queue
//! deadline (T21), the readiness of a rate-limited step (T22), the lease recording (T23, at each of its two commits) or
//! the cancellation of a waiter (T24) leaves nothing half done, and the next attempt decides once. A grant and a timeout
//! of one buffered start, and a grant and a cancellation of it, are decided by the first commit (20 rounds each). A
//! takeover of the key's shard waits for a grant in flight. A lost task hint delays nothing a poll would not find. A
//! restarted process goes on with the buffer, window after window. A flood of 1,000 starts on one key, across a restart,
//! never holds more than the limit in one window, grants in admission order and refuses beyond the buffer with its
//! code. A lowered policy ceiling slows later grants and revokes none. Replicas whose process clocks are ten minutes
//! apart grant on the database clock, and a database clock set back by one window grants nothing until it catches up.
//!
//! A crash is the termination of the deciding transaction's backend while it waits for a row lock the test holds, after
//! it has written what it writes before that lock: PostgreSQL rolls the transaction back, as it does when the process
//! dies. Every race is held by locks and observed through `pg_stat_activity`, never by sleeps; a window that must pass
//! is waited for on the database clock with a bound (`until_past`).
//!
//! The database clock cannot be set back inside the test's PostgreSQL. Every decision of the gate compares the stored
//! instants of a key's grants with `clock_timestamp()`, so a clock set back by one window is, for those decisions, the
//! same as every counted instant of the key moved one window ahead; the test moves the grant rows' instants (with the
//! guard that forbids it switched off in that transaction, as the owner) and moves them back before the consistency
//! check, which compares them with the journal (D022-072). The rest hint and the queue deadlines keep the true clock, so
//! a path that reads the clock for anything but counting grants is not exercised by it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sagat_application::clock::{Clock, truncate_to_micros};
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::flow_control::FlowControlRefusal;
use sagat_application::runs::{RunOperation, RunService, StartRun};
use sagat_domain::execution::RunId;
use sagat_domain::quota::QuotaResource;
use sagat_domain::sharding::ShardSet;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support::engine::{World, activity, ctx, document, with};
use crate::support::flow_control::{
    age_start_deadline, database_now_us, gate, gated_world, publish, set_start_deadline, until_past,
};
use crate::support::scale::{TTL, keeper, scheduler};

const SECOND_US: i64 = 1_000_000;

/// A one-step workflow `name` whose start takes a slot of `input.accountId`, `limit` per `per`.
fn rated_start(name: &str, limit: u32, per: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": per});
    doc
}

/// A one-step workflow `name` whose step `g` takes a slot of `input.accountId`, `limit` per `per`.
fn rated_step(name: &str, limit: u32, per: &str) -> Value {
    document(
        name,
        "g",
        vec![with(
            activity("g"),
            "rateLimit",
            json!({"group": "provider", "key": "input.accountId", "limit": limit, "per": per}),
        )],
    )
}

/// The run's own grant and waiter rows: (grants, waiters).
async fn start_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// The grant and waiter rows of the step `node` of `run`: (grants, waiters).
async fn step_rows(w: &World, run: RunId, node: &str) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND node = $2 AND limit_scope = 'STEP'), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND node = $2 AND limit_scope = 'STEP')",
    )
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// The waiters the tenant's windows count, summed over its keys.
async fn counted_waiters(w: &World) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(waiters), 0)::bigint FROM rate_limit_windows WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("waiters")
}

async fn count(w: &World, run: RunId, event: &str) -> usize {
    w.journal(run).await.iter().filter(|e| *e == event).count()
}

/// The `granted_at_us` of the last `rate_limit.granted` of `run`'s start.
async fn granted_us(w: &World, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT (detail->>'granted_at_us')::bigint FROM run_events \
          WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node IS NULL ORDER BY seq DESC LIMIT 1",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("a grant event")
}

/// The start grants of `runs` in the journals: (run, `granted_at_us`), restored copies left out, in no order.
async fn start_grants(w: &World, runs: &[RunId]) -> Vec<(Uuid, i64)> {
    let ids: Vec<Uuid> = runs.iter().map(|run| run.as_uuid()).collect();
    sqlx::query_as(
        "SELECT run_id, (detail->>'granted_at_us')::bigint FROM run_events \
          WHERE run_id = ANY($1) AND event_type = 'rate_limit.granted' AND node IS NULL \
            AND NOT coalesce((detail->>'restored')::boolean, false)",
    )
    .bind(ids)
    .fetch_all(&w.db.pool)
    .await
    .expect("grants")
}

/// SC-022-017: no interval `(t - period, t]` of the journals holds more than `limit` of `instants` (microseconds).
fn at_most_per_window(instants: &[i64], limit: usize, period_us: i64) {
    for at in instants {
        let within = instants
            .iter()
            .filter(|other| **other > at - period_us && **other <= *at)
            .count();
        assert!(
            within <= limit,
            "{within} grants in the window that ends at {at}, more than {limit}: {instants:?}"
        );
    }
}

/// Holds every row of `table` (the windows or the grants) of the tenant, as a deciding transaction holds its rows.
async fn hold(w: &World, table: &str) -> Transaction<'static, Postgres> {
    let statement = match table {
        "rate_limit_windows" => "SELECT 1 FROM rate_limit_windows WHERE tenant_id = $1 FOR UPDATE",
        "rate_limit_grants" => "SELECT 1 FROM rate_limit_grants WHERE tenant_id = $1 FOR UPDATE",
        other => panic!("no rows of {other} to hold"),
    };
    let mut held = w.db.pool.begin().await.expect("tx");
    sqlx::query(statement)
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&mut *held)
        .await
        .expect("the rows held");
    held
}

/// The backends of this database that wait for a lock.
async fn lock_waiters(w: &World) -> Vec<i32> {
    sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' \
            AND state = 'active' AND pid <> pg_backend_pid() ORDER BY pid",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("activity")
}

/// Waits until `n` backends wait for a lock; a path that never blocks fails the test by name within two minutes (a
/// bound on a hang, not a timing of the race).
async fn barrier(w: &World, n: usize) -> Vec<i32> {
    crate::support::bounded::within(
        format!("{n} backend(s) wait for a lock"),
        std::time::Duration::from_secs(120),
        async {
            loop {
                let pids = lock_waiters(w).await;
                if pids.len() >= n {
                    return pids;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await
}

/// Kills the one backend that waits for a lock: the deciding transaction dies before its commit.
async fn crash_the_waiting_transaction(w: &World) {
    let pids = barrier(w, 1).await;
    assert_eq!(pids.len(), 1, "one deciding transaction waits: {pids:?}");
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pids[0])
        .fetch_one(&w.db.pool)
        .await
        .expect("terminate");
    assert!(killed);
}

/// A fresh process: a new store (empty definition cache) over the world's execution pool, and a new scheduler.
fn restarted(
    w: &World,
) -> Scheduler<PgRunStore, JsonEvaluator, sagat_application::entropy::SystemEntropy> {
    let fresh = gate(PgRunStore::new(
        w.execution_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 64),
    ));
    Scheduler::new(
        fresh,
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    )
}

/// The earliest instant a window of the tenant with waiters may grant again (its rest hint), or now.
async fn next_eligible_us(w: &World) -> i64 {
    let hint: Option<i64> = sqlx::query_scalar(
        "SELECT (extract(epoch FROM min(next_eligible_at)) * 1000000)::bigint FROM rate_limit_windows \
          WHERE tenant_id = $1 AND waiters > 0",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("hint");
    match hint {
        Some(at) => at,
        None => database_now_us(&w.db.pool).await,
    }
}

/// Runs grant passes (from `pass`, a fresh process each time when it builds one) until the tenant's windows count no
/// waiter, waiting between passes for the earliest rest hint on the database clock; fails by name after two minutes.
async fn drain<F, Fut>(w: &World, mut pass: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = sagat_scheduler::Report>,
{
    crate::support::bounded::within(
        "the buffer drains".to_owned(),
        std::time::Duration::from_secs(120),
        async {
            loop {
                pass().await;
                if counted_waiters(w).await == 0 {
                    return;
                }
                let hint = next_eligible_us(w).await;
                until_past(&w.db.pool, hint).await;
            }
        },
    )
    .await;
}

/// The rest hint of the tenant's one window with waiters, in microseconds (none while no pass has rested it).
async fn window_hint(w: &World) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM next_eligible_at) * 1000000)::bigint FROM rate_limit_windows \
          WHERE tenant_id = $1 AND waiters > 0",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the waiter's window")
}

// ------------------------------------------------------------------------------------------------- crash points

/// T19: an admission that dies while it waits for its window leaves no run, no row and no count; the retried request
/// (same idempotency key) is buffered once, and a second retry answers the same run.
#[tokio::test]
async fn a_crash_inside_the_admission_leaves_nothing_and_the_retry_is_buffered_once() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-admission", 1, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0));
    let runs = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM runs WHERE tenant_id = $1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("runs")
    };
    let before = runs().await;
    let windows = hold(&w, "rate_limit_windows").await;
    let (crashed, ()) = tokio::join!(
        w.start_with(workflow, &input, "fault-rate-admission", None),
        crash_the_waiting_transaction(&w)
    );
    assert!(crashed.is_err(), "the admission died: {crashed:?}");
    windows.rollback().await.expect("released");
    assert_eq!(runs().await, before, "no run of the dead admission");
    assert_eq!(counted_waiters(&w).await, 0, "no count moved");

    let waiter = w
        .start_with(workflow, &input, "fault-rate-admission", None)
        .await
        .expect("the retry is admitted");
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    assert_eq!(count(&w, waiter, "rate_limit.queued").await, 1);
    let again = w
        .start_with(workflow, &input, "fault-rate-admission", None)
        .await
        .expect("the retry replays");
    assert_eq!(again, waiter, "one run for the request");
    assert_eq!(counted_waiters(&w).await, 1);
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T20: a grant from the buffer that dies while it waits for the window leaves the start buffered with its row and
/// count; the next pass grants it once.
#[tokio::test]
async fn a_crash_inside_the_grant_leaves_the_start_buffered_and_the_next_pass_grants_it_once() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-grant", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    until_past(&w.db.pool, granted_us(&w, holder).await + SECOND_US).await;
    let windows = hold(&w, "rate_limit_windows").await;
    let grants = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(grants.grant_gates(), crash_the_waiting_transaction(&w));
    assert_eq!(report.gates_granted, 0, "{report:?}");
    windows.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still buffered");
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 0);
    assert_eq!(counted_waiters(&w).await, 1);

    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 1);
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 0, "once");
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T21: a queue deadline that dies while it waits for the window leaves the start buffered; the next pass times it out
/// once, through `rate_limit.wait_timed_out`, and frees its row and count.
#[tokio::test]
async fn a_crash_inside_the_timeout_leaves_the_start_buffered_and_the_next_pass_times_it_out_once()
{
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-timeout", 1, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    age_start_deadline(&w, waiter).await;
    let windows = hold(&w, "rate_limit_windows").await;
    let timeouts = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(timeouts.time_out_gates(), crash_the_waiting_transaction(&w));
    assert_eq!(report.gate_waits_timed_out, 0, "{report:?}");
    windows.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still buffered");
    assert_eq!(count(&w, waiter, "rate_limit.wait_timed_out").await, 0);

    assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 1);
    assert_eq!(count(&w, waiter, "rate_limit.wait_timed_out").await, 1);
    assert_eq!(start_rows(&w, waiter).await, (0, 0));
    assert!(w.run_status(waiter).await.is_terminal());
    assert_eq!(counted_waiters(&w).await, 0);
    assert_eq!(
        w.scheduler.time_out_gates().await.gate_waits_timed_out,
        0,
        "once"
    );
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T22: the completion of `a` makes the rate-limited `b` ready inside the completion's transaction; a completion that
/// dies while it waits for `b`'s window completes nothing and grants nothing, and the worker's retry of the same report
/// grants `b` its slot once.
#[tokio::test]
async fn a_crash_inside_the_readiness_of_a_rate_limited_step_grants_nothing_and_the_retry_grants_once()
 {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "fault-rate-readiness",
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(
                    activity("b"),
                    "rateLimit",
                    json!({"group": "provider", "key": "input.accountId", "limit": 2, "per": "hour"}),
                ),
            ],
        ),
    )
    .await;
    let input = json!({"accountId": "acct-1"});
    let session = w.session(&["act.a"]).await;
    // the first run creates the window: its `b` takes one of the two slots
    let first = w.start(workflow, &input).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("first's a");
    assert_eq!(step_rows(&w, first, "b").await, (1, 0));

    let second = w.start(workflow, &input).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, second);
    let windows = hold(&w, "rate_limit_windows").await;
    let output = json!({});
    let (crashed, ()) = tokio::join!(
        w.complete(&session, &task, &output),
        crash_the_waiting_transaction(&w)
    );
    assert!(crashed.is_err(), "the completion died: {crashed:?}");
    windows.rollback().await.expect("released");
    assert_eq!(step_rows(&w, second, "b").await, (0, 0), "nothing granted");
    assert_eq!(count(&w, second, "rate_limit.granted").await, 0);
    assert_eq!(
        count(&w, second, "step.completed").await,
        0,
        "nothing completed"
    );

    w.complete(&session, &task, &json!({}))
        .await
        .expect("the retried report");
    assert_eq!(step_rows(&w, second, "b").await, (1, 0));
    assert_eq!(count(&w, second, "rate_limit.granted").await, 1);
    for run in [first, second] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T23 at both of its commits: the lease recording updates the grant rows in one statement and then lowers the rest hint
/// of each recorded window in a statement of its own. A recording that dies while it waits for a grant row records
/// nothing; one that dies while it waits for the window has recorded the lease and left the hint as it was, which only
/// keeps the key resting as long as before (under-delivery, never an early grant). The next pass records nothing twice,
/// and no pass grants the waiter while the leased slot counts.
#[tokio::test]
async fn a_crash_inside_the_lease_recording_records_once_and_never_grants_early() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("fault-rate-lease", 1, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    assert_eq!(step_rows(&w, holder, "g").await, (1, 0));
    assert_eq!(step_rows(&w, waiter, "g").await, (0, 1));
    let session = w.session(&["act.g"]).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    let leased_at = || async {
        sqlx::query_scalar::<_, Option<OffsetDateTime>>(
            "SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
        )
        .bind(holder.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the grant")
    };
    assert_eq!(leased_at().await, None, "not recorded yet");

    // the first commit: the grant rows
    let grants = hold(&w, "rate_limit_grants").await;
    let recording = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(
        recording.record_rate_leases(),
        crash_the_waiting_transaction(&w)
    );
    assert_eq!(report.rate_leases_recorded, 0, "{report:?}");
    grants.rollback().await.expect("released");
    assert_eq!(leased_at().await, None, "nothing recorded");

    // the second commit: the window's hint, after the grants' commit
    let hint_before = window_hint(&w).await;
    let windows = hold(&w, "rate_limit_windows").await;
    let recording = scheduler(&w, SchedulerSettings::default(), None);
    let (_, ()) = tokio::join!(
        recording.record_rate_leases(),
        crash_the_waiting_transaction(&w)
    );
    windows.rollback().await.expect("released");
    assert!(leased_at().await.is_some(), "the lease was recorded");
    assert_eq!(
        window_hint(&w).await,
        hint_before,
        "the hint statement died: the key rests as long as before, never shorter"
    );
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        0,
        "once"
    );
    assert_eq!(
        w.scheduler.grant_gates().await.gates_granted,
        0,
        "no early grant"
    );
    assert_eq!(step_rows(&w, waiter, "g").await, (0, 1));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T24: a cancellation of a buffered start that dies while it waits for the window cancels nothing and dequeues
/// nothing; the retried cancellation dequeues it once, through `rate_limit.dequeued`.
#[tokio::test]
async fn a_crash_inside_the_cancellation_of_a_waiter_dequeues_nothing_and_the_retry_dequeues_once()
{
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-cancel", 1, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    let windows = hold(&w, "rate_limit_windows").await;
    let (crashed, ()) = tokio::join!(
        w.operate(waiter, RunOperation::Cancel),
        crash_the_waiting_transaction(&w)
    );
    assert!(crashed.is_err(), "the cancellation died: {crashed:?}");
    windows.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still buffered");
    assert_eq!(count(&w, waiter, "rate_limit.dequeued").await, 0);
    assert!(!w.run_status(waiter).await.is_terminal());

    w.operate_run(waiter, RunOperation::Cancel).await;
    assert_eq!(count(&w, waiter, "rate_limit.dequeued").await, 1);
    assert_eq!(start_rows(&w, waiter).await, (0, 0));
    assert_eq!(counted_waiters(&w).await, 0);
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

// ------------------------------------------------------------------------------------------------------- races

/// A holder granted now and a buffered start behind it on the fresh key `account`, with the holder's slot past its
/// one-second window on the database clock, so a pass may grant the waiter.
async fn grantable(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    account: &str,
) -> (RunId, RunId) {
    let input = json!({"accountId": account});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    assert_eq!(start_rows(w, waiter).await, (0, 1), "{account}");
    until_past(&w.db.pool, granted_us(w, holder).await + SECOND_US).await;
    (holder, waiter)
}

/// SC-022-018: a grant and a timeout of one buffered start, 20 rounds in four orders: (0) the start due, the grant pass
/// first and then the timeout, both behind the held window; (1) the timeout first; (2) the grant commits before the
/// deadline; (3) the grant lists the start before its deadline and waits for the window, the deadline passes while it
/// waits, and the timeout, which lists the start due, waits for the run (only the grant's check of the deadline under
/// the window's lock refuses it). The first commit decides: exactly one of `rate_limit.granted` and
/// `rate_limit.wait_timed_out`, no row left behind, and the next pass decides nothing.
#[tokio::test]
async fn a_grant_and_a_timeout_of_a_buffered_start_are_decided_by_the_first_commit() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-timeout-race", 1, "second")).await;
    for round in 0..20 {
        let (holder, waiter) = grantable(&w, workflow, &format!("race-{round}")).await;
        let other = || scheduler(&w, SchedulerSettings::default(), None);
        match round % 4 {
            3 => {
                let deadline = set_start_deadline(&w.db.pool, waiter, 2 * SECOND_US).await;
                let windows = hold(&w, "rate_limit_windows").await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 1).await;
                assert!(
                    database_now_us(&w.db.pool).await < deadline,
                    "round {round}: the grant listed the start before its deadline"
                );
                until_past(&w.db.pool, deadline).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                windows.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(
                    g.gates_granted, 0,
                    "a start due under the window is never granted: {g:?}"
                );
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            0 => {
                age_start_deadline(&w, waiter).await;
                let windows = hold(&w, "rate_limit_windows").await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 1).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                windows.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 0, "a due start is never granted: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            1 => {
                age_start_deadline(&w, waiter).await;
                let windows = hold(&w, "rate_limit_windows").await;
                let (grants, timeouts) = (other(), other());
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 1).await;
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 2).await;
                windows.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 0, "the run is the timeout's: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            _ => {
                assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
                age_start_deadline(&w, waiter).await;
                assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 0);
            }
        }
        let (granted, timed_out) = (
            count(&w, waiter, "rate_limit.granted").await,
            count(&w, waiter, "rate_limit.wait_timed_out").await,
        );
        assert_eq!(granted + timed_out, 1, "round {round}: exactly one outcome");
        assert_eq!(timed_out == 1, round % 4 != 2, "round {round}");
        assert_eq!(
            start_rows(&w, waiter).await,
            (i64::from(granted == 1), 0),
            "round {round}: no row left behind"
        );
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted + report.gate_waits_timed_out,
            0,
            "round {round}: decided once: {report:?}"
        );
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
    assert_eq!(counted_waiters(&w).await, 0, "every buffer drained");
    w.assert_keys_consistent().await;
}

/// Releases `held` once `done` is set or `n` backends wait for a lock, whichever comes first.
async fn release_when(
    w: &World,
    held: Transaction<'static, Postgres>,
    done: &AtomicBool,
    n: usize,
) {
    crate::support::bounded::within(
        format!("the pass ends or {n} backend(s) wait"),
        std::time::Duration::from_secs(120),
        async {
            while !done.load(Ordering::SeqCst) && lock_waiters(w).await.len() < n {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    held.rollback().await.expect("released");
}

/// SC-022-018: a grant and a cancellation of one buffered start, 20 rounds in two orders: (0) the grant takes the run
/// and waits for the window, the cancellation waits for the run; (1) the cancellation takes the run and waits for the
/// window, and the grant pass, which skips a run another transaction holds, either ends without it or waits. The first
/// commit decides: exactly one of `rate_limit.granted` and `rate_limit.dequeued`, the run cancelled either way, no
/// waiter row left, and the next pass decides nothing.
#[tokio::test]
async fn a_grant_and_a_cancellation_of_a_buffered_start_are_decided_by_the_first_commit() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-cancel-race", 1, "second")).await;
    for round in 0..20 {
        let (holder, waiter) = grantable(&w, workflow, &format!("cancel-{round}")).await;
        let grants = scheduler(&w, SchedulerSettings::default(), None);
        let windows = hold(&w, "rate_limit_windows").await;
        if round % 2 == 0 {
            let (g, cancelled, ()) = tokio::join!(
                grants.grant_gates(),
                async {
                    barrier(&w, 1).await;
                    w.operate(waiter, RunOperation::Cancel).await
                },
                async {
                    barrier(&w, 2).await;
                    windows.rollback().await.expect("released");
                }
            );
            assert_eq!(
                g.gates_granted, 1,
                "round {round}: the grant came first: {g:?}"
            );
            cancelled.expect("the cancellation commits after it");
        } else {
            let done = AtomicBool::new(false);
            let (cancelled, g, ()) = tokio::join!(
                w.operate(waiter, RunOperation::Cancel),
                async {
                    barrier(&w, 1).await;
                    let report = grants.grant_gates().await;
                    done.store(true, Ordering::SeqCst);
                    report
                },
                async {
                    barrier(&w, 1).await;
                    release_when(&w, windows, &done, 2).await;
                }
            );
            cancelled.expect("the cancellation came first");
            assert_eq!(
                g.gates_granted, 0,
                "round {round}: the run is the cancellation's: {g:?}"
            );
        }
        let (granted, dequeued) = (
            count(&w, waiter, "rate_limit.granted").await,
            count(&w, waiter, "rate_limit.dequeued").await,
        );
        assert_eq!(granted + dequeued, 1, "round {round}: exactly one outcome");
        assert_eq!(dequeued == 1, round % 2 == 1, "round {round}");
        assert_eq!(
            start_rows(&w, waiter).await.1,
            0,
            "round {round}: no waiter row"
        );
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted, 0,
            "round {round}: decided once: {report:?}"
        );
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
    }
    assert_eq!(counted_waiters(&w).await, 0, "every buffer drained");
    w.assert_keys_consistent().await;
}

/// SC-022-018 (takeover duplicate grant): a grant holds its key's home-shard row from its fence to its commit, so a new
/// owner's takeover waits for a grant in flight; the old owner's grant commits once and the new owner grants nothing
/// again.
#[tokio::test]
async fn a_takeover_waits_for_a_grant_in_flight_and_no_slot_is_granted_twice() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-takeover", 1, "second")).await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let old_owner = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(alpha.placement().clone()),
    );
    let (holder, waiter) = grantable(&w, workflow, "acct-1").await;
    let windows = hold(&w, "rate_limit_windows").await;
    let grant = tokio::spawn(async move { old_owner.grant_gates().await });
    barrier(&w, 1).await;
    // alpha's lease runs out while its grant waits for the window: the expiry waits for the grant's fence
    let pool = w.db.pool.clone();
    let expire = tokio::spawn(async move {
        sqlx::query(
            "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'",
        )
        .execute(&pool)
        .await
        .expect("alpha expired")
    });
    barrier(&w, 2).await;
    windows.rollback().await.expect("released");
    let granted = grant.await.expect("grant");
    assert_eq!(
        granted.gates_granted, 1,
        "the grant in flight commits: {granted:?}"
    );
    expire.await.expect("expired");
    sqlx::query(
        "UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'",
    )
    .execute(&w.db.pool)
    .await
    .expect("alpha gone");
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    assert_eq!(beta.placement().owned(), ShardSet::all());
    let new_owner = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(beta.placement().clone()),
    );
    assert_eq!(
        new_owner.grant_gates().await.gates_granted,
        0,
        "nothing twice"
    );
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

// ------------------------------------------------------------------------------- hints, restarts and the clock

/// A grant announces its step once (`task.ready`, one outbox row), and the step is found without the announcement: the
/// row is marked delivered as a broker that lost it would leave it, and the worker's poll of PostgreSQL leases the
/// granted start's step all the same. This world runs no broker, so the test proves the announcement's count and the
/// poll path, not a hint consumer.
#[tokio::test]
async fn a_grant_announces_its_step_once_and_a_poll_finds_it_without_the_announcement() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-hint", 1, "second")).await;
    let (holder, waiter) = grantable(&w, workflow, "acct-1").await;
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    let lost = sqlx::query(
        "UPDATE event_outbox SET delivered_at = clock_timestamp() \
          WHERE event_type = 'task.ready' AND aggregate_id = $1 AND delivered_at IS NULL",
    )
    .bind(waiter.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("hint lost");
    assert_eq!(lost.rows_affected(), 1, "the grant announced its step once");
    let session = w.session(&["act.g"]).await;
    let tasks = w.poll(&session, 8).await;
    let task = tasks
        .iter()
        .find(|task| task.run_id == waiter)
        .expect("the poll finds the granted start's step");
    w.complete(&session, task, &json!({})).await.expect("done");
    assert!(w.run_status(waiter).await.is_terminal());
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// A restart after several windows: every pass is a new process (a fresh store and scheduler over the same database,
/// nothing carried over), and the buffer of three starts behind a holder is granted one window at a time, in
/// admission order, each once.
#[tokio::test]
async fn restarted_processes_go_on_with_the_buffer_window_after_window() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-restart", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let mut admitted = vec![holder];
    for _ in 0..3 {
        admitted.push(w.start(workflow, &input).await);
    }
    assert_eq!(counted_waiters(&w).await, 3);
    drain(&w, || async { restarted(&w).grant_gates().await }).await;
    let grants = start_grants(&w, &admitted).await;
    assert_eq!(grants.len(), 4, "each run granted once: {grants:?}");
    let instants: Vec<i64> = admitted
        .iter()
        .map(|run| {
            grants
                .iter()
                .find(|(id, _)| *id == run.as_uuid())
                .expect("a grant of each run")
                .1
        })
        .collect();
    for pair in instants.windows(2) {
        assert!(
            pair[1] >= pair[0] + SECOND_US,
            "one grant a window, in admission order: {instants:?}"
        );
    }
    for run in admitted {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// SC-022-017: `limit: 10, per: second` and 1,000 same-key starts, the buffer drained by one process and then by a
/// restarted one. The starts granted at admission and the 100 the key buffers (its default ceiling of waiters) are
/// admitted, the rest refused `RATE_LIMIT_QUEUE_FULL` with nothing of them created; no one-second interval of the
/// journals holds more than 10 grants; grants follow admission order; each admitted run is granted once.
#[tokio::test]
async fn a_flood_of_a_thousand_starts_on_one_key_never_holds_more_than_its_limit_in_a_window() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-flood", 10, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let mut admitted = Vec::new();
    let mut refused = 0_usize;
    for i in 0..1_000 {
        match w
            .start_with(workflow, &input, &format!("flood-start-{i:04}"), None)
            .await
        {
            Ok(run) => admitted.push(run),
            Err(AppError::FlowControl(FlowControlRefusal::RateLimitQueueFull)) => refused += 1,
            Err(other) => panic!("start {i}: {other:?}"),
        }
    }
    let buffered = usize::try_from(counted_waiters(&w).await).expect("a count");
    assert_eq!(
        buffered, 100,
        "the key buffers up to its ceiling of waiters"
    );
    let at_admission = admitted.len() - buffered;
    assert!(
        at_admission >= 10,
        "the first ten are granted at admission: {at_admission}"
    );
    assert_eq!(
        admitted.len() + refused,
        1_000,
        "every start is admitted or refused with its code"
    );
    let runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id = $1 AND workflow_id = $2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(workflow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("runs");
    assert_eq!(
        usize::try_from(runs).expect("a count"),
        admitted.len(),
        "a refused start creates nothing"
    );

    // the first process drains half the buffer, a restarted one the rest
    let first = scheduler(&w, SchedulerSettings::default(), None);
    crate::support::bounded::within(
        "half the buffer drains".to_owned(),
        std::time::Duration::from_secs(120),
        async {
            while counted_waiters(&w).await > 50 {
                first.grant_gates().await;
                let hint = next_eligible_us(&w).await;
                until_past(&w.db.pool, hint).await;
            }
        },
    )
    .await;
    let second = restarted(&w);
    drain(&w, || second.grant_gates()).await;

    let grants = start_grants(&w, &admitted).await;
    assert_eq!(
        grants.len(),
        admitted.len(),
        "each admitted run granted once"
    );
    let mut instants: Vec<i64> = grants.iter().map(|(_, at)| *at).collect();
    instants.sort_unstable();
    at_most_per_window(&instants, 10, SECOND_US);
    let in_order: Vec<i64> = admitted
        .iter()
        .map(|run| {
            grants
                .iter()
                .find(|(id, _)| *id == run.as_uuid())
                .expect("a grant of each run")
                .1
        })
        .collect();
    assert!(
        in_order.windows(2).all(|pair| pair[0] <= pair[1]),
        "grants follow admission order"
    );
    for run in admitted {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// SC-022-022: a policy ceiling lowered while runs wait slows later grants and revokes none. Three starts hold the
/// three slots of `limit: 3, per: second`; two more are buffered with that limit; the namespace then lowers the ceiling
/// of a one-second window to 1. The three grants stay (nothing returned or dequeued), and each later grant is evaluated
/// against the lowered ceiling at once: none within a second of another grant.
#[tokio::test]
async fn a_lowered_policy_ceiling_slows_later_grants_and_revokes_none() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-lowered", 3, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(w.start(workflow, &input).await);
    }
    let mut waiting = Vec::new();
    for _ in 0..2 {
        waiting.push(w.start(workflow, &input).await);
    }
    for run in &held {
        assert_eq!(start_rows(&w, *run).await, (1, 0));
    }
    assert_eq!(counted_waiters(&w).await, 2);
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::RateLimitPerSecond,
            1,
            &ctx(),
        )
        .await
        .expect("the ceiling lowered");
    drain(&w, || w.scheduler.grant_gates()).await;
    for run in &held {
        assert_eq!(
            count(&w, *run, "rate_limit.granted").await,
            1,
            "no grant revoked"
        );
        assert_eq!(count(&w, *run, "rate_limit.returned").await, 0);
        assert_eq!(count(&w, *run, "rate_limit.dequeued").await, 0);
    }
    let runs: Vec<RunId> = held.iter().chain(waiting.iter()).copied().collect();
    let grants = start_grants(&w, &runs).await;
    assert_eq!(grants.len(), 5, "each run granted once");
    let at = |run: RunId| {
        grants
            .iter()
            .find(|(id, _)| *id == run.as_uuid())
            .expect("a grant")
            .1
    };
    let last_held = held.iter().map(|run| at(*run)).max().expect("three");
    let (d, e) = (at(waiting[0]), at(waiting[1]));
    assert!(
        d >= last_held + SECOND_US && e >= d + SECOND_US,
        "with an effective limit of 1 no later grant shares a window with another: held up to {last_held}, then {d}, {e}"
    );
    for run in runs {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// A process clock `offset` away from the true time.
struct OffsetClock(time::Duration);

impl Clock for OffsetClock {
    fn now(&self) -> OffsetDateTime {
        truncate_to_micros(OffsetDateTime::now_utc() + self.0)
    }
}

type OffsetRuns = RunService<PgRunStore, OffsetClock, JsonEvaluator>;

/// A replica of the control plane whose process clock is `offset` away (its own store and definition cache).
fn replica_runs(w: &World, offset: time::Duration) -> OffsetRuns {
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 64);
    RunService::new(
        gate(PgRunStore::new(w.execution_pool.clone(), cache.clone())),
        OffsetClock(offset),
        Engine::new(JsonEvaluator),
        cache,
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        sagat_application::run_input::InputValidationMode::Enforce,
    ))
}

/// SC-022-023 (process clocks): two replicas whose process clocks are ten minutes ahead and ten minutes behind start
/// twelve runs of one key with `limit: 2, per: second`, alternately. Every grant at admission bears an instant of the
/// database clock read around its start, and every buffered start's queue deadline is the database clock's instant of
/// its admission plus the default queue timeout of ten minutes, never shifted by a process clock; no one-second interval
/// holds more than two grants; each run is granted once.
#[tokio::test]
async fn replicas_with_offset_process_clocks_grant_on_the_database_clock() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-clocks", 2, "second")).await;
    let ten_minutes = time::Duration::minutes(10);
    let replicas = [
        replica_runs(&w, ten_minutes),
        replica_runs(&w, -ten_minutes),
    ];
    let bytes = serde_json::to_vec(&json!({"accountId": "acct-1"})).expect("json");
    let mut admitted = Vec::new();
    for i in 0..12 {
        let before = database_now_us(&w.db.pool).await;
        let key = format!("clocks-{i}");
        let run = replicas[i % 2]
            .start(
                &w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &bytes,
                    correlation_id: None,
                    deadline: None,
                    key: &key,
                },
                &ctx(),
            )
            .await
            .expect("a start")
            .info
            .run_id;
        let after = database_now_us(&w.db.pool).await;
        admitted.push((run, before, after));
    }
    for (run, before, after) in &admitted {
        let (granted, queued): (Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT (SELECT (detail->>'granted_at_us')::bigint FROM run_events \
                      WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node IS NULL), \
                    (SELECT (detail->>'queue_deadline_us')::bigint FROM run_events \
                      WHERE run_id = $1 AND event_type = 'rate_limit.queued' AND node IS NULL)",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the start's decision");
        match (granted, queued) {
            (Some(at), None) => assert!(
                (*before..=*after).contains(&at),
                "a grant at admission bears the database clock: {at} outside [{before}, {after}]"
            ),
            (None, Some(deadline)) => {
                let timeout = 600 * SECOND_US;
                assert!(
                    (*before + timeout..=*after + timeout).contains(&deadline),
                    "a queue deadline is the database clock plus the timeout: {deadline} outside [{before}, {after}] + 10 min"
                );
            }
            other => panic!("one decision at admission: {other:?}"),
        }
    }
    drain(&w, || w.scheduler.grant_gates()).await;
    let runs: Vec<RunId> = admitted.iter().map(|(run, _, _)| *run).collect();
    let grants = start_grants(&w, &runs).await;
    assert_eq!(grants.len(), 12, "each run granted once");
    let mut instants: Vec<i64> = grants.iter().map(|(_, at)| *at).collect();
    instants.sort_unstable();
    at_most_per_window(&instants, 2, SECOND_US);
    for run in runs {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Moves the instants of `run`'s grant rows by `by_us` microseconds, as the owner, with the guard that forbids it
/// switched off for this transaction only (module comment: the database clock set back).
async fn move_grant_instants(w: &World, run: RunId, by_us: i64) {
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("ALTER TABLE rate_limit_grants DISABLE TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard off in this transaction");
    sqlx::query(
        "UPDATE rate_limit_grants SET granted_at = granted_at + $2 * interval '1 microsecond', \
                leased_at = leased_at + $2 * interval '1 microsecond' WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .bind(by_us)
    .execute(&mut *tx)
    .await
    .expect("the grant's instants moved");
    sqlx::query("ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard on again");
    tx.commit().await.expect("moved");
}

/// SC-022-023 (database clock): the database clock set back by one window after a holder's grant at `t0`, with
/// `limit: 1, per: second`. The holder's slot then counts until `t0 + 2 s` of the true time, so a pass between
/// `t0 + 1 s` and `t0 + 2 s` grants nothing, and the waiter's grant is never before `t0 + 2 s`.
#[tokio::test]
async fn a_database_clock_set_back_by_one_window_grants_nothing_until_it_catches_up() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-set-back", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    let t0 = granted_us(&w, holder).await;
    move_grant_instants(&w, holder, SECOND_US).await;
    until_past(&w.db.pool, t0 + SECOND_US).await;
    let report = w.scheduler.grant_gates().await;
    let after = database_now_us(&w.db.pool).await;
    // a pass that ended before the clock caught up decided before it too (a slower runner checks only the bound below)
    if after < t0 + 2 * SECOND_US {
        assert_eq!(
            report.gates_granted, 0,
            "the clock has not caught up: {report:?}"
        );
    }
    drain(&w, || w.scheduler.grant_gates()).await;
    let granted = granted_us(&w, waiter).await;
    assert!(
        granted >= t0 + 2 * SECOND_US,
        "no grant before the clock catches up: {granted} < {t0} + 2 s"
    );
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 1);
    move_grant_instants(&w, holder, -SECOND_US).await;
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}
