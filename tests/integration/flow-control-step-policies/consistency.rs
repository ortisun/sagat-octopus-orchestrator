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
    seed_tables(
        w,
        run,
        &[
            "concurrency_semaphores",
            "concurrency_holders",
            "concurrency_waiters",
        ],
        statements,
    )
    .await
}

/// [`seed`] over the given tables' user triggers.
async fn seed_tables(
    w: &World,
    run: RunId,
    tables: &[&'static str],
    statements: &[&'static str],
) -> String {
    let mut tx = w.db.pool.begin().await.expect("tx");
    for &table in tables {
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
    for &table in tables {
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

const RATE_TABLES: [&str; 3] = [
    "rate_limit_windows",
    "rate_limit_grants",
    "rate_limit_waiters",
];

/// A run-level rate-limit key of `limit: 1, per: hour` with a holder run and two buffered runs, consistent.
async fn rated_holder_and_waiters() -> (World, RunId, RunId, RunId) {
    let w = gated_world().await;
    let mut doc = crate::support::engine::document(
        "consistency-rated",
        "g",
        vec![crate::support::engine::activity("g")],
    );
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": 1, "per": "hour"});
    let workflow = publish(&w, doc).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    let first = w.start(workflow, &account).await;
    let second = w.start(workflow, &account).await;
    let waiters: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_limit_waiters WHERE tenant_id = $1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("waiters");
    assert_eq!(waiters, 2, "two runs buffered");
    w.assert_consistent(holder).await;
    (w, holder, first, second)
}

/// Decision D022-068 item 5: a window whose oldest wait is not its waiters' oldest is found (the rate-limit listing
/// orders by it), one microsecond off.
#[tokio::test]
async fn a_windows_oldest_wait_that_is_not_its_waiters_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &["UPDATE rate_limit_windows k SET oldest_wait_at = oldest_wait_at + interval '1 microsecond' \
             FROM rate_limit_waiters x \
            WHERE x.run_id = $1 AND k.tenant_id = x.tenant_id AND k.namespace_id = x.namespace_id AND k.pool = x.pool \
              AND k.limit_scope = x.limit_scope AND k.group_name = x.group_name AND k.key_digest = x.key_digest"],
    )
    .await;
    assert!(why.contains("a window's oldest wait differs"), "{why}");
}

/// Decision D022-068 item 5 (correctness Major 3 of the review of task T022-078): the oldest wait a window keeps is its
/// remaining waiters' oldest after one statement that removes some of its waiters or all of them, and after one that
/// replaces a waiter with an older and then a younger instant. Each is rolled back; every intermediate state passes the
/// row's CHECK (the statement would fail otherwise).
#[tokio::test]
async fn one_statement_that_adds_and_removes_waiters_of_a_window_leaves_its_oldest_wait() {
    let (w, holder, first, second) = rated_holder_and_waiters().await;
    let key = || {
        sqlx::query_as::<_, (i32, Option<time::OffsetDateTime>, Option<time::OffsetDateTime>)>(
            "SELECT k.waiters, k.oldest_wait_at, \
                    (SELECT min(x.enqueued_at) FROM rate_limit_waiters x WHERE x.tenant_id = k.tenant_id \
                        AND x.namespace_id = k.namespace_id AND x.pool = k.pool AND x.limit_scope = k.limit_scope \
                        AND x.group_name = k.group_name AND x.key_digest = k.key_digest) \
               FROM rate_limit_windows k JOIN rate_limit_grants g ON g.tenant_id = k.tenant_id \
                AND g.namespace_id = k.namespace_id AND g.pool = k.pool AND g.limit_scope = k.limit_scope \
                AND g.group_name = k.group_name AND g.key_digest = k.key_digest \
              WHERE g.run_id = $1",
        )
        .bind(holder.as_uuid())
    };
    let enqueued = |run: RunId| {
        sqlx::query_scalar::<_, time::OffsetDateTime>(
            "SELECT enqueued_at FROM rate_limit_waiters WHERE run_id = $1",
        )
        .bind(run.as_uuid())
    };
    let (waiters, oldest, min) = key().fetch_one(&w.db.pool).await.expect("window");
    let (e1, e2) = (
        enqueued(first).fetch_one(&w.db.pool).await.expect("first"),
        enqueued(second)
            .fetch_one(&w.db.pool)
            .await
            .expect("second"),
    );
    assert!(e1 < e2, "queued in order");
    assert_eq!(
        (waiters, oldest, min),
        (2, Some(e1), Some(e1)),
        "two waiters, the oldest kept"
    );

    // the oldest removed: the next one's instant, exactly
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("DELETE FROM rate_limit_waiters WHERE run_id = $1")
        .bind(first.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("the oldest removed");
    let (waiters, oldest, _) = key().fetch_one(&mut *tx).await.expect("window");
    assert_eq!(
        (waiters, oldest),
        (1, Some(e2)),
        "the remaining waiter's instant"
    );
    tx.rollback().await.expect("rolled back");

    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("DELETE FROM rate_limit_waiters WHERE run_id = ANY($1)")
        .bind(vec![first.as_uuid(), second.as_uuid()])
        .execute(&mut *tx)
        .await
        .expect("both removed by one statement");
    let (waiters, oldest, _) = key().fetch_one(&mut *tx).await.expect("window");
    assert_eq!((waiters, oldest), (0, None), "nobody waits");
    tx.rollback().await.expect("rolled back");

    for offset in ["- interval '1 day'", "+ interval '1 day'"] {
        let mut tx = w.db.pool.begin().await.expect("tx");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "WITH gone AS (DELETE FROM rate_limit_waiters WHERE run_id = $1 RETURNING *) \
             INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                    group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
             SELECT tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, key_digest, \
                    declared_limit, period_ms, enqueued_at {offset}, deadline_at + interval '2 days', shard FROM gone"
        )))
        .bind(first.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("replaced by one statement");
        let (waiters, oldest, min) = key().fetch_one(&mut *tx).await.expect("window");
        assert_eq!(waiters, 2, "{offset}");
        assert_eq!(oldest, min, "{offset}: the oldest of the rows");
        tx.rollback().await.expect("rolled back");
    }
}

/// The window of the rate-limit key `run` waits on, as a predicate on `k` (for the seeds below).
const WINDOW_OF: &str = "EXISTS (SELECT 1 FROM rate_limit_waiters x WHERE x.run_id = $1 AND k.tenant_id = x.tenant_id \
     AND k.namespace_id = x.namespace_id AND k.pool = x.pool AND k.limit_scope = x.limit_scope \
     AND k.group_name = x.group_name AND k.key_digest = x.key_digest)";

/// Task T022-079 (invariant 8 and `live_grants`): a window whose counters are not its rows is found.
#[tokio::test]
async fn a_windows_counter_that_differs_from_its_rows_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &[const_format(
            "UPDATE rate_limit_windows k SET live_grants = live_grants + 1, grants_total = grants_total + 1 WHERE ",
        )],
    )
    .await;
    assert!(
        why.contains("a window's counters differ from its rows"),
        "{why}"
    );
}

