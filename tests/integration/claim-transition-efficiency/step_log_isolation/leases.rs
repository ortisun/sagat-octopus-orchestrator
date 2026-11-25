//! The charge and the refill (ADR-0097 decisions 2 to 4): each writer's own quota lease, the usage row taken only by a
//! refill, its bounded wait, and the invariants I1 to I5 under random interleavings.

use std::time::Duration;

use sqlx::Connection as _;

use super::{
    Fx, LIMIT, MIB, ROW, exceeded, held, hold_the_usage_row, lease_of, lock_timeout, refusal,
    set_limit, slots_taken, waits_on,
};
use crate::support::bounded;

/// D041-117: each server backend charges its own quota lease; one writer's open transaction never makes another wait
/// or fail, and the quota leases are keyed by the writers' backends.
#[tokio::test]
async fn each_writer_charges_its_own_quota_lease_and_never_meets_another_writers_lock() {
    bounded::within("own quota lease", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let (mut a, mut b) = (fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut a.conn, 1_000)
            .await
            .expect("A's quota lease granted");
        fx.charge_committed(&mut b.conn, 1_000)
            .await
            .expect("B's quota lease granted");
        let mut open = a.conn.begin().await.expect("begin");
        fx.charge(&mut open, fx.tenant, fx.stream, 1_000)
            .await
            .expect("A holds its quota lease");
        let second = tokio::time::timeout(
            Duration::from_secs(20),
            fx.charge_committed(&mut b.conn, 1_000),
        )
        .await;
        assert!(
            matches!(second, Ok(Ok(()))),
            "A's open transaction made B wait or fail: {second:?}"
        );
        open.commit().await.expect("commit");
        let writers: Vec<i32> = sqlx::query_scalar(
            "SELECT writer FROM step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer",
        )
        .bind(fx.tenant)
        .fetch_all(fx.pool())
        .await
        .expect("the quota leases");
        let mut expected = vec![a.pid, b.pid];
        expected.sort_unstable();
        assert_eq!(
            writers, expected,
            "one quota lease a writer, keyed by its backend"
        );
        assert_eq!(
            fx.ledger(fx.tenant).await.usage(),
            4_000,
            "every charged byte is counted once"
        );
        fx.assert_exact(fx.tenant, 1 << 30).await;
    })
    .await;
}

/// A charge with room in the writer's quota lease never takes the usage row; only a refill waits for it, and one whose
/// row is held by a holder that updated it waits and writes once the holder ends.
#[tokio::test]
async fn a_charge_with_room_never_takes_the_usage_row_and_a_refill_alone_waits_for_it() {
    bounded::within("usage row", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        fx.without_the_refill_bound().await;
        let mut c = fx.writer().await;
        // The first charge refills: g = max(1,000, min(1 MiB, ⌊2^30 / 2⌋)) = 1 MiB.
        fx.charge_committed(&mut c.conn, 1_000)
            .await
            .expect("C's quota lease granted");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, c.pid).await,
            Some((MIB, 1_000))
        );
        let (row, holder) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let free = tokio::time::timeout(
            Duration::from_secs(20),
            fx.charge_committed(&mut c.conn, 1_000),
        )
        .await;
        assert!(
            matches!(free, Ok(Ok(()))),
            "a charge with room took the usage row: {free:?}"
        );
        // 4 MiB is more than the room left (1 MiB − 2,000): a refill, the one wait.
        let pid = c.pid;
        let refill = {
            let mut conn = c.conn;
            let (tenant, stream) = (fx.tenant, fx.stream);
            let first = fx.sequences(505);
            tokio::spawn(async move {
                let mut tx = conn.begin().await?;
                tx_wait_slots(&mut tx).await?;
                super::insert_messages(
                    &mut tx,
                    tenant,
                    stream,
                    first,
                    &super::message_lengths(4 * MIB),
                )
                .await?;
                tx.commit().await
            })
        };
        waits_on(fx.pool(), pid, holder).await;
        row.commit().await.expect("release the usage row");
        refill
            .await
            .expect("join")
            .expect("the refill completes once the row is free");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(l.g, l.granted);
        assert_eq!(l.usage(), 2_000 + 4 * MIB);
        fx.assert_exact(fx.tenant, 1 << 30).await;
    })
    .await;
}

