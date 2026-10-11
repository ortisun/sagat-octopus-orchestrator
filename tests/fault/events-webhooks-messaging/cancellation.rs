//! A future cut off while it begins a transaction must not leave its connection in one: the next
//! user of the connection would run inside it, take its locks and never commit. `sqlx` 0.9.0 does
//! leave it (it sends `BEGIN` and only afterwards records the transaction, so the guard that rolls
//! back on drop has nothing to roll back); the stores begin through `BeginSafely`.

use std::time::Duration;

use sagat_infra_postgres::BeginSafely;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::support::fresh_db;

const ROUNDS: u64 = 3000;

/// Begins a transaction in 3000 futures cut off after a few microseconds each, on a pool of one
/// connection so that every round meets what the one before left, and counts the rounds that began
/// inside a transaction somebody left open.
async fn left_open(pool: &PgPool, safely: bool) -> u32 {
    let mut open = 0;
    for round in 0..ROUNDS {
        let micros = 15 + (round * 37) % 700;
        let _ = tokio::time::timeout(Duration::from_micros(micros), async {
            let tx = if safely {
                pool.begin_safely().await?
            } else {
                pool.begin().await?
            };
            drop(tx);
            Ok::<(), sqlx::Error>(())
        })
        .await;
        // A statement on its own is its own transaction, so `now()` moves from one to the next;
        // inside a transaction that is still open it does not.
        let first: time::OffsetDateTime = sqlx::query_scalar("SELECT now()")
            .fetch_one(pool)
            .await
            .expect("query");
        let second: time::OffsetDateTime = sqlx::query_scalar("SELECT now()")
            .fetch_one(pool)
            .await
            .expect("query");
        if first == second {
            open += 1;
            sqlx::query("ROLLBACK").execute(pool).await.expect("reset");
        }
    }
    open
}

async fn one_connection() -> (crate::support::TestDb, PgPool) {
    let db = fresh_db().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.url)
        .await
        .expect("pool");
    (db, pool)
}

#[tokio::test]
async fn a_transaction_that_is_cut_off_while_it_begins_leaves_nothing_open_on_the_connection() {
    let (_db, pool) = one_connection().await;
    let open = left_open(&pool, true).await;
    assert_eq!(
        open, 0,
        "{open} of {ROUNDS} cut-off begins left the connection in a transaction"
    );
}

/// The reason for `BeginSafely`. When this test fails, `sqlx` has been fixed: remove the helper
/// (every `begin_safely` becomes `begin` again) and this test.
#[tokio::test]
async fn sqlx_itself_leaves_the_connection_in_a_transaction_when_a_begin_is_cut_off() {
    let (_db, pool) = one_connection().await;
    let open = left_open(&pool, false).await;
    assert!(
        open > 0,
        "none of {ROUNDS} cut-off begins left a transaction open: sqlx may have been fixed, and \
         BeginSafely with it"
    );
}
