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

/// Gap G022-013 (task T022-110), the takeover raced by a real keeper: alpha's keeper holds every shard with a lease of
/// one second; its grant pass takes the queued start's run and its key's home-shard fence and waits for the key, which
/// the test holds. Alpha's lease then runs out on the database clock, and beta's keeper takes over every shard it may:
/// the fenced one is skipped (`FOR UPDATE SKIP LOCKED` against the grant's `FOR SHARE`), so beta's grant pass cannot
/// serve that key while alpha's grant is in flight. The key released, alpha's grant commits once; beta's next round
/// takes the last shard, and its pass grants nothing again.
#[tokio::test]
async fn a_real_takeover_skips_the_shard_of_a_grant_in_flight_and_nothing_is_granted_twice() {
    let (w, workflow, holder) = held_key().await;
    let session = w.session(&["act.g"]).await;
    let alpha = keeper(&w, "alpha", std::time::Duration::from_secs(1));
    alpha.round().await.expect("alpha");
    assert_eq!(alpha.placement().owned(), ShardSet::all());
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
    let home: i16 = sqlx::query_scalar("SELECT key_shard FROM concurrency_semaphores")
        .fetch_one(&w.db.pool)
        .await
        .expect("the key's home shard");
    let home =
        sagat_domain::sharding::Shard::new(u8::try_from(home).expect("a shard")).expect("a shard");
    let key = hold_keys(&w).await;
    let grant = tokio::spawn(async move { old_owner.grant_gates().await });
    barrier(&w, 1).await;
    // alpha's leases (its shards' and its membership's) run out on the database clock; it does not renew them
    let expiry: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM greatest( \
                    (SELECT max(expires_at) FROM scheduler_shards WHERE owner = 'alpha'), \
                    (SELECT max(expires_at) FROM scheduler_members WHERE owner = 'alpha'))) * 1000000)::bigint",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("alpha's leases");
    crate::support::flow_control::until_past(&w.db.pool, expiry).await;
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    let taken = beta.placement().owned();
    assert!(
        !taken.contains(home) && taken.len() == ShardSet::all().len() - 1,
        "beta takes every shard but the one a grant in flight fences"
    );
    assert_eq!(
        scheduler(
            &w,
            SchedulerSettings::default(),
            Some(beta.placement().clone())
        )
        .grant_gates()
        .await
        .gates_granted,
        0,
        "beta does not serve the fenced key"
    );
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "not granted yet");
    key.rollback().await.expect("released");
    let granted = grant.await.expect("grant");
    assert_eq!(
        granted.gates_granted, 1,
        "the grant in flight commits: {granted:?}"
    );
    beta.round().await.expect("beta again");
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

