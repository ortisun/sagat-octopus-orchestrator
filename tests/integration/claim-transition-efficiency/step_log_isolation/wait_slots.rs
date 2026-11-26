//! The wait slots of decisions D041-125 and D041-134: a refill waits for a held usage row only while holding one of the
//! tenant's *P* slots and one of the replica's *Q* slots, both tried and never waited for; with none free it raises
//! `55P03` at once. Each test transaction sets `sagat.step_log_wait_slots` to 5 (*Q* = 5, *P* = ⌈5 / 2⌉ = 3) as the store
//! does, and the budget is replaced by 300 s so that the waiters wait until the test releases the row.

use std::collections::BTreeSet;
use std::time::Duration;

use sqlx::{Connection as _, PgConnection, PgPool};
use uuid::Uuid;

use super::{Fx, LIMIT, Writer, hold_the_usage_row, refusal, slots_taken, waits_on};
use crate::support::bounded;

/// Begins a transaction on `writer` with `slots` wait slots (none set when `None`) and inserts one line of 1,000 bytes:
/// a refill, since the writer has no quota lease of the tenant.
async fn refill(
    conn: &mut PgConnection,
    tenant: Uuid,
    stream: i64,
    first: i64,
    slots: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("BEGIN").execute(&mut *conn).await?;
    if let Some(slots) = slots {
        sqlx::query("SELECT set_config('sagat.step_log_wait_slots', $1, true)")
            .bind(slots)
            .execute(&mut *conn)
            .await?;
    }
    super::insert_messages(conn, tenant, stream, first, &[872]).await
}

/// The slots `pid` holds among `1..=count` of the key `prefix || slot`: granted advisory locks of a single 64-bit key
/// (`objsubid` 1), whose high and low 32 bits are `classid` and `objid`, equal to `hashtextextended(key, 0)`.
async fn slots_held(pool: &PgPool, pid: i32, prefix: &str, count: i32) -> BTreeSet<i32> {
    sqlx::query_scalar(
        "SELECT s FROM generate_series(1, $3) s \
          WHERE EXISTS (SELECT 1 FROM pg_locks l WHERE l.pid = $1 AND l.locktype = 'advisory' AND l.granted \
                          AND l.objsubid = 1 \
                          AND ((l.classid::bigint << 32) | l.objid::bigint) = hashtextextended($2 || s::text, 0))",
    )
    .bind(pid)
    .bind(prefix)
    .bind(count)
    .fetch_all(pool)
    .await
    .expect("pg_locks")
    .into_iter()
    .collect()
}

fn tenant_prefix(tenant: Uuid, replica: &str) -> String {
    format!("sagat:step_log_wait:t:{tenant}:{replica}:")
}

fn replica_prefix(replica: &str) -> String {
    format!("sagat:step_log_wait:r:{replica}:")
}

/// The subtransactions the backend `pid` has open (`pg_stat_get_backend_subxact`).
async fn subtransactions(pool: &PgPool, pid: i32) -> i32 {
    sqlx::query_scalar(
        "SELECT x.subxact_count FROM pg_stat_get_backend_idset() b, pg_stat_get_backend_subxact(b) x \
          WHERE pg_stat_get_backend_pid(b) = $1",
    )
    .bind(pid)
    .fetch_one(pool)
    .await
    .expect("the backend's subtransactions")
}

