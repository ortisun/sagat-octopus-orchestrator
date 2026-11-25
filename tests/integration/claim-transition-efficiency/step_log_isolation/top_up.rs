//! The top-up after a carrying report (ADR-0097 decision 3b; decisions D041-124 and D041-134, OD-2026-11-20-18): one
//! statement in its own transaction that takes no amount, never waits, takes no wait slot, reclaims no other quota lease
//! and answers without raising: `room`, `made`, `limit`, `busy` or `contended`, with the writer's room after it.

use std::time::Duration;

use sqlx::PgConnection;
use uuid::Uuid;

use super::{Fx, LIMIT, MARK, MIB, ROW, hold_the_usage_row, lease_of};
use crate::support::bounded;

async fn top_up(conn: &mut PgConnection, tenant: Uuid) -> (String, i64) {
    tokio::time::timeout(
        Duration::from_secs(20),
        sqlx::query_as("SELECT outcome, room FROM sagat_quota_step_log_top_up($1)")
            .bind(tenant)
            .fetch_one(conn),
    )
    .await
    .expect("the top-up answers at once, never after a wait")
    .expect("the top-up answers without raising")
}

/// The share a top-up grants from the committed ledger: min(1 MiB, ⌊F / (2(A + 1))⌋), F = L − S − G and A the other quota
/// leases holding a grant.
async fn share(fx: &Fx, limit: i64, pid: i32) -> i64 {
    let l = fx.ledger(fx.tenant).await;
    let others: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM step_log_quota_leases WHERE tenant_id = $1 AND writer <> $2 AND granted > 0",
    )
    .bind(fx.tenant)
    .bind(pid)
    .fetch_one(fx.pool())
    .await
    .expect("the other quota leases");
    MIB.min((limit - l.s - l.g).max(0) / (2 * (others + 1)))
}

/// With at least *M* = 65,536 bytes of room the top-up answers `room` without touching the usage row: it answers at once
/// while the row is held (a try of the row would answer `busy`, a wait would not answer).
#[tokio::test]
async fn with_room_at_the_mark_the_top_up_answers_room_and_takes_no_lock_on_the_usage_row() {
    bounded::within("room", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000)
            .await
            .expect("a quota lease of 1 MiB");
        let (row, _) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        assert_eq!(
            top_up(&mut w.conn, fx.tenant).await,
            ("room".to_owned(), MIB - 1_000)
        );
        row.rollback().await.expect("rollback");
        assert!(MIB - 1_000 >= MARK);
    })
    .await;
}

/// Below *M* the top-up grants exactly its share and charges nothing (the new hand mutation of task T041-5: a top-up that
/// charged what it grants fails here). L = 3,000,000: A's refill grants 1,048,576 (⌊3,000,000 / 2⌋ = 1,500,000 is over
/// the cap); A then charges 1,046,576 more, leaving 1,000 of room. B's refill finds 1,951,424 free, not under 1 MiB, and
/// grants ⌊1,951,424 / 4⌋ = 487,856; G = 1,536,432. A's top-up: the share is ⌊(3,000,000 − 1,536,432) / 4⌋ = 365,892,
/// above its room, so `made`: S = 1,047,576 (A's used bytes folded), G = 1,536,432 − 1,048,576 + 365,892 = 853,748, A's
/// quota lease (365,892, 0), and U = 1,047,576 + 1,000 = 1,048,576, as before.
#[tokio::test]
async fn a_top_up_charges_nothing() {
    bounded::within("made", LIMIT, async {
        const L: i64 = 3_000_000;
        let fx = Fx::new(L).await;
        let (mut a, mut b) = (fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut a.conn, 1_000)
            .await
            .expect("A's grant");
        fx.charge_committed(&mut a.conn, 1_046_576)
            .await
            .expect("A's room down to 1,000");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, a.pid).await,
            Some((1_048_576, 1_047_576))
        );
        fx.charge_committed(&mut b.conn, 1_000)
            .await
            .expect("B's grant");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, b.pid).await,
            Some((487_856, 1_000))
        );
        let before = fx.ledger(fx.tenant).await;
        assert_eq!(
            (before.s, before.g, before.usage()),
            (0, 1_536_432, 1_048_576)
        );
        assert_eq!(share(&fx, L, a.pid).await, 365_892);
        assert_eq!(
            top_up(&mut a.conn, fx.tenant).await,
            ("made".to_owned(), 365_892)
        );
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, a.pid).await,
            Some((365_892, 0))
        );
        let after = fx.ledger(fx.tenant).await;
        assert_eq!((after.s, after.g), (1_047_576, 853_748));
        assert_eq!(after.usage(), before.usage(), "a top-up charges nothing");
        fx.assert_exact(fx.tenant, L).await;
        // The next charge within the share takes the fast path: no refill, the usage row untouched.
        let (row, _) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let fast = tokio::time::timeout(
            Duration::from_secs(20),
            fx.charge_committed(&mut a.conn, 24_704),
        )
        .await;
        assert!(matches!(fast, Ok(Ok(()))), "{fast:?}");
        row.rollback().await.expect("rollback");
    })
    .await;
}

