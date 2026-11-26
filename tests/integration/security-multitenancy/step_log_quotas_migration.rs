//! The step-log quotas migration of design 041 (T041-4) against a real PostgreSQL: the four resources are
//! accepted by the ceilings tables, `tenant_retained_usage.step_log_bytes` exists, and the rollback refuses
//! while a limit or a ceiling of a step-log resource exists, then restores the previous lists.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::identity::TenantId;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sqlx::{Connection as _, PgPool};
use uuid::Uuid;

use crate::support::{TestDb, bounded, migrated_db};

/// The migration of T041-4.
const MIGRATION: i64 = 20_270_201_000_000;
const RESOURCES: [&str; 4] = [
    "tenant_step_log_bytes",
    "step_log_attempt_records",
    "step_log_attempt_bytes",
    "step_log_retention_days",
];

/// Rolls the schema back to `target` on a session of its own, as `schedules-bulk-operations/migration.rs`
/// does: a refused migration keeps its session advisory lock, so the lock is released and the session closed.
async fn undo_to(db: &TestDb, target: i64) -> Result<(), sqlx::migrate::MigrateError> {
    let mut connection = bounded::within(
        "connect the isolated migrator",
        bounded::START_LIMIT,
        sqlx::PgConnection::connect(&db.url),
    )
    .await
    .expect("a session for the migrator");
    let outcome = tokio::time::timeout(bounded::LIFECYCLE_LIMIT, async {
        let result = MIGRATOR.undo(&mut connection, target).await;
        let unlocked = if result.is_err() {
            Some(sqlx::migrate::Migrate::unlock(&mut connection).await)
        } else {
            None
        };
        (result, unlocked)
    })
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(15), connection.close())
        .await
        .expect("bounded close of the migrator session")
        .expect("close the migrator session");
    let (result, unlocked) = outcome.expect("the rollback finished within its limit");
    if let Some(unlocked) = unlocked {
        unlocked.expect("release the migration advisory lock after a refusal");
    }
    if result.is_err() {
        migrate(&db.pool)
            .await
            .expect("bring back the migrations undone before the refusal");
    }
    result
}

async fn tenant(pool: &PgPool) -> Uuid {
    let id = TenantId::generate().as_uuid();
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())")
        .bind(id)
        .bind(format!("t{}", &id.simple().to_string()[..12]))
        .execute(pool)
        .await
        .expect("a tenant");
    id
}

async fn ceiling(pool: &PgPool, tenant: Uuid, resource: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, $2, $3, 'test')",
    )
    .bind(tenant)
    .bind(resource)
    .bind(if resource == "step_log_attempt_bytes" { 1024_i64 } else { 1 })
    .execute(pool)
    .await
    .map(|_| ())
}

async fn limit(pool: &PgPool, tenant: Uuid, resource: &str) -> Result<(), sqlx::Error> {
    limit_of(
        pool,
        tenant,
        resource,
        if resource == "step_log_attempt_bytes" {
            1024
        } else {
            1
        },
    )
    .await
}

async fn limit_of(
    pool: &PgPool,
    tenant: Uuid,
    resource: &str,
    value: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id = $1 AND namespace_id IS NULL AND resource = $2",
    )
    .bind(tenant)
    .bind(resource)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, $2, $3, 'test')",
    )
    .bind(tenant)
    .bind(resource)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn namespace_of(pool: &PgPool, tenant: Uuid) -> Uuid {
    let workspace: Uuid = sqlx::query_scalar(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, 'main', now()) RETURNING id",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("a workspace");
    sqlx::query_scalar(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES (gen_random_uuid(), $1, $2, 'dev', 'dev', now()) RETURNING id",
    )
    .bind(tenant)
    .bind(workspace)
    .fetch_one(pool)
    .await
    .expect("a namespace")
}

async fn namespace_limit(
    pool: &PgPool,
    tenant: Uuid,
    namespace: Uuid,
    resource: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, $3, $4, 'test')",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(resource)
    .bind(if resource == "step_log_retention_days" {
        14_i64
    } else {
        1024
    })
    .execute(pool)
    .await
    .map(|_| ())
}

async fn ceiling_of(
    pool: &PgPool,
    tenant: Uuid,
    resource: &str,
    value: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM quota_ceilings WHERE tenant_id = $1 AND resource = $2")
        .bind(tenant)
        .bind(resource)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, $2, $3, 'test')",
    )
    .bind(tenant)
    .bind(resource)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn has_usage_column(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
          WHERE table_name = 'tenant_retained_usage' AND column_name = 'step_log_bytes')",
    )
    .fetch_one(pool)
    .await
    .expect("information schema")
}

