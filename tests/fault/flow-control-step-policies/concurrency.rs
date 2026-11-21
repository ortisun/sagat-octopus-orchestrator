//! Faults of a run's permit (T022-028; SC-022-003, SC-022-004; ADR-0056 evidence): a process that dies inside the
//! admission (T10), the grant (T11), the timeout (T12) or the release (T14, at the run's end) leaves nothing half done and
//! the next attempt decides once; a grant and a timeout of one queued start are decided by the first commit (20 rounds,
//! three orders); a takeover of the key's shard waits for a grant in flight, so no permit is granted twice; a lost task
//! hint delays nothing the next poll would not find; a restarted process (a new store and scheduler over the same
//! database) goes on with the queue.
//!
//! A crash is the termination of the deciding transaction's backend while it waits for a key lock the test holds, after
//! it has written what it writes before that lock: PostgreSQL rolls the transaction back, as it does when the process
//! dies. Every race is held by locks and observed through `pg_stat_activity`, never by sleeps.

use std::sync::Arc;

use sagat_application::execution::{DefinitionCache, Engine};
use sagat_domain::execution::RunId;
use sagat_domain::sharding::ShardSet;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

use crate::support::engine::{World, activity, document};
use crate::support::flow_control::{gate, gated_world, publish};
use crate::support::scale::{TTL, keeper, scheduler};

/// A one-step workflow `name` whose start takes a permit of `input.accountId` with `limit: 1`.
fn serial_start(name: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc
}

/// The run's start rows: (holders, waiters).
async fn start_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND holder_scope = 'RUN'), \
                (SELECT count(*) FROM concurrency_waiters WHERE run_id = $1 AND permit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// The tenant's key counters (holders, waiters), summed over its keys.
async fn counters(w: &World) -> (i64, i64) {
    sqlx::query_as(
        "SELECT COALESCE(sum(holders), 0)::bigint, COALESCE(sum(waiters), 0)::bigint FROM concurrency_semaphores \
          WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("counters")
}

async fn count(w: &World, run: RunId, event: &str) -> usize {
    w.journal(run).await.iter().filter(|e| *e == event).count()
}

/// Holds every key row of the tenant, as a deciding transaction holds its key.
async fn hold_keys(w: &World) -> Transaction<'static, Postgres> {
    let mut key = w.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 FOR UPDATE")
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&mut *key)
        .await
        .expect("the keys held");
    key
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

/// Replaces a queued start's waiter row by one past its deadline, and its projection with it, with one instant.
async fn age_start_deadline(w: &World, run: RunId) {
    crate::support::flow_control::age_start_deadline(w, run).await;
}

/// A world with a gated workflow, a worker session and a holder run of `acct-1` whose step is leased by the session.
async fn held_key() -> (World, sagat_domain::definition::WorkflowId, RunId) {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("fault-permit")).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0));
    (w, workflow, holder)
}