/// Task T022-079: a window row nobody holds a grant of or waits on is found.
#[tokio::test]
async fn an_empty_window_row_is_found() {
    let (w, holder, _, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        holder,
        &RATE_TABLES,
        &["INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
                                          key_version, key_shard, updated_at) \
           SELECT g.tenant_id, g.namespace_id, g.pool, g.limit_scope, 'emptied', g.key_digest, k.key_version, \
                  k.key_shard, clock_timestamp() \
             FROM rate_limit_grants g JOIN rate_limit_windows k ON k.tenant_id = g.tenant_id \
              AND k.namespace_id = g.namespace_id AND k.pool = g.pool AND k.limit_scope = g.limit_scope \
              AND k.group_name = g.group_name AND k.key_digest = g.key_digest \
            WHERE g.run_id = $1"],
    )
    .await;
    assert!(why.contains("an empty window row"), "{why}");
}

/// Task T022-079 (invariant 8): a waiter row whose deadline is not the one its journal queued it with is found.
#[tokio::test]
async fn a_rate_waiter_row_the_journal_does_not_wait_for_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &["UPDATE rate_limit_waiters SET deadline_at = deadline_at + interval '1 microsecond' WHERE run_id = $1"],
    )
    .await;
    assert!(
        why.contains("a rate-limit waiter row whose gates wait for no such slot"),
        "{why}"
    );
}

