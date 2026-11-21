//! The emergency end of a digest-key rotation (T022-095; ADR-0093, accepted with the force by OD-2026-11-19-2; O1170
//! (a) with (b) for incidents): the operator's audited force retires the old version at once. Its rows live on under
//! their old digest until they drain; a key decided after the force is filed under the active version, so a key may
//! have two rows meanwhile, and its limit is over-admitted by at most the holders (or counted grants) of its old row.
//!
//! The force waits for a deciding transaction in flight (both take the version table, the decider shared), and races
//! are held by locks and observed through `pg_stat_activity`, never by sleeps.

use sagat_domain::execution::RunId;
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::readiness::{granted, queued};
use crate::support::engine::{World, activity, ctx, document, with};
use crate::support::flow_control::{gated_world_with_keys, publish};

/// A ring with versions 1 and 2, as every process holds it during the incident.
fn two_keys() -> String {
    format!("1:{},2:{}", "5a".repeat(32), "6b".repeat(32))
}

fn permit(node: Value) -> Value {
    with(
        node,
        "concurrency",
        json!({"group": "acct-k", "key": "input.accountId", "limit": 1}),
    )
}

fn slot(node: Value) -> Value {
    with(
        node,
        "rateLimit",
        json!({"group": "prov-k", "key": "input.accountId", "limit": 1, "per": "hour"}),
    )
}

async fn activate(pool: &PgPool, version: i16) -> (String, i64) {
    sqlx::query_as("SELECT out_outcome, out_rows FROM flow_control_key_activate($1, 'operator')")
        .bind(version)
        .fetch_one(pool)
        .await
        .expect("the operator's command")
}

/// The operator's force: retires `version` at once, with a reason.
pub(super) async fn force(
    pool: &PgPool,
    version: i16,
    reason: &str,
) -> Result<(String, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT out_outcome, out_rows FROM flow_control_key_force_retire($1, 'operator:incident', $2)",
    )
    .bind(version)
    .bind(reason)
    .fetch_one(pool)
    .await
}

