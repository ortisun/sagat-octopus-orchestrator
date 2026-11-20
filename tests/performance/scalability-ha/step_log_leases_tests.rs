//! The exact leases of ADR-0097 as the spike's third run builds them (`step_log_spike::install_leases`): what the
//! measurement relies on, proved before it is measured. Races are made with held transactions and
//! `pg_stat_activity`, never with sleeps.

use std::sync::Arc;
use std::time::Duration;

use sagat_domain::quota::QuotaResource;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::support::bounded;
use crate::support::engine::{World, ctx, world};

const LIMIT: Duration = Duration::from_secs(60);

async fn leased_world(limit: u64) -> (World, Uuid) {
    let w = world().await;
    let tenant = w.admin.tenant_id();
    w.quotas
        .grant_ceiling(tenant, QuotaResource::TenantStepLogBytes, Some(limit))
        .await
        .expect("grant the ceiling");
    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::TenantStepLogBytes,
            limit,
            &ctx(),
        )
        .await
        .expect("set the limit");
    super::install(&w.db.pool).await;
    super::install_per_statement(&w.db.pool).await;
    super::install_leases(&w.db.pool).await;
    (w, tenant.as_uuid())
}

async fn charge(conn: &mut PgConnection, tenant: Uuid, bytes: i64) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT spike_041.step_log_charge($1, $2)")
        .bind(tenant)
        .bind(bytes)
        .execute(conn)
        .await
        .map(|_| ())
}

async fn charge_committed(pool: &PgPool, tenant: Uuid, bytes: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    charge(&mut tx, tenant, bytes).await?;
    tx.commit().await
}

/// S, G, Σ granted, Σ used and the lease count of the tenant.
async fn ledger(pool: &PgPool, tenant: Uuid) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT u.step_log_bytes, u.step_log_leased_bytes, \
                coalesce((SELECT sum(granted) FROM spike_041.step_log_quota_leases WHERE tenant_id = $1), 0)::bigint, \
                coalesce((SELECT sum(used) FROM spike_041.step_log_quota_leases WHERE tenant_id = $1), 0)::bigint, \
                (SELECT count(*) FROM spike_041.step_log_quota_leases WHERE tenant_id = $1) \
           FROM tenant_retained_usage u WHERE u.tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("the ledger")
}