/// T10: an admission that dies after its run's row, while it waits for its key, leaves no run, no row and no count; the
/// retried request (same idempotency key) is admitted once, queued, and a second retry answers the same run.
#[tokio::test]
async fn a_crash_inside_the_admission_leaves_nothing_and_the_retry_queues_once() {
    let (w, workflow, holder) = held_key().await;
    let input = json!({"accountId": "acct-1"});
    let runs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id = $1")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    let key = hold_keys(&w).await;
    let (crashed, ()) = tokio::join!(
        w.start_with(workflow, &input, "fault-admission", None),
        crash_the_waiting_transaction(&w)
    );
    assert!(crashed.is_err(), "the admission died: {crashed:?}");
    key.rollback().await.expect("released");
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id = $1")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    assert_eq!(runs_after, runs_before, "no run of the dead admission");
    assert_eq!(counters(&w).await, (1, 0), "no count moved");

    let waiter = w
        .start_with(workflow, &input, "fault-admission", None)
        .await
        .expect("the retry is admitted");
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    assert_eq!(count(&w, waiter, "concurrency.queued").await, 1);
    let again = w
        .start_with(workflow, &input, "fault-admission", None)
        .await
        .expect("the retry replays");
    assert_eq!(again, waiter, "one run for the request");
    assert_eq!(counters(&w).await, (1, 1));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// T11: a grant that dies while it waits for the freed key leaves the start queued with its row and count; the next
/// pass grants it once.
#[tokio::test]
async fn a_crash_inside_the_grant_leaves_the_start_queued_and_the_next_pass_grants_it_once() {
    let (w, workflow, holder) = held_key().await;
    let session = w.session(&["act.g"]).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the holder ends");
    assert_eq!(counters(&w).await, (0, 1));
    let key = hold_keys(&w).await;
    let grants = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(grants.grant_gates(), crash_the_waiting_transaction(&w));
    assert_eq!(report.gates_granted, 0, "{report:?}");
    key.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still queued");
    assert_eq!(count(&w, waiter, "concurrency.acquired").await, 0);
    assert_eq!(counters(&w).await, (0, 1));

    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    assert_eq!(count(&w, waiter, "concurrency.acquired").await, 1);
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 0, "once");
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// T12: a timeout that dies while it waits for the key leaves the start queued; the next pass times it out once,
/// through `concurrency.wait_timed_out`, and frees its row and count.
#[tokio::test]
async fn a_crash_inside_the_timeout_leaves_the_start_queued_and_the_next_pass_times_it_out_once() {
    let (w, workflow, holder) = held_key().await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    age_start_deadline(&w, waiter).await;
    let key = hold_keys(&w).await;
    let timeouts = scheduler(&w, SchedulerSettings::default(), None);
    let (report, ()) = tokio::join!(timeouts.time_out_gates(), crash_the_waiting_transaction(&w));
    assert_eq!(report.gate_waits_timed_out, 0, "{report:?}");
    key.rollback().await.expect("released");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still queued");
    assert_eq!(count(&w, waiter, "concurrency.wait_timed_out").await, 0);

    assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 1);
    assert_eq!(count(&w, waiter, "concurrency.wait_timed_out").await, 1);
    assert_eq!(start_rows(&w, waiter).await, (0, 0));
    assert!(w.run_status(waiter).await.is_terminal());
    assert_eq!(counters(&w).await, (1, 0));
    assert_eq!(
        w.scheduler.time_out_gates().await.gate_waits_timed_out,
        0,
        "once"
    );
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// T14 (T8, the run's end), 20 rounds on one world (SC-022-003; keys review, correctness M2): a completion that ends the
/// holder and dies while it waits for the key releases nothing and ends nothing; the worker's retry of the same report
/// ends the run, releases the permit once, and the next pass grants the queued start, once. Each round uses a fresh
/// key on a world that keeps the history of the rounds before (a hint not re-armed after a rolled-back release, or a
/// counter drift after a crash on a key with history, would show in a later round); the keys drain to nothing.
#[tokio::test]
async fn a_crash_inside_the_release_ends_nothing_and_the_retried_report_releases_once() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("fault-permit")).await;
    let session = w.session(&["act.g"]).await;
    for round in 0..20 {
        let account = json!({"accountId": format!("release-{round}")});
        let holder = w.start(workflow, &account).await;
        let waiter = w.start(workflow, &account).await;
        assert_eq!(start_rows(&w, holder).await, (1, 0), "round {round}");
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, holder, "round {round}");
        let key = hold_keys(&w).await;
        let output = json!({});
        let (crashed, ()) = tokio::join!(
            w.complete(&session, &task, &output),
            crash_the_waiting_transaction(&w)
        );
        assert!(
            crashed.is_err(),
            "round {round}: the completion died: {crashed:?}"
        );
        key.rollback().await.expect("released");
        assert!(
            !w.run_status(holder).await.is_terminal(),
            "round {round}: nothing ended"
        );
        assert_eq!(
            start_rows(&w, holder).await,
            (1, 0),
            "round {round}: nothing released"
        );
        assert_eq!(
            count(&w, holder, "concurrency.released").await,
            0,
            "round {round}"
        );

        w.complete(&session, &task, &json!({}))
            .await
            .expect("the retried report");
        assert!(w.run_status(holder).await.is_terminal(), "round {round}");
        assert_eq!(
            count(&w, holder, "concurrency.released").await,
            1,
            "round {round}"
        );
        assert_eq!(start_rows(&w, holder).await, (0, 0), "round {round}");
        assert_eq!(
            w.scheduler.grant_gates().await.gates_granted,
            1,
            "round {round}: one grant"
        );
        assert_eq!(
            w.scheduler.grant_gates().await.gates_granted,
            0,
            "round {round}: once"
        );
        assert_eq!(start_rows(&w, waiter).await, (1, 0), "round {round}");
        assert_eq!(
            count(&w, waiter, "concurrency.acquired").await,
            1,
            "round {round}"
        );
        for run in [holder, waiter] {
            w.assert_consistent(run).await;
        }
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, waiter, "round {round}");
        w.complete(&session, &task, &json!({}))
            .await
            .expect("drained");
        assert_eq!(counters(&w).await, (0, 0), "round {round}: the key drained");
        w.assert_consistent(waiter).await;
    }
}

