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

const TABLES: [&str; 33] = [
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
    // Added by feature 008.
    "telemetry_pending",
    "engine_leases",
    "operational_alerts",
    "worker_drains",
    // Added by feature 010.
    "scheduler_shards",
    "scheduler_members",
    "tenant_scheduling",
    "run_active_counts",
    "activity_quota_limits",
    // Added by feature 002 completion hardening.
    "blobs",
    // Added for G48: the worker pools a secret may be delivered to, and the pools an identity holds.
    "secret_pools",
    "worker_binding_pools",
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

    let leftover_shard_fn: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'run_shard')")
            .fetch_one(&db.pool)
            .await
            .expect("query");
    assert!(
        !leftover_shard_fn,
        "rollback must also drop the function the shard indexes are built on"
    );
    let leftover_counter_fn: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'run_active_counts_track')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("query");
    assert!(
        !leftover_counter_fn,
        "rollback must also drop the function that keeps the count of active runs"
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

/// The stamp of the migration before the one that gives a worker binding its profile (D035-136).
const BEFORE_BINDING_PROFILE: i64 = 20_261_231_000_000;

#[tokio::test]
async fn the_binding_profile_rolls_back_only_while_every_binding_is_of_the_database_profile() {
    let db = migrated_db().await;
    let tenant: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), 'acme', now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .expect("tenant");
    let workspace: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, 'main', now()) RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&db.pool)
    .await
    .expect("workspace");
    let namespace: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES (gen_random_uuid(), $1, $2, 'dev', 'dev', now()) RETURNING id",
    )
    .bind(tenant)
    .bind(workspace)
    .fetch_one(&db.pool)
    .await
    .expect("namespace");

    // A binding written without a profile reads as the database profile.
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
         VALUES ($1, $2, 'sql-runner', 'sagat.database.postgresql', 'admin')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("binding");
    let default: String =
        sqlx::query_scalar("SELECT profile FROM worker_bindings WHERE subject = 'sql-runner'")
            .fetch_one(&db.pool)
            .await
            .expect("profile");
    assert_eq!(default, "database");

    // Only the two names are a profile.
    let invalid = sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by, profile) \
         VALUES ($1, $2, 'odd-runner', '*', 'admin', 'sql')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await;
    assert!(invalid.is_err(), "a profile is `database` or `lake`");

    // A lake binding stops the rollback, with the message that says why.
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by, profile) \
         VALUES ($1, $2, 'lake-runner', 'sagat.database.iceberg.write', 'admin', 'lake')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("lake binding");
    // sqlx takes an advisory lock for a rollback and does not release it when a revert fails, so the refused
    // rollback runs on a connection of its own and lets the lock go: the next rollback would wait on it for as
    // long as the pool keeps the connection (about twenty minutes).
    let mut connection = db.pool.acquire().await.expect("a connection");
    let refused = MIGRATOR
        .undo(&mut *connection, BEFORE_BINDING_PROFILE)
        .await
        .expect_err("a lake binding blocks the rollback");
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *connection)
        .await
        .expect("release the migration lock");
    drop(connection);
    let message = refused.to_string();
    assert!(
        message.contains("cannot roll back worker binding profiles"),
        "{message}"
    );
    let still_there: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'worker_bindings' AND column_name = 'profile')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("column");
    assert!(still_there, "a refused rollback leaves the column in place");

    // Once the operator has re-bound it, the rollback goes through and forward restores the default.
    sqlx::query("DELETE FROM worker_bindings WHERE subject = 'lake-runner'")
        .execute(&db.pool)
        .await
        .expect("remove the lake binding");
    MIGRATOR
        .undo(&db.pool, BEFORE_BINDING_PROFILE)
        .await
        .expect("rollback of the profile alone");
    let gone: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'worker_bindings' AND column_name = 'profile')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("column");
    assert!(gone, "the column is dropped");
    migrate(&db.pool).await.expect("forward again");
    let restored: String =
        sqlx::query_scalar("SELECT profile FROM worker_bindings WHERE subject = 'sql-runner'")
            .fetch_one(&db.pool)
            .await
            .expect("profile after forward");
    assert_eq!(
        restored, "database",
        "the surviving binding reads as the default again"
    );
}
