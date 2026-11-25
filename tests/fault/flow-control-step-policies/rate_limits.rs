//! Faults of the rate gate (task T022-080; SC-022-017, SC-022-018, SC-022-022, SC-022-023; NFR-022-001, NFR-022-008;
//! ADR-0088 evidence). A process that dies inside the admission (T19), the grant from the buffer (T20), the queue
//! deadline (T21), the readiness of a rate-limited step (T22), the lease recording (T23, at each of its two commits) or
//! the cancellation of a waiter (T24) leaves nothing half done, and the next attempt decides once; T20, T21 and T24 die
//! a second time after their gate writes, waiting on the journal (decision D022-075 item 6). A grant and a timeout of a
//! buffered start, and a grant and a cancellation of it, are decided by the first commit (20 rounds each). A
//! takeover of the key's shard waits for a grant in flight. A lost task hint delays nothing a poll would not find. A
//! restarted process goes on with the buffer, window after window. A flood of 1,000 starts on one key, across a restart,
//! never holds more than the limit in one window, grants in admission order (a new start queues behind an eligible
//! waiter, and a pass counts as of its selection of the key: decision D022-073) and refuses beyond the buffer with its
//! code. A lowered policy ceiling slows later grants, revokes none and is recorded in the grants it decides. A waiter of a
//! longer window refused by its own limit does not hold back a shorter one. Replicas whose process clocks are ten
//! minutes apart grant on the database clock, and a database clock set back grants nothing until it catches up.
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

/// Holds an uncommitted journal row of `run` at its next sequence number, so the transaction that journals `run`'s next
/// event waits on the journal's primary key after every statement it runs before its journal (decision D022-075 item
/// 6). The row is written with the foreign keys' triggers off in this transaction only (`session_replication_role`, as
/// the owner), so it does not hold `run`'s row, which the deciding transaction locks first.
async fn hold_next_event(w: &World, run: RunId) -> Transaction<'static, Postgres> {
    let mut held = w.db.pool.begin().await.expect("tx");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *held)
        .await
        .expect("the foreign keys' triggers off in this transaction");
    let written = sqlx::query(
        "INSERT INTO run_events (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         SELECT tenant_id, run_id, max(seq) + 1, 'test.held', '{}'::jsonb, 'test', clock_timestamp() \
           FROM run_events WHERE run_id = $1 GROUP BY tenant_id, run_id",
    )
    .bind(run.as_uuid())
    .execute(&mut *held)
    .await
    .expect("the next journal row held")
    .rows_affected();
    assert_eq!(written, 1, "the run has a journal");
    held
}

/// Kills the one backend that waits for a lock once it waits on the journal, after its statements on each of `tables`
/// (it holds the lock a row write takes on each): the deciding transaction dies after its gate rows are written.
async fn crash_at_the_journal(w: &World, tables: &[&str]) {
    let pids = barrier(w, 1).await;
    assert_eq!(pids.len(), 1, "one deciding transaction waits: {pids:?}");
    let statement: String = sqlx::query_scalar("SELECT query FROM pg_stat_activity WHERE pid = $1")
        .bind(pids[0])
        .fetch_one(&w.db.pool)
        .await
        .expect("the waiting statement");
    assert!(
        statement.contains("run_events"),
        "the deciding transaction waits on the journal: {statement}"
    );
    for table in tables {
        let wrote: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_locks l JOIN pg_class c ON c.oid = l.relation \
                             WHERE l.pid = $1 AND c.relname = $2 AND l.mode = 'RowExclusiveLock' AND l.granted)",
        )
        .bind(pids[0])
        .bind(*table)
        .fetch_one(&w.db.pool)
        .await
        .expect("locks");
        assert!(
            wrote,
            "the deciding transaction wrote {table} before its journal"
        );
    }
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pids[0])
        .fetch_one(&w.db.pool)
        .await
        .expect("terminate");
    assert!(killed);
}

