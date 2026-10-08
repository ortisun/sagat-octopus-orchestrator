//! The least-privilege runtime role and the protections of the audit trail (security review,
//! finding 4). Raw SQL as that role: whatever it cannot do here, a compromised control plane
//! cannot do either.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Uuid;

use crate::support::{
    RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, TestDb, db_error, migrated_db, two_tenants,
};

const INSUFFICIENT_PRIVILEGE: &str = "42501";
const RESTRICT_VIOLATION: &str = "23001";

async fn runtime_pool(db: &TestDb) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD))
        .await
        .expect("connect as the runtime role")
}

#[tokio::test]
async fn the_runtime_role_can_do_exactly_what_the_control_plane_needs() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await; // provisioned by the owner, as an operator would
    let app = runtime_pool(&db).await;
    let tenant: Uuid = w.tenant_a.id().as_uuid();

    // Reads everywhere it must.
    for table in [
        "tenants",
        "memberships",
        "workspaces",
        "namespaces",
        "audit_records",
    ] {
        let sql = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&app)
            .await
            .expect(table);
        assert!(n >= 0);
    }
    // Writes where the API creates data.
    let workspace: Uuid = sqlx::query_scalar(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, 'platform', now()) RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&app)
    .await
    .expect("insert workspace");
    sqlx::query("INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) VALUES (gen_random_uuid(), $1, $2, 'dev', 'dev', now())")
        .bind(tenant)
        .bind(workspace)
        .execute(&app)
        .await
        .expect("insert namespace");
    sqlx::query("INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, created_at) VALUES (gen_random_uuid(), $1, 'alice', 'workspace.created', 'workspace', 'x', now())")
        .bind(tenant)
        .execute(&app)
        .await
        .expect("append audit");
}

#[tokio::test]
async fn the_runtime_role_cannot_rewrite_remove_or_restructure_anything() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let app = runtime_pool(&db).await;
    let tenant = w.tenant_a.id().as_uuid();
    // Something to attack in every table.
    sqlx::query("INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, created_at) VALUES (gen_random_uuid(), $1, 'alice', 'workspace.created', 'workspace', 'x', now())")
        .bind(tenant)
        .execute(&db.pool)
        .await
        .expect("seed audit row as owner");

    let audit_rows_before: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&db.pool)
        .await
        .expect("count");

    let forbidden: Vec<String> = ["tenants", "memberships", "workspaces", "namespaces", "audit_records"]
        .into_iter()
        .flat_map(|t| [format!("UPDATE {t} SET created_at = now()"), format!("DELETE FROM {t}"), format!("TRUNCATE {t}")])
        .chain(
            [
                "INSERT INTO tenants (id, slug, created_at) VALUES (gen_random_uuid(), 'rogue-tenant', now())",
                "INSERT INTO memberships (tenant_id, subject, role, created_at) SELECT id, 'mallory', 'admin', now() FROM tenants LIMIT 1",
                "DROP TABLE audit_records",
                "ALTER TABLE audit_records DISABLE TRIGGER ALL",
                "ALTER TABLE audit_records DISABLE TRIGGER audit_records_append_only",
                "DROP TRIGGER audit_records_append_only ON audit_records",
                "ALTER TABLE audit_records OWNER TO sagat_app",
                "CREATE TABLE evil (id int)",
                "CREATE FUNCTION evil() RETURNS int LANGUAGE sql AS 'SELECT 1'",
                "SET session_replication_role = replica",
                "SELECT * FROM _sqlx_migrations",
            ]
            .map(str::to_owned),
        )
        .collect();

    for statement in forbidden {
        let result = sqlx::query(sqlx::AssertSqlSafe(statement.clone()))
            .execute(&app)
            .await;
        let error = result.expect_err(&format!(
            "the runtime role must NOT be able to run: {statement}"
        ));
        assert_eq!(db_error(error).0, INSUFFICIENT_PRIVILEGE, "{statement}");
    }

    // PostgreSQL only WARNS when a non-owner without grant option runs GRANT (it grants nothing and
    // reports no error), so the assertion is on the effect: no privilege reached PUBLIC.
    let _ = sqlx::query("GRANT ALL ON audit_records TO PUBLIC")
        .execute(&app)
        .await;
    let public_grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.role_table_grants WHERE table_name = 'audit_records' AND grantee = 'PUBLIC'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("grants");
    assert_eq!(
        public_grants, 0,
        "the runtime role must not be able to widen access"
    );

    // And nothing changed.
    let audit_rows_after: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(audit_rows_after, audit_rows_before);
}

#[tokio::test]
async fn the_audit_triggers_fire_even_in_a_session_that_bypasses_triggers() {
    // `session_replication_role = replica` skips ordinary triggers; the migration marks these ALWAYS.
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    sqlx::query("INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, created_at) VALUES (gen_random_uuid(), $1, 'alice', 'workspace.created', 'workspace', 'x', now())")
        .bind(w.tenant_a.id().as_uuid())
        .execute(&db.pool)
        .await
        .expect("seed");

    let mut conn = db.pool.acquire().await.expect("owner connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("the owner may set it");
    for statement in [
        "UPDATE audit_records SET actor = 'mallory'",
        "DELETE FROM audit_records",
        "TRUNCATE audit_records",
    ] {
        let error = sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .expect_err(statement);
        assert_eq!(db_error(error).0, RESTRICT_VIOLATION, "{statement}");
    }

    let modes: Vec<String> = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger WHERE tgrelid = 'audit_records'::regclass AND NOT tgisinternal ORDER BY tgname",
    )
    .fetch_all(&db.pool)
    .await
    .expect("trigger modes");
    assert_eq!(modes, ["A", "A"], "both triggers are ENABLE ALWAYS");
}
