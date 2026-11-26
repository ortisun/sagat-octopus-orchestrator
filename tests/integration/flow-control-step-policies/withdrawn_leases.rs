//! A withdrawn lease of a gated step (feature 006's `step.lease_withdrawn`, ADR-0098 decision 3; decisions D022-138 and
//! D022-139; ADR-0096 decision 6) against PostgreSQL: the step is granted and not leased again. It keeps its concurrency
//! permit and its rate slot, is leased again without queueing and without a second slot, and gives them back once, at the
//! end of its attempt or by a pause, a hold or a cancellation. The slot counts from the lease that reached a handler: the
//! withdrawal clears the lease instant the scheduler had copied, and the scheduler records the attempt's last lease. Every
//! run is checked by `verify_run` and the key rows by `assert_consistent`. Races are held by locks and observed through
//! `pg_stat_activity`, never by sleeps.

use std::sync::Arc;

use sagat_application::instances::{Requirement, Requirements, ServingGate};
use sagat_application::runs::RunOperation;
use sagat_application::worker::{UndeliveredLease, WithdrawalOutcome};
use sagat_domain::execution::{RunId, WithdrawalCause};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::readiness::{granted, queued, rows, step};
use crate::run_admission::count;
use crate::support::engine::{Session, World, activity, document, with};
use crate::support::flow_control::{gated_world, publish};

/// `concurrency` on `input.accountId`, `limit: 1`.
fn permit(node: Value) -> Value {
    with(
        node,
        "concurrency",
        json!({"key": "input.accountId", "limit": 1}),
    )
}

/// `rateLimit` on `input.accountId`, `limit: 1` an hour (no slot ages out within a test).
fn slot(node: Value) -> Value {
    with(
        node,
        "rateLimit",
        json!({"key": "input.accountId", "limit": 1, "per": "hour"}),
    )
}

/// Records `lease-withdrawal-v1` for both kinds, as the operator's `capability-activate` does once every live process
/// lists it: the processes the gated world registered (which list the flow-control gates only) are made silent first.
async fn require_lease_withdrawal(w: &World) {
    sqlx::query(
        "UPDATE serving_instances SET last_seen = clock_timestamp() - interval '40 seconds'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the registered processes are silent");
    let recorded: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
         ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE'], 'test')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("activate");
    assert_eq!(recorded, "RECORDED");
}

/// The world's worker service, with the serving gate of an engine that withdraws.
fn withdrawing(w: &World) -> crate::support::engine::Workers {
    let gate = ServingGate::opened();
    gate.keep(&Requirements {
        epoch: 1,
        required: vec![Requirement {
            capability: "lease-withdrawal-v1".to_owned(),
            kinds: vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()],
        }],
    });
    w.workers
        .clone()
        .with_lease_withdrawal(Arc::new(gate))
        .with_hand_back_ceiling(8)
}

/// Withdraws the lease of `task` as one the server never sent.
async fn withdraw(w: &World, task: &sagat_application::worker::ClaimedTask) {
    let outcomes = withdrawing(w)
        .withdraw_undelivered(
            vec![UndeliveredLease::of(w.admin.tenant_id(), task)],
            WithdrawalCause::NotSent,
        )
        .await;
    assert_eq!(outcomes, [WithdrawalOutcome::Withdrawn]);
}

fn us(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).expect("an instant")
}

/// The step `g` grant of `run`: its `leased_at`, in microseconds.
async fn leased_at(w: &World, run: RunId) -> Option<i64> {
    let leased: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the grant");
    leased.map(us)
}

/// The instants of the `step.leased` events of step `g` of `run`, in journal order, in microseconds.
async fn leases(w: &World, run: RunId) -> Vec<i64> {
    let leases: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT occurred_at FROM run_events WHERE run_id = $1 AND node = 'g' AND event_type = 'step.leased' \
          ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("leases");
    leases.into_iter().map(us).collect()
}

async fn lease(
    w: &World,
    session: &Session,
    run: RunId,
    generation: u64,
) -> sagat_application::worker::ClaimedTask {
    let task = w.one_task(session).await;
    assert_eq!(task.run_id, run);
    assert_eq!(
        (task.fence.attempt, task.fence.generation),
        (1, generation),
        "the attempt is reused, with the next generation"
    );
    task
}