/// The share is 0 when F < 2(A + 1): `limit`, not `room`, though the writer's room (0) is under *M*. L = 897: seven empty
/// lines (896) are granted max(896, ⌊897 / 2⌋) = 896, so F = 1 and ⌊1 / 2⌋ = 0.
#[tokio::test]
async fn the_top_up_answers_limit_when_its_share_is_zero() {
    bounded::within("limit", LIMIT, async {
        let fx = Fx::new(897).await;
        let mut w = fx.writer().await;
        let first = fx.sequences(7);
        super::insert_messages(&mut w.conn, fx.tenant, fx.stream, first, &[0; 7])
            .await
            .expect("896 bytes");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, w.pid).await,
            Some((896, 896))
        );
        assert_eq!(
            top_up(&mut w.conn, fx.tenant).await,
            ("limit".to_owned(), 0)
        );
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, w.pid).await,
            Some((896, 896)),
            "nothing changed"
        );
    })
    .await;
}

/// A share not above the writer's current room changes nothing: `room`. L = 200,000: the refill grants
/// max(1,000, ⌊200,000 / 2⌋) = 100,000; a charge of 40,000 more leaves 59,000 of room, under *M*; the share is
/// ⌊(200,000 − 100,000) / 2⌋ = 50,000, not above 59,000.
#[tokio::test]
async fn the_top_up_answers_room_when_its_share_is_not_above_the_current_room() {
    bounded::within("share not above the room", LIMIT, async {
        const L: i64 = 200_000;
        let fx = Fx::new(L).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000)
            .await
            .expect("a grant of 100,000");
        fx.charge_committed(&mut w.conn, 40_000)
            .await
            .expect("room down to 59,000");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, w.pid).await,
            Some((100_000, 41_000))
        );
        assert_eq!(share(&fx, L, w.pid).await, 50_000);
        assert_eq!(
            top_up(&mut w.conn, fx.tenant).await,
            ("room".to_owned(), 59_000)
        );
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, w.pid).await,
            Some((100_000, 41_000))
        );
    })
    .await;
}

