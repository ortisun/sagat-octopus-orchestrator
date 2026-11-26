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
    w.workers.clone().with_lease_withdrawal(Arc::new(gate))
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

/// Decision D022-138 item 2 and ADR-0096 decision 6: a pause after a withdrawal gives back the permit and the slot of the
/// step, which is granted and not leased, even though the scheduler had recorded the withdrawn lease's instant; the key
/// serves the next waiter.
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
    sqlx::query("UPDATE rate_limit_grants SET leased_at = NULL WHERE run_id = $1 AND node = 'g'")
        .bind(holder.as_uuid())
        .execute(&mut *withdrawal)
        .await
        .expect("the withdrawal's write, not committed");
    // a second scheduler of the same store, so that its pass can wait while the test goes on
    let scheduler =
        crate::support::scale::scheduler(&w, sagat_scheduler::SchedulerSettings::default(), None);
    let pass = tokio::spawn(async move { scheduler.record_rate_leases().await });
    crate::support::bounded::within(
        "the pass waits for the grant row",
        std::time::Duration::from_secs(120),
        async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting >= 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
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
