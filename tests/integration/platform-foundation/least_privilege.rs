//! The runtime role guard (second security review, finding 10): outside development the control
//! plane refuses to serve with a database role that could rewrite the audit trail, however the
//! credentials ended up in `DATABASE_URL`.
#![allow(clippy::panic)]

use sagat_infra_postgres::{LeastPrivilegeError, PgIdentityStore, PrivilegeFinding};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, TestDb, fresh_db, migrated_db};

const PASSWORD: &str = "s3cr3t-role-pw-for-tests";

/// A fresh, unprivileged login role (names are cluster-wide, so each test makes its own).
async fn new_role(db: &TestDb) -> String {
    let role = format!("lp_{}", uuid_suffix());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN PASSWORD '{PASSWORD}' NOSUPERUSER NOCREATEDB NOCREATEROLE"
    )))
    .execute(&db.pool)
    .await
    .expect("create role");
    role
}

fn uuid_suffix() -> String {
    sagat_domain::identity::TenantId::generate()
        .as_uuid()
        .simple()
        .to_string()
}

async fn as_role(db: &TestDb, role: &str, password: &str) -> PgIdentityStore {
    let pool: PgPool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.url_for(role, password))
        .await
        .expect("connect as the role");
    PgIdentityStore::new(pool)
}

async fn grant(db: &TestDb, statement: String) {
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&db.pool)
        .await
        .expect("grant");
}

fn findings(error: LeastPrivilegeError) -> Vec<PrivilegeFinding> {
    match error {
        LeastPrivilegeError::TooPrivileged { findings, .. } => findings,
        other => panic!("expected TooPrivileged, got {other:?}"),
    }
}

#[tokio::test]
async fn the_runtime_role_is_accepted() {
    let db = migrated_db().await;
    let store = as_role(&db, RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD).await;

    store
        .verify_least_privilege()
        .await
        .expect("sagat_app is exactly the role the control plane should run as");
}

#[tokio::test]
async fn a_superuser_that_owns_the_audit_trail_is_refused_with_every_reason() {
    let db = migrated_db().await;
    // The migrations ran as this role, so it is the owner of audit_records and a superuser.
    let store = PgIdentityStore::new(db.pool.clone());

    let error = store
        .verify_least_privilege()
        .await
        .expect_err("the database owner must not serve requests");

    let found = findings(error);
    for expected in [
        PrivilegeFinding::Superuser,
        PrivilegeFinding::OwnsAuditTrail,
        PrivilegeFinding::CanUpdateAuditTrail,
        PrivilegeFinding::CanDeleteAuditTrail,
        PrivilegeFinding::CanTruncateAuditTrail,
    ] {
        assert!(
            found.contains(&expected),
            "{expected:?} missing in {found:?}"
        );
    }
}

#[tokio::test]
async fn an_owner_that_is_not_a_superuser_is_still_refused() {
    let db = migrated_db().await;
    let owner = new_role(&db).await;
    grant(&db, format!("ALTER TABLE audit_records OWNER TO {owner}")).await;
    let store = as_role(&db, &owner, PASSWORD).await;

    let found = findings(store.verify_least_privilege().await.expect_err("owner"));

    assert!(
        found.contains(&PrivilegeFinding::OwnsAuditTrail),
        "{found:?}"
    );
    assert!(!found.contains(&PrivilegeFinding::Superuser), "{found:?}");
}

#[tokio::test]
async fn belonging_to_the_owner_role_counts_as_owning() {
    let db = migrated_db().await;
    let owner = new_role(&db).await;
    let member = new_role(&db).await;
    grant(&db, format!("ALTER TABLE audit_records OWNER TO {owner}")).await;
    grant(&db, format!("GRANT {owner} TO {member}")).await;
    let store = as_role(&db, &member, PASSWORD).await;

    let found = findings(store.verify_least_privilege().await.expect_err("member"));

    assert!(
        found.contains(&PrivilegeFinding::OwnsAuditTrail),
        "{found:?}"
    );
}

#[tokio::test]
async fn each_write_privilege_on_the_audit_trail_is_refused_on_its_own() {
    let cases = [
        ("UPDATE", PrivilegeFinding::CanUpdateAuditTrail),
        // A grant on a single column is enough to rewrite that column of every audit row.
        ("UPDATE (actor)", PrivilegeFinding::CanUpdateAuditTrail),
        ("DELETE", PrivilegeFinding::CanDeleteAuditTrail),
        ("TRUNCATE", PrivilegeFinding::CanTruncateAuditTrail),
    ];
    for (privilege, expected) in cases {
        let db = migrated_db().await;
        let role = new_role(&db).await;
        grant(
            &db,
            format!("GRANT SELECT, INSERT ON audit_records TO {role}"),
        )
        .await;
        grant(&db, format!("GRANT {privilege} ON audit_records TO {role}")).await;
        let store = as_role(&db, &role, PASSWORD).await;

        let found = findings(store.verify_least_privilege().await.expect_err(privilege));

        assert_eq!(found, vec![expected], "only {privilege} was granted");
    }
}

