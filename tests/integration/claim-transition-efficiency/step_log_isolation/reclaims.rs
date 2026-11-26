//! The reclaims of ADR-0097 (decisions 1, 5 and 6): the change of a limit, the idle and ended-backend reclaim of the purge
//! pass, the release that never waits for a quota lease, and the deadlock cases of design 041 section 2b.

use std::sync::Arc;

use sqlx::Connection as _;

use super::{
    Fx, LIMIT, MIB, ROW, a_backend_waits_on_a_fence, exceeded, lease_of, refusal, set_limit,
    waits_on,
};
use crate::support::bounded;

async fn counters(fx: &Fx) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT refills, reclaims, idle, ended, lease_rows FROM sagat_quota_step_log_counters()",
    )
    .fetch_one(fx.pool())
    .await
    .expect("the counters")
}

/// A change of the limit (here a raise) reclaims every quota lease, whoever wrote the row: S takes Σ used, G and every
/// lease go to 0, and the reclaim counter advances once a quota lease that held a grant.
#[tokio::test]
async fn a_change_of_the_limit_reclaims_every_quota_lease() {
    bounded::within("limit change", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let (mut a, mut b) = (fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut a.conn, 1_000).await.expect("A");
        fx.charge_committed(&mut b.conn, 2_000).await.expect("B");
        let before = counters(&fx).await;
        let mut tx = fx.pool().begin().await.expect("begin");
        sqlx::query("UPDATE quota_limits SET max_value = $2 WHERE tenant_id = $1 AND resource = 'tenant_step_log_bytes'")
            .bind(fx.tenant)
            .bind(1_i64 << 31)
            .execute(&mut *tx)
            .await
            .expect("a raw write of the limit, no fence taken by the writer");
        tx.commit().await.expect("commit");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!((l.s, l.g, l.granted, l.used, l.leases), (3_000, 0, 0, 0, 2));
        let after = counters(&fx).await;
        assert_eq!(after.1 - before.1, 2, "two quota leases reclaimed");
        assert_eq!(after.0, before.0, "no refill");
    })
    .await;
}

/// A limit lowered above the usage still admits what fits: the change reclaimed every quota lease (S = 1,000, G = 0),
/// 2,000 more fit, the rest is 10,000 − 3,000 = 7,000: a row more is refused and exactly 7,000 is admitted.
#[tokio::test]
async fn a_limit_lowered_above_the_usage_still_admits_what_fits() {
    bounded::within("lowered above usage", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000)
            .await
            .expect("a grant of 1 MiB");
        set_limit(fx.pool(), fx.tenant, 10_000)
            .await
            .expect("lower the limit below the leased room");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            (l.s, l.g, l.used),
            (1_000, 0, 0),
            "the change reclaimed every quota lease"
        );
        fx.charge_committed(&mut w.conn, 2_000)
            .await
            .expect("what fits is admitted");
        assert_eq!(
            refusal(&fx.charge_committed(&mut w.conn, 7_000 + ROW).await),
            exceeded()
        );
        fx.charge_committed(&mut w.conn, 7_000)
            .await
            .expect("the exact rest is admitted");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(l.usage(), 10_000);
        fx.assert_exact(fx.tenant, 10_000).await;
    })
    .await;
}

/// A release (the purge's `DELETE`, its finalizer's step-log branch) never waits for a quota lease another transaction
/// holds, and refuses to take the usage U = S + Σ used below zero (`step_log_usage_underflow`).
#[tokio::test]
async fn a_release_never_waits_for_a_held_quota_lease_and_refuses_an_underflow() {
    bounded::within("release", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut seed = fx.writer().await;
        fx.charge_committed(&mut seed.conn, 5_000).await.expect("a charge to release");
        let mut holder = fx.writer().await;
        fx.charge_committed(&mut holder.conn, ROW).await.expect("the holder's quota lease");
        let mut open = holder.conn.begin().await.expect("begin");
        fx.charge(&mut open, fx.tenant, fx.stream, 1_000).await.expect("a quota lease held, no refill");
        let released = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            sqlx::query("DELETE FROM step_log_lines WHERE tenant_id = $1 AND sequence < $2")
                .bind(fx.tenant)
                .bind(2_i64)
                .execute(fx.pool()),
        )
        .await;
        assert!(matches!(released, Ok(Ok(_))), "a release waited for a held quota lease or failed: {released:?}");
        assert_eq!(fx.ledger(fx.tenant).await.usage(), ROW, "the 5,000 bytes released");
        // The ledger made wrong on purpose: S lowered by the holder's committed 128, so that releasing them would take
        // U below zero.
        sqlx::query("UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes - $2 WHERE tenant_id = $1")
            .bind(fx.tenant)
            .bind(ROW)
            .execute(fx.pool())
            .await
            .expect("a wrong ledger");
        let under = sqlx::query("DELETE FROM step_log_lines WHERE tenant_id = $1 AND sequence = 2")
            .bind(fx.tenant)
            .execute(fx.pool())
            .await
            .map(|_| ());
        assert_eq!(
            refusal(&under),
            Some(("22000".into(), Some("step_log_usage_underflow".into()))),
            "a release below zero was applied"
        );
        open.commit().await.expect("commit");
    })
    .await;
}