/// Waits until `pid` waits on a heavyweight lock, or panics after the bound.
async fn waits_on_a_lock(pool: &PgPool, pid: i32) {
    bounded::within("the backend waits on a lock", LIMIT, async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT coalesce(wait_event_type = 'Lock', false) FROM pg_stat_activity WHERE pid = $1",
            )
            .bind(pid)
            .fetch_one(pool)
            .await
            .expect("pg_stat_activity");
            if waiting {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

fn refused(result: Result<(), sqlx::Error>) -> bool {
    matches!(result, Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("54000"))
}

/// The constraint name of a refusal, if `result` is one.
fn refusal(result: &Result<(), sqlx::Error>) -> Option<String> {
    match result {
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("54000") => {
            e.constraint().map(str::to_owned)
        }
        _ => None,
    }
}

/// A connection of its own, outside the pool (the test pool is small), and its backend: under D041-117 a lease belongs
/// to the server backend that charges.
async fn writer(pool: &PgPool) -> (Box<PgConnection>, i32) {
    let options = pool.connect_options();
    let mut conn = Box::new(
        <PgConnection as sqlx::Connection>::connect_with(&options)
            .await
            .expect("a connection"),
    );
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("pid");
    (conn, pid)
}

/// A charge in a transaction of its own on `conn`, committed.
async fn charge_on(conn: &mut PgConnection, tenant: Uuid, bytes: i64) -> Result<(), sqlx::Error> {
    let mut tx = sqlx::Connection::begin(conn).await?;
    charge(&mut tx, tenant, bytes).await?;
    tx.commit().await
}

/// ADR-0097's invariants on the committed state: G = Σ granted, 0 ≤ U = S + Σ used ≤ S + G ≤ L.
async fn assert_exact(pool: &PgPool, tenant: Uuid, limit: i64) {
    let (s, g, granted, used, _) = ledger(pool, tenant).await;
    assert_eq!(g, granted, "G is the sum of the grants");
    assert!(s + used >= 0, "the usage {} is negative", s + used);
    assert!(
        s + used <= s + g,
        "the usage {} passed S + G {}",
        s + used,
        s + g
    );
    assert!(s + g <= limit, "S + G {} passed the limit {limit}", s + g);
}

/// Waits until some backend waits on an advisory lock (a fence), or panics after the bound.
async fn a_backend_waits_on_a_fence(pool: &PgPool) {
    bounded::within("a backend waits on a fence", LIMIT, async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND wait_event = 'advisory' \
                   AND datname = current_database()",
            )
            .fetch_one(pool)
            .await
            .expect("pg_stat_activity");
            if waiting > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// D041-117: each server backend charges its own lease; one writer's open transaction never makes another wait or fail,
/// and the leases are keyed by the writers' backends.
#[tokio::test]
async fn each_writer_charges_its_own_lease_and_never_meets_another_writers_lock() {
    bounded::within("own lease", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        let ((mut a, pid_a), (mut b, pid_b)) = (writer(pool).await, writer(pool).await);
        charge_on(&mut a, tenant, 1_000).await.expect("A's lease granted");
        charge_on(&mut b, tenant, 1_000).await.expect("B's lease granted");
        let mut held = sqlx::Connection::begin(&mut *a).await.expect("begin");
        charge(&mut held, tenant, 1_000).await.expect("A holds its lease");
        let second = tokio::time::timeout(Duration::from_secs(20), charge_on(&mut b, tenant, 1_000)).await;
        assert!(matches!(second, Ok(Ok(()))), "A's open transaction made B wait or fail: {second:?}");
        held.commit().await.expect("commit");
        let writers: Vec<i32> = sqlx::query_scalar(
            "SELECT writer FROM spike_041.step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer",
        )
        .bind(tenant)
        .fetch_all(pool)
        .await
        .expect("the leases");
        let mut expected = vec![pid_a, pid_b];
        expected.sort_unstable();
        assert_eq!(writers, expected, "one lease a writer, keyed by its backend");
        let (s, _, _, used, _) = ledger(pool, tenant).await;
        assert_eq!(s + used, 4_000, "every charged byte is counted once");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

#[tokio::test]
async fn a_charge_with_a_free_lease_never_takes_the_usage_row_and_a_refill_alone_waits_for_it() {
    bounded::within("usage row", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        let (mut c, pid) = writer(pool).await;
        without_the_refill_bound(pool).await;
        charge_on(&mut c, tenant, 1_000)
            .await
            .expect("C's lease granted");
        let mut row = pool.begin().await.expect("begin");
        sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
            .bind(tenant)
            .execute(&mut *row)
            .await
            .expect("hold the usage row");
        let free =
            tokio::time::timeout(Duration::from_secs(20), charge_on(&mut c, tenant, 1_000)).await;
        assert!(
            matches!(free, Ok(Ok(()))),
            "a charge with room in its own lease took the usage row: {free:?}"
        );
        // A charge larger than the writer's room needs a refill, which is the one wait.
        let refill = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            charge(&mut tx, tenant, 4 << 20).await?;
            tx.commit().await
        });
        waits_on_a_lock(pool, pid).await;
        row.commit().await.expect("release the usage row");
        refill
            .await
            .expect("join")
            .expect("the refill completes once the row is free");
        let (s, g, granted, used, _) = ledger(pool, tenant).await;
        assert_eq!(g, granted);
        assert_eq!(s + used, 2_000 + (4 << 20));
    })
    .await;
}

#[tokio::test]
async fn concurrent_charges_never_exceed_the_limit_and_count_every_byte_once() {
    bounded::within("exact under concurrency", LIMIT, async {
        const L: i64 = 200_000;
        const N: i64 = 3_000;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        let pool = w.db.pool.clone();
        let barrier = Arc::new(tokio::sync::Barrier::new(32));
        let tasks: Vec<_> = (0..32)
            .map(|_| {
                let (pool, barrier) = (pool.clone(), Arc::clone(&barrier));
                tokio::spawn(async move {
                    barrier.wait().await;
                    let mut kept = 0i64;
                    for _ in 0..10 {
                        match charge_committed(&pool, tenant, N).await {
                            Ok(()) => kept += N,
                            Err(sqlx::Error::Database(e))
                                if e.code().as_deref() == Some("54000") => {}
                            Err(e) => panic!("an unexpected error: {e}"),
                        }
                    }
                    kept
                })
            })
            .collect();
        let mut kept = 0;
        for task in tasks {
            kept += task.await.expect("join");
        }
        let (s, g, granted, used, _) = ledger(&pool, tenant).await;
        assert_eq!(g, granted, "G is the sum of the grants");
        assert_eq!(
            s + used,
            kept,
            "the usage is exactly what was charged and committed"
        );
        assert!(s + used <= L, "the usage {} passed the limit {L}", s + used);
        assert!(s + g <= L, "the leased capacity passed the limit");
        // Nothing is open now: every byte of room left is chargeable, and one more is refused.
        let room = L - (s + used);
        if room > 0 {
            charge_committed(&pool, tenant, room)
                .await
                .expect("the exact remainder is admitted");
        }
        assert!(
            refused(charge_committed(&pool, tenant, 1).await),
            "one byte past the limit was admitted"
        );
        let (s, _, _, used, _) = ledger(&pool, tenant).await;
        assert_eq!(s + used, L);
    })
    .await;
}

#[tokio::test]
async fn a_lowered_limit_stops_charges_into_leases_with_room() {
    bounded::within("lowered limit", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 0 granted with room");
        w.quotas
            .set(
                &w.admin,
                None,
                QuotaResource::TenantStepLogBytes,
                1_000,
                &ctx(),
            )
            .await
            .expect("lower the limit to the usage");
        assert!(
            refused(charge_committed(pool, tenant, 1).await),
            "a lease granted under the old limit was charged past the new one"
        );
    })
    .await;
}

/// D041-117's grant, min(1 MiB, F / (2(A + 1))), and the reclaim near the limit: the room of idle leases completes the
/// limit. L = 2 MiB: A's first grant is 1 MiB (F / 2), B's 262,144 (F = 1 MiB, one other lease); the room left
/// (2,095,152) is more than the unleased room (786,432), so only the leases' room completes it.
#[tokio::test]
async fn the_last_bytes_of_the_limit_are_admitted_from_the_room_of_idle_leases() {
    bounded::within("reclaim", LIMIT, async {
        const L: i64 = 2 * 1024 * 1024;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        let pool = &w.db.pool;
        let ((mut a, _), (mut b, _)) = (writer(pool).await, writer(pool).await);
        charge_on(&mut a, tenant, 1_000)
            .await
            .expect("A: 1 MiB granted");
        charge_on(&mut b, tenant, 1_000)
            .await
            .expect("B: 262,144 granted");
        let (s, g, granted, used, leases) = ledger(pool, tenant).await;
        assert_eq!(
            (s, g, granted, used, leases),
            (0, 1_310_720, 1_310_720, 2_000, 2)
        );
        charge_committed(pool, tenant, L - 2_000)
            .await
            .expect("the room of idle leases is given back near the limit");
        assert!(
            refused(charge_committed(pool, tenant, 1).await),
            "one byte past the limit was admitted"
        );
        let (s, _, _, used, _) = ledger(pool, tenant).await;
        assert_eq!(s + used, L);
        assert_exact(pool, tenant, L).await;
    })
    .await;
}

/// The coordinator's small-quota case (D041-117): a tenant with a small limit served by many writers is never refused
/// while the room of its idle leases would fit the charge. Twenty writers lease and go idle; the whole rest is admitted.
#[tokio::test]
async fn a_small_quota_with_many_idle_writers_admits_everything_their_leases_hold() {
    bounded::within("small quota", LIMIT, async {
        const L: i64 = 100_000;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        let pool = &w.db.pool;
        let mut writers = Vec::new();
        for _ in 0..20 {
            let (mut conn, _) = writer(pool).await;
            charge_on(&mut conn, tenant, 1)
                .await
                .expect("a writer leases");
            writers.push(conn);
        }
        let (s, g, granted, used, leases) = ledger(pool, tenant).await;
        assert_eq!(leases, 20);
        assert_eq!(g, granted);
        assert!(L - s - g < L - s - used, "the idle leases hold room");
        charge_committed(pool, tenant, L - s - used)
            .await
            .expect("everything the idle leases hold is admitted");
        assert!(
            refused(charge_committed(pool, tenant, 1).await),
            "one byte past the limit was admitted"
        );
        assert_exact(pool, tenant, L).await;
        drop(writers);
    })
    .await;
}

/// D041-117's bound on room held by open transactions: each grant leaves at least half of the unleased room, so with k
/// writers holding their leases at least L · C(2k, k) / 4^k stays chargeable by another writer (k = 8, L = 1 MiB:
/// 1,048,576 · 12,870 / 65,536 = 205,920), and only past it is a charge refused as held.
#[tokio::test]
async fn room_held_by_open_writers_is_bounded_by_the_grant() {
    bounded::within("held bound", LIMIT, async {
        const L: i64 = 1024 * 1024;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        let pool = &w.db.pool;
        let mut held = Vec::new();
        for _ in 0..8 {
            let (mut conn, _) = writer(pool).await;
            charge_on(&mut conn, tenant, 1)
                .await
                .expect("a writer leases");
            sqlx::query("BEGIN")
                .execute(&mut *conn)
                .await
                .expect("begin");
            charge(&mut conn, tenant, 1)
                .await
                .expect("the writer holds its lease");
            held.push(conn);
        }
        let (s, g, _, _, _) = ledger(pool, tenant).await;
        let free = L - s - g;
        assert!(
            free >= 205_920,
            "the unleased room {free} is under the bound 205,920"
        );
        let (mut other, _) = writer(pool).await;
        let past = charge_on(&mut other, tenant, free + 1).await;
        assert_eq!(
            refusal(&past).as_deref(),
            Some("tenant_step_log_quota_held"),
            "{past:?}"
        );
        charge_on(&mut other, tenant, free)
            .await
            .expect("the unleased room is chargeable while every lease is held");
        for mut conn in held {
            sqlx::query("COMMIT")
                .execute(&mut *conn)
                .await
                .expect("commit");
        }
        assert_exact(pool, tenant, L).await;
    })
    .await;
}

/// The cost breakdown's carry modes (D041-107) write what their names say, so that the diagnosis compares what it claims.
#[tokio::test]
async fn each_carry_mode_of_the_cost_breakdown_writes_only_its_storage() {
    bounded::within("cost modes", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        crate::step_log_diag::install(pool).await;
        crate::step_log_diag::install_cost(pool).await;
        crate::step_log_diag::set_charge(pool, tenant, "none").await;
        let count = |sql: &'static str| async move {
            sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.expect("count")
        };
        // (mode, streams, lines, noidx, nofk, unlogged, chunks) after the call, cumulative.
        let expected: [(&str, i64, i64, i64, i64, i64, i64); 7] = [
            ("locks", 0, 0, 0, 0, 0, 0),
            ("stream", 1, 0, 0, 0, 0, 0),
            ("full", 2, 10, 0, 0, 0, 0),
            ("noidx", 3, 10, 10, 0, 0, 0),
            ("nofk", 4, 10, 10, 10, 0, 0),
            ("unlogged", 5, 10, 10, 10, 10, 0),
            ("chunk", 6, 10, 10, 10, 10, 1),
        ];
        for (mode, streams, lines, noidx, nofk, unlogged, chunks) in expected {
            crate::step_log_diag::set_carry_mode(pool, mode).await;
            let outcome: String = sqlx::query_scalar(
                "SELECT spike_041.step_log_carry($1, $2, $3, $4, 'a', 1, 0, '{}'::text[], $5, \
                 ARRAY['step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'], $6, 'x')",
            )
            .bind(tenant)
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(format!("cost-{mode}"))
            .bind((0..10).map(|i| format!("line {i}")).collect::<Vec<_>>())
            .fetch_one(pool)
            .await
            .expect("the carry");
            assert_eq!(outcome, "written", "{mode}");
            let got = (
                count("SELECT count(*) FROM spike_041.step_log_streams").await,
                count("SELECT count(*) FROM spike_041.step_log_lines").await,
                count("SELECT count(*) FROM spike_041.lines_noidx").await,
                count("SELECT count(*) FROM spike_041.lines_nofk").await,
                count("SELECT count(*) FROM spike_041.lines_unlogged").await,
                count("SELECT count(*) FROM spike_041.chunks").await,
            );
            assert_eq!(got, (streams, lines, noidx, nofk, unlogged, chunks), "{mode}");
        }
        let records: i32 = sqlx::query_scalar("SELECT records FROM spike_041.chunks")
            .fetch_one(pool)
            .await
            .expect("the chunk");
        assert_eq!(records, 10, "a chunk row holds the carry's ten records");
    })
    .await;
}

