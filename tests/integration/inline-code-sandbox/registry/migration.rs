//! The migration of feature 005 goes back and forward.

use sagat_infra_postgres::{MIGRATOR, migrate};

use crate::support::migrated_db;

const TABLES: [&str; 6] = [
    "scripts",
    "script_versions",
    "script_artifacts",
    "script_build_jobs",
    "script_test_runs",
    "script_executions",
];

async fn exists(pool: &sqlx::PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{name}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

#[tokio::test]
async fn rolling_back_removes_the_scripts_and_going_forward_restores_them() {
    let db = migrated_db().await;
    for table in TABLES {
        assert!(
            exists(&db.pool, table).await,
            "{table} exists after migrating"
        );
    }
    // Back to the migration before this feature's.
    MIGRATOR
        .undo(&db.pool, 20_261_012_000_000)
        .await
        .expect("roll back");
    for table in TABLES {
        assert!(
            !exists(&db.pool, table).await,
            "{table} is gone after the rollback"
        );
    }
    let guards: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc WHERE proname IN ('script_versions_guard', 'script_artifacts_reject_mutation')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count");
    assert_eq!(guards, 0, "the guards go with the tables");
    let runner_allowed: bool = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) LIKE '%runner%' FROM pg_constraint WHERE conname = 'memberships_role_valid'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("constraint");
    assert!(!runner_allowed, "the role goes too");

    migrate(&db.pool).await.expect("forward again");
    for table in TABLES {
        assert!(exists(&db.pool, table).await, "{table} is back");
    }
}

#[tokio::test]
async fn a_rollback_with_published_versions_still_succeeds() {
    // The guards refuse the delete of a version; the rollback is the one operation that may remove
    // them, and it does so by dropping the guard first.
    let w = crate::support::engine::world().await;
    let (_, version) = crate::ready(&w, "to-be-rolled-back", "bytes").await;
    assert!(version.artifact.is_some());
    let _ = &w.store;
    MIGRATOR
        .undo(&w.db.pool, 20_261_012_000_000)
        .await
        .expect("roll back with data in the tables");
    assert!(!exists(&w.db.pool, "script_versions").await);
}
