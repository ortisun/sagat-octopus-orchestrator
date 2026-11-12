//! What a lease and a completion cost the database, as counts that a change must not raise
//! unnoticed (FR-038-001). Counts, not times: they do not depend on how loaded the machine is.
//!
//! One worker, one run at a time, no scheduler tick and no other session: nothing races, so the
//! counts are the same on every run and the budget is an equality. A change that makes the path
//! cheaper lowers the numbers here in the same change; one that makes it dearer says why.

use crate::support::engine::{World, activity, document, with, world};
use serde_json::json;
use std::collections::BTreeMap;

/// The statements, the transactions and the rows written by what ran between two readings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Work {
    statements: i64,
    transactions: i64,
    rows_written: i64,
}

impl std::ops::Sub for Work {
    type Output = Self;

    fn sub(self, before: Self) -> Self {
        Self {
            statements: self.statements - before.statements,
            transactions: self.transactions - before.transactions,
            rows_written: self.rows_written - before.rows_written,
        }
    }
}

/// Everything this test database has executed so far, but for the readings themselves. `BEGIN`
/// counts the transactions and is left out of the statements. Only top-level statements: what a trigger
/// runs inside one is counted or not by the server's `pg_stat_statements.track`, and the budget must
/// not depend on it.
async fn work(w: &World) -> Work {
    let (statements, transactions, rows_written): (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(calls) FILTER (WHERE query NOT IN ('BEGIN', 'COMMIT', 'ROLLBACK')), 0)::bigint, \
                coalesce(sum(calls) FILTER (WHERE query = 'BEGIN'), 0)::bigint, \
                coalesce(sum(rows) FILTER (WHERE query ~* '^\\s*(INSERT|UPDATE|DELETE)'), 0)::bigint \
         FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND toplevel AND query NOT LIKE '%pg_stat_statements%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("statement accounting for this test database");
    Work {
        statements,
        transactions,
        rows_written,
    }
}

/// Calls per statement so far, for the message of a failed budget: it names what was added.
async fn calls(w: &World) -> BTreeMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT left(regexp_replace(query, '\\s+', ' ', 'g'), 100), sum(calls)::bigint \
         FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND toplevel AND query NOT LIKE '%pg_stat_statements%' GROUP BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("statements of this test database");
    rows.into_iter().collect()
}

/// One more run, with the statements of the lease and of the completion of its second step listed.
async fn itemised(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    session: &crate::support::engine::Session,
) -> String {
    w.start(workflow, &json!({})).await;
    let first = w.poll(session, 1).await.remove(0);
    w.complete(session, &first, &json!({})).await.expect("a");
    let mut text = String::new();
    let mut before = calls(w).await;
    let task = w.poll(session, 1).await.remove(0);
    for (title, done) in [("lease", false), ("completion", true)] {
        if done {
            w.complete(session, &task, &json!({})).await.expect("b");
        }
        let after = calls(w).await;
        text.push_str(&format!("\n{title}:\n"));
        for (query, count) in &after {
            let delta = count - before.get(query).copied().unwrap_or(0);
            if delta > 0 {
                text.push_str(&format!("  {delta:>3}  {query}\n"));
            }
        }
        before = after;
    }
    text
}

/// The cost of each of the six commands of one run of three chained activities, in order: lease
/// and completion of `a`, of `b` and of `c` (whose completion ends the run).
async fn one_run(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    session: &crate::support::engine::Session,
) -> [Work; 6] {
    w.start(workflow, &json!({})).await;
    let mut costs = Vec::with_capacity(6);
    for _ in 0..3 {
        let before = work(w).await;
        let mut tasks = w.poll(session, 1).await;
        costs.push(work(w).await - before);
        assert_eq!(tasks.len(), 1, "the next step of the chain is leased");
        let task = tasks.remove(0);
        let before = work(w).await;
        w.complete(session, &task, &json!({}))
            .await
            .expect("completed");
        costs.push(work(w).await - before);
    }
    costs.try_into().expect("six commands")
}

const fn cost(statements: i64, transactions: i64, rows_written: i64) -> Work {
    Work {
        statements,
        transactions,
        rows_written,
    }
}

/// The budget. Lower a line in the change that earns it; raise one only with the reason beside it.
const LEASE: Work = cost(46, 2, 6);
const COMPLETION_WITH_A_NEXT_STEP: Work = cost(58, 1, 12);
const COMPLETION_THAT_ENDS_THE_RUN: Work = cost(55, 1, 10);

#[tokio::test]
async fn a_lease_and_a_completion_cost_what_the_budget_says() {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let (workflow, _) = w
        .publish(document(
            "budget-chain",
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(activity("b"), "next", json!("c")),
                activity("c"),
            ],
        ))
        .await;
    let session = w.session(&["act.a", "act.b", "act.c"]).await;
    // The first run pays for what is cached afterwards (the definition, prepared statements).
    one_run(&w, workflow, &session).await;
    let first = one_run(&w, workflow, &session).await;
    let second = one_run(&w, workflow, &session).await;
    assert_eq!(first, second, "the counts of a quiet database do not vary");
    let [lease_a, done_a, lease_b, done_b, lease_c, done_c] = first;
    if (lease_a, lease_b, lease_c) != (LEASE, LEASE, LEASE)
        || (done_a, done_b) != (COMPLETION_WITH_A_NEXT_STEP, COMPLETION_WITH_A_NEXT_STEP)
        || done_c != COMPLETION_THAT_ENDS_THE_RUN
    {
        panic!(
            "the work of a lease or of a completion left its budget (lower the budget with the \
             change that earns it; raise it only with the reason beside it).\n\
             lease: {lease_a:?} {lease_b:?} {lease_c:?}, budget {LEASE:?}\n\
             completion with a next step: {done_a:?} {done_b:?}, budget {COMPLETION_WITH_A_NEXT_STEP:?}\n\
             completion that ends the run: {done_c:?}, budget {COMPLETION_THAT_ENDS_THE_RUN:?}\n\
             statements of the second step of one more run:{}",
            itemised(&w, workflow, &session).await
        );
    }
}