/// A second charge in one transaction into the writer's quota lease with room takes no lock either (decision D041-143).
/// PostgreSQL runs a foreign key's check on an update that keeps the key when the row version was written by the
/// updating transaction, so a key from the quota leases to the usage row made the second charge take `FOR KEY SHARE`
/// on the row and wait, with no timeout, for a refill holding it. Under `lock_timeout = 1ms` any such wait raises.
#[tokio::test]
async fn a_second_charge_in_one_transaction_takes_no_lock_on_a_held_usage_row() {
    bounded::within("second charge", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut c = fx.writer().await;
        fx.charge_committed(&mut c.conn, 1_000)
            .await
            .expect("C's quota lease granted: 1 MiB");
        let mut tx = c.conn.begin().await.expect("begin");
        fx.charge(&mut tx, fx.tenant, fx.stream, 1_000)
            .await
            .expect("the first charge of the transaction, into the quota lease");
        let (row, _holder) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        sqlx::query("SET LOCAL lock_timeout = '1ms'")
            .execute(&mut *tx)
            .await
            .expect("no lock wait allowed");
        let second = fx.charge(&mut tx, fx.tenant, fx.stream, 1_000).await;
        assert!(
            second.is_ok(),
            "the second charge waited on the usage row: {second:?}"
        );
        tx.commit().await.expect("commit");
        row.commit().await.expect("release the usage row");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, c.pid).await,
            Some((MIB, 3_000))
        );
        fx.assert_exact(fx.tenant, 1 << 30).await;
    })
    .await;
}

/// Five wait slots (*Q* = 5, *P* = 3), as the store sets them per transaction.
async fn tx_wait_slots(conn: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
        .execute(conn)
        .await
        .map(|_| ())
}

/// D041-119: a refill inside a batch's insert waits for the usage row at most its budget b = 200 ms and raises `55P03`
/// (the batch's `RETRY_LATER`); a caller's `lock_timeout` of 0 means none, so the refill sets b.
#[tokio::test]
async fn a_refill_waits_for_a_held_usage_row_at_most_its_budget() {
    bounded::within("bounded refill", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let (row, holder) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let mut w = fx.writer().await;
        let mut tx = w.conn.begin().await.expect("begin");
        sqlx::query("SET LOCAL lock_timeout = 0")
            .execute(&mut *tx)
            .await
            .expect("no timeout of the caller's");
        let refused = fx.charge(&mut tx, fx.tenant, fx.stream, 1_000).await;
        assert_eq!(
            refusal(&refused),
            lock_timeout(),
            "the wait ended at b: {refused:?}"
        );
        tx.rollback().await.expect("rollback");
        drop(row);
        let _ = holder;
    })
    .await;
}

/// Mutation 21's test (validation-report.md, D041-120): a caller's lower `lock_timeout` is kept; with the budget
/// replaced by 300 s, a refill that overrode the caller's 50 ms would outlive this test's 60 s.
#[tokio::test]
async fn a_callers_lower_lock_timeout_is_kept() {
    bounded::within("lower timeout kept", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let shipped: f64 = sqlx::query_scalar(
            "SELECT extract(epoch FROM sagat_quota_step_log_refill_wait())::float8",
        )
        .fetch_one(fx.pool())
        .await
        .expect("the budget");
        assert!(
            (shipped - 0.2).abs() < 1e-9,
            "the shipped budget is 200 ms: {shipped}"
        );
        fx.without_the_refill_bound().await;
        let (row, _) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let mut w = fx.writer().await;
        let mut tx = w.conn.begin().await.expect("begin");
        sqlx::query("SET LOCAL lock_timeout = '50ms'")
            .execute(&mut *tx)
            .await
            .expect("the caller's lower timeout");
        let refused = fx.charge(&mut tx, fx.tenant, fx.stream, 1_000).await;
        assert_eq!(
            refusal(&refused),
            lock_timeout(),
            "the caller's 50 ms ended the wait: {refused:?}"
        );
        tx.rollback().await.expect("rollback");
        row.rollback().await.expect("rollback");
    })
    .await;
}

