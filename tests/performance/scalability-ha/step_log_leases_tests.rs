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