/// Hand mutation 16 of task T041-5 (the top-up waits for the usage row instead of trying it): while the row is held the
/// top-up answers `busy` at once, and inside a transaction it holds no wait slot afterwards (pg_locks); a tenant without
/// a usage row is `busy` too.
#[tokio::test]
async fn the_top_up_answers_busy_at_once_while_the_usage_row_is_held_and_takes_no_wait_slot() {
    bounded::within("busy", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let replica = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let mut w = fx.writer_named(&replica).await;
        fx.charge_committed(&mut w.conn, 1_000).await.expect("a grant of 1 MiB");
        fx.charge_committed(&mut w.conn, MIB - 2_000).await.expect("room down to 1,000");
        let (row, _) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        sqlx::query("BEGIN").execute(&mut w.conn).await.expect("begin");
        sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
            .execute(&mut w.conn)
            .await
            .expect("slots");
        assert_eq!(top_up(&mut w.conn, fx.tenant).await, ("busy".to_owned(), 1_000));
        let slots: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l, generate_series(1, 5) s \
              WHERE l.pid = $1 AND l.locktype = 'advisory' AND l.objsubid = 1 \
                AND ((l.classid::bigint << 32) | l.objid::bigint) IN \
                    (hashtextextended('sagat:step_log_wait:t:' || $2::text || ':' || $3 || ':' || s, 0), \
                     hashtextextended('sagat:step_log_wait:r:' || $3 || ':' || s, 0))",
        )
        .bind(w.pid)
        .bind(fx.tenant)
        .bind(&replica)
        .fetch_one(fx.pool())
        .await
        .expect("pg_locks");
        assert_eq!(slots, 0, "a top-up takes no wait slot");
        sqlx::query("COMMIT").execute(&mut w.conn).await.expect("commit");
        row.rollback().await.expect("rollback");
        let (other, _) = fx.another_tenant(1 << 30).await;
        sqlx::query("DELETE FROM tenant_retained_usage WHERE tenant_id = $1")
            .bind(other)
            .execute(fx.pool())
            .await
            .expect("no usage row");
        assert_eq!(top_up(&mut w.conn, other).await, ("busy".to_owned(), 0));
    })
    .await;
}

/// A top-up reclaims no other quota lease: near the limit an idle writer's room stays where it is. L = 2 MiB: B's refill
/// grants 1,048,576 and goes idle; A's finds 1,048,576 free (not under 1 MiB, so no reclaim) and grants ⌊1,048,576 / 4⌋ =
/// 262,144; a charge of 200,000 leaves A 61,144 of room, under *M*; A's top-up grants ⌊(2,097,152 − 1,310,720) / 4⌋ =
/// 196,608 and B's quota lease is unchanged.
#[tokio::test]
async fn the_top_up_reclaims_no_other_quota_lease() {
    bounded::within("no reclaim", LIMIT, async {
        const L: i64 = 2 * MIB;
        let fx = Fx::new(L).await;
        let (mut a, mut b) = (fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut b.conn, 1_000)
            .await
            .expect("B's grant");
        fx.charge_committed(&mut a.conn, 1_000)
            .await
            .expect("A's grant");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, a.pid).await,
            Some((262_144, 1_000))
        );
        fx.charge_committed(&mut a.conn, 200_000)
            .await
            .expect("A's room down to 61,144");
        assert_eq!(
            top_up(&mut a.conn, fx.tenant).await,
            ("made".to_owned(), 196_608)
        );
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, b.pid).await,
            Some((MIB, 1_000)),
            "B keeps its room"
        );
        fx.assert_exact(fx.tenant, L).await;
    })
    .await;
}

/// Hand mutation 17 (the top-up waits for the step-log fence instead of trying it): with the fence held exclusively, as
/// a change of the limit holds it, the top-up answers `contended` at once.
#[tokio::test]
async fn the_top_up_answers_contended_while_the_step_log_fence_is_held() {
    bounded::within("contended", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, ROW).await.expect("a grant");
        fx.charge_committed(&mut w.conn, MIB - ROW - 500).await.expect("room down to 500");
        let mut fence = fx.pool().begin().await.expect("begin");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || $1::text, 0))")
            .bind(fx.tenant)
            .execute(&mut *fence)
            .await
            .expect("hold the step-log fence");
        assert_eq!(top_up(&mut w.conn, fx.tenant).await, ("contended".to_owned(), 500));
        fence.rollback().await.expect("rollback");
    })
    .await;
}