/// Nested statements the server ran (`pg_stat_statements.track = 'all'`, as the test support starts it).
async fn nested_calls(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_statements \
          WHERE NOT toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(pool)
    .await
    .expect("pg_stat_statements")
}

/// Entries `pg_stat_statements` evicted so far: a measure taken while it evicted is not trusted (as `work_budget.rs`).
async fn evictions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(pool)
        .await
        .expect("pg_stat_statements_info")
}

/// Nested statements per query text, for the itemised budget.
async fn nested_by_query(pool: &PgPool) -> std::collections::BTreeMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT regexp_replace(query, '\\s+', ' ', 'g'), sum(calls)::bigint FROM pg_stat_statements \
          WHERE NOT toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) GROUP BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements")
    .into_iter()
    .collect()
}

/// The tenant's quota rows as the open transaction sees them: its leases, its usage row and the transaction's deltas.
type QuotaRows = (Vec<(i32, i64, i64)>, Option<(i64, i64)>, i64);

async fn quota_rows(conn: &mut PgConnection, tenant: Uuid) -> QuotaRows {
    let leases: Vec<(i32, i64, i64)> = sqlx::query_as(
        "SELECT writer, granted, used FROM spike_041.step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer",
    )
    .bind(tenant)
    .fetch_all(&mut *conn)
    .await
    .expect("the leases");
    let usage: Option<(i64, i64)> = sqlx::query_as(
        "SELECT step_log_bytes, step_log_leased_bytes FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_optional(&mut *conn)
    .await
    .expect("the usage row");
    let deltas: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM quota_transaction_deltas WHERE transaction_id = pg_current_xact_id()",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("the deltas");
    (leases, usage, deltas)
}

/// Quota rows written between two readings: lease rows changed or added, the usage row changed, delta rows. Compared by
/// value, so that rows a subtransaction wrote count too (their `xmin` is not the transaction's).
fn written(before: &QuotaRows, after: &QuotaRows) -> (i64, i64, i64) {
    let changed = after.0.iter().filter(|row| !before.0.contains(row)).count();
    (
        i64::try_from(changed).unwrap_or(i64::MAX),
        i64::from(before.1 != after.1),
        after.2,
    )
}

/// The budget line of a step-log insert statement under ADR-0097 (design 041 sections 2 and 2a): charged into a free lease,
/// it writes one lease row and no usage or delta row, runs six nested statements for the charge (the transition-table
/// read, the charge call, the three fences in one statement, the limit's read, the unlocked usage read, the lease
/// update) besides the one foreign-key check of each line, and queues no deferred call for the commit; a refill writes
/// the usage row once and one lease row.
#[tokio::test]
async fn a_step_log_insert_costs_its_budget_line() {
    bounded::within("budget line", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
            .execute(pool)
            .await
            .expect("the test PostgreSQL preloads pg_stat_statements");
        let track: String = sqlx::query_scalar("SELECT current_setting('pg_stat_statements.track')")
            .fetch_one(pool)
            .await
            .expect("track");
        assert_eq!(track, "all", "the budget counts nested statements: pg_stat_statements.track must be 'all'");
        let stream: i64 = sqlx::query_scalar(
            "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
               instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
             VALUES ($1, gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'a', '{}', '{}', \
               1, 0, 14, now(), now() + interval '14 days') RETURNING stream_id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await
        .expect("a stream");
        let insert = |from: i64| {
            sqlx::query(
                "INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
                 SELECT $1, $2, s, gen_random_uuid(), 'info', 'line', repeat('x', 200) FROM generate_series($3, $3 + 9) s",
            )
            .bind(tenant)
            .bind(stream)
            .bind(from)
        };
        // The first insert finds no lease of its writer: a refill. Every insert runs on one writer (D041-117).
        let (mut conn, _) = writer(pool).await;
        let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
        let start = quota_rows(&mut tx, tenant).await;
        insert(1).execute(&mut *tx).await.expect("the first ten lines");
        assert_eq!(
            written(&start, &quota_rows(&mut tx, tenant).await),
            (1, 1, 0),
            "a refill writes one lease row and the usage row once"
        );
        tx.commit().await.expect("commit");
        // The next ones find lease room: the hot path. Measured again while `pg_stat_statements` evicts entries.
        for attempt in 0..8 {
            let evicted = evictions(pool).await;
            let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
            let start = quota_rows(&mut tx, tenant).await;
            let before = nested_calls(pool).await;
            let items = nested_by_query(pool).await;
            insert(11 + 10 * attempt).execute(&mut *tx).await.expect("ten more lines");
            let during = nested_calls(pool).await;
            let ran: Vec<(String, i64)> = nested_by_query(pool)
                .await
                .into_iter()
                .map(|(q, n)| (q.chars().take(90).collect::<String>(), n - items.get(&q).copied().unwrap_or(0)))
                .filter(|(_, n)| *n > 0)
                .collect();
            let written = written(&start, &quota_rows(&mut tx, tenant).await);
            let at_commit = nested_calls(pool).await;
            tx.commit().await.expect("commit");
            let after = nested_calls(pool).await;
            if evictions(pool).await != evicted {
                continue;
            }
            let foreign_key: i64 = ran
                .iter()
                .filter(|(q, _)| q.starts_with("SELECT $3 FROM ONLY \"spike_041\".\"step_log_streams\""))
                .map(|(_, n)| n)
                .sum();
            assert_eq!(foreign_key, 10, "one foreign-key check a line: {ran:#?}");
            assert_eq!(written, (1, 0, 0), "a charge writes one lease row, no usage and no delta row");
            // The reads of this test are top-level statements of another connection, and a subquery is not a
            // statement: the difference is the insert's nested statements, its charge's six and one foreign-key check
            // for each line.
            assert_eq!(during - before, 6 + 10, "the nested statements of one charged insert: {ran:#?}");
            assert_eq!(after - at_commit, 0, "the commit runs no deferred quota call for a step-log-only transaction");
            return;
        }
        panic!("pg_stat_statements evicted entries during every one of eight attempts");
    })
    .await;
}

async fn third_run_world() -> (World, Uuid) {
    let (w, tenant) = leased_world(1 << 30).await;
    super::install_third_run_storage(&w.db.pool).await;
    (w, tenant)
}

/// Counts every update of a stream row (a test-only trigger): `pg_stat_xact_user_tables` also counts the work of a
/// subtransaction the carry's block rolled back, so it cannot tell a stream written once.
async fn count_stream_updates(pool: &PgPool) {
    for statement in [
        "CREATE TABLE spike_041.test_stream_updates (stream_id bigint NOT NULL)",
        "CREATE FUNCTION spike_041.test_stream_updated() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN INSERT INTO spike_041.test_stream_updates VALUES (NEW.stream_id); RETURN NULL; END $$",
        "CREATE TRIGGER test_stream_updated AFTER UPDATE ON spike_041.step_log_streams \
           FOR EACH ROW EXECUTE FUNCTION spike_041.test_stream_updated()",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .expect("the update counter");
    }
}

/// The streams of `run`, their updates and their lines, as the open transaction sees them.
async fn run_rows(conn: &mut PgConnection, run: Uuid) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM spike_041.step_log_streams WHERE run_id = $1), \
                (SELECT count(*) FROM spike_041.test_stream_updates u JOIN spike_041.step_log_streams s \
                    USING (stream_id) WHERE s.run_id = $1), \
                (SELECT count(*) FROM spike_041.step_log_lines WHERE run_id = $1)",
    )
    .bind(run)
    .fetch_one(conn)
    .await
    .expect("the run's rows")
}