/// With the tenant's usage row held and no quota lease, five refills on five backends of one replica: three wait, each
/// holding one tenant slot and one replica slot (slots 1 to 3 of each, single 64-bit keys) and no subtransaction; the
/// fourth and the fifth find the tenant's three slots taken and raise `55P03` at once. Released, the three write.
#[tokio::test]
async fn three_of_five_refills_wait_for_a_held_usage_row_and_two_are_refused_at_once() {
    bounded::within("tenant slots", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        fx.without_the_refill_bound().await;
        let replica = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let (row, holder) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let mut waiting = Vec::new();
        for _ in 0..3 {
            let Writer { mut conn, pid } = fx.writer_named(&replica).await;
            let (tenant, stream, first) = (fx.tenant, fx.stream, fx.sequences(1));
            let fx_pool = fx.pool().clone();
            let task = tokio::spawn(async move {
                sqlx::query("BEGIN").execute(&mut conn).await?;
                sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
                    .execute(&mut conn)
                    .await?;
                super::insert_messages(&mut conn, tenant, stream, first, &[872]).await?;
                sqlx::query("COMMIT").execute(&mut conn).await.map(|_| ())
            });
            waits_on(&fx_pool, pid, holder).await;
            waiting.push((pid, task));
        }
        let mut tenant_slots = BTreeSet::new();
        let mut replica_slots = BTreeSet::new();
        for (pid, _) in &waiting {
            let t = slots_held(fx.pool(), *pid, &tenant_prefix(fx.tenant, &replica), 5).await;
            let r = slots_held(fx.pool(), *pid, &replica_prefix(&replica), 5).await;
            assert_eq!(
                (t.len(), r.len()),
                (1, 1),
                "one tenant slot and one replica slot each: {t:?} {r:?}"
            );
            tenant_slots.extend(t);
            replica_slots.extend(r);
            assert_eq!(
                subtransactions(fx.pool(), *pid).await,
                0,
                "a waiting refill opens no subtransaction"
            );
        }
        assert_eq!(
            tenant_slots,
            BTreeSet::from([1, 2, 3]),
            "P = 3 tenant slots, one a waiter"
        );
        assert_eq!(replica_slots, BTreeSet::from([1, 2, 3]));
        for _ in 0..2 {
            let mut w = fx.writer_named(&replica).await;
            let refused = tokio::time::timeout(
                Duration::from_secs(20),
                refill(
                    &mut w.conn,
                    fx.tenant,
                    fx.stream,
                    fx.sequences(1),
                    Some("5"),
                ),
            )
            .await
            .expect("refused at once, not after a wait");
            assert_eq!(refusal(&refused), slots_taken());
        }
        row.commit().await.expect("release the usage row");
        for (_, task) in waiting {
            task.await
                .expect("join")
                .expect("a waiter writes once the row is free");
        }
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 3 * 1_000);
    })
    .await;
}

/// Five tenants' usage rows held, one waiter each, take the replica's five slots; a sixth tenant's refill on that replica
/// raises `55P03` at once, while one under another `application_name` takes slots of its own and waits.
#[tokio::test]
async fn with_every_replica_slot_taken_another_tenant_is_refused_at_once_and_another_replica_waits()
{
    bounded::within("replica slots", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        fx.without_the_refill_bound().await;
        let replica = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let mut tenants = vec![(fx.tenant, fx.stream)];
        for _ in 0..5 {
            tenants.push(fx.another_tenant(1 << 30).await);
        }
        let mut rows = Vec::new();
        for (tenant, _) in &tenants {
            rows.push(hold_the_usage_row(fx.pool(), *tenant).await);
        }
        let mut waiting = Vec::new();
        for (i, (tenant, stream)) in tenants.iter().take(5).enumerate() {
            let Writer { mut conn, pid } = fx.writer_named(&replica).await;
            let (tenant, stream, first) = (*tenant, *stream, fx.sequences(1));
            let task = tokio::spawn(async move {
                sqlx::query("BEGIN").execute(&mut conn).await?;
                sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
                    .execute(&mut conn)
                    .await?;
                super::insert_messages(&mut conn, tenant, stream, first, &[872]).await?;
                sqlx::query("COMMIT").execute(&mut conn).await.map(|_| ())
            });
            waits_on(fx.pool(), pid, rows[i].1).await;
            waiting.push(task);
        }
        let (sixth, sixth_stream) = tenants[5];
        let mut refused = fx.writer_named(&replica).await;
        let answer = tokio::time::timeout(
            Duration::from_secs(20),
            refill(
                &mut refused.conn,
                sixth,
                sixth_stream,
                fx.sequences(1),
                Some("5"),
            ),
        )
        .await
        .expect("refused at once");
        assert_eq!(refusal(&answer), slots_taken(), "Q = 5 slots, all held");
        assert!(
            slots_held(fx.pool(), refused.pid, &tenant_prefix(sixth, &replica), 5)
                .await
                .is_empty()
        );
        let other = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let Writer { mut conn, pid } = fx.writer_named(&other).await;
        let first = fx.sequences(1);
        let elsewhere = tokio::spawn(async move {
            sqlx::query("BEGIN").execute(&mut conn).await?;
            sqlx::query("SELECT set_config('sagat.step_log_wait_slots', '5', true)")
                .execute(&mut conn)
                .await?;
            super::insert_messages(&mut conn, sixth, sixth_stream, first, &[872]).await?;
            sqlx::query("COMMIT").execute(&mut conn).await.map(|_| ())
        });
        waits_on(fx.pool(), pid, rows[5].1).await;
        assert_eq!(
            slots_held(fx.pool(), pid, &replica_prefix(&other), 5).await,
            BTreeSet::from([1])
        );
        for (row, _) in rows {
            row.commit().await.expect("release");
        }
        for task in waiting {
            task.await.expect("join").expect("writes");
        }
        elsewhere
            .await
            .expect("join")
            .expect("the other replica's refill writes");
    })
    .await;
}