/// Deadlock case (a): A holds its own quota lease (a fast-path charge, open); B's refill near the limit reclaims what
/// is idle, skips A's quota lease and holds the usage row; A's next charge needs a refill and waits for that row; B
/// commits and A goes on. L = 1,572,864: A's grant is ⌊L / 2⌋ = 786,432; B's refill of 100,000 finds 786,432 free,
/// under 1 MiB.
#[tokio::test]
async fn a_writer_holding_its_quota_lease_waits_for_a_reclaiming_refill_without_a_deadlock() {
    bounded::within("deadlock a", LIMIT, async {
        const L: i64 = 1_572_864;
        let fx = Fx::new(L).await;
        fx.without_the_refill_bound().await;
        let (mut a, mut b) = (fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut a.conn, 1_000)
            .await
            .expect("A's quota lease");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, a.pid).await,
            Some((786_432, 1_000))
        );
        sqlx::query("BEGIN")
            .execute(&mut a.conn)
            .await
            .expect("begin");
        sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
            .execute(&mut a.conn)
            .await
            .expect("slots");
        fx.charge(&mut a.conn, fx.tenant, fx.stream, 1_000)
            .await
            .expect("A holds its quota lease");
        let b_pid = b.pid;
        let mut near = b.conn.begin().await.expect("begin");
        fx.charge(&mut near, fx.tenant, fx.stream, 100_000)
            .await
            .expect("B refills near the limit, skipping A's quota lease");
        let (tenant, stream, first) = (fx.tenant, fx.stream, fx.sequences(200));
        let a_pid = a.pid;
        let mut a_conn = a.conn;
        let refill = tokio::spawn(async move {
            super::insert_messages(
                &mut a_conn,
                tenant,
                stream,
                first,
                &super::message_lengths(900_000),
            )
            .await?;
            sqlx::query("COMMIT").execute(&mut a_conn).await.map(|_| ())
        });
        waits_on(fx.pool(), a_pid, b_pid).await;
        near.commit().await.expect("B commits");
        refill
            .await
            .expect("join")
            .expect("A's refill reclaims B's idle room and is granted its 900,000");
        // A's refill found S = 0 and G = 983,040: 589,824 free, under 1 MiB, so it reclaimed B's idle quota lease (S =
        // 100,000, G = 786,432), folded its own (S = 102,000, G = 0) and was granted max(900,000, ⌊1,470,864 / 2⌋) =
        // 900,000.
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, a_pid).await,
            Some((900_000, 900_000))
        );
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 1_002_000);
        fx.assert_exact(fx.tenant, L).await;
    })
    .await;
}

/// Deadlock case (c): a writer's refill waits behind a release that holds the usage row (its finalizer run at once),
/// while a change of the limit waits for the step-log fence both hold; the release ends, the refill writes and commits,
/// the change reclaims every quota lease; nothing deadlocks.
#[tokio::test]
async fn a_release_and_a_limit_change_around_a_waiting_refill_end_without_a_deadlock() {
    bounded::within("deadlock c", LIMIT, async {
        let fx = Arc::new(Fx::new(1 << 30).await);
        fx.without_the_refill_bound().await;
        let mut seed = fx.writer().await;
        fx.charge_committed(&mut seed.conn, 5_000)
            .await
            .expect("usage to release");
        let mut release = fx.pool().begin().await.expect("begin");
        let release_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *release)
            .await
            .expect("pid");
        sqlx::query("SET CONSTRAINTS step_log_lines_quota_flush IMMEDIATE")
            .execute(&mut *release)
            .await
            .expect("the finalizer at the statement's end");
        sqlx::query("DELETE FROM step_log_lines WHERE tenant_id = $1")
            .bind(fx.tenant)
            .execute(&mut *release)
            .await
            .expect("the release holds the usage row");
        let Spawned { pid, task } = {
            let mut w = fx.writer().await;
            let (tenant, stream, first) = (fx.tenant, fx.stream, fx.sequences(1));
            let pid = w.pid;
            let task = tokio::spawn(async move {
                let mut tx = w.conn.begin().await?;
                sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
                    .execute(&mut *tx)
                    .await?;
                super::insert_messages(&mut tx, tenant, stream, first, &[872]).await?;
                tx.commit().await
            });
            Spawned { pid, task }
        };
        waits_on(fx.pool(), pid, release_pid).await;
        let change = {
            let fx = Arc::clone(&fx);
            tokio::spawn(async move { set_limit(fx.pool(), fx.tenant, 500 * MIB).await })
        };
        a_backend_waits_on_a_fence(fx.pool()).await;
        release.commit().await.expect("the release commits");
        task.await.expect("join").expect("the refill writes");
        change
            .await
            .expect("join")
            .expect("the change of the limit commits");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            (l.g, l.granted, l.usage()),
            (0, 0, 1_000),
            "the change reclaimed every quota lease"
        );
        fx.assert_exact(fx.tenant, 500 * MIB).await;
    })
    .await;
}

