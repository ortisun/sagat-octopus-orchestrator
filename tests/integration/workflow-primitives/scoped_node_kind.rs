//! A scoped step's kind is checked, not rewritten, when the step is written (design 038,
//! FR-038-006): the write of a step inside a scope carries the check of its stored kind.

use super::scoped_worker::{fixture, poll};
use crate::support::engine::task_ref;
use serde_json::json;

/// Times the separate statement that rewrote a step's kind has run in this database so far.
async fn kind_rewrites(world: &crate::support::engine::World) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND query LIKE 'UPDATE steps SET node_kind=%'",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("statement accounting")
}

#[tokio::test]
async fn a_scoped_report_does_not_rewrite_the_kind_its_step_already_has() {
    let fixture = fixture("scoped-kind-unchanged").await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&fixture.world.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let task = poll(&fixture, 1).await.remove(0);
    let before = kind_rewrites(&fixture.world).await;
    fixture
        .workers
        .complete(
            &fixture.session.principal,
            &task_ref(&task),
            &serde_json::to_vec(&json!({"value": 1})).expect("output"),
            "scoped-kind-unchanged-done",
        )
        .await
        .expect("completed");
    assert_eq!(
        kind_rewrites(&fixture.world).await - before,
        0,
        "the step already holds its kind: nothing rewrites it"
    );
}

/// The check stays: a scoped step whose stored kind is not its definition's is refused, and is
/// not repaired.
#[tokio::test]
async fn a_scoped_step_whose_stored_kind_is_not_its_definitions_is_refused() {
    let fixture = fixture("scoped-kind-mismatch").await;
    let task = poll(&fixture, 1).await.remove(0);
    let changed = sqlx::query(
        "UPDATE steps SET node_kind = 'timer' WHERE tenant_id = $1 AND run_id = $2 \
           AND step_id = $3",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(task.run_id.as_uuid())
    .bind(task.step_id)
    .execute(&fixture.world.db.pool)
    .await
    .expect("rewrite the stored kind")
    .rows_affected();
    assert_eq!(changed, 1);
    let refused = fixture
        .workers
        .complete(
            &fixture.session.principal,
            &task_ref(&task),
            &serde_json::to_vec(&json!({"value": 1})).expect("output"),
            "scoped-kind-mismatch-done",
        )
        .await;
    assert!(
        refused.is_err(),
        "a step with another kind is refused: {refused:?}"
    );
    let kind: String = sqlx::query_scalar("SELECT node_kind FROM steps WHERE step_id = $1")
        .bind(task.step_id)
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("stored kind");
    assert_eq!(kind, "timer", "and is not repaired");
}

/// A scoped row written before kinds were stored is given its kind by its next write.
#[tokio::test]
async fn a_legacy_scoped_step_is_given_its_kind_by_its_next_write() {
    let fixture = fixture("scoped-kind-legacy").await;
    let task = poll(&fixture, 1).await.remove(0);
    sqlx::query("UPDATE steps SET node_kind = 'legacyControl' WHERE step_id = $1")
        .bind(task.step_id)
        .execute(&fixture.world.db.pool)
        .await
        .expect("make the row legacy");
    fixture
        .workers
        .complete(
            &fixture.session.principal,
            &task_ref(&task),
            &serde_json::to_vec(&json!({"value": 1})).expect("output"),
            "scoped-kind-legacy-done",
        )
        .await
        .expect("completed");
    let kind: String = sqlx::query_scalar("SELECT node_kind FROM steps WHERE step_id = $1")
        .bind(task.step_id)
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("stored kind");
    assert_eq!(kind, "activity");
}