/// The refill restores the caller's `lock_timeout` after its lock of the usage row, a lower one and none alike.
#[tokio::test]
async fn a_refill_restores_the_callers_lock_timeout() {
    bounded::within("lock timeout restored", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        for setting in ["150ms", "0"] {
            // A writer of its own each time: its first charge is a refill.
            let mut w = fx.writer().await;
            let mut tx = w.conn.begin().await.expect("begin");
            sqlx::query("SELECT set_config('lock_timeout', $1, true)")
                .bind(setting)
                .execute(&mut *tx)
                .await
                .expect("the caller's timeout");
            fx.charge(&mut tx, fx.tenant, fx.stream, 1_000)
                .await
                .expect("a refill");
            let after: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
                .fetch_one(&mut *tx)
                .await
                .expect("after");
            assert_eq!(after, setting, "the caller's lock_timeout after a refill");
            tx.commit().await.expect("commit");
            assert!(
                lease_of(fx.pool(), fx.tenant, w.pid).await.is_some(),
                "the charge refilled"
            );
        }
    })
    .await;
}

/// Concurrent writers never pass the limit and every committed byte is counted once; nothing open, the exact remainder
/// is admitted and one more row is refused. L = 200,000 and each charge 3,000 bytes, so the room left is
/// 200,000 − 3,000 m ≥ 2,000 for every m ≤ 66 committed charges: always a chargeable remainder.
#[tokio::test]
async fn concurrent_inserts_never_pass_the_limit_and_count_every_byte_once() {
    bounded::within("exact under concurrency", Duration::from_secs(240), async {
        const L: i64 = 200_000;
        const N: i64 = 3_000;
        let fx = std::sync::Arc::new(Fx::new(L).await);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let (fx, barrier) = (std::sync::Arc::clone(&fx), std::sync::Arc::clone(&barrier));
                tokio::spawn(async move {
                    let mut w = fx.writer().await;
                    barrier.wait().await;
                    let mut kept = 0_i64;
                    for _ in 0..10 {
                        let mut tx = w.conn.begin().await.expect("begin");
                        tx_wait_slots(&mut tx).await.expect("wait slots");
                        match fx.charge(&mut tx, fx.tenant, fx.stream, N).await {
                            Ok(()) => {
                                tx.commit().await.expect("commit");
                                kept += N;
                            }
                            Err(sqlx::Error::Database(e))
                                if matches!(e.code().as_deref(), Some("54000" | "55P03")) =>
                            {
                                tx.rollback().await.expect("rollback");
                            }
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
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            l.usage(),
            kept,
            "the usage is exactly what was charged and committed"
        );
        fx.assert_exact(fx.tenant, L).await;
        let room = L - l.usage();
        assert!(room >= 2_000, "{room}");
        let mut last = fx.writer().await;
        fx.charge_committed(&mut last.conn, room)
            .await
            .expect("the exact remainder is admitted");
        assert_eq!(
            refusal(&fx.charge_committed(&mut last.conn, ROW).await),
            exceeded()
        );
        assert_eq!(fx.ledger(fx.tenant).await.usage(), L);
    })
    .await;
}

/// I5: a lowered limit stops charges into quota leases with room even when no reclaim took that room (the limit-change
/// trigger disabled here): S + G > L sends the charge to the refill, which folds and refuses.
#[tokio::test]
async fn a_lowered_limit_stops_charges_into_quota_leases_with_room() {
    bounded::within("lowered limit", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000)
            .await
            .expect("a quota lease of 1 MiB with room");
        for statement in [
            "ALTER TABLE quota_limits DISABLE TRIGGER quota_limits_step_log_reclaim",
            "ALTER TABLE quota_ceilings DISABLE TRIGGER quota_ceilings_step_log_reclaim",
        ] {
            sqlx::query(statement)
                .execute(fx.pool())
                .await
                .expect("no reclaim on the change");
        }
        set_limit(fx.pool(), fx.tenant, 1_000)
            .await
            .expect("lower the limit to the usage");
        assert_eq!(
            fx.ledger(fx.tenant).await.g,
            MIB,
            "the room is still leased"
        );
        assert_eq!(
            refusal(&fx.charge_committed(&mut w.conn, ROW).await),
            exceeded(),
            "a quota lease granted under the old limit was charged past the new one"
        );
        // The refused refill had folded the writer's quota lease (S = 1,000, G = 0) inside the insert that raised, so
        // the rollback leaves the committed state of before it: S = 0, G = 1 MiB, Σ used = 1,000.
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            (l.usage(), l.s + l.g),
            (1_000, MIB),
            "nothing was charged, nothing folded"
        );
    })
    .await;
}