struct Spawned<T> {
    pid: i32,
    task: tokio::task::JoinHandle<T>,
}

/// A change of the limit waits for an open idle reclaim (which holds the step-log fence shared) instead of failing on
/// its `NOWAIT` of the quota leases.
#[tokio::test]
async fn a_limit_change_waits_for_an_open_idle_reclaim_instead_of_failing() {
    bounded::within("idle reclaim and limit change", LIMIT, async {
        let fx = Arc::new(Fx::new(1 << 30).await);
        let mut idle = fx.writer().await;
        fx.charge_committed(&mut idle.conn, 1_000).await.expect("a quota lease");
        sqlx::query("UPDATE step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' WHERE writer = $1")
            .bind(idle.pid)
            .execute(fx.pool())
            .await
            .expect("idle");
        let mut reclaimer = fx.writer().await;
        let mut reclaim = reclaimer.conn.begin().await.expect("begin");
        let (n, skipped): (i32, bool) = sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
            .bind(fx.tenant)
            .fetch_one(&mut *reclaim)
            .await
            .expect("the idle reclaim, open");
        assert_eq!((n, skipped), (1, false));
        let change = {
            let fx = Arc::clone(&fx);
            tokio::spawn(async move { set_limit(fx.pool(), fx.tenant, 500 * MIB).await })
        };
        a_backend_waits_on_a_fence(fx.pool()).await;
        reclaim.commit().await.expect("the reclaim commits");
        change.await.expect("join").expect("the change of the limit commits");
        fx.assert_exact(fx.tenant, 500 * MIB).await;
    })
    .await;
}

/// The idle reclaim folds and deletes only quota leases idle past the interval: a fresh one stays, and so does one an
/// open transaction holds (`SKIP LOCKED`). The usage is unchanged; S takes the idle quota lease's 1,000 used bytes.
#[tokio::test]
async fn the_idle_reclaim_folds_and_deletes_only_quota_leases_idle_past_the_interval() {
    bounded::within("idle reclaim", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let (mut idle, fresh, mut busy) = (fx.writer().await, fx.writer().await, fx.writer().await);
        let mut fresh = fresh;
        fx.charge_committed(&mut idle.conn, 1_000).await.expect("the idle writer's quota lease");
        fx.charge_committed(&mut fresh.conn, 2_000).await.expect("the fresh writer's quota lease");
        fx.charge_committed(&mut busy.conn, 3_000).await.expect("the busy writer's quota lease");
        sqlx::query(
            "UPDATE step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' \
              WHERE tenant_id = $1 AND writer = ANY ($2)",
        )
        .bind(fx.tenant)
        .bind(vec![idle.pid, busy.pid])
        .execute(fx.pool())
        .await
        .expect("two quota leases idle past the interval");
        let idle_grant = lease_of(fx.pool(), fx.tenant, idle.pid).await.expect("idle").0;
        let mut held = busy.conn.begin().await.expect("begin");
        fx.charge(&mut held, fx.tenant, fx.stream, ROW).await.expect("the busy writer holds its quota lease");
        let before = fx.ledger(fx.tenant).await;
        let counted = counters(&fx).await;
        let mut reclaimer = fx.writer().await;
        let (reclaimed, skipped): (i32, bool) =
            sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
                .bind(fx.tenant)
                .fetch_one(&mut reclaimer.conn)
                .await
                .expect("the idle reclaim");
        held.commit().await.expect("commit");
        assert_eq!((reclaimed, skipped), (1, false), "only the idle quota lease no transaction holds");
        let writers: Vec<i32> = sqlx::query_scalar("SELECT writer FROM step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer")
            .bind(fx.tenant)
            .fetch_all(fx.pool())
            .await
            .expect("the quota leases");
        let mut kept = vec![fresh.pid, busy.pid];
        kept.sort_unstable();
        assert_eq!(writers, kept);
        let after = fx.ledger(fx.tenant).await;
        assert_eq!(after.usage(), before.usage() + ROW, "the usage is unchanged but for the busy writer's commit");
        assert_eq!((after.s, after.g), (before.s + 1_000, before.g - idle_grant), "the idle quota lease was folded");
        let now = counters(&fx).await;
        assert_eq!((now.2 - counted.2, now.3 - counted.3), (1, 0), "counted idle, not ended");
        fx.assert_exact(fx.tenant, 1 << 30).await;
    })
    .await;
}