/// The journal of `run`, the rate gate's rows of its start and the waiters the tenant's windows count: what a crash
/// after the gate's writes must leave as it was.
async fn rate_state(w: &World, run: RunId) -> (Vec<String>, (i64, i64), i64) {
    (
        w.journal(run).await,
        start_rows(w, run).await,
        counted_waiters(w).await,
    )
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

/// T20: a grant from the buffer that dies while it waits for the window, or after its gate writes while it waits on the
/// journal, leaves the start buffered with its row and count; the next pass grants it once.
#[tokio::test]
async fn a_crash_inside_the_grant_leaves_the_start_buffered_and_the_next_pass_grants_it_once() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-grant", 1, "second")).await;
    let (holder, waiter) = grantable(&w, workflow, "acct-1").await;
    let windows = hold(&w, "rate_limit_windows").await;
    let grants = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(grants.grant_gates(), crash_the_waiting_transaction(&w));
    assert_eq!(report.gates_granted, 0, "{report:?}");
    windows.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still buffered");
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 0);
    assert_eq!(counted_waiters(&w).await, 1);

    // the second crash point (decision D022-075 item 6): the waiter row deleted, the grant row and the window's hint
    // written, the transaction waits on the journal and dies there
    let before = rate_state(&w, waiter).await;
    let journal = hold_next_event(&w, waiter).await;
    let grants = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(
        grants.grant_gates(),
        crash_at_the_journal(&w, &["rate_limit_waiters", "rate_limit_grants"])
    );
    assert_eq!(report.gates_granted, 0, "{report:?}");
    journal.rollback().await.expect("released");
    assert_eq!(rate_state(&w, waiter).await, before, "nothing half done");

    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 1);
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 0, "once");
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T21: a queue deadline that dies while it waits for the window, or after its waiter row's delete while it waits on the
/// journal, leaves the start buffered; the next pass times it out once, through `rate_limit.wait_timed_out`, and frees
/// its row and count.
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

    // the second crash point (decision D022-075 item 6): the waiter row deleted, the timeout waits on the journal
    let before = rate_state(&w, waiter).await;
    let journal = hold_next_event(&w, waiter).await;
    let timeouts = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(
        timeouts.time_out_gates(),
        crash_at_the_journal(&w, &["rate_limit_waiters"])
    );
    assert_eq!(report.gate_waits_timed_out, 0, "{report:?}");
    journal.rollback().await.expect("released");
    assert_eq!(rate_state(&w, waiter).await, before, "nothing half done");
    assert!(!w.run_status(waiter).await.is_terminal());

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
/// and the waiter is granted no earlier than the recorded lease plus its one-second window (minor 2 of the review: with
/// `per: second` the window ends during the test, so an early grant would show).
#[tokio::test]
async fn a_crash_inside_the_lease_recording_records_once_and_never_grants_early() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("fault-rate-lease", 1, "second")).await;
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
    let leased_us = leased_at()
        .await
        .map(|at| i64::try_from(at.unix_timestamp_nanos() / 1_000).expect("an instant"))
        .expect("recorded");
    drain(&w, || w.scheduler.grant_gates()).await;
    let granted: i64 = sqlx::query_scalar(
        "SELECT (detail->>'granted_at_us')::bigint FROM run_events \
          WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node = 'g'",
    )
    .bind(waiter.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the waiter's grant");
    assert!(
        granted >= leased_us + SECOND_US,
        "never an early grant: {granted} < the lease {leased_us} + 1 s"
    );
    assert_eq!(step_rows(&w, waiter, "g").await, (1, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// T24: a cancellation of a buffered start that dies while it waits for the window, or after its waiter row's delete
/// while it waits on the journal, cancels nothing and dequeues nothing; the retried cancellation dequeues it once,
/// through `rate_limit.dequeued`.
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

    // the second crash point (decision D022-075 item 6): the waiter row deleted, the cancellation waits on the journal
    let before = rate_state(&w, waiter).await;
    let journal = hold_next_event(&w, waiter).await;
    let (crashed, ()) = tokio::join!(
        w.operate(waiter, RunOperation::Cancel),
        crash_at_the_journal(&w, &["rate_limit_waiters"])
    );
    assert!(crashed.is_err(), "the cancellation died: {crashed:?}");
    journal.rollback().await.expect("released");
    assert_eq!(rate_state(&w, waiter).await, before, "nothing half done");
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

/// How far a holder's counted instants are moved ahead while the starts behind it are admitted, so that they queue at a
/// full window whatever the runner's speed (decision D022-072): a minute, far beyond any delay between two admissions.
const ADMISSION_MARGIN_US: i64 = 60 * SECOND_US;

/// A holder granted now and a buffered start behind it on the fresh key `account`, with the holder's slot past its
/// one-second window on the database clock, so a pass may grant the waiter. The holder's instants are moved ahead while
/// the waiter is admitted and moved back after (the runtime-role run of 2026-11-24 granted the waiter at its admission on
/// a loaded runner, a second after the holder's grant), so the waiter queues whatever the runner's speed; the key's hint
/// is the waiter's queueing instant, which the move does not touch.
async fn grantable(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    account: &str,
) -> (RunId, RunId) {
    let pair = admitted_behind(w, workflow, account, false).await;
    until_past(&w.db.pool, granted_us(w, pair.0).await + SECOND_US).await;
    pair
}

/// A start of `workflow` by the first tenant or, with `other`, by the second.
async fn start_by(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    input: &Value,
    other: bool,
) -> RunId {
    if other {
        w.start_as_other_tenant(workflow, input)
            .await
            .expect("started")
    } else {
        w.start(workflow, input).await
    }
}

/// A holder granted now and a buffered start behind it on the fresh key `account`, started by the first tenant or, with
/// `other`, by the second (whose `workflow` it is): [`grantable`] without the wait for the holder's window.
async fn admitted_behind(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    account: &str,
    other: bool,
) -> (RunId, RunId) {
    let input = json!({"accountId": account});
    let holder = start_by(w, workflow, &input, other).await;
    move_grant_instants(w, holder, ADMISSION_MARGIN_US).await;
    let waiter = start_by(w, workflow, &input, other).await;
    assert_eq!(start_rows(w, waiter).await, (0, 1), "{account}");
    move_grant_instants(w, holder, -ADMISSION_MARGIN_US).await;
    (holder, waiter)
}

/// Two holders granted now and two buffered starts behind them on the fresh key `account` of a workflow with `limit: 2`,
/// the holders' slots past their one-second window on the database clock: (first, second) in admission order. As
/// [`grantable`], the holders' instants are moved ahead while the two are admitted.
async fn grantable_pair(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    account: &str,
) -> ([RunId; 2], RunId, RunId) {
    let input = json!({"accountId": account});
    let holders = [
        w.start(workflow, &input).await,
        w.start(workflow, &input).await,
    ];
    for holder in holders {
        move_grant_instants(w, holder, ADMISSION_MARGIN_US).await;
    }
    let first = w.start(workflow, &input).await;
    let second = w.start(workflow, &input).await;
    for waiter in [first, second] {
        assert_eq!(start_rows(w, waiter).await, (0, 1), "{account}");
    }
    for holder in holders {
        move_grant_instants(w, holder, -ADMISSION_MARGIN_US).await;
    }
    until_past(&w.db.pool, granted_us(w, holders[1]).await + SECOND_US).await;
    (holders, first, second)
}

/// Holds `run`'s row, as any transaction of the run does (a signal, an operator's command): a pass that takes the runs
/// it decides `SKIP LOCKED` skips it.
async fn hold_run(w: &World, run: RunId) -> Transaction<'static, Postgres> {
    let mut held = w.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run.as_uuid())
        .execute(&mut *held)
        .await
        .expect("the run held");
    held
}

/// SC-022-018: a grant and a timeout of a buffered start, 20 rounds in four orders of five, each reaching the decision it
/// names (decision D022-075 item 1):
/// - (0) the grant lists the start before its deadline, takes its run and waits for the window; the deadline passes and
///   the timeout, which lists the start due, waits for the run; the grant's check of the deadline under the window's
///   lock refuses it and the timeout decides;
/// - (1) the grant lists the start before its deadline and finds its run held: it skips it, and after the deadline the
///   timeout decides;
/// - (2) the grant commits before the deadline and the timeout finds nothing;
/// - (3) the grant has two starts of one key in hand and waits for the window on the first; the second's deadline passes
///   and its timeout, holding the second's run, waits for the window too; whichever commits first, the grant grants the
///   first and not the second, which the timeout decides.
///
/// Each start ends with exactly one of `rate_limit.granted` and `rate_limit.wait_timed_out` and no row left behind, and the
/// next pass decides nothing.
#[tokio::test]
async fn a_grant_and_a_timeout_of_a_buffered_start_are_decided_by_the_first_commit() {
    // a deadline far enough ahead for the grant to list the start on a loaded runner (minor 6 of the review)
    const MARGIN_US: i64 = 3 * SECOND_US;
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-timeout-race", 1, "second")).await;
    let pair = publish(&w, rated_start("fault-rate-timeout-race-pair", 2, "second")).await;
    for round in 0..20 {
        let other = || scheduler(&w, SchedulerSettings::default(), None);
        // (run, whether it is granted)
        let mut decided: Vec<(RunId, bool)> = Vec::new();
        let mut runs: Vec<RunId> = Vec::new();
        match round % 4 {
            0 => {
                let (holder, waiter) = grantable(&w, workflow, &format!("race-{round}")).await;
                let deadline = set_start_deadline(&w.db.pool, waiter, MARGIN_US).await;
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
                    "round {round}: a start due under the window is never granted: {g:?}"
                );
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
                runs.extend([holder, waiter]);
                decided.push((waiter, false));
            }
            1 => {
                let (holder, waiter) = grantable(&w, workflow, &format!("race-{round}")).await;
                let deadline = set_start_deadline(&w.db.pool, waiter, MARGIN_US).await;
                let held = hold_run(&w, waiter).await;
                let g = other().grant_gates().await;
                assert!(
                    database_now_us(&w.db.pool).await < deadline,
                    "round {round}: the grant listed the start before its deadline"
                );
                assert_eq!(
                    (g.gates_granted, g.skipped),
                    (0, 1),
                    "round {round}: the grant skips the held run: {g:?}"
                );
                until_past(&w.db.pool, deadline).await;
                held.rollback().await.expect("released");
                let t = other().time_out_gates().await;
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
                runs.extend([holder, waiter]);
                decided.push((waiter, false));
            }
            2 => {
                let (holder, waiter) = grantable(&w, workflow, &format!("race-{round}")).await;
                assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
                age_start_deadline(&w, waiter).await;
                assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 0);
                runs.extend([holder, waiter]);
                decided.push((waiter, true));
            }
            _ => {
                let (holders, first, second) =
                    grantable_pair(&w, pair, &format!("pair-{round}")).await;
                // set in admission order: the first stays ahead of the second
                set_start_deadline(&w.db.pool, first, 60 * SECOND_US).await;
                let deadline = set_start_deadline(&w.db.pool, second, MARGIN_US).await;
                let windows = hold(&w, "rate_limit_windows").await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 1).await;
                assert!(
                    database_now_us(&w.db.pool).await < deadline,
                    "round {round}: the grant listed both starts before the second's deadline"
                );
                until_past(&w.db.pool, deadline).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                windows.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 1, "round {round}: the first only: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
                runs.extend(holders);
                runs.extend([first, second]);
                decided.extend([(first, true), (second, false)]);
            }
        }
        for (run, expected) in decided {
            let (granted, timed_out) = (
                count(&w, run, "rate_limit.granted").await,
                count(&w, run, "rate_limit.wait_timed_out").await,
            );
            assert_eq!(granted + timed_out, 1, "round {round}: exactly one outcome");
            assert_eq!(granted == 1, expected, "round {round}");
            assert_eq!(
                start_rows(&w, run).await,
                (i64::from(granted == 1), 0),
                "round {round}: no row left behind"
            );
        }
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted + report.gate_waits_timed_out,
            0,
            "round {round}: decided once: {report:?}"
        );
        for run in runs {
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
        // a run's slot counts from its grant, which is its lease, and is never returned: a start granted and then
        // cancelled keeps its grant row for its window; a dequeued one has none (minor 1 of the review)
        assert_eq!(
            start_rows(&w, waiter).await,
            (i64::from(granted == 1), 0),
            "round {round}: the slot row each order leaves, no waiter row"
        );
        assert_eq!(count(&w, waiter, "rate_limit.returned").await, 0);
        assert_eq!(
            w.run_status(waiter).await,
            sagat_domain::execution::RunStatus::Cancelled,
            "round {round}: the run cancelled either way"
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
    // the holder's counted instants moved ahead while the three are admitted, so they queue whatever the runner's speed
    // (the runtime-role run of 2026-11-25 admitted one of them a second after the holder's grant), and moved back
    move_grant_instants(&w, holder, ADMISSION_MARGIN_US).await;
    for _ in 0..3 {
        admitted.push(w.start(workflow, &input).await);
    }
    assert_eq!(counted_waiters(&w).await, 3);
    move_grant_instants(&w, holder, -ADMISSION_MARGIN_US).await;
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
/// restarted one. The first ten are granted at admission and the next four buffered; the ten grants then age out on the
/// database clock, and the fifteenth start, which finds room in the window, queues behind the four (strict admission
/// order, decision D022-073), as does every later start until the key's default ceiling of 100 waiters; the rest are
/// refused `RATE_LIMIT_QUEUE_FULL` with nothing of them created: exactly 10 granted at admission, 100 buffered, 890
/// refused, whatever the runner's speed. The ten grants' counted instants are moved ahead while the next four are
/// admitted, so that the four find the window full however long the ten took (the run of 2026-11-24 took 3.3 s for the
/// first ten on a loaded runner and granted the eleventh at its admission), and moved back before the window ages out
/// (decision D022-072). No one-second interval of the journals holds more than 10 grants; grants follow admission order;
/// each admitted run is granted once.
#[tokio::test]
async fn a_flood_of_a_thousand_starts_on_one_key_never_holds_more_than_its_limit_in_a_window() {
    const MARGIN_US: i64 = 600 * SECOND_US;
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-flood", 10, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let mut admitted = Vec::new();
    let mut refused = 0_usize;
    for i in 0..1_000 {
        if i == 10 {
            assert_eq!(
                start_grants(&w, &admitted).await.len(),
                10,
                "the first ten are granted at admission"
            );
            for run in &admitted {
                move_grant_instants(&w, *run, MARGIN_US).await;
            }
        }
        if i == 14 {
            for run in &admitted[..10] {
                move_grant_instants(&w, *run, -MARGIN_US).await;
            }
            let first_ten = start_grants(&w, &admitted[..10]).await;
            let last = first_ten.iter().map(|(_, at)| *at).max().expect("ten");
            until_past(&w.db.pool, last + SECOND_US).await;
        }
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
    assert_eq!(
        start_grants(&w, &admitted).await.len(),
        10,
        "only the first ten are granted at admission: the window had room for the fifteenth, which queued behind four"
    );
    assert_eq!(
        count(&w, admitted[14], "rate_limit.queued").await,
        1,
        "the fifteenth start queues although its window has room"
    );
    assert_eq!((admitted.len(), refused), (110, 890));
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
    let out_of_order: Vec<(usize, i64, i64)> = in_order
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] > pair[1])
        .map(|(i, pair)| (i, pair[0], pair[1]))
        .collect();
    assert!(
        out_of_order.is_empty(),
        "grants follow admission order: (index, its grant, the next one's) {out_of_order:?}"
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
    // the holders' counted instants are moved one minute ahead while the two starts are admitted, so the two queue at a
    // full window whatever the runner's speed (decision D022-072; re-review minor 6), and moved back before the drain
    const MOVED_US: i64 = 60 * SECOND_US;
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-lowered", 3, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(w.start(workflow, &input).await);
    }
    for run in &held {
        assert_eq!(start_rows(&w, *run).await, (1, 0));
        move_grant_instants(&w, *run, MOVED_US).await;
    }
    let mut waiting = Vec::new();
    for _ in 0..2 {
        waiting.push(w.start(workflow, &input).await);
    }
    for run in &waiting {
        assert_eq!(
            start_rows(&w, *run).await,
            (0, 1),
            "the window counts the three holders"
        );
    }
    for run in &held {
        move_grant_instants(&w, *run, -MOVED_US).await;
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
    // the grant records the limit it was decided with, in its row and in its journal (minor 3 of the review)
    for run in &waiting {
        let decided: (i32, i64) = sqlx::query_as(
            "SELECT (SELECT declared_limit FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                    (SELECT (detail->>'limit')::bigint FROM run_events \
                      WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node IS NULL)",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the grant");
        assert_eq!(decided, (1, 1), "the limit the grant was decided with");
    }
    for run in runs {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// D022-009 and decision D022-075 item 4 (no head-of-line blocking across window lengths): one key shared by a start of
/// `limit: 1, per: hour` and one of `limit: 1, per: second` (warned `RATE_LIMIT_MISMATCH` at publication). The hour's
/// holder counts for both; the hour's waiter `wa`, behind it, is refused by its own limit for an hour; the second's waiter
/// `wb`, behind `wa`, counts the key's grants over its own second. The holder's instants are moved two seconds ahead
/// first, so that `wb` finds its own window full whatever the runner's speed. Once that slot is past one second, a pass
/// grants `wb` and not `wa`: `wa` is not ahead of it, since its own window still counts the holder. Counting the waiters
/// ahead over `wb`'s window would hold `wb` back for the hour.
#[tokio::test]
async fn a_waiter_of_a_longer_window_refused_by_its_own_limit_does_not_hold_back_a_shorter_one() {
    const MOVED_US: i64 = 2 * SECOND_US;
    let w = gated_world().await;
    let hourly = publish(&w, rated_start("fault-rate-mixed-hour", 1, "hour")).await;
    let secondly = publish(&w, rated_start("fault-rate-mixed-second", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(hourly, &input).await;
    let wa = w.start(hourly, &input).await;
    let moved = granted_us(&w, holder).await + MOVED_US;
    move_grant_instants(&w, holder, MOVED_US).await;
    let wb = w.start(secondly, &input).await;
    assert_eq!(start_rows(&w, wa).await, (0, 1));
    assert_eq!(
        start_rows(&w, wb).await,
        (0, 1),
        "the second's window counts the holder: {:?}",
        w.journal(wb).await
    );
    until_past(&w.db.pool, moved + SECOND_US).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(
        start_rows(&w, wb).await,
        (1, 0),
        "the shorter window is granted"
    );
    assert_eq!(
        start_rows(&w, wa).await,
        (0, 1),
        "the hour's waiter waits for its own window"
    );
    move_grant_instants(&w, holder, -MOVED_US).await;
    for run in [holder, wa, wb] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-073 item 2 (the pass grants in admission order): every decision of one pass counts the key's grants as
/// of the pass's selection of the key, so a grant that ages out while the pass runs never lets a younger waiter be granted
/// before an older one the same pass refused. With `limit: 2, per: second`, two slots counted, an older waiter `x` (rate
/// only) and a younger `y` that also waits for a permit with room: the pass (one key at a time) refuses `x`, and `y`'s
/// decision is held on its permit's key, held by the test. Meanwhile both counted slots age out: their instants are set
/// to one window before the database clock read after the pass blocked (decision D022-072), which is after the pass's
/// selection, so they count as of the selection and not as of `y`'s own instant. Counted at its own instant, `y` would
/// find room for itself and `x` ahead and be granted first; counted as of the selection, it is refused, and the next pass
/// grants `x` and then `y`. No step depends on real elapsed time: the holders' instants are moved ahead while `x` and `y`
/// are admitted and while the pass selects.
#[tokio::test]
async fn a_grant_that_ages_out_during_a_pass_never_puts_a_younger_waiter_first() {
    let w = gated_world().await;
    let rated = publish(&w, rated_start("fault-rate-pass-order", 2, "second")).await;
    let mut dual = rated_start("fault-rate-pass-order-dual", 2, "second");
    dual["concurrency"] = json!({"group": "serial", "key": "input.accountId", "limit": 5});
    let dual = publish(&w, dual).await;
    let input = json!({"accountId": "acct-1"});
    let holders = [w.start(rated, &input).await, w.start(rated, &input).await];
    let mut granted_at = Vec::new();
    for holder in holders {
        granted_at.push(granted_us(&w, holder).await);
        move_grant_instants(&w, holder, ADMISSION_MARGIN_US).await;
    }
    let x = w.start(rated, &input).await;
    let y = w.start(dual, &input).await;
    for run in [x, y] {
        assert_eq!(start_rows(&w, run).await, (0, 1), "the window is full");
    }
    // the key is due now (a start queued at a full window rests it to the moved slots' age-out)
    keys_due(&w).await;
    let mut permits = w.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 FOR UPDATE")
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&mut *permits)
        .await
        .expect("the permit's key held");
    let one_at_a_time = SchedulerSettings {
        concurrency: 1,
        ..SchedulerSettings::default()
    };
    let grants = scheduler(&w, one_at_a_time, None);
    let pass = tokio::spawn(async move { grants.grant_gates().await });
    barrier(&w, 1).await;
    // the pass selected the key before this instant; the slots now aged out as of any later one
    let aged = database_now_us(&w.db.pool).await - SECOND_US + 1;
    for holder in holders {
        set_grant_instants(&w, holder, aged).await;
    }
    permits.rollback().await.expect("released");
    let report = pass.await.expect("the pass");
    assert_eq!(
        report.gates_granted, 0,
        "counted as of the selection, neither is granted by this pass: {report:?}"
    );
    // the holders' rows get their journal's instants back, and the next pass runs once those have aged on the database
    // clock, so that the journals, which the consistency check reads, never show three grants in one window (the wait
    // decides nothing above: whatever its length, the outcome is the same)
    for (holder, at) in holders.into_iter().zip(&granted_at) {
        set_grant_instants(&w, holder, *at).await;
    }
    until_past(&w.db.pool, granted_at[1] + SECOND_US).await;
    keys_due(&w).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 2, "{report:?}");
    let (gx, gy) = (granted_us(&w, x).await, granted_us(&w, y).await);
    assert!(gx < gy, "admission order: x at {gx}, y at {gy}");
    for run in holders.iter().copied().chain([x, y]) {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-076 item 1 (re-review Major 1; ADR-0089 decision 3, "never late"): a start queued behind an eligible
/// waiter while a pass works on its key is granted by the next pass. With `limit: 4, per: hour`, one slot counted, and
/// `w` waiting for both its slot (with room) and a permit that was full at its admission and has since been freed, the
/// pass (one key at a time, the window first by its hint) lists `w` and holds on `w`'s permit key, which the test holds;
/// a rate-only start `n` then queues behind `w` with room. The pass grants `w` and rests the window: the rest would set
/// its hint an hour ahead, past `n`'s queue timeout, had `n` not written the hint to its own instant.
#[tokio::test]
async fn a_start_queued_while_a_pass_works_on_its_key_is_granted_by_the_next_pass() {
    let w = gated_world().await;
    let rated = publish(&w, rated_start("fault-rate-pass-wakeup", 4, "hour")).await;
    let mut dual = rated_start("fault-rate-pass-wakeup-dual", 4, "hour");
    dual["concurrency"] = json!({"group": "serial", "key": "input.accountId", "limit": 1});
    let dual = publish(&w, dual).await;
    let input = json!({"accountId": "acct-1"});
    let counted = w.start(rated, &input).await;
    let permit = w.start(dual, &input).await;
    assert_eq!(start_rows(&w, permit).await, (1, 0), "it holds both");
    let first = w.start(dual, &input).await;
    assert_eq!(start_rows(&w, first).await, (0, 1), "its permit is full");
    w.operate_run(permit, RunOperation::Cancel).await;
    let mut permits = w.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 FOR UPDATE")
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&mut *permits)
        .await
        .expect("the permit's key held");
    let one_at_a_time = SchedulerSettings {
        concurrency: 1,
        ..SchedulerSettings::default()
    };
    let grants = scheduler(&w, one_at_a_time, None);
    let pass = tokio::spawn(async move { grants.grant_gates().await });
    let pids = barrier(&w, 1).await;
    let late = w.start(rated, &input).await;
    assert_eq!(
        start_rows(&w, late).await,
        (0, 1),
        "an eligible waiter is ahead: it queues although its window has room"
    );
    assert_eq!(
        lock_waiters(&w).await,
        pids,
        "the start was decided while the pass still waits"
    );
    permits.rollback().await.expect("released");
    let report = pass.await.expect("the pass");
    assert_eq!(
        report.gates_granted, 1,
        "the pass grants `first`: {report:?}"
    );
    assert_eq!(start_rows(&w, first).await, (1, 0));
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 1,
        "the next pass grants the start queued during the first: {report:?}"
    );
    assert_eq!(start_rows(&w, late).await, (1, 0));
    for run in [counted, permit, first, late] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-076 item 2 (re-review Major 2): on a key shared by an hour's and a second's limit, a start queued at a
/// full window lowers the key's hint to the first instant a counted grant ages out of its own window. The hour's holder
/// `ha` and its waiter `wa` behind it: a pass rests the key an hour ahead. Once `ha`'s slot is past one second on the
/// database clock (only the hour counts it then), the second's holder `hb` is granted at once (`wa` is not eligible),
/// and the second's waiter `wb` queues at a full second (`hb`'s instants moved five seconds ahead, so whatever the
/// runner's speed): the hint drops to `hb`'s slot plus one second, not the hour, and the pass at that instant grants
/// `wb`.
#[tokio::test]
async fn a_start_queued_at_a_full_window_of_mixed_lengths_wakes_when_its_own_window_frees() {
    // `hb`'s slot moved this far ahead, so `wb` finds its second full whatever the runner's speed; the pass then waits for
    // the moved slot to age out on the database clock
    const AHEAD_US: i64 = 5 * SECOND_US;
    let w = gated_world().await;
    let hourly = publish(&w, rated_start("fault-rate-mixed-hint-hour", 1, "hour")).await;
    let secondly = publish(&w, rated_start("fault-rate-mixed-hint-second", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let ha = w.start(hourly, &input).await;
    let wa = w.start(hourly, &input).await;
    assert_eq!(start_rows(&w, wa).await, (0, 1));
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 0);
    let rested = window_hint(&w).await.expect("the pass rested the key");
    assert!(
        rested > database_now_us(&w.db.pool).await + 3_000 * SECOND_US,
        "the hour's waiter rests the key about an hour ahead: {rested}"
    );
    until_past(&w.db.pool, granted_us(&w, ha).await + SECOND_US).await;
    let hb = w.start(secondly, &input).await;
    assert_eq!(
        start_rows(&w, hb).await,
        (1, 0),
        "the hour's waiter is not eligible: no one is ahead"
    );
    move_grant_instants(&w, hb, AHEAD_US).await;
    let wb = w.start(secondly, &input).await;
    assert_eq!(start_rows(&w, wb).await, (0, 1), "its own second is full");
    let frees = granted_us(&w, hb).await + AHEAD_US + SECOND_US;
    let hint = window_hint(&w).await.expect("a hint");
    assert!(
        hint <= frees,
        "the hint is the instant the second's slot ages out, not the hour's: {hint} > {frees}"
    );
    until_past(&w.db.pool, hint).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(
        start_rows(&w, wb).await,
        (1, 0),
        "the second's waiter is granted"
    );
    assert_eq!(
        start_rows(&w, wa).await,
        (0, 1),
        "the hour's waits for its hour"
    );
    move_grant_instants(&w, hb, -AHEAD_US).await;
    for run in [ha, wa, hb, wb] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-075 item 4 at admission (re-review minor 3): a new arrival counts the waiters ahead of it each over its
/// own window. The hour's holder and the hour's waiter behind it, which its own limit refuses for the hour; once the
/// holder's slot is past one second on the database clock (the hour counts it, the second does not), a start of `limit:
/// 1, per: second` on the same key finds its second empty and no eligible waiter, and is granted at its admission. Counted over the arrival's second,
/// the hour's waiter would look eligible and the start would queue.
#[tokio::test]
async fn an_arrival_counts_a_waiter_ahead_over_its_own_window() {
    let w = gated_world().await;
    let hourly = publish(&w, rated_start("fault-rate-mixed-arrival-hour", 1, "hour")).await;
    let secondly = publish(
        &w,
        rated_start("fault-rate-mixed-arrival-second", 1, "second"),
    )
    .await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(hourly, &input).await;
    let wa = w.start(hourly, &input).await;
    assert_eq!(start_rows(&w, wa).await, (0, 1));
    until_past(&w.db.pool, granted_us(&w, holder).await + SECOND_US).await;
    let arrival = w.start(secondly, &input).await;
    assert_eq!(
        start_rows(&w, arrival).await,
        (1, 0),
        "the hour's waiter is not ahead of a second's arrival: {:?}",
        w.journal(arrival).await
    );
    for run in [holder, wa, arrival] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// A one-step workflow `name` whose step `g` takes a slot of `input.accountId` (`limit` per hour) and, with `permit`, a
/// permit of that group and key with room for five.
fn rated_hourly_step(name: &str, limit: u32, permit: Option<&str>) -> Value {
    let mut step = with(
        activity("g"),
        "rateLimit",
        json!({"group": "provider", "key": "input.accountId", "limit": limit, "per": "hour"}),
    );
    if let Some(group) = permit {
        step = with(
            step,
            "concurrency",
            json!({"group": group, "key": "input.accountId", "limit": 5}),
        );
    }
    document(name, "g", vec![step])
}

/// Decision D022-078 item 2 (re-review minor 1; FR-022-042): within one pass, a waiter the key refused for lack of room
/// holds back the younger waiters of no looser limit and window, so frees committed between two decisions of the pass
/// never put a younger waiter first. A step key of `limit: 2, per: hour` with two unleased slots granted; `x` and then
/// `y` queue behind them, each also waiting for a permit of its own group with room. The pass (one key a pass: the
/// window, whose hint is the oldest) refuses `x`; `y`'s decision waits on its permit's key, held by the test, while the
/// two slots are returned (their runs cancelled). Counted after the frees, `y` would find room for itself and `x` ahead
/// and be granted first; the pass grants nothing, and the next pass grants `x` and then `y` (each permit's pass, in the
/// order of their hints).
#[tokio::test]
async fn frees_between_two_decisions_of_a_pass_never_put_a_younger_waiter_first() {
    let w = gated_world().await;
    let rated = publish(&w, rated_hourly_step("fault-rate-pass-frees", 2, None)).await;
    let older = publish(
        &w,
        rated_hourly_step("fault-rate-pass-frees-x", 2, Some("serial-x")),
    )
    .await;
    let younger = publish(
        &w,
        rated_hourly_step("fault-rate-pass-frees-y", 2, Some("serial-y")),
    )
    .await;
    let input = json!({"accountId": "acct-1"});
    let holders = [w.start(rated, &input).await, w.start(rated, &input).await];
    for run in holders {
        assert_eq!(step_rows(&w, run, "g").await, (1, 0), "an unleased slot");
    }
    let x = w.start(older, &input).await;
    let y = w.start(younger, &input).await;
    for run in [x, y] {
        assert_eq!(step_rows(&w, run, "g").await, (0, 1), "the window is full");
    }
    let mut permits = w.db.pool.begin().await.expect("tx");
    sqlx::query(
        "SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 AND group_name = 'serial-y' FOR UPDATE",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&mut *permits)
    .await
    .expect("`y`'s permit key held");
    // one key a pass: the window, whose hint `x`'s queueing wrote before its permit's (a permit's pass would grant by
    // its other key, the exception of decision D022-073 item 2)
    let window_only = SchedulerSettings {
        concurrency: 1,
        batch: 1,
        ..SchedulerSettings::default()
    };
    let grants = scheduler(&w, window_only, None);
    let pass = tokio::spawn(async move { grants.grant_gates().await });
    barrier(&w, 1).await;
    for run in holders {
        w.operate_run(run, RunOperation::Cancel).await;
        assert_eq!(step_rows(&w, run, "g").await, (0, 0), "its slot returned");
    }
    permits.rollback().await.expect("released");
    let report = pass.await.expect("the pass");
    assert_eq!(
        report.gates_granted, 0,
        "the refusal of `x` holds `y` back in this pass: {report:?}"
    );
    // one key at a time, in the order of their hints: `x`'s permit (its queueing), `y`'s, then the window (the frees)
    let one_at_a_time = SchedulerSettings {
        concurrency: 1,
        ..SchedulerSettings::default()
    };
    let report = scheduler(&w, one_at_a_time, None).grant_gates().await;
    assert_eq!(report.gates_granted, 2, "{report:?}");
    let granted: Vec<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM rate_limit_grants WHERE run_id = ANY($1) AND limit_scope = 'STEP' \
          ORDER BY granted_at, run_id",
    )
    .bind(vec![x.as_uuid(), y.as_uuid()])
    .fetch_all(&w.db.pool)
    .await
    .expect("grants");
    assert_eq!(granted, [x.as_uuid(), y.as_uuid()], "admission order");
    for run in holders.iter().copied().chain([x, y]) {
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

/// Makes every rate key of the database with waiters due now, as the owner: a hint only says when a pass may look.
async fn keys_due(w: &World) {
    sqlx::query("UPDATE rate_limit_windows SET next_eligible_at = now() - interval '1 second' WHERE waiters > 0")
        .execute(&w.db.pool)
        .await
        .expect("due");
}

/// Sets the instants of `run`'s run-level grant row to `at_us` (microseconds since the epoch), as the owner, with the guard
/// switched off for this transaction only (decision D022-072); a lock it would wait for fails it within five seconds
/// instead of hanging the test.
async fn set_grant_instants(w: &World, run: RunId, at_us: i64) {
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await
        .expect("a bound on lock waits");
    sqlx::query("ALTER TABLE rate_limit_grants DISABLE TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard off in this transaction");
    sqlx::query(
        "UPDATE rate_limit_grants \
            SET granted_at = 'epoch'::timestamptz + $2 * interval '1 microsecond', \
                leased_at = 'epoch'::timestamptz + $2 * interval '1 microsecond' \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .bind(at_us)
    .execute(&mut *tx)
    .await
    .expect("the grant's instants set");
    sqlx::query("ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard on again");
    tx.commit().await.expect("set");
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

/// SC-022-023 (database clock): the database clock set back after a holder's grant at `t0`, with `limit: 1, per:
/// second`. Setting the clock back by `w` is, for the gate's counts, the holder's counted instants moved `w` ahead, so
/// they are moved five windows ahead (to `m = t0 + 5 s`) and a pass decides at once, while the database clock is still
/// below `m`: a grant counted in the future keeps counting, so it grants nothing (decision D022-075 item 2: a count that
/// left out instants after the decision's own would grant here). The waiter's grant is no earlier than `m + 1 s`, when the
/// holder's slot, as moved, ages out.
#[tokio::test]
async fn a_database_clock_set_back_grants_nothing_until_it_catches_up() {
    const MOVED_US: i64 = 5 * SECOND_US;
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-set-back", 1, "second")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    let moved = granted_us(&w, holder).await + MOVED_US;
    move_grant_instants(&w, holder, MOVED_US).await;
    // the key is due since the waiter queued (its hint is its queueing instant): the pass decides at once
    let before = database_now_us(&w.db.pool).await;
    assert!(
        before < moved,
        "the pass decides while the moved grant is still ahead of the clock: {before} >= {moved}"
    );
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 0,
        "a grant counted in the future keeps counting: {report:?}"
    );
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    drain(&w, || w.scheduler.grant_gates()).await;
    let granted = granted_us(&w, waiter).await;
    assert!(
        granted >= moved + SECOND_US,
        "no grant before the clock catches up: {granted} < {moved} + 1 s"
    );
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 1);
    move_grant_instants(&w, holder, -MOVED_US).await;
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Task T022-115, the second clause of SC-022-017 (decision D022-075 item 7): (1) a second key whose own buffer is
/// saturated at the same time, its grants coming from the pass, is served at its own limit: with `limit: 1, per:
/// second` on `sat-a` and `limit: 2, per: second` on `sat-b`, both windows full and three starts buffered behind `sat-b`,
/// one pass grants `sat-a` one and `sat-b` exactly two, and the third waits; (2) with more keys and tenants holding
/// buffered starts than one pass visits (a batch of 4 keys; the first tenant has four keys, the second two), the first
/// pass grants its budget and serves the second tenant, and every eligible key of either tenant is granted within two
/// passes. Each holder's counted instants are moved ahead while its waiter is admitted (decision D022-072), so every
/// start queues at a full window whatever the runner's speed; the windows age out on the database clock.
#[tokio::test]
async fn a_second_saturated_key_is_served_at_its_own_limit_and_every_tenant_within_two_passes() {
    let w = gated_world().await;
    // (1) two saturated keys of their own limits
    let one = publish(&w, rated_start("fault-rate-sat-one", 1, "second")).await;
    let two = publish(&w, rated_start("fault-rate-sat-two", 2, "second")).await;
    let (holder_a, waiter_a) = admitted_behind(&w, one, "sat-a", false).await;
    let input = json!({"accountId": "sat-b"});
    let holders_b = [w.start(two, &input).await, w.start(two, &input).await];
    for holder in holders_b {
        move_grant_instants(&w, holder, ADMISSION_MARGIN_US).await;
    }
    let mut waiters_b = Vec::new();
    for _ in 0..3 {
        let waiter = w.start(two, &input).await;
        assert_eq!(start_rows(&w, waiter).await, (0, 1), "sat-b is full");
        waiters_b.push(waiter);
    }
    for holder in holders_b {
        move_grant_instants(&w, holder, -ADMISSION_MARGIN_US).await;
    }
    let mut latest = granted_us(&w, holder_a).await;
    for holder in holders_b {
        latest = latest.max(granted_us(&w, holder).await);
    }
    until_past(&w.db.pool, latest + SECOND_US).await;
    // the keys are due now: a start queued at a full window rests its key to the moved slots' age-out
    keys_due(&w).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 3,
        "one of sat-a, two of sat-b: {report:?}"
    );
    assert_eq!(start_rows(&w, waiter_a).await, (1, 0));
    assert_eq!(start_rows(&w, waiters_b[0]).await, (1, 0));
    assert_eq!(start_rows(&w, waiters_b[1]).await, (1, 0));
    assert_eq!(
        start_rows(&w, waiters_b[2]).await,
        (0, 1),
        "sat-b is served at its own limit"
    );
    let mut b_grants: Vec<i64> = Vec::new();
    for waiter in &waiters_b[..2] {
        b_grants.push(granted_us(&w, *waiter).await);
    }
    at_most_per_window(&b_grants, 2, SECOND_US);
    // the third leaves, so that the passes below count only the keys of their own clause
    w.operate_run(waiters_b[2], sagat_application::runs::RunOperation::Cancel)
        .await;

    // (2) more keys and tenants than one pass visits
    let doc = rated_start("fault-rate-tenants", 1, "second");
    let mine = publish(&w, doc.clone()).await;
    let theirs = crate::support::flow_control::publish_as(&w, &w.other_admin, doc).await;
    let mut waiters = Vec::new();
    for (other, keys) in [(false, 4), (true, 2)] {
        for key in 0..keys {
            let workflow = if other { theirs } else { mine };
            let account = format!("tenant-{}-key-{key}", u8::from(other));
            let (holder, waiter) = admitted_behind(&w, workflow, &account, other).await;
            waiters.push((other, holder, waiter));
        }
    }
    let mut latest = 0;
    for (_, holder, _) in &waiters {
        latest = latest.max(granted_us(&w, *holder).await);
    }
    until_past(&w.db.pool, latest + SECOND_US).await;
    keys_due(&w).await;
    let narrow = scheduler(
        &w,
        SchedulerSettings {
            batch: 4,
            ..SchedulerSettings::default()
        },
        None,
    );
    let first = narrow.grant_gates().await;
    assert_eq!(
        first.gates_granted, 4,
        "one pass's budget of keys: {first:?}"
    );
    let mut other_served = 0;
    for (other, _, waiter) in &waiters {
        if *other && start_rows(&w, *waiter).await.0 == 1 {
            other_served += 1;
        }
    }
    assert!(
        other_served >= 1,
        "the second tenant is served in the first pass"
    );
    let second = narrow.grant_gates().await;
    assert_eq!(second.gates_granted, 2, "{second:?}");
    for (other, holder, waiter) in &waiters {
        assert_eq!(
            start_rows(&w, *waiter).await,
            (1, 0),
            "every eligible key granted within two passes"
        );
        let grants: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'rate_limit.granted'",
        )
        .bind(waiter.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("grants");
        assert_eq!(grants, 1, "granted once");
        // the second tenant's runs are checked by the key-wide check below (the world reads the first tenant's)
        if !*other {
            w.assert_consistent(*holder).await;
            w.assert_consistent(*waiter).await;
        }
    }
    for run in [holder_a, waiter_a]
        .into_iter()
        .chain(holders_b)
        .chain(waiters_b)
    {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-077 item 1 at its call site: a grant pass visits the scheduler's batch of keys, never more than 256
/// (`KEYS_PER_GRANT_PASS`). With a batch of 300 and 257 rate keys due (each with a holder and a start buffered behind
/// it at `limit: 1, per: hour`, every window's hint set to a second ago as the owner), one pass visits exactly 256: it
/// grants nothing (each window is full) and rests every key it visited to its age-out instant, so one key is left due;
/// the next pass visits that one.
#[tokio::test]
async fn a_grant_pass_visits_at_most_256_keys_whatever_its_batch() {
    const KEYS: usize = 257;
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("fault-rate-wide-pass", 1, "hour")).await;
    for key in 0..KEYS {
        let input = json!({"accountId": format!("wide-{key}")});
        w.start(workflow, &input).await;
        let waiter = w.start(workflow, &input).await;
        assert_eq!(start_rows(&w, waiter).await, (0, 1));
    }
    sqlx::query("UPDATE rate_limit_windows SET next_eligible_at = now() - interval '1 second'")
        .execute(&w.db.pool)
        .await
        .expect("every key due");
    let due = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM rate_limit_windows WHERE waiters > 0 AND next_eligible_at <= now()",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("due keys")
    };
    assert_eq!(due().await, i64::try_from(KEYS).expect("keys"));
    let wide = scheduler(
        &w,
        SchedulerSettings {
            batch: 300,
            ..SchedulerSettings::default()
        },
        None,
    );
    let first = wide.grant_gates().await;
    assert_eq!(first.gates_granted, 0, "{first:?}");
    assert_eq!(
        due().await,
        1,
        "one pass visits 256 keys, not the batch of 300"
    );
    let second = wide.grant_gates().await;
    assert_eq!(second.gates_granted, 0, "{second:?}");
    assert_eq!(due().await, 0, "the next pass visits the one left");
}