#[tokio::test]
async fn a_role_that_can_only_read_and_append_is_accepted() {
    let db = migrated_db().await;
    let role = new_role(&db).await;
    grant(
        &db,
        format!("GRANT SELECT, INSERT ON audit_records TO {role}"),
    )
    .await;
    let store = as_role(&db, &role, PASSWORD).await;

    store
        .verify_least_privilege()
        .await
        .expect("append-only access is what the runtime needs");
}

#[tokio::test]
async fn an_unmigrated_database_cannot_be_verified_and_is_refused() {
    // Without the table the privileges cannot be established, and the owner role that later runs
    // the migrations would own the table the process then serves with. Fail closed.
    let db = fresh_db().await;
    let role = new_role(&db).await;
    let store = as_role(&db, &role, PASSWORD).await;

    let error = store
        .verify_least_privilege()
        .await
        .expect_err("no audit table");

    assert_eq!(error, LeastPrivilegeError::SchemaMissing);
    assert!(error.to_string().contains("migrat"), "{error}");
}

#[tokio::test]
async fn the_refusal_names_the_problem_and_never_a_credential() {
    let db = migrated_db().await;
    let owner = new_role(&db).await;
    grant(&db, format!("ALTER TABLE audit_records OWNER TO {owner}")).await;
    let store = as_role(&db, &owner, PASSWORD).await;

    let message = store
        .verify_least_privilege()
        .await
        .expect_err("owner")
        .to_string();

    assert!(
        message.contains("owns") && message.contains("audit"),
        "the message says what is wrong: {message}"
    );
    for secret in [PASSWORD, "postgres://", "@127.0.0.1"] {
        assert!(
            !message.contains(secret),
            "credential material in: {message}"
        );
    }
}

#[tokio::test]
async fn each_append_only_table_of_the_execution_plane_is_guarded_too() {
    for table in [
        "run_events",
        "payloads",
        "workflow_versions",
        "leaf_collection_success_certifications",
    ] {
        for (privilege, finding) in [
            ("UPDATE", PrivilegeFinding::CanRewriteAppendOnly(table)),
            ("DELETE", PrivilegeFinding::CanRewriteAppendOnly(table)),
            ("TRUNCATE", PrivilegeFinding::CanRewriteAppendOnly(table)),
        ] {
            let db = migrated_db().await;
            let role = new_role(&db).await;
            grant(
                &db,
                format!("GRANT SELECT, INSERT ON audit_records TO {role}"),
            )
            .await;
            grant(
                &db,
                format!("GRANT SELECT, INSERT, {privilege} ON {table} TO {role}"),
            )
            .await;
            let store = as_role(&db, &role, PASSWORD).await;

            let found = findings(
                store
                    .verify_least_privilege()
                    .await
                    .expect_err(&format!("{privilege} on {table}")),
            );
            assert_eq!(found, vec![finding], "{privilege} on {table}");
        }
    }
}

#[tokio::test]
async fn a_role_that_owns_a_journal_table_is_refused() {
    let db = migrated_db().await;
    let role = new_role(&db).await;
    grant(
        &db,
        format!("GRANT SELECT, INSERT ON audit_records TO {role}"),
    )
    .await;
    grant(&db, format!("ALTER TABLE run_events OWNER TO {role}")).await;
    let store = as_role(&db, &role, PASSWORD).await;

    let found = findings(store.verify_least_privilege().await.expect_err("owner"));
    assert!(
        found.contains(&PrivilegeFinding::OwnsAppendOnly("run_events")),
        "{found:?}"
    );
}

#[tokio::test]
async fn the_runtime_role_cannot_delete_an_outbox_message_that_is_still_owed() {
    let db = migrated_db().await;
    let tenant = sagat_domain::identity::TenantId::generate();
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, 'acme', now())")
        .bind(tenant.as_uuid())
        .execute(&db.pool)
        .await
        .expect("tenant");
    for event in ["owed", "delivered"] {
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload, delivered_at) \
             VALUES (gen_random_uuid(), $1, gen_random_uuid(), $2, '{}', \
                     CASE WHEN $2 = 'delivered' THEN now() END)",
        )
        .bind(tenant.as_uuid())
        .bind(event)
        .execute(&db.pool)
        .await
        .expect("message");
    }
    let runtime: PgPool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD))
        .await
        .expect("connect as the runtime role");

    let refused = sqlx::query("DELETE FROM event_outbox WHERE event_type = 'owed'")
        .execute(&runtime)
        .await
        .expect_err("an undelivered message must survive");
    assert!(
        refused.to_string().contains("cannot be deleted"),
        "{refused}"
    );
    // Even a statement that matches both kinds of row stops at the owed one.
    assert!(
        sqlx::query("DELETE FROM event_outbox")
            .execute(&runtime)
            .await
            .is_err()
    );

    let deleted = sqlx::query("DELETE FROM event_outbox WHERE event_type = 'delivered'")
        .execute(&runtime)
        .await
        .expect("a delivered one may go")
        .rows_affected();
    assert_eq!(deleted, 1);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(left, 1);
}