#[tokio::test]
async fn the_step_log_resources_are_accepted_and_a_rollback_keeps_them() {
    let db = migrated_db().await;
    let t = tenant(&db.pool).await;
    assert!(has_usage_column(&db.pool).await);
    for resource in RESOURCES {
        ceiling(&db.pool, t, resource)
            .await
            .unwrap_or_else(|error| panic!("{resource} as a ceiling: {error}"));
        limit(&db.pool, t, resource)
            .await
            .unwrap_or_else(|error| panic!("{resource} as a tenant limit: {error}"));
    }
    // The ranges of data-model.md §6: a byte quota up to 2^50, 0 as the capture switch, and the minimums.
    for (resource, value, accepted) in [
        ("tenant_step_log_bytes", 1_i64 << 50, true),
        ("tenant_step_log_bytes", (1_i64 << 50) + 1, false),
        ("step_log_attempt_records", 0, true),
        ("step_log_attempt_records", 100_001, false),
        ("step_log_attempt_bytes", 1_023, false),
        ("step_log_attempt_bytes", 16 * 1024 * 1024, true),
        ("step_log_attempt_bytes", 16 * 1024 * 1024 + 1, false),
        ("step_log_retention_days", 0, false),
        ("step_log_retention_days", 365, true),
        ("step_log_retention_days", 366, false),
    ] {
        assert_eq!(
            limit_of(&db.pool, t, resource, value).await.is_ok(),
            accepted,
            "{resource} = {value}"
        );
    }
    // A ceiling takes the byte range too: the 1 GiB default and 2^50 (the ELSE branch capped it at 10^9).
    for (value, accepted) in [
        (1_i64 << 30, true),
        (1_i64 << 50, true),
        ((1_i64 << 50) + 1, false),
    ] {
        assert_eq!(
            ceiling_of(&db.pool, t, "tenant_step_log_bytes", value)
                .await
                .is_ok(),
            accepted,
            "tenant_step_log_bytes ceiling = {value}"
        );
    }
    // The three namespace resources may be set per namespace; the tenant quota may not.
    let ns = namespace_of(&db.pool, t).await;
    for resource in &RESOURCES[1..] {
        namespace_limit(&db.pool, t, ns, resource)
            .await
            .unwrap_or_else(|error| panic!("{resource} per namespace: {error}"));
    }
    assert!(
        namespace_limit(&db.pool, t, ns, "tenant_step_log_bytes")
            .await
            .is_err(),
        "tenant_step_log_bytes is tenant-scoped"
    );
    let usage: i64 = sqlx::query_scalar(
        "INSERT INTO tenant_retained_usage (tenant_id, content_bytes, content_reserved_bytes, log_bytes) \
         VALUES ($1, 0, 0, 0) ON CONFLICT (tenant_id) DO UPDATE SET tenant_id = EXCLUDED.tenant_id \
         RETURNING step_log_bytes",
    )
    .bind(t)
    .fetch_one(&db.pool)
    .await
    .expect("a usage row");
    assert_eq!(usage, 0, "the usage of the step-log class starts at 0");

    // Retained step-log usage refuses the rollback on its own.
    sqlx::query("DELETE FROM quota_limits WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("drop the limits");
    sqlx::query("DELETE FROM quota_ceilings WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("drop the ceilings");
    sqlx::query("UPDATE tenant_retained_usage SET step_log_bytes = 1 WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("usage");
    assert!(
        undo_to(&db, MIGRATION - 1).await.is_err(),
        "retained step-log usage refuses the rollback"
    );
    sqlx::query("UPDATE tenant_retained_usage SET step_log_bytes = 0 WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("usage");
    for resource in RESOURCES {
        ceiling(&db.pool, t, resource)
            .await
            .expect("a ceiling again");
        limit(&db.pool, t, resource).await.expect("a limit again");
    }

    // A limit or a ceiling of a step-log resource is a decision: the rollback refuses while one exists.
    assert!(
        undo_to(&db, MIGRATION - 1).await.is_err(),
        "the rollback refuses"
    );
    assert!(has_usage_column(&db.pool).await, "nothing was undone");
    sqlx::query("DELETE FROM quota_limits WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("drop the limits");
    assert!(
        undo_to(&db, MIGRATION - 1).await.is_err(),
        "a ceiling alone still refuses"
    );
    sqlx::query("DELETE FROM quota_ceilings WHERE tenant_id = $1")
        .bind(t)
        .execute(&db.pool)
        .await
        .expect("drop the ceilings");
    undo_to(&db, MIGRATION - 1)
        .await
        .expect("an empty rollback");
    assert!(!has_usage_column(&db.pool).await, "the column is gone");
    for resource in RESOURCES {
        assert!(
            ceiling(&db.pool, t, resource).await.is_err(),
            "{resource} is refused again"
        );
    }
    ceiling(&db.pool, t, "pending_resets")
        .await
        .expect("the previous list is restored");
    migrate(&db.pool).await.expect("migrate again");
    assert!(has_usage_column(&db.pool).await);
}
