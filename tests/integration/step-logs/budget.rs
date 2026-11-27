//! Design 041 section 2, "Statements per batch", task T041-6: a batch costs exactly 13 round trips whatever the
//! number of its records: `BEGIN`, T1 (which sets the lock timeout too, decision D041-146 (12)), T2, the session, the statement that takes the six policy
//! fences and the stream lock, the one that reads the three values, T4, the tenant allowance, the binding's patterns,
//! the membership, T8, T9 and `COMMIT`. Counted with `pg_stat_statements` as `work_budget.rs` counts: top-level
//! statements only, so that what the quota trigger runs inside T8 is not counted (its own budget line is task
//! T041-5's).

use sagat_application::step_logs::StepLogVerdict;

use crate::support::engine::{World, world};
use sagat_application::clock::SystemClock;
use sagat_application::step_logs::StepLogIngest;
use sagat_infra_postgres::PgStepLogStore;

use crate::support::step_logs::{Attempt, ingest_on_own_pool, leased, records, send};

/// The round trips of one stored batch (an equality, not a ceiling).
const BATCH: i64 = 13;

/// Every top-level call this test database has executed so far, `BEGIN` and `COMMIT` included, but for the readings.
async fn calls(w: &World) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_statements \
          WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND toplevel AND query NOT LIKE '%pg_stat_statements%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("statement accounting for this test database")
}

/// Calls per statement so far, for the message of a failed budget: it names what was added.
async fn itemised(w: &World) -> std::collections::BTreeMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT left(regexp_replace(query, '\\s+', ' ', 'g'), 90), sum(calls)::bigint FROM pg_stat_statements \
          WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND toplevel AND query NOT LIKE '%pg_stat_statements%' GROUP BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("statements of this test database");
    rows.into_iter().collect()
}

fn added(
    before: &std::collections::BTreeMap<String, i64>,
    after: &std::collections::BTreeMap<String, i64>,
) -> String {
    after
        .iter()
        .filter_map(|(query, calls)| {
            let was = before.get(query).copied().unwrap_or(0);
            (*calls != was).then(|| format!("{:+} {query}", calls - was))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn evictions(w: &World) -> i64 {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(&w.db.pool)
        .await
        .expect("eviction counter")
}

/// The calls one batch of `count` records costs, measured again when `pg_stat_statements` evicted entries in the
/// window (every test database adds entries, and a full table restarts the evicted counts).
async fn measured(
    w: &World,
    ingest: &StepLogIngest<PgStepLogStore, SystemClock>,
    a: &Attempt,
    first: u64,
    count: u64,
) -> (i64, String) {
    let mut next = first;
    for _ in 0..8 {
        let evicted = evictions(w).await;
        let items = itemised(w).await;
        let before = calls(w).await;
        let answer = send(ingest, a, records(next..next + count)).await;
        let after = calls(w).await;
        let listed = added(&items, &itemised(w).await);
        assert_eq!(answer.verdict, StepLogVerdict::Continue);
        next += count;
        if evictions(w).await == evicted {
            return (after - before, listed);
        }
    }
    panic!("pg_stat_statements evicted entries during every one of eight attempts");
}

#[tokio::test]
async fn a_batch_costs_thirteen_round_trips_whatever_its_number_of_records() {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let a = leased(&w, "a").await;
    // One connection, whose first batch pays for what is prepared or looked up once per connection.
    let ingest = ingest_on_own_pool(&w, 1).await;
    send(&ingest, &a, records(1..=500)).await;
    let one = measured(&w, &ingest, &a, 600, 1).await;
    let many = measured(&w, &ingest, &a, 1_000, 500).await;
    assert_eq!(
        (one.0, many.0),
        (BATCH, BATCH),
        "one record:\n{}\n500 records:\n{}",
        one.1,
        many.1
    );
}