/// The grant min(1 MiB, ⌊F / (2(A + 1))⌋) and the reclaim near the limit: the room of idle quota leases completes the
/// limit. L = 2 MiB = 2,097,152: A's grant is 1,048,576 (⌊F / 2⌋, at the cap); B's refill finds L − S − G = 1,048,576,
/// not under max(1,000, 1 MiB), so it reclaims nothing and grants ⌊1,048,576 / 4⌋ = 262,144; G = 1,310,720. A third
/// writer's charge of L − 2,000 = 2,095,152 finds 786,432 free, reclaims both (S = 2,000, G = 0) and is granted exactly
/// its charge (F = 2,095,152); one more row is refused.
#[tokio::test]
async fn the_last_bytes_of_the_limit_are_admitted_from_the_room_of_idle_quota_leases() {
    bounded::within("reclaim near the limit", LIMIT, async {
        const L: i64 = 2 * MIB;
        let fx = Fx::new(L).await;
        let (mut a, mut b, mut c) = (fx.writer().await, fx.writer().await, fx.writer().await);
        fx.charge_committed(&mut a.conn, 1_000)
            .await
            .expect("A: 1 MiB granted");
        fx.charge_committed(&mut b.conn, 1_000)
            .await
            .expect("B: 262,144 granted");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(
            (l.s, l.g, l.granted, l.used, l.leases),
            (0, 1_310_720, 1_310_720, 2_000, 2)
        );
        fx.charge_committed(&mut c.conn, L - 2_000)
            .await
            .expect("the room of idle quota leases is given back near the limit");
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, c.pid).await,
            Some((2_095_152, 2_095_152))
        );
        assert_eq!(
            refusal(&fx.charge_committed(&mut c.conn, ROW).await),
            exceeded()
        );
        let l = fx.ledger(fx.tenant).await;
        assert_eq!((l.s, l.usage()), (2_000, L));
        fx.assert_exact(fx.tenant, L).await;
    })
    .await;
}

/// The small-quota case (D041-117): a tenant with a small limit served by many writers is never refused while the room
/// of its idle quota leases would fit the charge. Twenty writers charge one empty-text row each (128 bytes) and go idle;
/// the rest, 100,000 − 20 × 128 = 97,440, is admitted at once and one more row is refused.
#[tokio::test]
async fn a_small_quota_with_many_idle_writers_admits_everything_their_quota_leases_hold() {
    bounded::within("small quota", LIMIT, async {
        const L: i64 = 100_000;
        let fx = Fx::new(L).await;
        let mut writers = Vec::new();
        for _ in 0..20 {
            let mut w = fx.writer().await;
            fx.charge_committed(&mut w.conn, ROW)
                .await
                .expect("a writer leases");
            writers.push(w);
        }
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(l.usage(), 20 * ROW);
        assert!(l.g > l.used, "the idle quota leases hold room: {l:?}");
        let mut other = fx.writer().await;
        fx.charge_committed(&mut other.conn, L - 20 * ROW)
            .await
            .expect("everything the idle quota leases hold is admitted");
        assert_eq!(
            refusal(&fx.charge_committed(&mut other.conn, ROW).await),
            exceeded()
        );
        assert_eq!(fx.ledger(fx.tenant).await.usage(), L);
        fx.assert_exact(fx.tenant, L).await;
        drop(writers);
    })
    .await;
}