/// Hand mutation 19 (the reclaim's clause on ended backends dropped): the quota lease of a backend that ended is found by
/// `sagat_quota_step_log_idle_tenants` and reclaimed at once, counted `ended`.
#[tokio::test]
async fn the_quota_lease_of_an_ended_backend_is_reclaimed_at_once() {
    bounded::within("ended backend", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let gone = fx.writer().await;
        let pid = gone.pid;
        let mut conn = gone.conn;
        fx.charge_committed(&mut conn, 1_000)
            .await
            .expect("a quota lease");
        conn.close().await.expect("the backend ends");
        bounded::within("the backend is gone", LIMIT, async {
            loop {
                let alive: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
                )
                .bind(pid)
                .fetch_one(fx.pool())
                .await
                .expect("pg_stat_activity");
                if !alive {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        let mut purge = fx.writer().await;
        let tenants: Vec<uuid::Uuid> =
            sqlx::query_scalar("SELECT sagat_quota_step_log_idle_tenants(10)")
                .fetch_all(&mut purge.conn)
                .await
                .expect("the tenants to serve");
        assert_eq!(tenants, vec![fx.tenant]);
        let before = fx.ledger(fx.tenant).await;
        let counted = counters(&fx).await;
        let (n, skipped): (i32, bool) =
            sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
                .bind(fx.tenant)
                .fetch_one(&mut purge.conn)
                .await
                .expect("the reclaim");
        assert_eq!((n, skipped), (1, false));
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            (l.g, l.leases),
            (0, 0),
            "the ended backend's quota lease is gone"
        );
        assert_eq!(l.usage(), before.usage(), "its used bytes folded into S");
        let now = counters(&fx).await;
        assert_eq!(
            (now.2 - counted.2, now.3 - counted.3),
            (0, 1),
            "counted ended"
        );
        assert_eq!(now.4, 0, "no quota lease row left");
    })
    .await;
}

/// With the tenant's step-log fence held (a change of the limit holds it exclusively) the idle reclaim answers
/// `skipped` and reclaims nothing; with nothing idle it answers `(0, false)` before any lock.
#[tokio::test]
async fn the_idle_reclaim_skips_a_tenant_whose_fence_is_held_and_answers_nothing_idle_at_once() {
    bounded::within("skipped", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000).await.expect("a quota lease");
        let mut purge = fx.writer().await;
        let answer: (i32, bool) = sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
            .bind(fx.tenant)
            .fetch_one(&mut purge.conn)
            .await
            .expect("nothing idle");
        assert_eq!(answer, (0, false));
        sqlx::query("UPDATE step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' WHERE writer = $1")
            .bind(w.pid)
            .execute(fx.pool())
            .await
            .expect("idle");
        let mut fence = fx.pool().begin().await.expect("begin");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || $1::text, 0))")
            .bind(fx.tenant)
            .execute(&mut *fence)
            .await
            .expect("hold the fence");
        let answer: (i32, bool) = sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
            .bind(fx.tenant)
            .fetch_one(&mut purge.conn)
            .await
            .expect("skipped");
        assert_eq!(answer, (0, true));
        fence.rollback().await.expect("rollback");
        assert_eq!(fx.ledger(fx.tenant).await.leases, 1, "nothing reclaimed");
        let answer: (i32, bool) = sqlx::query_as("SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)")
            .bind(fx.tenant)
            .fetch_one(&mut purge.conn)
            .await
            .expect("the next pass");
        assert_eq!(answer, (1, false));
    })
    .await;
}
