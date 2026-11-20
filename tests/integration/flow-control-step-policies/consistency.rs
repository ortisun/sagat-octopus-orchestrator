//! Invariants 1 and 2 of `data-model.md` across runs (T022-027; FR-022-020): the consistency assertion every test of the
//! suites makes (`World::assert_consistent`) fails on each kind of seeded inconsistency between a key's counters, its
//! holder and waiter rows and the permits the runs' journals hold and wait for, and passes on the state the gate leaves.

use sagat_domain::execution::RunId;
use serde_json::json;

use crate::run_admission::{serial_start, start_rows};
use crate::support::engine::World;
use crate::support::flow_control::{gated_world, key_consistency, publish};

/// A key of `limit: 1` with a holder run and a queued run, consistent.
async fn holder_and_waiter() -> (World, RunId, RunId) {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("consistency", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0));
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    let runs = key_consistency(&w.db.pool, w.admin.tenant_id())
        .await
        .expect("the gate leaves its keys consistent");
    let mut expected = vec![holder, waiter];
    let mut named = runs.clone();
    expected.sort_by_key(|r| r.as_uuid());
    named.sort_by_key(|r| r.as_uuid());
    assert_eq!(named, expected, "the runs the rows name");
    w.assert_consistent(holder).await;
    (w, holder, waiter)
}

/// Applies `statements` (each bound to `run`) in one transaction with the user triggers of the three tables disabled
/// (the counter, immutability and pruning triggers fire whatever the replication role), as a defect that wrote around
/// them would, and returns why the assertion now fails.
async fn seed(w: &World, run: RunId, statements: &[&'static str]) -> String {
    const TABLES: [&str; 3] = [
        "concurrency_semaphores",
        "concurrency_holders",
        "concurrency_waiters",
    ];
    let mut tx = w.db.pool.begin().await.expect("tx");
    for table in TABLES {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} DISABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await
        .expect("triggers off");
    }
    for &statement in statements {
        let done = sqlx::query(statement)
            .bind(run.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("seeded");
        assert_eq!(done.rows_affected(), 1, "{statement}");
    }
    for table in TABLES {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} ENABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await
        .expect("triggers on");
    }
    tx.commit().await.expect("seeded");
    key_consistency(&w.db.pool, w.admin.tenant_id())
        .await
        .expect_err("a seeded inconsistency is found")
}

#[tokio::test]
async fn a_counter_that_differs_from_its_rows_is_found() {
    let (w, holder, _) = holder_and_waiter().await;
    let why = seed(
        &w,
        holder,
        &["UPDATE concurrency_semaphores s SET holders = holders + 1 FROM concurrency_holders h \
           WHERE h.run_id = $1 AND s.tenant_id = h.tenant_id AND s.namespace_id = h.namespace_id AND s.pool = h.pool \
             AND s.permit_scope = h.holder_scope AND s.group_name = h.group_name AND s.key_digest = h.key_digest"],
    )
    .await;
    assert!(why.contains("counters differ"), "{why}");
}

#[tokio::test]
async fn a_holder_row_the_journal_does_not_hold_is_found() {
    let (w, holder, _) = holder_and_waiter().await;
    let why = seed(
        &w,
        holder,
        &["UPDATE runs SET flow_control_gates = jsonb_set(flow_control_gates, '{concurrency,phase}', '\"ENDED\"') \
           WHERE id = $1"],
    )
    .await;
    assert!(
        why.contains("a holder row whose gates hold no such permit"),
        "{why}"
    );
}

#[tokio::test]
async fn a_held_permit_without_its_holder_row_is_found() {
    let (w, holder, _) = holder_and_waiter().await;
    let why = seed(
        &w,
        holder,
        &[
            "UPDATE concurrency_semaphores s SET holders = holders - 1 FROM concurrency_holders h \
             WHERE h.run_id = $1 AND s.tenant_id = h.tenant_id AND s.namespace_id = h.namespace_id AND s.pool = h.pool \
               AND s.permit_scope = h.holder_scope AND s.group_name = h.group_name AND s.key_digest = h.key_digest",
            "DELETE FROM concurrency_holders WHERE run_id = $1",
        ],
    )
    .await;
    assert!(
        why.contains("a permit the gates hold without its holder row"),
        "{why}"
    );
}

#[tokio::test]
async fn a_waiter_row_with_another_deadline_than_its_journal_is_found() {
    let (w, _, waiter) = holder_and_waiter().await;
    let why = seed(
        &w,
        waiter,
        &["UPDATE concurrency_waiters SET deadline_at = deadline_at + interval '1 microsecond' WHERE run_id = $1"],
    )
    .await;
    assert!(
        why.contains("a waiter row whose gates wait for no such permit"),
        "{why}"
    );
}

#[tokio::test]
async fn a_queued_permit_without_its_waiter_row_is_found() {
    let (w, _, waiter) = holder_and_waiter().await;
    let why = seed(
        &w,
        waiter,
        &[
            "UPDATE concurrency_semaphores s SET waiters = waiters - 1 FROM concurrency_waiters x \
             WHERE x.run_id = $1 AND s.tenant_id = x.tenant_id AND s.namespace_id = x.namespace_id AND s.pool = x.pool \
               AND s.permit_scope = x.permit_scope AND s.group_name = x.group_name AND s.key_digest = x.key_digest",
            "DELETE FROM concurrency_waiters WHERE run_id = $1",
        ],
    )
    .await;
    assert!(
        why.contains("a permit the gates wait for without its waiter row"),
        "{why}"
    );
}