/// The streams and lines of `run`, as the open transaction sees them.
async fn run_lines(conn: &mut PgConnection, run: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM spike_041.step_log_streams WHERE run_id = $1), \
                (SELECT count(*) FROM spike_041.step_log_lines WHERE run_id = $1)",
    )
    .bind(run)
    .fetch_one(conn)
    .await
    .expect("the run's rows")
}

fn carry_call<'q>(
    tenant: Uuid,
    run: Uuid,
    step: Uuid,
) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, String, sqlx::postgres::PgArguments> {
    sqlx::query_scalar(
        "SELECT spike_041.step_log_carry($1, $2, $3, $4, 'a', 1, 0, '{}'::text[], $5, \
         ARRAY['step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'], $6, 'x')",
    )
    .bind(tenant)
    .bind(Uuid::new_v4())
    .bind(run)
    .bind(step)
    .bind(format!("stream-lock-{run}-{step}"))
    .bind((0..10).map(|i| format!("{i:0>200}")).collect::<Vec<_>>())
}

/// Inserts, updates and HOT updates of a table by the open transaction.
async fn table_writes(conn: &mut PgConnection, table: &str) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT coalesce(sum(n_tup_ins), 0)::bigint, coalesce(sum(n_tup_upd), 0)::bigint, \
                coalesce(sum(n_tup_hot_upd), 0)::bigint \
           FROM pg_stat_xact_user_tables WHERE schemaname = 'spike_041' AND relname = $1",
    )
    .bind(table)
    .fetch_one(conn)
    .await
    .expect("pg_stat_xact_user_tables")
}

/// D041-111: a short step's carry writes its new stream once (no update in the same transaction), and its ten lines;
/// the budget line of the whole call is pinned in nested statements.
#[tokio::test]
async fn a_short_steps_carry_writes_its_new_stream_once() {
    bounded::within("one write", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
            .execute(pool)
            .await
            .expect("the test PostgreSQL preloads pg_stat_statements");
        count_stream_updates(pool).await;
        // A first carry leases (a refill), so that the measured one charges its writer's lease (D041-117).
        let (mut conn, _) = writer(pool).await;
        let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
        let first = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("a first carry");
        assert_eq!(first, "written:refill", "the first carry leases on the slow path");
        tx.commit().await.expect("commit");
        for _ in 0..8 {
            let evicted = evictions(pool).await;
            let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
            let run = Uuid::new_v4();
            let start = quota_rows(&mut tx, tenant).await;
            let before = nested_calls(pool).await;
            let outcome = carry_call(tenant, run, Uuid::new_v4())
                .fetch_one(&mut *tx)
                .await
                .expect("the carry");
            let during = nested_calls(pool).await;
            assert_eq!(outcome, "written");
            let rows = run_rows(&mut tx, run).await;
            let written = written(&start, &quota_rows(&mut tx, tenant).await);
            let at_commit = nested_calls(pool).await;
            tx.commit().await.expect("commit");
            let after = nested_calls(pool).await;
            if evictions(pool).await != evicted {
                continue;
            }
            assert_eq!(rows, (1, 0, 10), "one stream, inserted once and never updated, and its ten lines");
            assert_eq!(written, (1, 0, 0), "one lease row, no usage or delta row");
            assert_eq!(after - at_commit, 0, "no deferred quota call at the commit");
            // The fast path (D041-113, D041-117): the function's four statements (the application name, the policy read,
            // the usage read, the plan's size), the reservation's three (the usage read, the limit, the writer's lease
            // locked), the writes' two (the stream insert, the lines insert; both functions are called as expressions,
            // which are not statements), the stream's path check (`execution_instance_path_valid`, a SQL function its
            // CHECK calls), the charge's six and the ten foreign-key checks. Derived first as 22 (seen failing at 23: the
            // path check), then 25 with the no-wait setting (seen failing at 24: a reset left out), then 29 with the fast
            // path (seen failing at 27: a function called in an `IF` or a `RETURN` is an expression, not a statement),
            // then 26 when D041-117 dropped the no-wait setting (seen failing at 26 against the 27 left unchanged).
            assert_eq!(during - before, 4 + 3 + 2 + 1 + 6 + 10, "the nested statements of a short step's carry");
        let (records, bytes, last, xid_set): (i32, i64, i64, bool) = sqlx::query_as(
            "SELECT records, bytes, last_sequence, last_stored_xid IS NOT NULL FROM spike_041.step_log_streams \
              ORDER BY stream_id DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("the stream");
        assert_eq!((records, bytes, last, xid_set), (10, 2_000, 10, true), "the counters the plan wrote");
            return;
        }
        panic!("pg_stat_statements evicted entries during every one of eight attempts");
    })
    .await;
}

/// D041-111: a carry onto an existing stream (a long step's) updates it HOT when the day of its expiry is unchanged.
#[tokio::test]
async fn a_carry_onto_an_existing_stream_updates_it_hot() {
    bounded::within("hot update", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        let (run, step) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query(
            "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
               instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
             VALUES ($1, gen_random_uuid(), gen_random_uuid(), $2, $3, 'a', '{}', '{}', 1, 0, 14, clock_timestamp(), \
               (date_trunc('day', clock_timestamp() AT TIME ZONE 'UTC') + interval '14 days 12 hours') AT TIME ZONE 'UTC')",
        )
        .bind(tenant)
        .bind(run)
        .bind(step)
        .execute(pool)
        .await
        .expect("a long step's stream");
        let mut tx = pool.begin().await.expect("begin");
        let outcome = carry_call(tenant, run, step).fetch_one(&mut *tx).await.expect("the carry");
        assert_eq!(outcome, "written:refill", "the tenant's first carry leases on the slow path");
        assert_eq!(table_writes(&mut tx, "step_log_streams").await, (0, 1, 1), "one update, HOT");
        tx.commit().await.expect("commit");
    })
    .await;
}

/// ADR-0097 after review (D041-112): a limit lowered below the leased room but above the usage reclaims the idle leases
/// at the next charge instead of refusing every charge.
#[tokio::test]
async fn a_limit_lowered_above_the_usage_still_admits_what_fits() {
    bounded::within("lowered above usage", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 0: 1 MiB granted");
        w.quotas
            .set(
                &w.admin,
                None,
                QuotaResource::TenantStepLogBytes,
                10_000,
                &ctx(),
            )
            .await
            .expect("lower the limit above the usage, below the leased room");
        let (s, g, _, used, _) = ledger(pool, tenant).await;
        assert_eq!(
            (s, g, used),
            (1_000, 0, 0),
            "the change of the limit reclaimed every lease"
        );
        charge_committed(pool, tenant, 2_000)
            .await
            .expect("what fits under the new limit is admitted");
        assert!(
            refused(charge_committed(pool, tenant, 7_001).await),
            "past the new limit is refused"
        );
        charge_committed(pool, tenant, 7_000)
            .await
            .expect("the exact rest is admitted");
        let (s, g, granted, used, _) = ledger(pool, tenant).await;
        assert_eq!(g, granted);
        assert_eq!(s + used, 10_000);
    })
    .await;
}

