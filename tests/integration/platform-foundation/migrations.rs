//! Migration behaviour: idempotent, reversible, safe under concurrent runners (FR-001-004, US03).

use sagat_infra_postgres::{MIGRATOR, migrate};

use crate::support::{fresh_db, migrated_db};

async fn table_exists(pool: &sqlx::PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{name}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

/// Number of forward migrations. The migrator lists `.up` and `.down` files as separate entries but
/// `_sqlx_migrations` records only the forward ones.
fn forward_migrations() -> i64 {
    let count = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .count();
    i64::try_from(count).expect("count fits")
}

const TABLES: [&str; 21] = [
    "tenants",
    "workspaces",
    "namespaces",
    "memberships",
    "audit_records",
    // Added by feature 002; the rollback must remove them too.
    "workflows",
    "workflow_versions",
    "command_dedupe",
    "payloads",
    "runs",
    "run_events",
    "steps",
    "worker_sessions",
    "event_outbox",
    // Added by feature 009.
    "quota_limits",
    "quota_ceilings",
    "worker_bindings",
    "secrets",
    "secret_versions",
    "secret_access",
    "egress_policies",
];

#[tokio::test]
async fn applying_migrations_twice_is_a_no_op() {
    let db = fresh_db().await;

    migrate(&db.pool).await.expect("first run");
    migrate(&db.pool).await.expect("second run must not fail");

    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(applied, forward_migrations());
    for table in TABLES {
        assert!(table_exists(&db.pool, table).await, "{table} must exist");
    }
}

#[tokio::test]
async fn rollback_removes_everything_and_forward_restores_it() {
    let db = migrated_db().await;

    MIGRATOR.undo(&db.pool, 0).await.expect("rollback to empty");
    for table in TABLES {
        assert!(
            !table_exists(&db.pool, table).await,
            "{table} must be gone after rollback"
        );
    }
    let leftover_fn: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'audit_records_reject_mutation')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("query");
    assert!(!leftover_fn, "rollback must also drop the trigger function");
    let leftover_execution: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'execution_reject_mutation')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("query");
    assert!(
        !leftover_execution,
        "rollback must also drop the journal's trigger function"
    );
    let leftover_outbox: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'outbox_reject_undelivered_delete')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("query");
    assert!(
        !leftover_outbox,
        "rollback must also drop the outbox's trigger function"
    );

    migrate(&db.pool).await.expect("forward again");
    for table in TABLES {
        assert!(
            table_exists(&db.pool, table).await,
            "{table} must exist after re-applying"
        );
    }
}

#[tokio::test]
async fn concurrent_runners_do_not_corrupt_or_fail() {
    // Rolling deploys start several instances at once; sqlx serialises them with an advisory lock.
    let db = fresh_db().await;

    let runs: Vec<_> = (0..4)
        .map(|_| {
            let pool = db.pool.clone();
            tokio::spawn(async move { migrate(&pool).await })
        })
        .collect();
    for run in runs {
        run.await
            .expect("join")
            .expect("every concurrent runner succeeds");
    }

    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(applied, forward_migrations(), "applied once");
}

#[tokio::test]
async fn rolled_back_schema_can_hold_data_again_after_re_applying() {
    let db = migrated_db().await;
    MIGRATOR.undo(&db.pool, 0).await.expect("rollback");
    migrate(&db.pool).await.expect("forward");

    sqlx::query(
        "INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), 'acme', now())",
    )
    .execute(&db.pool)
    .await
    .expect("schema is usable");
}

#[test]
fn every_forward_migration_has_a_rollback() {
    let up = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .count();
    let down = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_down_migration())
        .count();
    assert!(up > 0, "no migrations embedded");
    assert_eq!(
        up, down,
        "each forward migration needs a matching .down.sql"
    );
}
