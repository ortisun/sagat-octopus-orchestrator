//! The schema's own defences, exercised with raw SQL that bypasses the application entirely
//! (T001-028: reject tenant spoofing even if the application layer is bypassed).

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::support::{db_error, migrated_db};

async fn tenant(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), $1, now()) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("insert tenant")
}

async fn try_workspace(pool: &PgPool, tenant: Uuid, slug: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, $2, now()) RETURNING id")
        .bind(tenant)
        .bind(slug)
        .fetch_one(pool)
        .await
}

async fn workspace(pool: &PgPool, tenant: Uuid, slug: &str) -> Uuid {
    try_workspace(pool, tenant, slug)
        .await
        .expect("insert workspace")
}

async fn try_namespace(
    pool: &PgPool,
    tenant: Uuid,
    workspace: Uuid,
    slug: &str,
    environment: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES (gen_random_uuid(), $1, $2, $3, $4, now()) RETURNING id",
    )
    .bind(tenant)
    .bind(workspace)
    .bind(slug)
    .bind(environment)
    .fetch_one(pool)
    .await
}

async fn try_membership(
    pool: &PgPool,
    tenant: Uuid,
    subject: &str,
    role: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) VALUES ($1, $2, $3, now())",
    )
    .bind(tenant)
    .bind(subject)
    .bind(role)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn try_audit(
    pool: &PgPool,
    tenant: Uuid,
    trace_id: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, trace_id, created_at) \
         VALUES (gen_random_uuid(), $1, 'alice', 'workspace.created', 'workspace', 'x', $2, now()) RETURNING id",
    )
    .bind(tenant)
    .bind(trace_id)
    .fetch_one(pool)
    .await
}

const CHECK_VIOLATION: &str = "23514";
const UNIQUE_VIOLATION: &str = "23505";
const FK_VIOLATION: &str = "23503";
const RESTRICT_VIOLATION: &str = "23001";

#[tokio::test]
async fn a_namespace_cannot_point_at_another_tenants_workspace() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let b = tenant(&db.pool, "globex").await;
    let workspace_of_b = workspace(&db.pool, b, "platform").await;

    // Tenant spoofing attempt: tenant A claims a workspace that belongs to tenant B.
    let err = try_namespace(&db.pool, a, workspace_of_b, "dev", "dev")
        .await
        .expect_err("rejected");

    let (code, constraint) = db_error(err);
    assert_eq!(code, FK_VIOLATION);
    assert_eq!(
        constraint.as_deref(),
        Some("namespaces_workspace_same_tenant_fk")
    );
}

#[tokio::test]
async fn a_namespace_in_the_right_tenant_is_accepted() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let ws = workspace(&db.pool, a, "platform").await;
    try_namespace(&db.pool, a, ws, "dev", "dev")
        .await
        .expect("same tenant");
}

