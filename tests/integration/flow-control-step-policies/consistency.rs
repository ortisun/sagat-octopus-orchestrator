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
            "UPDATE concurrency_semaphores s SET waiters = waiters - 1, oldest_wait_at = NULL FROM concurrency_waiters x \
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

/// Invariant 12 (keys review, correctness m7): a waiter row of a run that has ended is found.
#[tokio::test]
async fn a_waiter_row_of_a_terminal_run_is_found() {
    let (w, _, waiter) = holder_and_waiter().await;
    let why = seed(
        &w,
        waiter,
        &["UPDATE runs SET status = 'CANCELLED', finished_at = clock_timestamp() WHERE id = $1"],
    )
    .await;
    assert!(why.contains("a waiter row of a terminal run"), "{why}");
}

/// FR-022-020 (keys review, correctness m7): a key row nobody holds or waits for is found.
#[tokio::test]
async fn an_empty_key_row_is_found() {
    let (w, holder, _) = holder_and_waiter().await;
    let why = seed(
        &w,
        holder,
        &["INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
                                             holders, waiters, key_version, key_shard, updated_at) \
           SELECT s.tenant_id, s.namespace_id, s.pool, s.permit_scope, 'emptied', s.key_digest, 0, 0, s.key_version, \
                  s.key_shard, clock_timestamp() \
             FROM concurrency_semaphores s JOIN concurrency_holders h ON s.tenant_id = h.tenant_id \
              AND s.namespace_id = h.namespace_id AND s.pool = h.pool AND s.permit_scope = h.holder_scope \
              AND s.group_name = h.group_name AND s.key_digest = h.key_digest \
            WHERE h.run_id = $1"],
    )
    .await;
    assert!(why.contains("an empty key row"), "{why}");
}

/// D022-058: a key whose oldest wait is not its waiters' oldest is found (the listing orders by it).
#[tokio::test]
async fn an_oldest_wait_that_is_not_the_waiters_is_found() {
    let (w, _, waiter) = holder_and_waiter().await;
    let why = seed(
        &w,
        waiter,
        &["UPDATE concurrency_semaphores s SET oldest_wait_at = oldest_wait_at - interval '1 microsecond' \
             FROM concurrency_waiters x \
            WHERE x.run_id = $1 AND s.tenant_id = x.tenant_id AND s.namespace_id = x.namespace_id AND s.pool = x.pool \
              AND s.permit_scope = x.permit_scope AND s.group_name = x.group_name AND s.key_digest = x.key_digest"],
    )
    .await;
    assert!(why.contains("oldest wait differs"), "{why}");
}

/// Keys review (2026-11-20), architecture M2: a column the checks name that is gone (renamed) fails the check, where
/// it used to pass it as "a schema before the gates" in every suite at once.
#[tokio::test]
async fn a_renamed_column_fails_the_check_instead_of_passing_it() {
    let (w, _, _) = holder_and_waiter().await;
    sqlx::query("ALTER TABLE concurrency_waiters RENAME COLUMN deadline_at TO deadline_at_renamed")
        .execute(&w.db.pool)
        .await
        .expect("renamed");
    let why = key_consistency(&w.db.pool, w.admin.tenant_id())
        .await
        .expect_err("a check that cannot run is a failure");
    assert!(why.contains("could not run"), "{why}");
}

/// D022-058: the oldest wait a key's row keeps is its waiters' oldest after any one statement, however many of the key's
/// waiters it adds and removes (its row triggers see all of them while the counter moves by one each): two waiters
/// removed by one statement, and a waiter replaced by one statement with an older and then a younger instant. Each is
/// rolled back; every intermediate state passes the row's CHECK (the statement would fail otherwise).
#[tokio::test]
async fn one_statement_that_adds_and_removes_waiters_of_a_key_leaves_its_oldest_wait() {
    let (w, _, first) = holder_and_waiter().await;
    let workflow = publish(&w, serial_start("consistency-third", 1)).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, second).await, (0, 1));
    let key = |run: RunId| async move {
        sqlx::query_as::<_, (i32, Option<time::OffsetDateTime>, Option<time::OffsetDateTime>)>(
            "SELECT s.waiters, s.oldest_wait_at, \
                    (SELECT min(x.enqueued_at) FROM concurrency_waiters x WHERE x.tenant_id = s.tenant_id \
                        AND x.namespace_id = s.namespace_id AND x.pool = s.pool AND x.permit_scope = s.permit_scope \
                        AND x.group_name = s.group_name AND x.key_digest = s.key_digest) \
               FROM concurrency_semaphores s JOIN concurrency_holders h ON s.tenant_id = h.tenant_id \
                AND s.namespace_id = h.namespace_id AND s.pool = h.pool AND s.permit_scope = h.holder_scope \
                AND s.group_name = h.group_name AND s.key_digest = h.key_digest \
              WHERE h.run_id = $1",
        )
        .bind(run.as_uuid())
    };
    let holder: RunId = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT run_id FROM concurrency_holders WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .map(RunId::new)
    .expect("the holder");
    let (waiters, oldest, min) = key(holder).await.fetch_one(&w.db.pool).await.expect("key");
    assert_eq!((waiters, oldest), (2, min), "two waiters, the oldest kept");

    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("DELETE FROM concurrency_waiters WHERE run_id = ANY($1)")
        .bind(vec![first.as_uuid(), second.as_uuid()])
        .execute(&mut *tx)
        .await
        .expect("both removed by one statement");
    let (waiters, oldest, _) = key(holder).await.fetch_one(&mut *tx).await.expect("key");
    assert_eq!((waiters, oldest), (0, None), "nobody waits");
    tx.rollback().await.expect("rolled back");

    for offset in ["- interval '1 day'", "+ interval '1 day'"] {
        let mut tx = w.db.pool.begin().await.expect("tx");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "WITH gone AS (DELETE FROM concurrency_waiters WHERE run_id = $1 RETURNING *) \
             INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                    group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
             SELECT tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, group_name, key_digest, \
                    declared_limit, enqueued_at {offset}, deadline_at + interval '2 days', shard FROM gone"
        )))
        .bind(first.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("replaced by one statement");
        let (waiters, oldest, min) = key(holder).await.fetch_one(&mut *tx).await.expect("key");
        assert_eq!(waiters, 2, "{offset}");
        assert_eq!(oldest, min, "{offset}: the oldest of the rows");
        tx.rollback().await.expect("rolled back");
    }
}
