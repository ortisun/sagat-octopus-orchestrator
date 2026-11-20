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

#[tokio::test]
async fn a_charge_skips_a_lease_another_transaction_holds_and_never_waits_for_it() {
    bounded::within("held lease", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("a first charge leases 0");
        let mut held = pool.begin().await.expect("begin");
        charge(&mut held, tenant, 1_000)
            .await
            .expect("the held transaction takes lease 0");
        // Lease 0 has room but is held: the second charge must neither wait for it nor fail.
        let second = tokio::time::timeout(
            Duration::from_secs(20),
            charge_committed(pool, tenant, 1_000),
        )
        .await;
        assert!(
            matches!(second, Ok(Ok(()))),
            "a held lease made a charge wait or fail: {second:?}"
        );
        held.commit().await.expect("commit");
        let (s, g, granted, used, leases) = ledger(pool, tenant).await;
        assert_eq!(g, granted, "G is the sum of the grants");
        assert_eq!(s + used, 3_000, "every charged byte is counted once");
        assert_eq!(leases, 2, "the held lease was skipped, a second one leased");
    })
    .await;
}

#[tokio::test]
async fn a_charge_with_a_free_lease_never_takes_the_usage_row_and_a_refill_alone_waits_for_it() {
    bounded::within("usage row", LIMIT, async {
        let (w, tenant) = leased_world(1 << 30).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 0 granted");
        let mut row = pool.begin().await.expect("begin");
        sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
            .bind(tenant)
            .execute(&mut *row)
            .await
            .expect("hold the usage row");
        let free = tokio::time::timeout(
            Duration::from_secs(20),
            charge_committed(pool, tenant, 1_000),
        )
        .await;
        assert!(
            matches!(free, Ok(Ok(()))),
            "a charge into a free lease took the usage row: {free:?}"
        );
        // A charge larger than any lease's room needs a refill, which is the one wait.
        let mut refill_conn = pool.acquire().await.expect("a connection");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *refill_conn)
            .await
            .expect("pid");
        let refill = tokio::spawn(async move {
            let mut tx = sqlx::Connection::begin(&mut *refill_conn).await?;
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

#[tokio::test]
async fn the_last_bytes_of_the_limit_are_admitted_from_the_room_of_idle_leases() {
    bounded::within("reclaim", LIMIT, async {
        // 2 MiB: far from the limit a grant is 1/64 of the free room, so the first two leases keep room.
        const L: i64 = 2 * 1024 * 1024;
        let (w, tenant) = leased_world(u64::try_from(L).expect("positive")).await;
        let pool = &w.db.pool;
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 0: 32,768 granted");
        let mut held = pool.begin().await.expect("begin");
        charge(&mut held, tenant, 1).await.expect("lease 0 held");
        charge_committed(pool, tenant, 1_000)
            .await
            .expect("lease 1: 32,256 granted");
        held.commit().await.expect("commit");
        let (s, g, granted, used, leases) = ledger(pool, tenant).await;
        assert_eq!((s, g, granted, used, leases), (0, 65_024, 65_024, 2_001, 2));
        // The room left (2,095,151) is more than the unleased room (2,032,128): only the leases' room completes it.
        charge_committed(pool, tenant, L - 2_001)
            .await
            .expect("the room of idle leases is given back near the limit");
        assert!(
            refused(charge_committed(pool, tenant, 1).await),
            "one byte past the limit was admitted"
        );
        let (s, g, granted, used, _) = ledger(pool, tenant).await;
        assert_eq!(g, granted);
        assert_eq!(s + used, L);
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
type QuotaRows = (Vec<(i16, i64, i64)>, Option<(i64, i64)>, i64);

async fn quota_rows(conn: &mut PgConnection, tenant: Uuid) -> QuotaRows {
    let leases: Vec<(i16, i64, i64)> = sqlx::query_as(
        "SELECT lease, granted, used FROM spike_041.step_log_quota_leases WHERE tenant_id = $1 ORDER BY lease",
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
        // The first insert finds no lease: a refill.
        let mut tx = pool.begin().await.expect("begin");
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
            let mut tx = pool.begin().await.expect("begin");
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
        // A first carry leases (a refill), so that the measured one charges a free lease.
        let mut tx = pool.begin().await.expect("begin");
        carry_call(tenant, Uuid::new_v4(), Uuid::new_v4())
            .fetch_one(&mut *tx)
            .await
            .expect("a first carry");
        tx.commit().await.expect("commit");
        for _ in 0..8 {
            let evicted = evictions(pool).await;
            let mut tx = pool.begin().await.expect("begin");
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
            // The function's seven statements (the application name, the no-wait setting, the policy read, the usage
            // read, the plan's size, the stream insert, the lines insert), the stream's path check
            // (`execution_instance_path_valid`, a SQL function its CHECK calls), the charge's six and the ten
            // foreign-key checks. Derived first as 22 (seen failing at 23: the path check had been left out), then
            // with D041-112's no-wait setting as 25 (seen failing at 24: the new stream's path returns before a reset,
            // which was then removed as needless: the setting is transaction-local and the carry is the report's last
            // statement).
            assert_eq!(during - before, 7 + 1 + 6 + 10, "the nested statements of a short step's carry");
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
        assert_eq!(outcome, "written");
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
        let mut held = pool.begin().await.expect("begin");
        charge(&mut held, tenant, 1_000)
            .await
            .expect("a lease held");
        let release = |bytes: i64| async move {
            let mut tx = pool.begin().await?;
            sqlx::query("SELECT spike_041.step_log_release($1, $2)")
                .bind(tenant)
                .bind(bytes)
                .execute(&mut *tx)
                .await?;
            tx.commit().await
        };
        let released = tokio::time::timeout(Duration::from_secs(20), release(5_000)).await;
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

/// D041-112: a carry whose charge needs a refill while the usage row is held does not wait: it answers `contended` and
/// writes nothing, and its transaction commits.
#[tokio::test]
async fn a_carry_never_waits_for_the_usage_row() {
    bounded::within("carry nowait", LIMIT, async {
        let (w, tenant) = third_run_world().await;
        let pool = &w.db.pool;
        sqlx::query(
            "INSERT INTO tenant_retained_usage (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .execute(pool)
        .await
        .expect("the usage row");
        let mut row = pool.begin().await.expect("begin");
        sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
            .bind(tenant)
            .execute(&mut *row)
            .await
            .expect("hold the usage row");
        let mut tx = pool.begin().await.expect("begin");
        let run = Uuid::new_v4();
        let outcome = tokio::time::timeout(
            Duration::from_secs(20),
            carry_call(tenant, run, Uuid::new_v4()).fetch_one(&mut *tx),
        )
        .await;
        assert!(
            matches!(&outcome, Ok(Ok(o)) if o == "contended"),
            "the carry waited or did not answer contended: {outcome:?}"
        );
        assert_eq!(run_lines(&mut tx, run).await, (0, 0), "nothing stored");
        tx.commit().await.expect("the report's transaction commits");
        row.commit().await.expect("commit");
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
        assert_eq!(outcome, "mark_only");
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