/// The held-room bound (ADR-0097 decision 4, D041-119): k writers each take a fresh grant one after another and hold
/// their quota leases open; at least F₀ · C(2k, k) / 4^k of the room F₀ free before them stays chargeable by another
/// writer, and only past what is free is a charge refused as held. Started with a quarter of L = 1 MiB used, so
/// F₀ = 786,432, for k = 1, 2, 8 and 32. Every other writer takes its grant from a top-up, which grants its share and no
/// more, so it is counted at its share as the bound counts it.
#[tokio::test]
async fn room_held_by_open_writers_is_bounded_from_the_free_room() {
    bounded::within("held bound from F0", Duration::from_secs(240), async {
        for k in [1_u32, 2, 8, 32] {
            const L: i64 = MIB;
            let fx = Fx::new(L).await;
            sqlx::query(
                "UPDATE tenant_retained_usage SET step_log_bytes = $2 WHERE tenant_id = $1",
            )
            .bind(fx.tenant)
            .bind(L / 4)
            .execute(fx.pool())
            .await
            .expect("a quarter of L already used");
            let f0 = u128::try_from(L - L / 4).expect("positive");
            let mut open = Vec::new();
            for i in 0..k {
                let mut w = fx.writer().await;
                if i % 2 == 1 {
                    let (outcome, _): (String, i64) =
                        sqlx::query_as("SELECT outcome, room FROM sagat_quota_step_log_top_up($1)")
                            .bind(fx.tenant)
                            .fetch_one(&mut w.conn)
                            .await
                            .expect("a top-up");
                    assert_eq!(outcome, "made", "k = {k}, writer {i}");
                } else {
                    fx.charge_committed(&mut w.conn, ROW)
                        .await
                        .expect("a writer leases");
                }
                sqlx::query("BEGIN")
                    .execute(&mut w.conn)
                    .await
                    .expect("begin");
                fx.charge(&mut w.conn, fx.tenant, fx.stream, ROW)
                    .await
                    .expect("the writer holds its quota lease");
                assert!(lease_of(fx.pool(), fx.tenant, w.pid).await.is_some());
                open.push(w);
            }
            let l = fx.ledger(fx.tenant).await;
            let free = L - l.s - l.g;
            // C(2k, k) / 4^k, exactly: C(64, 32) and 4^32 fit in u128.
            let kk = u128::from(k);
            let mut binomial = 1_u128;
            for i in 0..kk {
                binomial = binomial * (2 * kk - i) / (i + 1);
            }
            let floor = i64::try_from(f0 * binomial / 4_u128.pow(k)).expect("fits");
            assert!(
                free >= floor,
                "k = {k}: the unleased room {free} is under the bound {floor}"
            );
            let mut other = fx.writer().await;
            assert_eq!(
                refusal(&fx.charge_committed(&mut other.conn, free + ROW).await),
                held(),
                "k = {k}"
            );
            fx.charge_committed(&mut other.conn, free)
                .await
                .expect("the unleased room is chargeable while every quota lease is held");
            for mut w in open {
                sqlx::query("COMMIT")
                    .execute(&mut w.conn)
                    .await
                    .expect("commit");
            }
            fx.assert_exact(fx.tenant, L).await;
        }
    })
    .await;
}

/// With `step_log_enforcement_enabled` off a refill grants past the limit (I4 and I5 not checked) and the usage stays
/// exact.
#[tokio::test]
async fn with_enforcement_off_a_refill_grants_past_the_limit_and_the_usage_stays_exact() {
    bounded::within("enforcement off", LIMIT, async {
        let fx = Fx::new(10_000).await;
        sqlx::query("UPDATE quota_accounting_control SET step_log_enforcement_enabled = false")
            .execute(fx.pool())
            .await
            .expect("enforcement off");
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 20_000)
            .await
            .expect("past the limit, not enforced");
        fx.charge_committed(&mut w.conn, 5_000)
            .await
            .expect("again");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(l.usage(), 25_000, "the usage stays exact");
        assert_eq!(l.g, l.granted);
        sqlx::query("UPDATE quota_accounting_control SET step_log_enforcement_enabled = true")
            .execute(fx.pool())
            .await
            .expect("enforcement on");
        assert_eq!(
            refusal(&fx.charge_committed(&mut w.conn, ROW).await),
            exceeded()
        );
    })
    .await;
}