/// *Q* is the transaction's own setting, clamped to 1 to 64 and 1 when unset, empty or not a number; it ends with the
/// transaction. With none set, *Q* = *P* = 1: a second waiter on the same replica is refused at once.
#[tokio::test]
async fn a_transaction_that_sets_no_wait_slots_has_one_and_the_setting_ends_with_it() {
    bounded::within("one slot", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        for (value, slots) in [
            ("5", 5),
            ("0", 1),
            ("100", 64),
            ("64", 64),
            ("x", 1),
            ("", 1),
            ("-3", 1),
        ] {
            let mut tx = w.conn.begin().await.expect("begin");
            sqlx::query("SELECT set_config('sagat.step_log_wait_slots', $1, true)")
                .bind(value)
                .execute(&mut *tx)
                .await
                .expect("the transaction's setting");
            let read: i32 = sqlx::query_scalar("SELECT sagat_quota_step_log_wait_slots()")
                .fetch_one(&mut *tx)
                .await
                .expect("the slots");
            assert_eq!(read, slots, "{value:?}");
            tx.commit().await.expect("commit");
            let after: i32 = sqlx::query_scalar("SELECT sagat_quota_step_log_wait_slots()")
                .fetch_one(&mut w.conn)
                .await
                .expect("the next transaction's slots");
            assert_eq!(
                after, 1,
                "the setting of one transaction does not reach the next"
            );
        }
        fx.without_the_refill_bound().await;
        let replica = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let (row, holder) = hold_the_usage_row(fx.pool(), fx.tenant).await;
        let Writer { mut conn, pid } = fx.writer_named(&replica).await;
        let (tenant, stream, first) = (fx.tenant, fx.stream, fx.sequences(1));
        let first_waiter = tokio::spawn(async move {
            sqlx::query("BEGIN").execute(&mut conn).await?;
            super::insert_messages(&mut conn, tenant, stream, first, &[872]).await?;
            sqlx::query("COMMIT").execute(&mut conn).await.map(|_| ())
        });
        waits_on(fx.pool(), pid, holder).await;
        let mut second = fx.writer_named(&replica).await;
        let answer = tokio::time::timeout(
            Duration::from_secs(20),
            refill(
                &mut second.conn,
                fx.tenant,
                fx.stream,
                fx.sequences(1),
                None,
            ),
        )
        .await
        .expect("refused at once");
        assert_eq!(
            refusal(&answer),
            slots_taken(),
            "P = 1 when no value is set"
        );
        row.commit().await.expect("release");
        first_waiter
            .await
            .expect("join")
            .expect("the first waiter writes");
    })
    .await;
}

/// The first insert of a tenant whose usage row does not exist takes its slots before it inserts the row: with the
/// tenant's one slot (*P* = 1) held by another session, it is refused and the row is still absent; with the slot free,
/// it inserts the row and is charged.
#[tokio::test]
async fn the_first_insert_of_a_tenant_without_a_usage_row_takes_its_slots_before_it_inserts_the_row()
 {
    bounded::within("absent row", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        sqlx::query("DELETE FROM tenant_retained_usage WHERE tenant_id = $1")
            .bind(fx.tenant)
            .execute(fx.pool())
            .await
            .expect("no usage row");
        let replica = format!("sagat-octopus-orchestrator/{}", Uuid::new_v4());
        let mut blocker = fx.pool().acquire().await.expect("a session");
        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(format!("{}1", tenant_prefix(fx.tenant, &replica)))
            .execute(&mut *blocker)
            .await
            .expect("hold the tenant's one slot");
        let mut w = fx.writer_named(&replica).await;
        let answer = refill(&mut w.conn, fx.tenant, fx.stream, fx.sequences(1), None).await;
        assert_eq!(refusal(&answer), slots_taken());
        sqlx::query("ROLLBACK")
            .execute(&mut w.conn)
            .await
            .expect("rollback");
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1)",
        )
        .bind(fx.tenant)
        .fetch_one(fx.pool())
        .await
        .expect("the usage row");
        assert!(!exists, "the slots are taken before the row is inserted");
        sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(format!("{}1", tenant_prefix(fx.tenant, &replica)))
            .execute(&mut *blocker)
            .await
            .expect("free the slot");
        refill(&mut w.conn, fx.tenant, fx.stream, fx.sequences(1), None)
            .await
            .expect("now it inserts the row");
        sqlx::query("COMMIT")
            .execute(&mut w.conn)
            .await
            .expect("commit");
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 1_000);
    })
    .await;
}