/// D041-112: a release applies to the folded usage under the usage row only: it never waits for a lease another
/// transaction holds, and it refuses to take the usage below zero.
#[tokio::test]
async fn a_release_never_waits_for_a_lease_and_never_goes_below_zero() {
    bounded::within("release", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 5_000)
            .await
            .expect("a charge");
        let (mut holder, _) = writer(pool).await;
        charge_on(&mut holder, tenant, 1)
            .await
            .expect("the holder's lease");
        let mut held = sqlx::Connection::begin(&mut *holder).await.expect("begin");
        charge(&mut held, tenant, 1_000)
            .await
            .expect("a lease held, no refill");
        let release = |bytes: i64| async move {
            let mut tx = pool.begin().await?;
            sqlx::query("SELECT spike_041.step_log_release($1, $2)")
                .bind(tenant)
                .bind(bytes)
                .execute(&mut *tx)
                .await?;
            tx.commit().await
        };
        let released = tokio::time::timeout(Duration::from_secs(20), release(5_001)).await;
        assert!(
            matches!(released, Ok(Ok(()))),
            "a release waited for a held lease or failed: {released:?}"
        );
        let under = release(1).await;
        assert!(
            matches!(&under, Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("22000")),
            "a release below zero was applied: {under:?}"
        );
        held.commit().await.expect("commit");
        let (s, _, _, used, _) = ledger(pool, tenant).await;
        assert_eq!(s + used, 1_000, "the usage is what is still stored");
    })
    .await;
}

/// D041-117: a carry whose writer's lease is short waits for the usage row a transaction holds (one that updated it, as
/// every real holder does) and writes once the holder ends: a tenant with room never loses the carry's records.
#[tokio::test]
async fn a_carry_whose_refill_meets_a_held_usage_row_waits_and_writes() {
    bounded::within("carry waits", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        without_the_refill_bound(pool).await;
        sqlx::query(
            "INSERT INTO tenant_retained_usage (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .execute(pool)
        .await
        .expect("the usage row");
        let mut row = pool.begin().await.expect("begin");
        sqlx::query(
            "UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes WHERE tenant_id = $1",
        )
        .bind(tenant)
        .execute(&mut *row)
        .await
        .expect("hold the usage row, updated");
        let (mut conn, pid) = writer(pool).await;
        let run = Uuid::new_v4();
        let carry = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *conn).await?;
            let outcome = carry_call(tenant, run, Uuid::new_v4())
                .fetch_one(&mut *tx)
                .await?;
            let lines = run_lines(&mut tx, run).await;
            tx.commit().await?;
            Ok::<_, sqlx::Error>((outcome, lines))
        });
        waits_on_a_lock(pool, pid).await;
        row.commit().await.expect("the holder ends");
        let (outcome, lines) = carry.await.expect("join").expect("the carry");
        assert_eq!(outcome, "written:refill");
        assert_eq!(lines, (1, 10), "the stream and its ten lines");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

/// D041-113: a long step's carry refused by the limit records its final mark and its drops on the existing stream.
#[tokio::test]
async fn a_long_steps_carry_refused_by_the_limit_records_its_mark_on_its_stream() {
    bounded::within("long step refused", LIMIT, async {
        let (w, tenant) = leased_world(100).await;
        super::install_third_run_storage(&w.db.pool).await;
        let pool = &w.db.pool;
        let (run, step) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query(
            "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
               instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
             VALUES ($1, gen_random_uuid(), gen_random_uuid(), $2, $3, 'a', '{}', '{}', 1, 0, 14, clock_timestamp(), \
               clock_timestamp() + interval '14 days')",
        )
        .bind(tenant)
        .bind(run)
        .bind(step)
        .execute(pool)
        .await
        .expect("a long step's stream");
        let mut tx = pool.begin().await.expect("begin");
        let outcome = carry_call(tenant, run, step).fetch_one(&mut *tx).await.expect("the carry answers");
        assert_eq!(outcome, "mark_only:quota");
        tx.commit().await.expect("the report's transaction commits");
        let (reason, dropped, final_set, lines): (Option<String>, i64, bool, i64) = sqlx::query_as(
            "SELECT ceiling_reason, dropped_by_server, final_at IS NOT NULL, \
                    (SELECT count(*) FROM spike_041.step_log_lines WHERE run_id = $1) \
               FROM spike_041.step_log_streams WHERE run_id = $1",
        )
        .bind(run)
        .fetch_one(pool)
        .await
        .expect("the stream");
        assert_eq!((reason.as_deref(), dropped, final_set, lines), (Some("tenant_quota"), 10, true, 0));
    })
    .await;
}

/// D041-113: a carry charged into a lease it reserved writes with no subtransaction (its rows carry the transaction's own
/// id); a carry that needs a refill writes inside the block (its rows carry a subtransaction's id).
#[tokio::test]
async fn only_a_carry_that_needs_a_refill_opens_a_subtransaction() {
    bounded::within("fast path", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        let own = |run: Uuid| {
            sqlx::query_scalar::<_, bool>(
                "SELECT bool_and(xmin = pg_current_xact_id()::xid) FROM spike_041.step_log_lines WHERE run_id = $1",
            )
            .bind(run)
        };
        let (mut conn, _) = writer(pool).await;
        let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
        let first = Uuid::new_v4();
        carry_call(tenant, first, Uuid::new_v4()).fetch_one(&mut *tx).await.expect("a first carry");
        assert!(!own(first).fetch_one(&mut *tx).await.expect("xmin"), "the refill's carry wrote in a subtransaction");
        tx.commit().await.expect("commit");
        let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("begin");
        let second = Uuid::new_v4();
        carry_call(tenant, second, Uuid::new_v4()).fetch_one(&mut *tx).await.expect("a second carry");
        assert!(own(second).fetch_one(&mut *tx).await.expect("xmin"), "a fast-path carry opened a subtransaction");
        tx.commit().await.expect("commit");
    })
    .await;
}

/// D041-112: with S + G past the limit and no reclaim by a limit change (the trigger disabled, as when the default limit
/// changes in a release), a charge goes to the refill, which reclaims and admits what fits.
#[tokio::test]
async fn a_charge_past_the_leased_room_reclaims_before_it_refuses() {
    bounded::within("past the leased room", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 0: 1 MiB granted");
        for statement in [
            "ALTER TABLE quota_limits DISABLE TRIGGER quota_limits_step_log_reclaim",
            "ALTER TABLE quota_ceilings DISABLE TRIGGER quota_ceilings_step_log_reclaim",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .expect("no reclaim on the change");
        }
        w.quotas
            .set(
                &w.admin,
                None,
                QuotaResource::TenantStepLogBytes,
                10_000,
                &ctx(),
            )
            .await
            .expect("lower the limit below the leased room");
        let (_, g, _, _, _) = ledger(pool, tenant).await;
        assert!(g > 10_000, "the leased room is past the new limit");
        charge_committed(pool, tenant, 2_000)
            .await
            .expect("what fits is admitted after a reclaim");
        assert!(
            refused(charge_committed(pool, tenant, 7_001).await),
            "past the limit is refused"
        );
        let (s, g, granted, used, _) = ledger(pool, tenant).await;
        assert_eq!(g, granted);
        assert_eq!(s + used, 3_000);
    })
    .await;
}

