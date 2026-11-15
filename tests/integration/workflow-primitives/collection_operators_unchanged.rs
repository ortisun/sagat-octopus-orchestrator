//! FR-042-012, T042-51: a `forEach` without `operators` behaves and costs as it did before design 042. The entry of
//! such a collection runs the same statements, in the same transactions, writing the same rows, and stores the same
//! snapshot bytes as the release before the chain was applied (`0e94bf0e`, slice S0, where the numbers below were
//! measured with this same test).
//!
//! Counts, not times: nothing else runs in the test's database while the window is open, so the budget is an equality.

use sagat_domain::execution::ExecutionInstanceId;
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};
use sqlx::Row;

use crate::support::engine::{World, world};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Work {
    statements: i64,
    transactions: i64,
    rows_written: i64,
}

/// What the entry of the collection below cost before the chain was applied: measured with this test at `0e94bf0e` (S0)
/// and at `a4f6e9db` (develop before design 042), both 69, 2 and 22 (`specs/042-stream-operators/validation-report.md`).
const BEFORE: Work = Work {
    statements: 69,
    transactions: 2,
    rows_written: 22,
};

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

async fn evictions(w: &World) -> i64 {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(&w.db.pool)
        .await
        .expect("eviction counter")
}

fn collection(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "unchanged.line", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 10, "maxConcurrency": 2, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// One measured entry; `None` when `pg_stat_statements` evicted entries during the window.
async fn once() -> Option<(Work, Vec<u8>)> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("pg_stat_statements is preloaded by the test server");
    crate::limits::allow_iterations(&w, 4).await;
    let (workflow, _) = w.publish(collection("operators-unchanged")).await;
    let run = w
        .start(
            workflow,
            // A raw document, not `json!`: the macro writes `-0` as the integer `0` before anything reads it.
            &serde_json::from_str::<Value>(
                r#"{"lines": [{"v": 1, "k": "a"}, 2, "x", [1.0, -0], -0, -0.0, 1e2]}"#,
            )
            .expect("json"),
        )
        .await;
    let scheduler = crate::limits::scheduler(&w, "operators-unchanged").await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("node"));
    let evicted = evictions(&w).await;
    let before = work(&w).await;
    let entered = scheduler
        .advance_collection_from_run(w.admin.tenant_id(), run, &owner)
        .await
        .expect("entered");
    let after = work(&w).await;
    assert_eq!(entered, sagat_scheduler::CollectionAdvance::Entered);
    if evictions(&w).await != evicted {
        return None;
    }
    let snapshot: Vec<u8> = sqlx::query(
        "SELECT payload.content FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.snapshot_digest \
         WHERE scope.run_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the snapshot")
    .get("content");
    Some((
        Work {
            statements: after.statements - before.statements,
            transactions: after.transactions - before.transactions,
            rows_written: after.rows_written - before.rows_written,
        },
        snapshot,
    ))
}

#[tokio::test]
async fn collection_entry_without_operators_is_unchanged() {
    for _ in 0..8 {
        let Some((spent, snapshot)) = once().await else {
            continue;
        };
        println!("FR-042-012 entry without operators: {spent:?}");
        assert_eq!(
            String::from_utf8(snapshot).expect("utf-8"),
            r#"[{"k":"a","v":1},2,"x",[1.0,-0.0],-0.0,-0.0,100.0]"#,
            "the same snapshot bytes"
        );
        assert_eq!(spent, BEFORE, "the same statements, transactions and rows");
        return;
    }
    panic!("pg_stat_statements evicted entries during every one of eight attempts");
}