#[tokio::test]
async fn slug_format_is_enforced_by_the_database_itself() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let ws = workspace(&db.pool, a, "platform").await;

    for bad in [
        "Bad_Slug",
        "ab",
        "-abc",
        "abc-",
        "a--b",
        "UPPER",
        "has space",
        "../x",
        "",
    ] {
        let t = sqlx::query(
            "INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), $1, now())",
        )
        .bind(bad)
        .execute(&db.pool)
        .await
        .expect_err("tenant slug");
        assert_eq!(
            db_error(t),
            (CHECK_VIOLATION.into(), Some("tenants_slug_format".into())),
            "{bad:?}"
        );

        let w = try_workspace(&db.pool, a, bad)
            .await
            .expect_err("workspace slug");
        assert_eq!(
            db_error(w),
            (
                CHECK_VIOLATION.into(),
                Some("workspaces_slug_format".into())
            ),
            "{bad:?}"
        );

        let n = try_namespace(&db.pool, a, ws, bad, "dev")
            .await
            .expect_err("namespace slug");
        assert_eq!(
            db_error(n),
            (
                CHECK_VIOLATION.into(),
                Some("namespaces_slug_format".into())
            ),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn slugs_are_unique_per_tenant_but_reusable_across_tenants() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let b = tenant(&db.pool, "globex").await;

    // Workspaces: unique per tenant.
    let ws_a = workspace(&db.pool, a, "platform").await;
    let dup_ws = try_workspace(&db.pool, a, "platform")
        .await
        .expect_err("dup workspace");
    assert_eq!(
        db_error(dup_ws),
        (
            UNIQUE_VIOLATION.into(),
            Some("workspaces_tenant_slug_uniq".into())
        )
    );
    let ws_b = workspace(&db.pool, b, "platform").await; // same name, other tenant: fine

    // Namespaces: unique per TENANT (not per workspace), so a name resolves unambiguously.
    let other_ws_a = workspace(&db.pool, a, "second").await;
    try_namespace(&db.pool, a, ws_a, "dev", "dev")
        .await
        .expect("first");
    let dup_ns = try_namespace(&db.pool, a, other_ws_a, "dev", "prod")
        .await
        .expect_err("dup ns");
    assert_eq!(
        db_error(dup_ns),
        (
            UNIQUE_VIOLATION.into(),
            Some("namespaces_tenant_slug_uniq".into())
        )
    );
    try_namespace(&db.pool, b, ws_b, "dev", "dev")
        .await
        .expect("same name, other tenant: fine");

    // Tenant slugs are global.
    let dup_tenant = sqlx::query(
        "INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), 'acme', now())",
    )
    .execute(&db.pool)
    .await
    .expect_err("dup tenant");
    assert_eq!(
        db_error(dup_tenant),
        (UNIQUE_VIOLATION.into(), Some("tenants_slug_uniq".into()))
    );
}

#[tokio::test]
async fn environment_role_and_subject_values_are_constrained() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let ws = workspace(&db.pool, a, "platform").await;

    let env = try_namespace(&db.pool, a, ws, "qa-1", "qa")
        .await
        .expect_err("environment");
    assert_eq!(
        db_error(env),
        (
            CHECK_VIOLATION.into(),
            Some("namespaces_environment_valid".into())
        )
    );

    let role = try_membership(&db.pool, a, "mallory", "root")
        .await
        .expect_err("role");
    assert_eq!(
        db_error(role),
        (
            CHECK_VIOLATION.into(),
            Some("memberships_role_valid".into())
        )
    );

    for bad in ["", "line\nbreak", "tab\there"] {
        let s = try_membership(&db.pool, a, bad, "viewer")
            .await
            .expect_err("subject");
        assert_eq!(
            db_error(s),
            (
                CHECK_VIOLATION.into(),
                Some("memberships_subject_valid".into())
            ),
            "{bad:?}"
        );
    }
    try_membership(&db.pool, a, "alice", "viewer")
        .await
        .expect("valid membership");
}

#[tokio::test]
async fn a_subject_has_at_most_one_membership_per_tenant() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let b = tenant(&db.pool, "globex").await;

    try_membership(&db.pool, a, "alice", "admin")
        .await
        .expect("first");
    let dup = try_membership(&db.pool, a, "alice", "viewer")
        .await
        .expect_err("duplicate");
    assert_eq!(db_error(dup).0, UNIQUE_VIOLATION);
    try_membership(&db.pool, b, "alice", "viewer")
        .await
        .expect("same subject, other tenant");
}

#[tokio::test]
async fn the_audit_trail_is_append_only() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    let id = try_audit(&db.pool, a, Some("4bf92f3577b34da6a3ce929d0e0e4736"))
        .await
        .expect("append");

    let update = sqlx::query("UPDATE audit_records SET actor = 'mallory' WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .expect_err("update");
    assert_eq!(db_error(update).0, RESTRICT_VIOLATION);

    let delete = sqlx::query("DELETE FROM audit_records WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .expect_err("delete");
    assert_eq!(db_error(delete).0, RESTRICT_VIOLATION);

    let truncate = sqlx::query("TRUNCATE audit_records")
        .execute(&db.pool)
        .await
        .expect_err("truncate");
    assert_eq!(db_error(truncate).0, RESTRICT_VIOLATION);

    let actor: String = sqlx::query_scalar("SELECT actor FROM audit_records WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("row survives");
    assert_eq!(actor, "alice");
}

#[tokio::test]
async fn audit_trace_ids_must_be_w3c_trace_ids() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;

    try_audit(&db.pool, a, None)
        .await
        .expect("absent trace id is allowed");
    try_audit(&db.pool, a, Some("4bf92f3577b34da6a3ce929d0e0e4736"))
        .await
        .expect("valid");
    for bad in [
        "abc",
        "4BF92F3577B34DA6A3CE929D0E0E4736",
        "4bf92f3577b34da6a3ce929d0e0e473z",
        "'; DROP TABLE tenants;--",
    ] {
        let err = try_audit(&db.pool, a, Some(bad))
            .await
            .expect_err("bad trace id");
        assert_eq!(
            db_error(err),
            (
                CHECK_VIOLATION.into(),
                Some("audit_records_trace_id_format".into())
            ),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn deleting_a_tenant_that_owns_data_is_blocked_not_cascaded() {
    let db = migrated_db().await;
    let a = tenant(&db.pool, "acme").await;
    workspace(&db.pool, a, "platform").await;

    let err = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(a)
        .execute(&db.pool)
        .await
        .expect_err("no cascade");

    assert_eq!(db_error(err).0, FK_VIOLATION);
}