/// D041-112: a carry refused by the tenant's limit records a mark-only stream in the same call, and its transaction
/// commits: no re-run of the report.
#[tokio::test]
async fn a_carry_refused_by_the_limit_records_its_mark_in_the_same_call() {
    bounded::within("carry refused", LIMIT, async {
        let (w, tenant) = leased_world(100).await;
        super::install_third_run_storage(&w.db.pool).await;
        let pool = &w.db.pool;
        let mut tx = pool.begin().await.expect("begin");
        let run = Uuid::new_v4();
        let outcome = carry_call(tenant, run, Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("the carry answers");
        assert_eq!(outcome, "mark_only:quota");
        assert_eq!(run_lines(&mut tx, run).await, (1, 0), "the mark-only stream, no line kept");
        tx.commit().await.expect("the report's transaction commits");
        let (reason, dropped, final_set): (Option<String>, i64, bool) = sqlx::query_as(
            "SELECT ceiling_reason, dropped_by_server, final_at IS NOT NULL FROM spike_041.step_log_streams",
        )
        .fetch_one(pool)
        .await
        .expect("the mark-only stream");
        assert_eq!((reason.as_deref(), dropped, final_set), (Some("tenant_quota"), 10, true));
    })
    .await;
}

/// T041-35's outcome split: a carry refused while the room it needs is in a lease an open transaction holds records its
/// mark as `mark_only:held`, told apart from a refusal by the limit (`mark_only:quota`). L = 2 MiB; writer A leases
/// 1 MiB (F / 2) with a charge of 1 byte and then holds its lease in an open transaction; writer B charges
/// 2,097,152 − 1,048,576 − 2,000 = 1,046,576 (its grant, past min(1 MiB, F / 4)), leaving 2,000 unleased; the carry, on a
/// third writer, needs 10 × (200 + 128) = 3,280: more than 2,000 once B's idle lease is folded, less than 2,000 plus A's
/// held room.
#[tokio::test]
async fn a_carry_refused_while_the_room_is_in_a_held_lease_says_held() {
    bounded::within("carry held", LIMIT, async {
        let (w, tenant) = leased_world(2_097_152).await;
        super::install_third_run_storage(&w.db.pool).await;
        let pool = &w.db.pool;
        let ((mut a, _), (mut b, _)) = (writer(pool).await, writer(pool).await);
        charge_on(&mut a, tenant, 1).await.expect("A's lease");
        let mut holder = sqlx::Connection::begin(&mut *a).await.expect("begin");
        charge(&mut holder, tenant, 1)
            .await
            .expect("A holds its lease");
        charge_on(&mut b, tenant, 2_097_152 - 1_048_576 - 2_000)
            .await
            .expect("all but 2,000 bytes and A's lease");
        let (mut c, _) = writer(pool).await;
        let mut tx = sqlx::Connection::begin(&mut *c).await.expect("begin");
        let run = Uuid::new_v4();
        let outcome = carry_call(tenant, run, Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("the carry answers");
        assert_eq!(outcome, "mark_only:held");
        assert_eq!(
            run_lines(&mut tx, run).await,
            (1, 0),
            "the mark-only stream, no line kept"
        );
        tx.commit().await.expect("the report's transaction commits");
        holder.rollback().await.expect("rollback");
        assert_exact(pool, tenant, 2_097_152).await;
    })
    .await;
}

/// T041-35's outcome split: each answer of the carry lands in one counter, a refill's write in `written` too.
#[test]
fn the_carry_hook_counts_each_outcome_in_its_own_counter() {
    let hook = super::CarryHook::new(10, 200);
    for outcome in [
        "written",
        "written:refill",
        "mark_only:quota",
        "mark_only:held",
        "mark_only:held",
        "mark_only:busy",
        "mark_only:busy",
        "mark_only:busy",
        "contended",
        "contended:busy",
        "off",
    ] {
        hook.count(outcome);
    }
    let c = hook.counts();
    assert_eq!(
        (
            c.written,
            c.written_refill,
            c.mark_only_quota,
            c.mark_only_held,
            c.mark_only_busy,
            c.contended,
            c.neither
        ),
        (2, 1, 1, 2, 3, 2, 1)
    );
}

/// The tests that hold the usage row while they stage other transactions replace the refill's budget in their own
/// database with the test's bound, so that no result depends on how fast the machine stages them (the budget has its
/// own test, `a_refill_past_its_budget_answers_busy_and_the_report_lands`).
async fn without_the_refill_bound(pool: &PgPool) {
    sqlx::query(
        "CREATE OR REPLACE FUNCTION spike_041.step_log_refill_wait() RETURNS interval LANGUAGE sql IMMUTABLE \
           SET search_path = pg_catalog, pg_temp AS $$ SELECT interval '60 seconds' $$",
    )
    .execute(pool)
    .await
    .expect("the test's bound");
}

/// A stream of the tenant for direct line inserts (an ingest batch), committed.
async fn a_stream(pool: &PgPool, tenant: Uuid) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
           instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
         VALUES ($1, gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'a', '{}', '{}', \
           1, 0, 14, now(), now() + interval '14 days') RETURNING stream_id",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("a stream")
}

/// Ten lines of 200 bytes on `stream` from `from`: an ingest batch, its charge by the statement trigger.
async fn ingest(
    conn: &mut PgConnection,
    tenant: Uuid,
    stream: i64,
    from: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
         SELECT $1, $2, s, gen_random_uuid(), 'info', 'line', repeat('x', 200) FROM generate_series($3, $3 + 9) s",
    )
    .bind(tenant)
    .bind(stream)
    .bind(from)
    .execute(conn)
    .await
    .map(|_| ())
}

async fn usage_row(pool: &PgPool, tenant: Uuid) {
    sqlx::query("INSERT INTO tenant_retained_usage (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(tenant)
        .execute(pool)
        .await
        .expect("the usage row");
}

/// The coordinator's case (a), D041-117: a completion that took the usage row for its content bytes before its own carry
/// refills goes through, while another writer's refill waits for it; neither deadlocks.
#[tokio::test]
async fn a_completion_holding_the_usage_row_refills_while_another_refill_waits() {
    bounded::within("deadlock a1", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        without_the_refill_bound(pool).await;
        usage_row(pool, tenant).await;
        let ((mut a, _), (mut b, pid_b)) = (writer(pool).await, writer(pool).await);
        let mut first = sqlx::Connection::begin(&mut *a).await.expect("begin");
        sqlx::query("UPDATE tenant_retained_usage SET content_bytes = content_bytes + 100 WHERE tenant_id = $1")
            .bind(tenant)
            .execute(&mut *first)
            .await
            .expect("A's content bytes, on the usage row");
        let run_b = Uuid::new_v4();
        let other = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *b).await?;
            let outcome = carry_call(tenant, run_b, Uuid::new_v4()).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(outcome)
        });
        waits_on_a_lock(pool, pid_b).await;
        let mine = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *first)
            .await
            .expect("A's carry refills under its own lock of the row");
        assert_eq!(mine, "written:refill");
        first.commit().await.expect("A commits");
        let theirs = other.await.expect("join");
        assert!(matches!(&theirs, Ok(o) if o == "written:refill"), "B: {theirs:?}");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

/// The coordinator's case (a), the other order (D041-117): A holds its own lease on the fast path and then waits for the
/// usage row (its content bytes) that B's refill holds near the limit; B's reclaim skips A's lease, so B never waits for
/// A and both commit. L = 1.5 MiB: A's grant is F / 2 = 786,432, leaving 786,432 unleased, under 1 MiB, so B's refill
/// reclaims and meets A's held lease.
#[tokio::test]
async fn a_writer_holding_its_lease_waits_for_a_reclaiming_refill_without_a_deadlock() {
    bounded::within("deadlock a2", LIMIT, async {
        const L: i64 = 1_572_864;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        super::install_third_run_storage(&w.db.pool).await;
        let pool = &w.db.pool;
        let ((mut a, pid_a), (mut b, _)) = (writer(pool).await, writer(pool).await);
        let warm = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *a)
            .await
            .expect("A's lease");
        assert_eq!(warm, "written:refill");
        sqlx::query("BEGIN").execute(&mut *a).await.expect("begin");
        let fast = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *a)
            .await
            .expect("A's fast path");
        assert_eq!(fast, "written", "A holds its own lease");
        let mut near = sqlx::Connection::begin(&mut *b).await.expect("begin");
        charge(&mut near, tenant, 100_000)
            .await
            .expect("B refills near the limit and reclaims what is idle, skipping A's lease");
        let content = tokio::spawn(async move {
            sqlx::query("UPDATE tenant_retained_usage SET content_bytes = content_bytes + 100 WHERE tenant_id = $1")
                .bind(tenant)
                .execute(&mut *a)
                .await?;
            sqlx::query("COMMIT").execute(&mut *a).await.map(|_| ())
        });
        waits_on_a_lock(pool, pid_a).await;
        near.commit().await.expect("B commits");
        content.await.expect("join").expect("A commits once B is done");
        assert_exact(pool, tenant, L).await;
    })
    .await;
}