/// Decision D022-138 item 1: a step that holds a concurrency permit and whose lease is withdrawn is `READY` again at the
/// same attempt with its holder row; the waiter behind it is not granted; it is leased again without a new decision and
/// releases its permit once, at the end of its attempt, which lets the waiter through.
#[tokio::test]
async fn a_withdrawn_lease_keeps_its_permit_and_is_leased_again_without_queueing() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(
        &w,
        document("withdrawn-permit", "g", vec![permit(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    granted(&w, holder, "g", (true, false)).await;
    let waiter = w.start(workflow, &account).await;
    queued(&w, waiter, "g", (true, false)).await;

    let first = lease(&w, &session, holder, 1).await;
    withdraw(&w, &first).await;
    granted(&w, holder, "g", (true, false)).await;
    assert_eq!(step(&w, holder, "g").await.2, 0, "no attempt spent");
    w.assert_consistent(holder).await;
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        0,
        "the permit is still the holder's"
    );
    queued(&w, waiter, "g", (true, false)).await;

    let second = lease(&w, &session, holder, 2).await;
    for (event, expected) in [
        ("concurrency.acquired", 1),
        ("concurrency.queued", 0),
        ("concurrency.released", 0),
    ] {
        assert_eq!(count(&w, holder, event).await, expected, "{event}");
    }
    w.complete(&session, &second, &json!({}))
        .await
        .expect("done");
    assert_eq!(count(&w, holder, "concurrency.released").await, 1);
    assert_eq!(rows(&w, holder, "g").await, (0, 0, 0, 0));
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        1,
        "the waiter is granted"
    );
    granted(&w, waiter, "g", (true, false)).await;
    let task = lease(&w, &session, waiter, 1).await;
    w.complete(&session, &task, &json!({})).await.expect("done");
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// Decisions D022-138 items 2 and 3 and D022-139: a step that holds a rate slot and whose lease is withdrawn after the
/// scheduler recorded that lease's instant keeps its slot, counting as unleased again (the instant is cleared, and
/// nothing is recorded while it is not leased); it is leased again without a second slot, and the scheduler then records
/// the lease that ran, the attempt's last; the waiter behind it is never granted within the window.
#[tokio::test]
async fn a_withdrawn_lease_keeps_its_slot_and_the_slot_counts_from_the_lease_that_ran() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(
        &w,
        document("withdrawn-slot", "g", vec![slot(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    granted(&w, holder, "g", (false, true)).await;
    let waiter = w.start(workflow, &account).await;
    queued(&w, waiter, "g", (false, true)).await;

    let first = lease(&w, &session, holder, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    assert_eq!(
        leased_at(&w, holder).await,
        leases(&w, holder).await.first().copied()
    );
    withdraw(&w, &first).await;
    granted(&w, holder, "g", (false, true)).await;
    assert_eq!(
        leased_at(&w, holder).await,
        None,
        "the withdrawn lease no longer starts the slot's window: it counts as unleased"
    );
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        0,
        "a withdrawn lease is not recorded again"
    );
    w.assert_consistent(holder).await;
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        0,
        "the slot still counts"
    );

    let second = lease(&w, &session, holder, 2).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let journal = leases(&w, holder).await;
    assert_eq!(journal.len(), 2);
    assert_eq!(
        leased_at(&w, holder).await,
        journal.last().copied(),
        "the lease that ran"
    );
    for (event, expected) in [
        ("rate_limit.granted", 1),
        ("rate_limit.queued", 0),
        ("rate_limit.returned", 0),
    ] {
        assert_eq!(count(&w, holder, event).await, expected, "{event}");
    }
    w.complete(&session, &second, &json!({}))
        .await
        .expect("done");
    assert_eq!(
        rows(&w, holder, "g").await,
        (0, 0, 1, 0),
        "a slot that ran counts by time"
    );
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        0,
        "one slot an hour"
    );
    queued(&w, waiter, "g", (false, true)).await;
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// Decision D022-138 item 2 and ADR-0096 decision 6: a pause after a withdrawal gives back the permit and the slot of
/// the step, which is granted and not leased: the withdrawal's commit cleared the lease instant the scheduler had
/// recorded, so the give-back finds an unleased grant. (The give-back of a recorded instant in the withdrawal's own commit
/// is the cancellation test's.) The key serves the next waiter.
#[tokio::test]
async fn a_pause_after_a_withdrawal_gives_back_the_permit_and_the_slot() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(
        &w,
        document("withdrawn-pause", "g", vec![slot(permit(activity("g")))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let paused = w.start(workflow, &account).await;
    granted(&w, paused, "g", (true, true)).await;
    let waiter = w.start(workflow, &account).await;
    queued(&w, waiter, "g", (true, true)).await;

    let first = lease(&w, &session, paused, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    withdraw(&w, &first).await;
    w.operate_run(paused, RunOperation::Pause).await;
    assert_eq!(count(&w, paused, "concurrency.released").await, 1);
    assert_eq!(count(&w, paused, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, paused, "g").await, (0, 0, 0, 0));
    w.assert_consistent(paused).await;
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        1,
        "the waiter is granted"
    );
    granted(&w, waiter, "g", (true, true)).await;
    w.assert_consistent(waiter).await;
}

/// Decision D022-138 item 2, in one commit: the lease of a step of a run being cancelled is withdrawn, and the same
/// commit cancels the step and gives back its permit and its slot, although the scheduler had recorded the withdrawn
/// lease's instant on the grant (the give-back runs before the commit clears it).
#[tokio::test]
async fn a_withdrawal_that_cancels_the_step_gives_back_its_slot_in_the_same_commit() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(
        &w,
        document("withdrawn-cancel", "g", vec![slot(permit(activity("g")))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let cancelled = w.start(workflow, &account).await;
    let waiter = w.start(workflow, &account).await;
    queued(&w, waiter, "g", (true, true)).await;

    let first = lease(&w, &session, cancelled, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    w.operate_run(cancelled, RunOperation::Cancel).await;
    withdraw(&w, &first).await;
    assert_eq!(w.run_status(cancelled).await.as_str(), "CANCELLED");
    assert_eq!(count(&w, cancelled, "concurrency.released").await, 1);
    assert_eq!(count(&w, cancelled, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, cancelled, "g").await, (0, 0, 0, 0));
    w.assert_consistent(cancelled).await;
    assert_eq!(
        w.scheduler.tick().await.gates_granted,
        1,
        "the waiter is granted"
    );
    granted(&w, waiter, "g", (true, true)).await;
    w.assert_consistent(waiter).await;
}

/// Decision D022-139: a lease pass that read a grant before a withdrawal changed it, and then waited for the row, does
/// not write it: the withdrawal's write made a new row version. The transaction that holds the row here stands for the
/// withdrawal's commit (it writes what the withdrawal writes to the grant). Once it commits, the waiting pass records
/// nothing; the next pass reads the row again and records the lease.
#[tokio::test]
async fn a_lease_pass_does_not_write_a_grant_a_withdrawal_changed_after_the_pass_read_it() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("withdrawn-race", "g", vec![slot(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    lease(&w, &session, holder, 1).await;

    let mut withdrawal = w.db.pool.begin().await.expect("a transaction");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *withdrawal)
        .await
        .expect("the holder's backend");
    sqlx::query("UPDATE rate_limit_grants SET leased_at = NULL WHERE run_id = $1 AND node = 'g'")
        .bind(holder.as_uuid())
        .execute(&mut *withdrawal)
        .await
        .expect("the withdrawal's write, not committed");
    // a second scheduler of the same store, so that its pass can wait while the test goes on
    let scheduler =
        crate::support::scale::scheduler(&w, sagat_scheduler::SchedulerSettings::default(), None);
    let pass = tokio::spawn(async move { scheduler.record_rate_leases().await });
    blocked(
        &w,
        holder_pid,
        "UPDATE rate_limit_grants g SET leased_at = greatest%",
        "the pass waits for the grant row",
    )
    .await;
    withdrawal.commit().await.expect("committed");
    let report = pass.await.expect("the pass ended");
    assert_eq!(
        report.rate_leases_recorded, 0,
        "a row changed since it was read is not written: {report:?}"
    );
    assert_eq!(leased_at(&w, holder).await, None);
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1,
        "the next pass records it"
    );
    assert_eq!(
        leased_at(&w, holder).await,
        leases(&w, holder).await.last().copied()
    );
    w.assert_consistent(holder).await;
}

/// The grant row of step `g` of `run`: (rows, granted_at, leased_at) in microseconds.
async fn grant_row(w: &World, run: RunId) -> (i64, Option<i64>, Option<i64>) {
    let (rows, granted, leased): (i64, Option<OffsetDateTime>, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT count(*), min(granted_at), min(leased_at) FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the grant");
    (rows, granted.map(us), leased.map(us))
}

/// Decision D022-146 (correctness review of 2026-11-26, Major 1): the purge keeps the grant of a step still leased at its
/// attempt, however long ago its lease instant; so a later withdrawal finds the row, and a pause gives the slot back. As
/// of an instant past the window and a minute, the purge deletes nothing; once the step has run and ended, the same purge
/// deletes the grant.
#[tokio::test]
async fn the_purge_keeps_the_grant_of_a_step_still_leased_and_a_pause_after_its_withdrawal_succeeds()
 {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(
        &w,
        document("withdrawn-purge", "g", vec![slot(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let held = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let first = lease(&w, &session, held, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let leased = leased_at(&w, held).await.expect("recorded");
    let past = OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(leased + 3_600_000_000 + 61_000_000) * 1_000,
    )
    .expect("an instant");
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(past, 64, 100)
            .await
            .expect("purge"),
        0,
        "the grant of a step still leased is kept"
    );
    assert_eq!(grant_row(&w, held).await.0, 1);
    withdraw(&w, &first).await;
    w.operate(held, RunOperation::Pause)
        .await
        .expect("the pause gives the slot back");
    assert_eq!(count(&w, held, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, held, "g").await, (0, 0, 0, 0));
    w.assert_consistent(held).await;
    w.operate_run(held, RunOperation::Resume).await;
    let again = w.one_task(&session).await;
    assert_eq!(again.run_id, held);
    w.complete(&session, &again, &json!({}))
        .await
        .expect("done");
    w.scheduler.record_rate_leases().await;
    let leased = leased_at(&w, held).await.expect("the lease that ran");
    let past = OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(leased + 3_600_000_000 + 61_000_000) * 1_000,
    )
    .expect("an instant");
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(past, 64, 100)
            .await
            .expect("purge"),
        1,
        "an ended attempt's grant is purged as before"
    );
}

/// Decision D022-147 (correctness minor 1, security minor 1 of 2026-11-26): a give-back of a grant whose lease instant is
/// recorded, for an attempt no withdrawal gave back, is refused: the pause fails and nothing changes. (The domain never
/// asks it; the instant is written here by hand, as a stand-in for a mistake.)
#[tokio::test]
async fn a_give_back_of_a_recorded_grant_whose_lease_was_not_withdrawn_is_refused() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("withdrawn-refused", "g", vec![slot(activity("g"))]),
    )
    .await;
    let held = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, held, "g", (false, true)).await;
    sqlx::query("UPDATE rate_limit_grants SET leased_at = clock_timestamp() WHERE run_id = $1 AND node = 'g'")
        .bind(held.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("a lease instant recorded by hand");
    let refused = w.operate(held, RunOperation::Pause).await;
    assert!(
        matches!(refused, Err(sagat_application::error::AppError::Internal)),
        "a corrupt projection, refused: {refused:?}"
    );
    assert_eq!(count(&w, held, "rate_limit.returned").await, 0);
    assert_eq!(
        rows(&w, held, "g").await,
        (0, 0, 1, 0),
        "the grant still counts"
    );
}

/// Waits until the backend running a statement like `query` is blocked by the backend `holder` (`pg_blocking_pids` of
/// that very backend), for at most two minutes.
async fn blocked(w: &World, holder: i32, query: &'static str, stage: &'static str) {
    crate::support::bounded::within(stage, std::time::Duration::from_secs(120), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() \
                                AND query LIKE $2 AND $1 = ANY (pg_blocking_pids(pid)))",
            )
            .bind(holder)
            .bind(query)
            .fetch_one(&w.db.pool)
            .await
            .expect("activity");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// How many backends of this database are blocked by the backend `holder`.
async fn blocked_count(w: &World, holder: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND $1 = ANY (pg_blocking_pids(pid))",
    )
    .bind(holder)
    .fetch_one(&w.db.pool)
    .await
    .expect("activity")
}

/// A step `g` of group `shared` on `input.accountId`, `limit` per `per`, with two attempts.
fn shared(name: &str, limit: u32, per: &str) -> Value {
    document(
        name,
        "g",
        vec![with(
            with(
                activity("g"),
                "rateLimit",
                json!({"group": "shared", "key": "input.accountId", "limit": limit, "per": per}),
            ),
            "retry",
            json!({"maxAttempts": 2, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}),
        )],
    )
}

/// The `rate_limit.granted` events of step `g` of `run` for `attempt`.
async fn grants_of_attempt(w: &World, run: RunId, attempt: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND node = 'g' AND event_type = 'rate_limit.granted' \
            AND (detail ->> 'attempt')::int = $2",
    )
    .bind(run.as_uuid())
    .bind(attempt)
    .fetch_one(&w.db.pool)
    .await
    .expect("grants")
}

/// Decision D022-157: a withdrawn slot whose window has passed counts again when no other grant of the key was made since
/// it stopped counting: the withdrawal is honoured and the attempt kept. Here a per-second key, nothing else on it.
#[tokio::test]
async fn a_withdrawn_slot_counts_again_when_no_other_grant_used_the_room_it_left() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let workflow = publish(&w, shared("withdrawn-alone", 1, "second")).await;
    let session = w.session(&["act.g"]).await;
    let held = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let first = lease(&w, &session, held, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let leased = leased_at(&w, held).await.expect("recorded");
    crate::support::flow_control::until_past(&w.db.pool, leased + 1_000_000).await;
    withdraw(&w, &first).await;
    assert_eq!(leased_at(&w, held).await, None, "it counts again");
    let second = lease(&w, &session, held, 2).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    assert_eq!(
        leased_at(&w, held).await,
        leases(&w, held).await.last().copied()
    );
    w.complete(&session, &second, &json!({}))
        .await
        .expect("done");
    assert_eq!(grants_of_attempt(&w, held, 1).await, 1, "one slot");
    w.assert_consistent(held).await;
}

/// Decisions D022-157 and D022-158 (security re-read Medium 1): one key, two definitions, per hour and per second. An
/// hourly slot leased and recorded at `L` stops counting for the per-second waiter at `L + 1 s`, which is then granted.
/// The hourly lease, still within its own hour, may no longer be withdrawn: not sent, it is refused `slot_spent` with
/// nothing changed; handed back by a host of protocol 1.8, it is the ordinary failure; its retry, attempt 2, is granted
/// exactly one slot. The key rows agree with the journals throughout.
#[tokio::test]
async fn a_withdrawal_is_refused_when_a_grant_of_another_period_used_the_room() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let hourly = publish(&w, shared("withdrawn-hourly", 3, "hour")).await;
    let secondly = publish(&w, shared("withdrawn-secondly", 1, "second")).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let held = w.start(hourly, &account).await;
    let task = lease(&w, &session, held, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let leased = leased_at(&w, held).await.expect("recorded");
    let other = w.start(secondly, &account).await;
    crate::support::flow_control::until_past(&w.db.pool, leased + 1_000_000).await;
    crate::support::flow_control::keys_due(&w.db.pool).await;
    for _ in 0..8 {
        if rows(&w, other, "g").await.2 == 1 {
            break;
        }
        w.scheduler.tick().await;
    }
    granted(&w, other, "g", (false, true)).await;
    let before = rows(&w, held, "g").await;
    let outcomes = withdrawing(&w)
        .withdraw_undelivered(
            vec![UndeliveredLease::of(w.admin.tenant_id(), &task)],
            WithdrawalCause::NotSent,
        )
        .await;
    assert_eq!(outcomes, [WithdrawalOutcome::SlotSpent]);
    assert_eq!(step(&w, held, "g").await.0, "LEASED");
    assert_eq!(rows(&w, held, "g").await, before, "nothing changed");
    assert_eq!(leased_at(&w, held).await, Some(leased));
    w.assert_consistent(held).await;
    sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 8 WHERE id = $1")
        .bind(session.worker_id)
        .execute(&w.db.pool)
        .await
        .expect("a host of protocol 1.8");
    withdrawing(&w)
        .fail(
            &session.principal,
            &crate::support::engine::task_ref(&task),
            "TASK_NOT_STARTED",
            true,
            "spent-hand-back",
        )
        .await
        .expect("an ordinary failure");
    assert_eq!(count(&w, held, "step.lease_withdrawn").await, 0);
    for _ in 0..16 {
        if grants_of_attempt(&w, held, 2).await > 0 {
            break;
        }
        crate::support::flow_control::keys_due(&w.db.pool).await;
        w.scheduler.tick().await;
    }
    assert_eq!(grants_of_attempt(&w, held, 1).await, 1);
    assert_eq!(
        grants_of_attempt(&w, held, 2).await,
        1,
        "the retry asks for a new slot, once"
    );
    for run in [held, other] {
        w.assert_consistent(run).await;
    }
}

/// Decision D022-156 (correctness re-read, minor 2): the refusal is decided before the command, under the window's lock.
/// The per-second waiter's grant pass waits for the window a test transaction holds, and the withdrawal of the hourly
/// lease queues behind it; once the window is free the pass grants the waiter (the room the hourly slot left), and the
/// withdrawal, deciding after it, sees that grant and is refused `slot_spent`, with nothing changed.
#[tokio::test]
async fn a_grant_made_while_the_withdrawal_waits_for_the_window_refuses_it() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let hourly = publish(&w, shared("withdrawn-race-h", 3, "hour")).await;
    let secondly = publish(&w, shared("withdrawn-race-s", 1, "second")).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let held = w.start(hourly, &account).await;
    let task = lease(&w, &session, held, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let leased = leased_at(&w, held).await.expect("recorded");
    let other = w.start(secondly, &account).await;
    queued(&w, other, "g", (false, true)).await;
    crate::support::flow_control::until_past(&w.db.pool, leased + 1_000_000).await;
    crate::support::flow_control::keys_due(&w.db.pool).await;
    let mut holder = w.db.pool.begin().await.expect("a transaction");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's backend");
    sqlx::query("SELECT 1 FROM rate_limit_windows WHERE group_name = 'shared' FOR UPDATE")
        .execute(&mut *holder)
        .await
        .expect("the window is held");
    let pass_scheduler =
        crate::support::scale::scheduler(&w, sagat_scheduler::SchedulerSettings::default(), None);
    let pass = tokio::spawn(async move { pass_scheduler.grant_gates().await });
    crate::support::bounded::within(
        "the pass waits",
        std::time::Duration::from_secs(120),
        async {
            while blocked_count(&w, holder_pid).await < 1 {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    let service = withdrawing(&w);
    let lease_of = UndeliveredLease::of(w.admin.tenant_id(), &task);
    let withdrawal = tokio::spawn(async move {
        service
            .withdraw_undelivered(vec![lease_of], WithdrawalCause::NotSent)
            .await
    });
    crate::support::bounded::within(
        "the withdrawal waits",
        std::time::Duration::from_secs(120),
        async {
            // queued behind the pass: a second waiter for a row is blocked by the first waiter, not by the holder
            while blocked_count(&w, holder_pid).await + blocked_behind_waiters(&w, holder_pid).await
                < 2
            {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    holder.commit().await.expect("the window is free");
    assert_eq!(pass.await.expect("the pass ended").gates_granted, 1);
    assert_eq!(
        withdrawal.await.expect("the withdrawal ended"),
        [WithdrawalOutcome::SlotSpent]
    );
    assert_eq!(step(&w, held, "g").await.0, "LEASED");
    assert_eq!(leased_at(&w, held).await, Some(leased), "nothing changed");
    granted(&w, other, "g", (false, true)).await;
    for run in [held, other] {
        w.assert_consistent(run).await;
    }
}

/// Decision D022-156 (correctness re-read, Major 1): one run's two rate-gated steps on keys X1 < X2 (groups `alpha` and
/// `beta`), both leased and not sent, are handed to the withdrawal as (X2, X1) while another transaction holds X1 and then
/// asks for X2, in the sorted order. The withdrawal locks both windows in one sorted pass, so it waits for X1 holding
/// nothing; the other transaction takes X2 and ends; no deadlock, and both leases are withdrawn.
#[tokio::test]
async fn two_leases_of_one_run_lock_their_windows_in_the_sorted_order() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let slot_of = |id: &str, group: &str| {
        with(
            activity(id),
            "rateLimit",
            json!({"group": group, "key": "input.accountId", "limit": 1, "per": "hour"}),
        )
    };
    let workflow = publish(
        &w,
        document(
            "withdrawn-order",
            "p",
            vec![
                crate::support::engine::parallel_node("p", &["a", "b"], "all"),
                slot_of("a", "alpha"),
                slot_of("b", "beta"),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.a", "act.b"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 2);
    let alpha = tasks
        .iter()
        .find(|t| t.activity.as_str() == "act.a")
        .expect("a");
    let beta = tasks
        .iter()
        .find(|t| t.activity.as_str() == "act.b")
        .expect("b");
    let mut holder = w.db.pool.begin().await.expect("a transaction");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's backend");
    sqlx::query("SELECT 1 FROM rate_limit_windows WHERE group_name = 'alpha' FOR UPDATE")
        .execute(&mut *holder)
        .await
        .expect("X1 is held");
    let service = withdrawing(&w);
    let leases = vec![
        UndeliveredLease::of(w.admin.tenant_id(), beta),
        UndeliveredLease::of(w.admin.tenant_id(), alpha),
    ];
    let withdrawal = tokio::spawn(async move {
        service
            .withdraw_undelivered(leases, WithdrawalCause::NotSent)
            .await
    });
    crate::support::bounded::within(
        "the withdrawal waits for X1",
        std::time::Duration::from_secs(120),
        async {
            while blocked_count(&w, holder_pid).await < 1 {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    sqlx::query("SET LOCAL lock_timeout = '10s'")
        .execute(&mut *holder)
        .await
        .expect("a bound");
    sqlx::query("SELECT 1 FROM rate_limit_windows WHERE group_name = 'beta' FOR UPDATE")
        .execute(&mut *holder)
        .await
        .expect("X2, in the sorted order, without a deadlock");
    holder.commit().await.expect("committed");
    assert_eq!(
        withdrawal.await.expect("the withdrawal ended"),
        [WithdrawalOutcome::Withdrawn, WithdrawalOutcome::Withdrawn]
    );
    w.assert_consistent(run).await;
}

/// How many backends of this database are blocked by a backend that is itself blocked by `holder` (a queue for one row).
async fn blocked_behind_waiters(w: &World, holder: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity second WHERE second.datname = current_database() \
            AND EXISTS (SELECT 1 FROM pg_stat_activity first WHERE $1 = ANY (pg_blocking_pids(first.pid)) \
                        AND first.pid = ANY (pg_blocking_pids(second.pid))) \
            AND NOT ($1 = ANY (pg_blocking_pids(second.pid)))",
    )
    .bind(holder)
    .fetch_one(&w.db.pool)
    .await
    .expect("activity")
}

/// Decision D022-156 as amended (security check of 2026-11-26, Low 1): the hand-back of the last step of a run being
/// cancelled ends the run, whose commit locks the active-run counter row and gives back the run's own permit. The
/// withdrawal takes the counter row first, then the semaphores (the run's own among them), then the windows, as ADR-0050
/// orders them. A transaction holding the counter row and the run's semaphore, as an admission of the namespace would, then
/// takes the step's window: it finishes without a deadlock, and so does the hand-back, which withdraws the lease and ends
/// the run.
#[tokio::test]
async fn a_hand_back_that_ends_a_cancelling_run_takes_the_counter_row_before_the_keys() {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let mut doc = document("withdrawn-ends-run", "g", vec![slot(activity("g"))]);
    doc["concurrency"] = json!({"key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = lease(&w, &session, run, 1).await;
    w.operate_run(run, RunOperation::Cancel).await;
    sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 8 WHERE id = $1")
        .bind(session.worker_id)
        .execute(&w.db.pool)
        .await
        .expect("a host of protocol 1.8");
    let tenant = w.admin.tenant_id().as_uuid();
    let mut holder = w.db.pool.begin().await.expect("a transaction");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's backend");
    sqlx::query("SELECT run_active_counts_lock_slot($1, $2)")
        .bind(tenant)
        .bind(run.as_uuid())
        .execute(&mut *holder)
        .await
        .expect("the counter row is held");
    sqlx::query("SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 AND permit_scope = 'RUN' FOR UPDATE")
        .bind(tenant)
        .execute(&mut *holder)
        .await
        .expect("the run's semaphore is held");
    let service = withdrawing(&w);
    let principal = session.principal.clone();
    let handed = task.clone();
    let hand_back = tokio::spawn(async move {
        service
            .fail(
                &principal,
                &crate::support::engine::task_ref(&handed),
                "TASK_NOT_STARTED",
                true,
                "hand-back-ends-run",
            )
            .await
    });
    crate::support::bounded::within(
        "the hand-back waits for the counter row",
        std::time::Duration::from_secs(120),
        async {
            while blocked_count(&w, holder_pid).await < 1 {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    sqlx::query("SET LOCAL lock_timeout = '10s'")
        .execute(&mut *holder)
        .await
        .expect("a bound");
    sqlx::query(
        "SELECT 1 FROM rate_limit_windows WHERE tenant_id = $1 AND limit_scope = 'STEP' FOR UPDATE",
    )
    .bind(tenant)
    .execute(&mut *holder)
    .await
    .expect("the window, after the counter row and the semaphore, without a deadlock");
    holder.commit().await.expect("committed");
    hand_back
        .await
        .expect("the hand-back ended")
        .expect("the hand-back is withdrawn");
    assert_eq!(count(&w, run, "step.lease_withdrawn").await, 1);
    assert_eq!(w.run_status(run).await.as_str(), "CANCELLED");
    assert_eq!(count(&w, run, "rate_limit.returned").await, 1);
    assert_eq!(rows(&w, run, "g").await, (0, 0, 0, 0));
    w.assert_consistent(run).await;
}

/// Decision D022-157, its boundary (security check of 2026-11-26): another grant decided at exactly the withdrawn lease's
/// instant plus that grant's window no longer counted the slot (the count is half-open), so the withdrawal is refused; one
/// microsecond earlier the slot still counted for it, so the withdrawal is honoured. The other grant's instant is moved by
/// hand, so only the journal is checked afterwards.
#[tokio::test]
async fn a_grant_at_exactly_the_end_of_the_window_refuses_the_withdrawal_and_one_microsecond_earlier_does_not()
 {
    let w = gated_world().await;
    require_lease_withdrawal(&w).await;
    let hourly = publish(&w, shared("withdrawn-edge-h", 3, "hour")).await;
    let secondly = publish(&w, shared("withdrawn-edge-s", 1, "second")).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let held = w.start(hourly, &account).await;
    let task = lease(&w, &session, held, 1).await;
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        1
    );
    let leased = leased_at(&w, held).await.expect("recorded");
    let other = w.start(secondly, &account).await;
    crate::support::flow_control::until_past(&w.db.pool, leased + 1_000_000).await;
    crate::support::flow_control::keys_due(&w.db.pool).await;
    for _ in 0..8 {
        if rows(&w, other, "g").await.2 == 1 {
            break;
        }
        w.scheduler.tick().await;
    }
    granted(&w, other, "g", (false, true)).await;
    let place = |offset_us: i64| {
        let pool = w.db.pool.clone();
        async move {
            let mut tx = pool.begin().await.expect("a transaction");
            for statement in [
                "ALTER TABLE rate_limit_grants DISABLE TRIGGER USER",
                "UPDATE rate_limit_grants o SET granted_at = (SELECT g.leased_at FROM rate_limit_grants g \
                    WHERE g.run_id = $1 AND g.node = 'g') + interval '1 second' + $3 * interval '1 microsecond' \
                  WHERE o.run_id = $2 AND o.node = 'g'",
                "ALTER TABLE rate_limit_grants ENABLE TRIGGER USER",
            ] {
                sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
                    .bind(held.as_uuid())
                    .bind(other.as_uuid())
                    .bind(offset_us)
                    .execute(&mut *tx)
                    .await
                    .expect(statement);
            }
            tx.commit().await.expect("placed");
        }
    };
    let withdraw_once = || {
        let service = withdrawing(&w);
        let lease_of = UndeliveredLease::of(w.admin.tenant_id(), &task);
        async move {
            service
                .withdraw_undelivered(vec![lease_of], WithdrawalCause::NotSent)
                .await
        }
    };
    place(0).await;
    assert_eq!(
        withdraw_once().await,
        [WithdrawalOutcome::SlotSpent],
        "exactly at the end"
    );
    assert_eq!(leased_at(&w, held).await, Some(leased), "nothing changed");
    place(-1).await;
    assert_eq!(
        withdraw_once().await,
        [WithdrawalOutcome::Withdrawn],
        "one microsecond earlier"
    );
    assert_eq!(leased_at(&w, held).await, None);
    w.store
        .verify_run(w.admin.tenant_id(), held)
        .await
        .expect("the journal replays");
}