/// SC-022-004 at run level (gap G022-013, task T022-110): five same-key starts of `limit: 1`. The first holds the permit
/// and four queue; each time the holder ends, one pass grants the oldest waiter. After each step of the sequence (a start,
/// a holder's end, a pass) the key has at most one holder, and the permits are granted in admission order; what happens
/// inside a commit is the readiness and grant suites' (review of 2026-11-25, correctness nit).
#[tokio::test]
async fn five_same_key_starts_hold_at_most_one_permit_and_are_granted_in_admission_order() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("fault-permit-five")).await;
    let session = w.session(&["act.g"]).await;
    let mut runs = Vec::new();
    for _ in 0..5 {
        runs.push(w.start(workflow, &json!({"accountId": "acct-1"})).await);
    }
    assert_eq!(counters(&w).await, (1, 4));
    for (i, run) in runs.iter().enumerate() {
        assert_eq!(
            start_rows(&w, *run).await,
            (1, 0),
            "run {i} holds the permit"
        );
        assert!(counters(&w).await.0 <= 1, "at most one holder");
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, *run, "admission order");
        w.complete(&session, &task, &json!({})).await.expect("done");
        assert_eq!(counters(&w).await.0, 0, "released by its end");
        let report = w.scheduler.grant_gates().await;
        let expected = u32::from(i + 1 < runs.len());
        assert_eq!(
            report.gates_granted, expected,
            "one grant a release: {report:?}"
        );
    }
    let acquired: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM run_events WHERE event_type = 'concurrency.acquired' AND run_id = ANY($1) \
          ORDER BY occurred_at, seq",
    )
    .bind(runs.iter().map(|run| run.as_uuid()).collect::<Vec<_>>())
    .fetch_all(&w.db.pool)
    .await
    .expect("grants");
    assert_eq!(
        acquired,
        runs.iter().map(|run| run.as_uuid()).collect::<Vec<_>>(),
        "granted in admission order"
    );
    for run in runs {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// SC-022-004, a mixed-limit pair sharing a group (gap G022-013; decision D022-009, each acquirer applies its own limit
/// to the shared holders): `wide` declares `limit: 2` and `narrow` `limit: 1` in the group `accounts`, on one key. A
/// `wide` run holds; a `narrow` start queues (one holder reaches its limit of one); a second `wide` start is granted at
/// once (the narrow waiter, refused by its own limit, does not hold it back, D1196). The narrow waiter is granted only
/// once no run holds the key, and the key never has more than two holders.
#[tokio::test]
async fn a_mixed_limit_pair_sharing_a_group_applies_each_limit_to_the_shared_holders() {
    let w = gated_world().await;
    let mut wide = document("fault-permit-wide", "g", vec![activity("g")]);
    wide["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 2});
    let wide = publish(&w, wide).await;
    let narrow = publish(&w, serial_start("fault-permit-narrow")).await;
    let session = w.session(&["act.g"]).await;
    let input = json!({"accountId": "acct-1"});
    let first = w.start(wide, &input).await;
    let waiter = w.start(narrow, &input).await;
    assert_eq!(
        start_rows(&w, waiter).await,
        (0, 1),
        "one holder is narrow's limit"
    );
    let second = w.start(wide, &input).await;
    assert_eq!(
        start_rows(&w, second).await,
        (1, 0),
        "wide's limit has room and the narrow waiter is not eligible"
    );
    assert_eq!(counters(&w).await, (2, 1));
    let tasks = w.poll(&session, 8).await;
    let mut leased: Vec<_> = tasks.iter().map(|task| task.run_id).collect();
    leased.sort_by_key(|run| run.as_uuid());
    let mut holders = vec![first, second];
    holders.sort_by_key(|run| run.as_uuid());
    assert_eq!(leased, holders, "only the two holders run");
    for task in &tasks {
        assert_eq!(
            w.scheduler.grant_gates().await.gates_granted,
            0,
            "the narrow waiter waits while a run holds"
        );
        w.complete(&session, task, &json!({})).await.expect("done");
        assert!(counters(&w).await.0 <= 2);
    }
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(
        start_rows(&w, waiter).await,
        (1, 0),
        "granted once the key is free"
    );
    assert_eq!(count(&w, waiter, "concurrency.acquired").await, 1);
    for run in [first, second, waiter] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

// ------------------------------------------------------------------------------------------- held signals

/// A workflow `name` whose entry waits for `payment.confirmed` (then runs `notify`) and whose start takes a permit of
/// `input.accountId` with `limit: 1`: a Signal-with-Start whose start queues holds its signal (decision D022-087).
fn held_signal_waiter(name: &str) -> Value {
    let mut doc = document(
        name,
        "w",
        vec![
            crate::support::engine::with(
                crate::support::engine::wait_node(
                    "w",
                    "payment.confirmed",
                    "input.k",
                    300_000,
                    None,
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    );
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc
}

/// A Signal-with-Start of `workflow` for business identifier `id` on `account`.
async fn signal_with_start(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    id: &str,
    account: &str,
) -> sagat_application::runs::SignalStarted {
    use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
    let key = format!("fault-held-{}", uuid::Uuid::now_v7());
    let input = format!(r#"{{"k": "s", "accountId": "{account}"}}"#);
    w.runs
        .signal_with_start(
            &w.admin,
            &sagat_application::runs::StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: input.as_bytes(),
                correlation_id: None,
                deadline: None,
                key: &key,
            },
            sagat_application::runs::BusinessStart {
                id: BusinessId::new(id).expect("an identifier"),
                conflict: ConflictPolicy::UseExisting,
                reuse: ReusePolicy::AllowDuplicate,
                window: ReuseWindow::default(),
            },
            sagat_application::runs::SignalAtStart {
                name: "payment.confirmed",
                payload: br#"{"paid": true}"#,
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("a Signal-with-Start")
}

/// The held rows of `run`, and its audit records of a delivered and of a dropped signal (with the drop's reasons).
async fn held_outcome(w: &World, run: RunId) -> (i64, usize, Vec<String>) {
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM flow_control_held_signals WHERE run_id = $1")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("held rows");
    let audited = |action: &'static str| {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT reason FROM audit_records WHERE resource_id = $1 AND action = $2 ORDER BY created_at",
        )
        .bind(run.to_string())
        .bind(action)
        .fetch_all(&w.db.pool)
    };
    let delivered = audited("run.signalled").await.expect("audit").len();
    let dropped = audited("run.signal_dropped")
        .await
        .expect("audit")
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    (held, delivered, dropped)
}

/// Batch 5 correctness nit and the re-read's minor 2 (decision D022-125): the commit of a grant that delivered a held
/// signal fails (a deferred constraint trigger refuses the delivery's `run.signalled` audit record at commit, as the
/// content quota's deferred flush would). The failure is the delivery's: the grant is tried once more without the
/// signal, which is dropped with its audit record (`DELIVERY_FAILED`), and the start is granted in the same pass.
/// Before, a commit failure was the grant's own, so the start failed on every pass with its signal still held.
#[tokio::test]
async fn a_commit_that_fails_after_the_delivery_of_a_held_signal_grants_the_start_without_it() {
    let w = gated_world().await;
    let workflow = publish(&w, held_signal_waiter("fault-held-commit")).await;
    let holder = signal_with_start(&w, workflow, "cart-1", "acct-1").await;
    let queued = signal_with_start(&w, workflow, "cart-2", "acct-1").await;
    assert!(queued.held, "{queued:?}");
    let run = queued.run.info.run_id;
    for statement in [
        format!(
            "CREATE FUNCTION test_refuse_signalled_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.resource_id = '{run}' AND NEW.action = 'run.signalled' THEN \
                 RAISE EXCEPTION 'the test refuses this delivery at commit'; END IF; RETURN NULL; END $$"
        ),
        "CREATE CONSTRAINT TRIGGER test_refuse_signalled_commit AFTER INSERT ON audit_records \
           DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_refuse_signalled_commit()"
            .to_owned(),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&w.db.pool)
            .await
            .expect("the trigger that refuses the commit");
    }
    w.operate_run(
        holder.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    let report = w.scheduler.grant_gates().await;
    for statement in [
        "DROP TRIGGER test_refuse_signalled_commit ON audit_records",
        "DROP FUNCTION test_refuse_signalled_commit()",
    ] {
        sqlx::query(statement)
            .execute(&w.db.pool)
            .await
            .expect("the trigger dropped");
    }
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_rows(&w, run).await, (1, 0), "granted");
    let (held, delivered, dropped) = held_outcome(&w, run).await;
    assert_eq!((held, delivered), (0, 0), "nothing delivered, nothing held");
    assert!(
        dropped.len() == 1 && dropped[0].starts_with("DELIVERY_FAILED"),
        "{dropped:?}"
    );
    assert_eq!(
        w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Waiting),
        "the start runs without the signal"
    );
    w.assert_consistent(run).await;
}

/// Batch 5 correctness nit (decision D022-130 item 7): the grant and the timeout of a queued start that holds a signal,
/// in the four orders of `a_grant_and_a_timeout_of_a_queued_start_are_decided_by_the_first_commit`, held by key locks and
/// observed through `pg_stat_activity`. The first commit decides the signal too: the grant delivers it once
/// (`run.signalled`) or the timeout drops it once (`run.signal_dropped`, `RUN_ENDED`), never both, never neither, and no
/// held row is left behind.
#[tokio::test]
async fn a_grant_and_a_timeout_of_a_queued_start_with_a_held_signal_deliver_or_drop_it_once() {
    let w = gated_world().await;
    let workflow = publish(&w, held_signal_waiter("fault-held-race")).await;
    let session = w.session(&["act.notify"]).await;
    for round in 0..8 {
        let account = format!("held-race-{round}");
        let holder = signal_with_start(&w, workflow, &format!("hold-{round}"), &account).await;
        let queued = signal_with_start(&w, workflow, &format!("wait-{round}"), &account).await;
        assert!(queued.held, "round {round}: {queued:?}");
        let waiter = queued.run.info.run_id;
        let holder_task = w.one_task(&session).await;
        assert_eq!(holder_task.run_id, holder.run.info.run_id, "round {round}");
        w.complete(&session, &holder_task, &json!({}))
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
                crate::support::flow_control::until_past(&w.db.pool, deadline).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                barrier(&w, 2).await;
                key.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 0, "round {round}: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
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
                assert_eq!(g.gates_granted, 0, "round {round}: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
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
                assert_eq!(g.gates_granted, 0, "round {round}: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
            }
            _ => {
                assert_eq!(
                    w.scheduler.grant_gates().await.gates_granted,
                    1,
                    "round {round}"
                );
                age_start_deadline(&w, waiter).await;
                assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 0);
            }
        }
        let (held, delivered, dropped) = held_outcome(&w, waiter).await;
        assert_eq!(held, 0, "round {round}: no held row is left");
        assert_eq!(
            delivered + dropped.len(),
            1,
            "round {round}: delivered or dropped, once: {delivered} {dropped:?}"
        );
        assert_eq!(delivered == 1, round % 4 == 2, "round {round}");
        assert!(
            dropped.iter().all(|reason| reason.starts_with("RUN_ENDED")),
            "round {round}: {dropped:?}"
        );
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted + report.gate_waits_timed_out,
            0,
            "round {round}: decided once: {report:?}"
        );
        w.assert_consistent(waiter).await;
        if delivered == 1 {
            let task = w.one_task(&session).await;
            assert_eq!(task.run_id, waiter, "round {round}");
            w.complete(&session, &task, &json!({})).await.expect("done");
        }
    }
    assert_eq!(counters(&w).await, (0, 0), "every key drained");
}