/// The coordinator's case (b), D041-117: an ingest batch's refill and a carry's refill, each order; the second waits for
/// the first's commit and both write.
#[tokio::test]
async fn an_ingest_refill_and_a_carry_refill_wait_for_each_other_in_turn() {
    bounded::within("deadlock b", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        without_the_refill_bound(pool).await;
        usage_row(pool, tenant).await;
        let stream = a_stream(pool, tenant).await;
        // The ingest first.
        let ((mut i, _), (mut c, pid_c)) = (writer(pool).await, writer(pool).await);
        let mut batch = sqlx::Connection::begin(&mut *i).await.expect("begin");
        ingest(&mut batch, tenant, stream, 1)
            .await
            .expect("the batch refills");
        let carry = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            let outcome = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(outcome)
        });
        waits_on_a_lock(pool, pid_c).await;
        batch.commit().await.expect("the batch commits");
        let outcome = carry.await.expect("join");
        assert!(
            matches!(&outcome, Ok(o) if o == "written:refill"),
            "{outcome:?}"
        );
        // The carry first.
        let ((mut c2, _), (mut i2, pid_i)) = (writer(pool).await, writer(pool).await);
        let mut report = sqlx::Connection::begin(&mut *c2).await.expect("begin");
        let first = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *report)
            .await
            .expect("the carry refills");
        assert_eq!(first, "written:refill");
        let batch = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *i2).await?;
            ingest(&mut tx, tenant, stream, 11).await?;
            tx.commit().await
        });
        waits_on_a_lock(pool, pid_i).await;
        report.commit().await.expect("the report commits");
        batch.await.expect("join").expect("the batch commits");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

/// The coordinator's case (c), D041-117: a carry's refill waits behind a release that holds the usage row, while a change
/// of the limit waits for the fence the carry holds; the release ends, the carry writes and commits, the change reclaims
/// every lease; nothing deadlocks.
#[tokio::test]
async fn a_release_and_a_limit_change_around_a_waiting_refill_end_without_a_deadlock() {
    bounded::within("deadlock c", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let w = Arc::new(w);
        let pool = w.db.pool.clone();
        without_the_refill_bound(&pool).await;
        let (mut seed, _) = writer(&pool).await;
        charge_on(&mut seed, tenant, 5_000)
            .await
            .expect("usage to release");
        let mut release = pool.begin().await.expect("begin");
        sqlx::query("SELECT spike_041.step_log_release($1, $2)")
            .bind(tenant)
            .bind(1_000_i64)
            .execute(&mut *release)
            .await
            .expect("the release holds the usage row");
        let (mut c, pid_c) = writer(&pool).await;
        let carry = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            let outcome = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(outcome)
        });
        waits_on_a_lock(&pool, pid_c).await;
        let change = {
            let w = Arc::clone(&w);
            tokio::spawn(async move {
                w.quotas
                    .set(
                        &w.admin,
                        None,
                        QuotaResource::TenantStepLogBytes,
                        500 << 20,
                        &ctx(),
                    )
                    .await
                    .map(|_| ())
            })
        };
        a_backend_waits_on_a_fence(&pool).await;
        release.commit().await.expect("the release commits");
        let outcome = carry.await.expect("join");
        assert!(
            matches!(&outcome, Ok(o) if o == "written:refill"),
            "{outcome:?}"
        );
        change
            .await
            .expect("join")
            .expect("the change of the limit commits");
        let (_, g, granted, _, _) = ledger(&pool, tenant).await;
        assert_eq!((g, granted), (0, 0), "the change reclaimed every lease");
        assert_exact(&pool, tenant, 500 << 20).await;
    })
    .await;
}