/// The charge raises at the limit, under its own constraint name, and an empty line costs its 128 bytes. L = 1,000:
/// seven empty lines (896) fit; the refill granted max(896, ⌊1,000 / 2⌋) = 896, so the eighth (128) finds 104 free and
/// no other quota lease, and is refused as exceeded.
#[tokio::test]
async fn the_charge_raises_at_the_limit_and_an_empty_line_costs_128_bytes() {
    bounded::within("charge at the limit", LIMIT, async {
        let fx = Fx::new(1_000).await;
        let mut w = fx.writer().await;
        let first = fx.sequences(8);
        super::insert_messages(&mut w.conn, fx.tenant, fx.stream, first, &[0; 7])
            .await
            .expect("seven empty lines");
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 7 * ROW);
        assert_eq!(
            lease_of(fx.pool(), fx.tenant, w.pid).await,
            Some((896, 896))
        );
        let refused = sqlx::query(
            "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind) \
             VALUES ($1, $2, $3, gen_random_uuid(), 'debug', 'line')",
        )
        .bind(fx.tenant)
        .bind(fx.stream)
        .bind(first + 7)
        .execute(&mut w.conn)
        .await
        .map(|_| ());
        assert_eq!(
            refusal(&refused),
            exceeded(),
            "a line with no text costs 128 bytes"
        );
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 7 * ROW);
    })
    .await;
}