/// Task T022-079 (invariant 8): a slot the journal waits for without its waiter row is found (the counters kept right).
#[tokio::test]
async fn a_slot_the_journal_waits_for_without_its_waiter_row_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &[
            const_format(
                "UPDATE rate_limit_windows k SET waiters = waiters - 1, \
                        oldest_wait_at = (SELECT min(y.enqueued_at) FROM rate_limit_waiters y \
                                           WHERE y.run_id <> $1 AND y.key_digest = k.key_digest) WHERE ",
            ),
            "DELETE FROM rate_limit_waiters WHERE run_id = $1",
        ],
    )
    .await;
    assert!(
        why.contains("a slot the gates wait for without its rate-limit waiter row"),
        "{why}"
    );
}

/// Task T022-079 (invariant 9, "fails on a seeded extra grant"): a grant row no `rate_limit.granted` made is found.
#[tokio::test]
async fn a_seeded_extra_grant_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &[
            "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                                           group_name, key_digest, declared_limit, period_ms, granted_at, leased_at, \
                                           shard) \
             SELECT tenant_id, namespace_id, run_id, '', 0, pool, limit_scope, group_name, key_digest, declared_limit, \
                    period_ms, clock_timestamp(), clock_timestamp(), shard FROM rate_limit_waiters WHERE run_id = $1",
            const_format(
                "UPDATE rate_limit_windows k SET live_grants = live_grants + 1, grants_total = grants_total + 1 WHERE ",
            ),
        ],
    )
    .await;
    assert!(
        why.contains("a grant row without its rate_limit.granted"),
        "{why}"
    );
}

/// Task T022-079 (invariant 9): a run's slot that still counts by its journal and has no grant row is found.
#[tokio::test]
async fn a_counted_slot_without_its_grant_row_is_found() {
    let (w, holder, _, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        holder,
        &RATE_TABLES,
        &[
            "UPDATE rate_limit_windows k SET live_grants = live_grants - 1 FROM rate_limit_grants g \
              WHERE g.run_id = $1 AND k.tenant_id = g.tenant_id AND k.namespace_id = g.namespace_id \
                AND k.pool = g.pool AND k.limit_scope = g.limit_scope AND k.group_name = g.group_name \
                AND k.key_digest = g.key_digest",
            "DELETE FROM rate_limit_grants WHERE run_id = $1",
        ],
    )
    .await;
    assert!(
        why.contains("a run's counted slot without its grant row"),
        "{why}"
    );
}

/// Task T022-079 (invariant 9: `grants_total` grows by one per grant): a total above the key's journaled grants is
/// found.
#[tokio::test]
async fn a_grants_total_above_the_journaled_grants_is_found() {
    let (w, _, first, _) = rated_holder_and_waiters().await;
    let why = seed_tables(
        &w,
        first,
        &RATE_TABLES,
        &[const_format(
            "UPDATE rate_limit_windows k SET grants_total = grants_total + 1 WHERE ",
        )],
    )
    .await;
    assert!(
        why.contains("a window's grants_total outside its rows and its journaled grants"),
        "{why}"
    );
}

/// `head` followed by [`WINDOW_OF`], leaked once per seed so it is `&'static` like the other seeds' text.
fn const_format(head: &str) -> &'static str {
    Box::leak(format!("{head}{WINDOW_OF}").into_boxed_str())
}