/// SC-022-003: a grant and a timeout of one queued start, 20 rounds in four orders: (0) the start due, the grant first
/// and then the timeout, both behind a held key (the grant pass does not list a due start and only rests the key);
/// (1) the timeout first; (2) the grant commits before the deadline; (3) the grant lists the start before its deadline,
/// holds the run and waits for the key, the deadline passes while it waits, and the timeout, which lists the start
/// due, waits for the run (keys review, correctness M1: only the grant's check of the deadline under the key's lock
/// refuses it). The first commit decides: exactly one of `concurrency.acquired` and `concurrency.wait_timed_out`, no
/// row left behind, and the next pass decides nothing.
#[tokio::test]
async fn a_grant_and_a_timeout_of_a_queued_start_are_decided_by_the_first_commit() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("fault-start-race")).await;
    let session = w.session(&["act.g"]).await;
    for round in 0..20 {
        let account = json!({"accountId": format!("race-{round}")});
        let holder = w.start(workflow, &account).await;
        let waiter = w.start(workflow, &account).await;
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, holder);
        w.complete(&session, &task, &json!({}))
            .await
            .expect("the key frees");
        let other = || scheduler(&w, SchedulerSettings::default(), None);
        match round % 4 {
            3 => {
                let deadline =
                    crate::support::flow_control::set_start_deadline(&w.db.pool, waiter, 2_000_000)
                        .await;
                let key = hold_keys(&w).await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 1).await;
                assert!(
                    crate::support::flow_control::database_now_us(&w.db.pool).await < deadline,
                    "round {round}: the grant listed the start before its deadline"
                );
                crate::support::flow_control::until_past(&w.db.pool, deadline).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                key.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(
                    g.gates_granted, 0,
                    "a start due under the key is never granted: {g:?}"
                );
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            0 => {
                age_start_deadline(&w, waiter).await;
                let key = hold_keys(&w).await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 1).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                key.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 0, "a due start is never granted: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            1 => {
                age_start_deadline(&w, waiter).await;
                let key = hold_keys(&w).await;
                let (grants, timeouts) = (other(), other());
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 1).await;
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                barrier(&w, 2).await;
                key.rollback().await.expect("released");
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
        let (acquired, timed_out) = (
            count(&w, waiter, "concurrency.acquired").await,
            count(&w, waiter, "concurrency.wait_timed_out").await,
        );
        assert_eq!(
            acquired + timed_out,
            1,
            "round {round}: exactly one outcome"
        );
        assert_eq!(timed_out == 1, round % 4 != 2, "round {round}");
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted + report.gate_waits_timed_out,
            0,
            "round {round}: decided once: {report:?}"
        );
        w.assert_consistent(waiter).await;
        if acquired == 1 {
            let task = w.one_task(&session).await;
            assert_eq!(task.run_id, waiter);
            w.complete(&session, &task, &json!({})).await.expect("done");
        }
    }
    assert_eq!(counters(&w).await, (0, 0), "every key drained");
}

/// SC-022-004 (takeover duplicate grant): a grant holds the key's home-shard row from its fence to its commit, so a new
/// owner's takeover waits for a grant in flight; the old owner's grant commits once and the new owner finds nothing to
/// grant again.
#[tokio::test]
async fn a_takeover_waits_for_a_grant_in_flight_and_nothing_is_granted_twice() {
    let (w, workflow, holder) = held_key().await;
    let session = w.session(&["act.g"]).await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let old_owner = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(alpha.placement().clone()),
    );
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the holder ends");
    let key = hold_keys(&w).await;
    let grant = tokio::spawn(async move { old_owner.grant_gates().await });
    barrier(&w, 1).await;
    // alpha's lease runs out while its grant waits for the key: the expiry waits for the grant's fence
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
    key.rollback().await.expect("released");
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
    assert_eq!(count(&w, waiter, "concurrency.acquired").await, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// A grant announces its step once (`task.ready`, one outbox row), and the step is found without the announcement: the
/// row is marked delivered as a broker that lost it would leave it, and the worker's poll of PostgreSQL leases the
/// granted start's step all the same. This world runs no broker, so the test proves the announcement's count and the
/// poll path, not a hint consumer (keys review, correctness m6).
#[tokio::test]
async fn a_grant_announces_its_step_once_and_a_poll_finds_it_without_the_announcement() {
    let (w, workflow, holder) = held_key().await;
    let session = w.session(&["act.g"]).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the holder ends");
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
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, waiter);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert!(w.run_status(waiter).await.is_terminal());
    assert_eq!(counters(&w).await, (0, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// A restart: the process that queued a start is gone, and a new one (a fresh store and scheduler over the same
/// database, no state carried over) grants it once its holder ends.
#[tokio::test]
async fn a_restarted_process_goes_on_with_the_queue() {
    let (w, workflow, holder) = held_key().await;
    let session = w.session(&["act.g"]).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the holder ends");
    let fresh = gate(PgRunStore::new(
        w.db.pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 64),
    ));
    let restarted = Scheduler::new(
        fresh,
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    );
    assert_eq!(restarted.grant_gates().await.gates_granted, 1);
    assert_eq!(start_rows(&w, waiter).await, (1, 0));
    assert_eq!(count(&w, waiter, "concurrency.acquired").await, 1);
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}