/// The wait-slot refusal is the refill's when the usage row is held and no slot is free; a test of its own is in
/// `wait_slots`, here only that the constraint names stay apart.
#[test]
fn the_refusals_have_constraint_names_of_their_own() {
    let names = [exceeded(), held(), slots_taken(), lock_timeout()];
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

/// ADR-0097's invariants I1 to I5 under random interleavings (a seeded generator; the seed is in the failure): three
/// writers insert in open or committed transactions, commit or roll back; the owner deletes committed lines (a
/// release), ages and reclaims idle quota leases, and changes the limit (never below the usage). After every step the
/// committed state has G = Σ granted, 0 ≤ U ≤ S + G ≤ L, and U equals the model: the bytes of the committed lines still
/// stored. The owner's steps that lock the usage row run only while no open writer holds it (a refill holds it to its
/// commit, which the refill counter tells), and a change of the limit only while no writer is open (it takes the
/// step-log fence exclusively): this test is sequential, so a wait there would never end.
#[tokio::test]
async fn the_invariants_hold_under_random_interleavings() {
    bounded::within("invariants", Duration::from_secs(280), async {
        for seed in 1_u64..=6 {
            interleave(seed).await;
        }
    })
    .await;
}

/// One writer of the interleaving: its connection, whether a transaction is open, whether that transaction refilled
/// (so holds the usage row), and the lines it stored there: (sequence, charge).
struct Open {
    writer: super::Writer,
    open: bool,
    refilled: bool,
    lines: Vec<(i64, i64)>,
}

async fn refills(fx: &Fx) -> i64 {
    sqlx::query_scalar("SELECT refills FROM sagat_quota_step_log_counters()")
        .fetch_one(fx.pool())
        .await
        .expect("the counters")
}

#[allow(clippy::too_many_lines)]
async fn interleave(seed: u64) {
    let mut limit = 400_000_i64;
    let fx = Fx::new(limit).await;
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move |n: u64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng % n
    };
    let mut writers = Vec::new();
    for _ in 0..3 {
        writers.push(Open {
            writer: fx.writer().await,
            open: false,
            refilled: false,
            lines: Vec::new(),
        });
    }
    let mut stored: Vec<(i64, i64)> = Vec::new();
    let mut model = 0_i64;
    for step in 0..60 {
        let w = usize::try_from(next(3)).expect("small");
        let what = next(10);
        let any_open = writers.iter().any(|o| o.open);
        let row_held = writers.iter().any(|o| o.open && o.refilled);
        match what {
            0..=4 => {
                let bytes = ROW + i64::try_from(next(30_000)).expect("small");
                let lengths = super::message_lengths(bytes);
                let first = fx.sequences(i64::try_from(lengths.len()).expect("few"));
                let before = refills(&fx).await;
                let o = &mut writers[w];
                if !o.open {
                    sqlx::query("BEGIN")
                        .execute(&mut o.writer.conn)
                        .await
                        .expect("begin");
                    sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
                        .execute(&mut o.writer.conn)
                        .await
                        .expect("slots");
                    o.open = true;
                }
                sqlx::query("SAVEPOINT s")
                    .execute(&mut o.writer.conn)
                    .await
                    .expect("savepoint");
                let charged = super::insert_messages(
                    &mut o.writer.conn,
                    fx.tenant,
                    fx.stream,
                    first,
                    &lengths,
                )
                .await;
                match refusal(&charged) {
                    None => {
                        sqlx::query("RELEASE SAVEPOINT s")
                            .execute(&mut o.writer.conn)
                            .await
                            .expect("release");
                        o.refilled |= refills(&fx).await != before;
                        for (i, n) in lengths.iter().enumerate() {
                            o.lines
                                .push((first + i64::try_from(i).expect("few"), n + ROW));
                        }
                    }
                    Some(r)
                        if [exceeded(), held(), lock_timeout(), slots_taken()]
                            .contains(&Some(r.clone())) =>
                    {
                        sqlx::query("ROLLBACK TO SAVEPOINT s")
                            .execute(&mut o.writer.conn)
                            .await
                            .expect("rollback to");
                    }
                    Some(r) => panic!("seed {seed} step {step}: an unexpected refusal {r:?}"),
                }
            }
            5 | 6 => {
                let o = &mut writers[w];
                if o.open {
                    if what == 5 {
                        sqlx::query("COMMIT")
                            .execute(&mut o.writer.conn)
                            .await
                            .expect("commit");
                        model += o.lines.iter().map(|(_, c)| c).sum::<i64>();
                        stored.append(&mut o.lines);
                    } else {
                        sqlx::query("ROLLBACK")
                            .execute(&mut o.writer.conn)
                            .await
                            .expect("rollback");
                    }
                    o.open = false;
                    o.refilled = false;
                    o.lines.clear();
                }
            }
            7 if !stored.is_empty() && !row_held => {
                let take = usize::try_from(next(u64::try_from(stored.len()).expect("few")) + 1)
                    .expect("few");
                let gone: Vec<(i64, i64)> = stored.drain(..take).collect();
                let sequences: Vec<i64> = gone.iter().map(|(s, _)| *s).collect();
                sqlx::query(
                    "DELETE FROM step_log_lines WHERE tenant_id = $1 AND sequence = ANY ($2)",
                )
                .bind(fx.tenant)
                .bind(&sequences)
                .execute(fx.pool())
                .await
                .expect("a release");
                model -= gone.iter().map(|(_, c)| c).sum::<i64>();
            }
            8 if !row_held => {
                let idle: Vec<i32> = writers
                    .iter()
                    .filter(|o| !o.open)
                    .map(|o| o.writer.pid)
                    .collect();
                sqlx::query(
                    "UPDATE step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' \
                      WHERE tenant_id = $1 AND writer = ANY ($2)",
                )
                .bind(fx.tenant)
                .bind(&idle)
                .execute(fx.pool())
                .await
                .expect("age the idle quota leases");
                let (_, skipped): (i32, bool) = sqlx::query_as(
                    "SELECT reclaimed, skipped FROM sagat_quota_step_log_reclaim_idle($1)",
                )
                .bind(fx.tenant)
                .fetch_one(fx.pool())
                .await
                .expect("the idle reclaim");
                assert!(
                    !skipped,
                    "seed {seed} step {step}: no one holds the fence exclusively"
                );
            }
            9 if !any_open => {
                let usage = fx.ledger(fx.tenant).await.usage();
                limit = usage + i64::try_from(next(600_000)).expect("small");
                set_limit(fx.pool(), fx.tenant, limit)
                    .await
                    .expect("a change of the limit");
                let l = fx.ledger(fx.tenant).await;
                assert_eq!(
                    (l.g, l.granted),
                    (0, 0),
                    "seed {seed} step {step}: the change reclaimed every quota lease"
                );
            }
            _ => {}
        }
        let l = fx.ledger(fx.tenant).await;
        assert_eq!(l.g, l.granted, "seed {seed} step {step}: I2 {l:?}");
        assert!(
            0 <= l.usage() && l.usage() <= l.s + l.g,
            "seed {seed} step {step}: I1, I3 {l:?}"
        );
        assert!(
            l.s + l.g <= limit,
            "seed {seed} step {step}: I4, I5 under {limit}: {l:?}"
        );
        assert_eq!(
            l.usage(),
            model,
            "seed {seed} step {step}: the usage is the committed lines' bytes"
        );
    }
    for o in &mut writers {
        if o.open {
            sqlx::query("ROLLBACK")
                .execute(&mut o.writer.conn)
                .await
                .expect("rollback");
        }
    }
}