/// The rows of key `acct-1` of the concurrency group: (version, holders, waiters), by version.
async fn semaphores(w: &World) -> Vec<(i16, i32, i32)> {
    sqlx::query_as(
        "SELECT key_version, holders, waiters FROM concurrency_semaphores WHERE group_name = 'acct-k' \
         ORDER BY key_version",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("semaphores")
}

/// The rows of key `acct-1` of the rate group: (version, counted grants, waiters), by version.
async fn windows(w: &World) -> Vec<(i16, i32, i32)> {
    sqlx::query_as(
        "SELECT key_version, live_grants, waiters FROM rate_limit_windows WHERE group_name = 'prov-k' \
         ORDER BY key_version",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("windows")
}

/// The bound of ADR-0093 decision 5, at this instant: each row of the key stays within the limit (1) on its own, so the
/// key is over-admitted by at most what its old row holds now; and the old row has gained no waiter since the force
/// (`old_waiters`: what it had then).
async fn bounded(w: &World, old_waiters: (i32, i32)) {
    for (rows, name, waiters_at_force) in [
        (semaphores(w).await, "semaphore", old_waiters.0),
        (windows(w).await, "window", old_waiters.1),
    ] {
        for (version, held, waiters) in &rows {
            assert!(*held <= 1, "a {name} row over its own limit: {rows:?}");
            if *version == 1 {
                assert!(
                    *waiters <= waiters_at_force,
                    "the old {name} row gained a waiter after the force: {rows:?}"
                );
            }
        }
    }
}

/// Fails the test when `wait` has not ended within thirty seconds (a deadline, not a pause: a broken lock fails
/// instead of hanging).
async fn within<T>(what: &str, wait: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(30), wait)
        .await
        .unwrap_or_else(|_| panic!("{what} did not happen"))
}

/// Waits until a backend running a statement that contains `query` waits for a lock.
async fn lock_waiter(w: &World, query: &str) {
    within("a lock wait", async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND wait_event_type = 'Lock' AND state = 'active' AND query LIKE '%' || $1 || '%' \
                 AND pid <> pg_backend_pid()",
            )
            .bind(query)
            .fetch_one(&w.db.pool)
            .await
            .expect("activity");
            if waiting > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// OD-2026-11-19-2: a forced retirement waits for a decision in flight, then files every new decision of a key under
/// the active version; the key's old row keeps its holder and its own waiter, gains nothing, grants only its own waiter
/// and goes with its last row, so the key is over-admitted by at most its old row's holders, until they drain.
#[tokio::test]
async fn a_forced_retirement_over_admits_a_key_by_at_most_its_old_rows_holders_until_they_drain() {
    let w = gated_world_with_keys(&two_keys()).await;
    let pool = &w.db.pool;
    let permits = publish(
        &w,
        document("forced-permit", "g", vec![permit(activity("g"))]),
    )
    .await;
    let slots = publish(&w, document("forced-slot", "r", vec![slot(activity("r"))])).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});

    // under version 1: a holder and a waiter of the permit, a counted grant of the slot
    let holder = w.start(permits, &account).await;
    granted(&w, holder, "g", (true, false)).await;
    let old_waiter = w.start(permits, &account).await;
    queued(&w, old_waiter, "g", (true, false)).await;
    let slot_holder = w.start(slots, &account).await;
    granted(&w, slot_holder, "r", (false, true)).await;
    assert_eq!(activate(pool, 2).await, ("ACTIVATED".to_owned(), 2));
    assert_eq!(semaphores(&w).await, vec![(1, 1, 1)], "per key: one row");

    // a deciding transaction in flight holds the versions shared: the force waits for it
    let runtime = w.db.runtime_pool().await;
    let mut decider = runtime.begin().await.expect("tx");
    let (active, retiring): (Option<i16>, Option<i16>) =
        sqlx::query_as("SELECT active, retiring FROM flow_control_key_versions_in_force()")
            .fetch_one(&mut *decider)
            .await
            .expect("in force");
    assert_eq!((active, retiring), (Some(2), Some(1)));
    let forced = tokio::spawn({
        let pool = pool.clone();
        async move { force(&pool, 1, "version 1 leaked (incident 42)").await }
    });
    within("the force waiting for the decision", async {
        loop {
            assert!(
                !forced.is_finished(),
                "the force must wait for a decision in flight"
            );
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND wait_event_type = 'Lock' AND query LIKE '%flow_control_key_force_retire%' \
                 AND pid <> pg_backend_pid()",
            )
            .fetch_one(pool)
            .await
            .expect("activity");
            if waiting > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    decider.commit().await.expect("the decision commits first");
    let forced = forced.await.expect("joined").expect("the force");
    assert_eq!(
        forced,
        ("FORCED".to_owned(), 2),
        "two key rows left under 1"
    );
    let (active, retiring): (Option<i16>, Option<i16>) =
        sqlx::query_as("SELECT active, retiring FROM flow_control_key_versions_in_force()")
            .fetch_one(pool)
            .await
            .expect("in force");
    assert_eq!(
        (active, retiring),
        (Some(2), None),
        "1 is out of force at once"
    );

    // after the force: the same key is decided on a second row, under 2, from zero
    let second = w.start(permits, &account).await;
    granted(&w, second, "g", (true, false)).await;
    assert_eq!(semaphores(&w).await, vec![(1, 1, 1), (2, 1, 0)]);
    bounded(&w, (1, 0)).await;
    let second_slot = w.start(slots, &account).await;
    granted(&w, second_slot, "r", (false, true)).await;
    assert_eq!(
        windows(&w).await,
        vec![(1, 1, 0), (2, 1, 0)],
        "the window over-admits by its old row's counted grant"
    );

    // a decision that contends for the new row's lock waits for it, then counts its holder: queued there
    let mut held = pool.begin().await.expect("tx");
    sqlx::query(
        "SELECT 1 FROM concurrency_semaphores WHERE group_name = 'acct-k' AND key_version = 2 FOR UPDATE",
    )
    .fetch_one(&mut *held)
    .await
    .expect("the new row held");
    let (third, ()) = tokio::join!(w.start(permits, &account), async {
        lock_waiter(&w, "concurrency_semaphores").await;
        held.commit().await.expect("released");
    });
    queued(&w, third, "g", (true, false)).await;
    let third_slot = w.start(slots, &account).await;
    queued(&w, third_slot, "r", (false, true)).await;
    assert_eq!(
        semaphores(&w).await,
        vec![(1, 1, 1), (2, 1, 1)],
        "the old row gains nothing after the force"
    );
    assert_eq!(windows(&w).await, vec![(1, 1, 0), (2, 1, 1)]);
    bounded(&w, (1, 0)).await;

    // the old row drains: its holder ends, the pass grants its own waiter there and nothing else
    let tasks = w.poll(&session, 8).await;
    let runs: Vec<RunId> = tasks.iter().map(|t| t.run_id).collect();
    assert!(runs.contains(&holder) && runs.contains(&second), "{runs:?}");
    let holder_task = tasks.iter().find(|t| t.run_id == holder).expect("task");
    w.complete(&session, holder_task, &json!({}))
        .await
        .expect("done");
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    granted(&w, old_waiter, "g", (true, false)).await;
    queued(&w, third, "g", (true, false)).await;
    assert_eq!(semaphores(&w).await, vec![(1, 1, 0), (2, 1, 1)]);
    bounded(&w, (1, 0)).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, old_waiter);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(
        semaphores(&w).await,
        vec![(2, 1, 1)],
        "the old row went with its last holder"
    );

    // from here the key has one row and its limit again
    let second_task = tasks.iter().find(|t| t.run_id == second).expect("task");
    w.complete(&session, second_task, &json!({}))
        .await
        .expect("done");
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    granted(&w, third, "g", (true, false)).await;
    assert_eq!(semaphores(&w).await, vec![(2, 1, 0)]);

    // the force is on the record: who, as which database role, why, and what it left
    let audit: (i16, i64, String, bool, String) = sqlx::query_as(
        "SELECT version, rows_left, actor, database_user = session_user, reason \
           FROM flow_control_key_forced_retirements",
    )
    .fetch_one(pool)
    .await
    .expect("one forced retirement");
    assert_eq!(
        audit,
        (
            1,
            2,
            "operator:incident".to_owned(),
            true,
            "version 1 leaked (incident 42)".to_owned()
        )
    );
    for run in [
        holder,
        old_waiter,
        second,
        third,
        slot_holder,
        second_slot,
        third_slot,
    ] {
        w.assert_consistent(run).await;
    }
}

/// Correctness review of 2026-11-20, M2: after a force, a key whose only row is under the forced version is filed again
/// under the active one. The namespace's live-keys ceiling counts the rows of the versions in force, so that key is
/// decided as before, not refused `FLOW_CONTROL_LIMIT` for the old row it is replacing; the overshoot of the ceiling is
/// at most the rows the force left, until they drain.
#[tokio::test]
async fn a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling() {
    let w = gated_world_with_keys(&two_keys()).await;
    let pool = &w.db.pool;
    let permits = publish(
        &w,
        document(
            "forced-ceiling",
            "g",
            vec![
                with(permit(activity("g")), "onError", json!("recover")),
                activity("recover"),
            ],
        ),
    )
    .await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ConcurrencyLiveKeys,
            1,
            &ctx(),
        )
        .await
        .expect("one live key");
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(permits, &account).await;
    granted(&w, holder, "g", (true, false)).await;
    assert_eq!(activate(pool, 2).await, ("ACTIVATED".to_owned(), 1));
    assert_eq!(
        force(pool, 1, "version 1 leaked").await.expect("forced"),
        ("FORCED".to_owned(), 1)
    );
    let again = w.start(permits, &account).await;
    granted(&w, again, "g", (true, false)).await;
    assert_eq!(semaphores(&w).await, vec![(1, 1, 0), (2, 1, 0)]);
    // a key that is new under the versions in force is still refused at the ceiling
    let other = w.start(permits, &json!({"accountId": "acct-2"})).await;
    let code: Option<String> =
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'g'")
            .bind(other.as_uuid())
            .fetch_one(pool)
            .await
            .expect("step");
    assert_eq!(code.as_deref(), Some("FLOW_CONTROL_LIMIT"));
    for run in [holder, again, other] {
        w.assert_consistent(run).await;
    }
}