/// D041-119: a fallback refill inside a report waits for the usage row at most its budget (200 ms), then the carry
/// answers in the same call with a mark-only stream that says why (`tenant_quota_busy`), and the report commits: never a
/// failed report, never a silent loss. No setting of the caller lengthens the budget: neither a custom setting nor a
/// `lock_timeout` of 0.
#[tokio::test]
async fn a_refill_past_its_budget_answers_busy_and_the_report_lands() {
    bounded::within("refill budget", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        usage_row(pool, tenant).await;
        let mut row = pool.begin().await.expect("begin");
        sqlx::query(
            "UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes WHERE tenant_id = $1",
        )
        .bind(tenant)
        .execute(&mut *row)
        .await
        .expect("hold the usage row");
        let (mut c, _) = writer(pool).await;
        let mut tx = sqlx::Connection::begin(&mut *c).await.expect("begin");
        sqlx::query("SET LOCAL sagat.step_log_refill_wait = '1h'")
            .execute(&mut *tx)
            .await
            .expect("a setting that must not lengthen the budget");
        sqlx::query("SET LOCAL lock_timeout = 0")
            .execute(&mut *tx)
            .await
            .expect("no timeout of the caller's own");
        let run = Uuid::new_v4();
        let outcome = carry_call(tenant, run, Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("the carry answers");
        assert_eq!(outcome, "mark_only:busy");
        assert_eq!(
            run_lines(&mut tx, run).await,
            (1, 0),
            "the mark-only stream, no line"
        );
        let reason: Option<String> = sqlx::query_scalar(
            "SELECT ceiling_reason FROM spike_041.step_log_streams WHERE run_id = $1",
        )
        .bind(run)
        .fetch_one(&mut *tx)
        .await
        .expect("the stream");
        assert_eq!(
            reason.as_deref(),
            Some("tenant_quota_busy"),
            "the reader is told why"
        );
        tx.commit().await.expect("the report commits");
        row.commit().await.expect("commit");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

/// D041-119: a refill keeps a caller's lower `lock_timeout` (an ingest's 200 ms, here 150 ms) and restores the caller's
/// own value after its lock, whatever it was.
#[tokio::test]
async fn a_refill_restores_the_callers_lock_timeout() {
    bounded::within("lock timeout restored", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        for setting in ["150ms", "0"] {
            let (mut c, _) = writer(pool).await;
            let mut tx = sqlx::Connection::begin(&mut *c).await.expect("begin");
            sqlx::query("SELECT set_config('lock_timeout', $1, true)")
                .bind(setting)
                .execute(&mut *tx)
                .await
                .expect("the caller's timeout");
            let before: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
                .fetch_one(&mut *tx)
                .await
                .expect("before");
            charge(&mut tx, tenant, 1_000).await.expect("a refill");
            let after: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
                .fetch_one(&mut *tx)
                .await
                .expect("after");
            assert_eq!(after, before, "the caller's lock_timeout after a refill");
            tx.commit().await.expect("commit");
        }
    })
    .await;
}

/// D041-119: the ensure before a report gives the writer's lease room for the planned carry in its own transaction, so
/// the carry then takes the fast path; with room already there it returns without touching the usage row.
#[tokio::test]
async fn the_ensure_before_a_report_gives_the_carry_its_fast_path() {
    bounded::within("ensure", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        let (mut c, _) = writer(pool).await;
        sqlx::query("SELECT spike_041.step_log_ensure($1, 3280)")
            .bind(tenant)
            .execute(&mut *c)
            .await
            .expect("the ensure");
        let mut tx = sqlx::Connection::begin(&mut *c).await.expect("begin");
        let outcome = carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("the carry");
        assert_eq!(outcome, "written", "the carry found its room");
        tx.commit().await.expect("commit");
        let mut row = pool.begin().await.expect("begin");
        sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
            .bind(tenant)
            .execute(&mut *row)
            .await
            .expect("hold the usage row");
        let again = tokio::time::timeout(
            Duration::from_secs(20),
            sqlx::query("SELECT spike_041.step_log_ensure($1, 3280)")
                .bind(tenant)
                .execute(&mut *c),
        )
        .await;
        assert!(
            matches!(again, Ok(Ok(_))),
            "an ensure with room waited for the usage row: {again:?}"
        );
        row.commit().await.expect("commit");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}

/// D041-119: the idle reclaim holds the step-log fence shared while it holds leases, so a change of the limit made
/// meanwhile waits for it (and then reclaims) instead of failing on its `NOWAIT`.
#[tokio::test]
async fn a_limit_change_waits_for_an_open_idle_reclaim_instead_of_failing() {
    bounded::within("idle reclaim and limit change", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let w = Arc::new(w);
        let pool = w.db.pool.clone();
        let (mut idle, pid_idle) = writer(&pool).await;
        charge_on(&mut idle, tenant, 1_000).await.expect("a lease");
        sqlx::query("UPDATE spike_041.step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' WHERE writer = $1")
            .bind(pid_idle)
            .execute(&pool)
            .await
            .expect("idle");
        let mut reclaim = pool.begin().await.expect("begin");
        let n: i32 = sqlx::query_scalar("SELECT spike_041.step_log_reclaim_idle($1)")
            .bind(tenant)
            .fetch_one(&mut *reclaim)
            .await
            .expect("the idle reclaim, open");
        assert_eq!(n, 1);
        let change = {
            let w = Arc::clone(&w);
            tokio::spawn(async move {
                w.quotas
                    .set(&w.admin, None, QuotaResource::TenantStepLogBytes, 500 << 20, &ctx())
                    .await
                    .map(|_| ())
            })
        };
        a_backend_waits_on_a_fence(&pool).await;
        reclaim.commit().await.expect("the reclaim commits");
        change.await.expect("join").expect("the change of the limit commits");
        assert_exact(&pool, tenant, 500 << 20).await;
    })
    .await;
}

/// The coordinating session's held-room bound, D041-119: k writers each take a fresh grant one after another and hold
/// their leases open; at least F₀ · C(2k, k) / 4^k of the room that was free before them stays chargeable by another
/// writer (each grant takes at most F / (2(A + 1)) with A ≥ the writers before it, so F_i ≥ F_{i−1} (1 − 1/(2i)), and
/// the product of (2i − 1)/(2i) over i = 1..k is C(2k, k) / 4^k), and only past what is free is a charge refused as
/// held. Started with a quarter of L already used, for k = 1, 2, 8 and 32.
#[tokio::test]
async fn room_held_by_open_writers_is_bounded_from_the_free_room() {
    bounded::within("held bound from F0", Duration::from_secs(240), async {
        for k in [1_u32, 2, 8, 32] {
            const L: i64 = 1024 * 1024;
            let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
            let pool = &w.db.pool;
            usage_row(pool, tenant).await;
            sqlx::query(
                "UPDATE tenant_retained_usage SET step_log_bytes = $2 WHERE tenant_id = $1",
            )
            .bind(tenant)
            .bind(L / 4)
            .execute(pool)
            .await
            .expect("a quarter of L already used");
            let f0 = u128::try_from(L - L / 4).expect("positive");
            let mut held = Vec::new();
            for _ in 0..k {
                let (mut conn, _) = writer(pool).await;
                charge_on(&mut conn, tenant, 1)
                    .await
                    .expect("a writer leases");
                sqlx::query("BEGIN")
                    .execute(&mut *conn)
                    .await
                    .expect("begin");
                charge(&mut conn, tenant, 1)
                    .await
                    .expect("the writer holds its lease");
                held.push(conn);
            }
            let (s, g, _, _, _) = ledger(pool, tenant).await;
            let free = L - s - g;
            // C(2k, k) / 4^k, the product of (2i − 1) / (2i), exactly: C(64, 32) and 4^32 fit in u128.
            let k = u128::from(k);
            let mut binomial = 1_u128;
            for i in 0..k {
                binomial = binomial * (2 * k - i) / (i + 1);
            }
            let floor = i64::try_from(f0 * binomial / 4_u128.pow(u32::try_from(k).expect("small")))
                .expect("fits");
            assert!(
                free >= floor,
                "k = {k}: the unleased room {free} is under the bound {floor}"
            );
            let (mut other, _) = writer(pool).await;
            let past = charge_on(&mut other, tenant, free + 1).await;
            assert_eq!(
                refusal(&past).as_deref(),
                Some("tenant_step_log_quota_held"),
                "k = {k}: {past:?}"
            );
            charge_on(&mut other, tenant, free)
                .await
                .expect("the unleased room is chargeable while every lease is held");
            for mut conn in held {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .expect("commit");
            }
            assert_exact(pool, tenant, L).await;
        }
    })
    .await;
}

/// D041-117's idle reclaim (the purge pass, one tenant a transaction): a lease not charged for longer than the interval
/// is folded back (S += used, G −= granted) and deleted; a fresh one and one an open transaction holds stay.
#[tokio::test]
async fn the_idle_reclaim_folds_and_deletes_only_leases_idle_past_the_interval() {
    bounded::within("idle reclaim", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        let ((mut idle, pid_idle), (mut fresh, pid_fresh), (mut busy, pid_busy)) =
            (writer(pool).await, writer(pool).await, writer(pool).await);
        charge_on(&mut idle, tenant, 1_000).await.expect("the idle writer's lease");
        charge_on(&mut fresh, tenant, 2_000).await.expect("the fresh writer's lease");
        charge_on(&mut busy, tenant, 3_000).await.expect("the busy writer's lease");
        sqlx::query(
            "UPDATE spike_041.step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' \
              WHERE tenant_id = $1 AND writer = ANY($2)",
        )
        .bind(tenant)
        .bind(vec![pid_idle, pid_busy])
        .execute(pool)
        .await
        .expect("two leases idle past the interval");
        let mut held = sqlx::Connection::begin(&mut *busy).await.expect("begin");
        sqlx::query("SELECT 1 FROM spike_041.step_log_quota_leases WHERE tenant_id = $1 AND writer = $2 FOR UPDATE")
            .bind(tenant)
            .bind(pid_busy)
            .execute(&mut *held)
            .await
            .expect("an open transaction holds the busy writer's lease");
        let (s0, g0, _, used0, _) = ledger(pool, tenant).await;
        let mut tx = pool.begin().await.expect("begin");
        let reclaimed: i32 = sqlx::query_scalar("SELECT spike_041.step_log_reclaim_idle($1)")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await
            .expect("the idle reclaim");
        tx.commit().await.expect("commit");
        held.commit().await.expect("commit");
        assert_eq!(reclaimed, 1, "only the idle lease no transaction holds");
        let writers: Vec<i32> = sqlx::query_scalar(
            "SELECT writer FROM spike_041.step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer",
        )
        .bind(tenant)
        .fetch_all(pool)
        .await
        .expect("the leases");
        let mut kept = vec![pid_fresh, pid_busy];
        kept.sort_unstable();
        assert_eq!(writers, kept);
        let (s, g, _, used, _) = ledger(pool, tenant).await;
        assert_eq!(s + used, s0 + used0, "the usage is unchanged");
        assert!(g < g0 && s == s0 + 1_000, "the idle lease's room and use were folded");
        assert_exact(pool, tenant, 1 << 30).await;
    })
    .await;
}
