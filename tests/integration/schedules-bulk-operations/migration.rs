//! The schedules migration against a real PostgreSQL (`T018-008`): what the schema refuses, what the
//! runtime role may do, and a rollback that never destroys schedules.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::identity::TenantId;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sqlx::{Connection as _, PgPool};
use uuid::Uuid;

use crate::support::{RUNTIME_ROLE, TestDb, bounded, fresh_db, migrated_db};

const MIGRATION: i64 = 20_261_230_000_000;

fn id() -> Uuid {
    TenantId::generate().as_uuid()
}

/// The SQLSTATE of a failed statement.
fn code<T>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(_) => panic!("the statement was expected to fail"),
        Err(sqlx::Error::Database(e)) => e.code().map(|c| c.to_string()).unwrap_or_default(),
        Err(other) => panic!("not a database error: {other}"),
    }
}

const CHECK_VIOLATION: &str = "23514";
const FOREIGN_KEY_VIOLATION: &str = "23503";
const UNIQUE_VIOLATION: &str = "23505";
const RESTRICT_VIOLATION: &str = "23001";
const NOT_IN_PREREQUISITE_STATE: &str = "55000";

/// A tenant with one workspace and one namespace; returns (tenant, namespace).
async fn namespace(pool: &PgPool) -> (Uuid, Uuid) {
    let (tenant, workspace, namespace) = (id(), id(), id());
    let short = &tenant.simple().to_string()[..12];
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())")
        .bind(tenant)
        .bind(format!("tn-{short}"))
        .execute(pool)
        .await
        .expect("tenant");
    sqlx::query(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES ($1, $2, 'main', now())",
    )
    .bind(workspace)
    .bind(tenant)
    .execute(pool)
    .await
    .expect("workspace");
    sqlx::query(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES ($1, $2, $3, 'prod', 'prod', now())",
    )
    .bind(namespace)
    .bind(tenant)
    .bind(workspace)
    .execute(pool)
    .await
    .expect("namespace");
    (tenant, namespace)
}

/// A schedule and its first revision, committed together.
async fn schedule(pool: &PgPool, tenant: Uuid, namespace: Uuid, name: &str) -> Uuid {
    let schedule = id();
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, state, head_revision, \
         authorized_by, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'API', 'ACTIVE', 1, 'alice', now(), now())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(schedule)
    .bind(name)
    .execute(&mut *tx)
    .await
    .expect("schedule");
    sqlx::query(
        "INSERT INTO schedule_revisions (tenant_id, schedule_id, revision, spec, policies, action, actor) \
         VALUES ($1, $2, 1, '{}'::jsonb, '{}'::jsonb, '{}'::jsonb, 'alice')",
    )
    .bind(tenant)
    .bind(schedule)
    .execute(&mut *tx)
    .await
    .expect("revision");
    tx.commit().await.expect("commit");
    schedule
}

async fn action(
    pool: &PgPool,
    tenant: Uuid,
    schedule: Uuid,
    nominal: i64,
    outcome: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         VALUES ($1, $2, 'SPEC', to_timestamp($3), 'SCHEDULED', 1, $4, now())",
    )
    .bind(tenant)
    .bind(schedule)
    .bind(nominal as f64)
    .bind(outcome)
    .execute(pool)
    .await
}

async fn table_exists(pool: &PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{name}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

/// Rolls the schema back to `target` on a session of its own. SQLx 0.9 leaves a refused migration
/// holding its session advisory lock, so the lock is released explicitly and the session closed,
/// never returned to a pool where the next rollback would wait for it for ever.
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
    result
}

const TABLES: [&str; 6] = [
    "schedules",
    "schedule_revisions",
    "schedule_actions",
    "schedule_backfills",
    "schedule_obligations",
    "schedule_trigger_handover",
];

#[tokio::test]
async fn the_tables_exist_and_the_cron_handover_starts_legacy() {
    let db = migrated_db().await;
    for table in TABLES {
        assert!(table_exists(&db.pool, table).await, "{table}");
    }
    let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover")
        .fetch_one(&db.pool)
        .await
        .expect("handover");
    assert_eq!(state, "LEGACY");
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO schedule_trigger_handover (only_row, state) VALUES (false, 'LEGACY')"
            )
            .execute(&db.pool)
            .await
        ),
        CHECK_VIOLATION,
        "only one row can exist"
    );
    assert_eq!(
        code(
            sqlx::query("DELETE FROM schedule_trigger_handover")
                .execute(&db.pool)
                .await
        ),
        RESTRICT_VIOLATION
    );
}

#[tokio::test]
async fn a_schedule_without_a_head_revision_cannot_commit() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let mut tx = db.pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, state, head_revision, \
         authorized_by, created_at, updated_at) \
         VALUES ($1, $2, $3, 'orphan', 'API', 'ACTIVE', 1, 'alice', now(), now())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(id())
    .execute(&mut *tx)
    .await
    .expect("the insert itself waits for the commit");
    assert_eq!(code(tx.commit().await), FOREIGN_KEY_VIOLATION);
    // and with its revision it commits
    schedule(&db.pool, tenant, namespace, "with-revision").await;
}

/// Tries to insert a schedule (and rolls it back): `columns` and `values` add what the case varies.
async fn try_schedule(
    pool: &PgPool,
    (tenant, namespace): (Uuid, Uuid),
    (name, origin, state): (&str, &str, &str),
    (columns, values): (&str, &str),
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    let mut tx = pool.begin().await.expect("begin");
    // `origin`, `state`, `columns` and `values` are literals of this file, never user input.
    let result = sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, state, head_revision, \
         authorized_by, created_at, updated_at{columns}) \
         VALUES ($1, $2, $3, $4, '{origin}', '{state}', 1, 'alice', now(), now(){values})"
    )))
    .bind(tenant)
    .bind(namespace)
    .bind(id())
    .bind(name)
    .execute(&mut *tx)
    .await;
    tx.rollback().await.expect("rollback");
    result
}

#[tokio::test]
async fn the_shape_of_a_schedule_is_checked_by_the_schema() {
    let db = migrated_db().await;
    let scope = namespace(&db.pool).await;
    macro_rules! try_with {
        ($name:expr, $origin:expr, $state:expr, $extra:expr) => {
            try_schedule(&db.pool, scope, ($name, $origin, $state), $extra)
        };
    }
    assert!(
        try_with!("fine-name", "API", "ACTIVE", ("", ""))
            .await
            .is_ok()
    );
    let long = "x".repeat(129);
    for bad in ["Upper", "-leading", "has space", "", long.as_str()] {
        assert_eq!(
            code(try_with!(bad, "API", "ACTIVE", ("", "")).await),
            CHECK_VIOLATION,
            "{bad:?}"
        );
    }
    assert_eq!(
        code(try_with!("n", "ELSEWHERE", "ACTIVE", ("", "")).await),
        CHECK_VIOLATION,
        "origin"
    );
    assert_eq!(
        code(try_with!("n", "TRIGGER", "ACTIVE", ("", "")).await),
        CHECK_VIOLATION,
        "a trigger schedule names its trigger"
    );
    assert_eq!(
        code(try_with!("n", "API", "PAUSED2", ("", "")).await),
        CHECK_VIOLATION,
        "state"
    );
    assert_eq!(
        code(try_with!("n", "API", "DELETED", ("", "")).await),
        CHECK_VIOLATION,
        "deleted needs deleted_at"
    );
    assert_eq!(
        code(try_with!("n", "API", "ACTIVE", (", remaining_actions", ", 1000001")).await),
        CHECK_VIOLATION,
        "remaining actions stop at a million"
    );
    assert!(
        try_with!("n", "API", "ACTIVE", (", remaining_actions", ", 0"))
            .await
            .is_ok(),
        "zero is exhausted, not invalid"
    );
}

#[tokio::test]
async fn a_name_is_unique_while_the_schedule_exists_and_free_after_it_is_deleted() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let first = schedule(&db.pool, tenant, namespace, "nightly").await;
    let mut tx = db.pool.begin().await.expect("begin");
    let duplicate = sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, state, head_revision, \
         authorized_by, created_at, updated_at) \
         VALUES ($1, $2, $3, 'nightly', 'API', 'ACTIVE', 1, 'alice', now(), now())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(id())
    .execute(&mut *tx)
    .await;
    assert_eq!(code(duplicate), UNIQUE_VIOLATION);
    tx.rollback().await.expect("rollback");

    sqlx::query("UPDATE schedules SET state = 'DELETED', deleted_at = now() WHERE tenant_id = $1 AND id = $2")
        .bind(tenant)
        .bind(first)
        .execute(&db.pool)
        .await
        .expect("delete");
    schedule(&db.pool, tenant, namespace, "nightly").await;
}

#[tokio::test]
async fn the_identity_of_a_schedule_and_its_revisions_never_change_and_nothing_is_deleted() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let schedule = schedule(&db.pool, tenant, namespace, "pinned").await;
    for statement in [
        "UPDATE schedules SET origin = 'TRIGGER' WHERE id = $1",
        "UPDATE schedules SET created_at = created_at + interval '1 second' WHERE id = $1",
        "UPDATE schedules SET namespace_id = gen_random_uuid() WHERE id = $1",
        "DELETE FROM schedules WHERE id = $1",
    ] {
        assert_eq!(
            code(
                sqlx::query(statement)
                    .bind(schedule)
                    .execute(&db.pool)
                    .await
            ),
            RESTRICT_VIOLATION,
            "{statement}"
        );
    }
    sqlx::query("UPDATE schedules SET state_version = state_version + 1, state = 'PAUSED', note = 'x', note_by = 'bob', note_at = now() WHERE id = $1")
        .bind(schedule)
        .execute(&db.pool)
        .await
        .expect("the mutable columns change");
    for statement in [
        "UPDATE schedule_revisions SET actor = 'mallory' WHERE schedule_id = $1",
        "DELETE FROM schedule_revisions WHERE schedule_id = $1",
    ] {
        assert_eq!(
            code(
                sqlx::query(statement)
                    .bind(schedule)
                    .execute(&db.pool)
                    .await
            ),
            RESTRICT_VIOLATION,
            "{statement}"
        );
    }
    // schedule_actions is referenced by no foreign key, so the trigger is what refuses the truncate
    assert_eq!(
        code(
            sqlx::query("TRUNCATE schedule_actions")
                .execute(&db.pool)
                .await
        ),
        RESTRICT_VIOLATION
    );
    // PostgreSQL itself refuses to truncate a table that a foreign key references
    assert!(
        sqlx::query("TRUNCATE schedule_revisions")
            .execute(&db.pool)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_action_changes_only_from_buffered_to_a_final_outcome() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let schedule = schedule(&db.pool, tenant, namespace, "actions").await;

    action(&db.pool, tenant, schedule, 1_000, "BUFFERED")
        .await
        .expect("buffered");
    action(&db.pool, tenant, schedule, 2_000, "SKIPPED_OVERLAP")
        .await
        .expect("final");

    let update = |nominal: i64, to: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query("UPDATE schedule_actions SET outcome = $3 WHERE schedule_id = $1 AND nominal_at = to_timestamp($2)")
                .bind(schedule)
                .bind(nominal as f64)
                .bind(to)
                .execute(&pool)
                .await
        }
    };
    assert_eq!(
        code(update(2_000, "STARTED").await),
        RESTRICT_VIOLATION,
        "a final outcome stays"
    );
    assert_eq!(
        code(update(1_000, "BUFFERED").await),
        RESTRICT_VIOLATION,
        "BUFFERED to BUFFERED is not a change"
    );
    assert_eq!(
        code(sqlx::query("UPDATE schedule_actions SET revision = 2 WHERE schedule_id = $1 AND nominal_at = to_timestamp(1000)")
            .bind(schedule)
            .execute(&db.pool)
            .await),
        RESTRICT_VIOLATION,
        "the revision it was decided under stays"
    );
    assert!(
        update(1_000, "DROPPED_DELETED").await.is_ok(),
        "BUFFERED to final"
    );

    action(&db.pool, tenant, schedule, 3_000, "BUFFERED")
        .await
        .expect("buffered again");
    assert_eq!(
        code(sqlx::query("DELETE FROM schedule_actions WHERE schedule_id = $1 AND nominal_at = to_timestamp(3000)")
            .bind(schedule)
            .execute(&db.pool)
            .await),
        RESTRICT_VIOLATION,
        "a waiting action is never purged"
    );
    sqlx::query(
        "DELETE FROM schedule_actions WHERE schedule_id = $1 AND nominal_at = to_timestamp(2000)",
    )
    .bind(schedule)
    .execute(&db.pool)
    .await
    .expect("a final action may be purged by the maintenance owner");
}

#[tokio::test]
async fn the_shape_of_an_action_is_checked_by_the_schema() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let schedule = schedule(&db.pool, tenant, namespace, "shapes").await;
    assert_eq!(
        code(action(&db.pool, tenant, schedule, 1, "STARTED").await),
        CHECK_VIOLATION,
        "a started action names its run"
    );
    assert_eq!(
        code(action(&db.pool, tenant, schedule, 2, "TERMINATED").await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(action(&db.pool, tenant, schedule, 3, "COALESCED").await),
        CHECK_VIOLATION,
        "a summary carries its count"
    );
    assert_eq!(
        code(sqlx::query(
            "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
             VALUES ($1, $2, 'MANUAL', now(), 'SCHEDULED', 1, 'SKIPPED_OVERLAP', now())",
        )
        .bind(tenant)
        .bind(schedule)
        .execute(&db.pool)
        .await),
        CHECK_VIOLATION,
        "only trigger-now has the MANUAL origin"
    );
    sqlx::query(
        "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, \
         decided_at, summary_count, summary_last_at) \
         VALUES ($1, $2, 'SPEC', to_timestamp(4), 'SCHEDULED', 1, 'COALESCED', now(), 3, to_timestamp(9))",
    )
    .bind(tenant)
    .bind(schedule)
    .execute(&db.pool)
    .await
    .expect("a summary with its count and its last time");
    assert_eq!(
        code(action(&db.pool, tenant, schedule, 4, "SKIPPED_OVERLAP").await),
        UNIQUE_VIOLATION,
        "one action per schedule, origin and nominal time"
    );
}

#[tokio::test]
async fn the_runtime_role_has_exactly_the_privileges_the_stores_need() {
    let db = migrated_db().await;
    let has = |table: &'static str, privilege: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>("SELECT has_table_privilege($1, $2, $3)")
                .bind(RUNTIME_ROLE)
                .bind(table)
                .bind(privilege)
                .fetch_one(&pool)
                .await
                .expect("privilege")
        }
    };
    for table in [
        "schedules",
        "schedule_actions",
        "schedule_backfills",
        "schedule_obligations",
    ] {
        for privilege in ["SELECT", "INSERT", "UPDATE"] {
            assert!(has(table, privilege).await, "{table} {privilege}");
        }
        assert!(
            !has(table, "DELETE").await,
            "{table}: purge runs through the maintenance owner"
        );
        assert!(!has(table, "TRUNCATE").await, "{table}");
    }
    assert!(has("schedule_revisions", "SELECT").await && has("schedule_revisions", "INSERT").await);
    assert!(
        !has("schedule_revisions", "UPDATE").await && !has("schedule_revisions", "DELETE").await
    );
    assert!(has("schedule_trigger_handover", "SELECT").await);
    for privilege in ["INSERT", "UPDATE", "DELETE"] {
        assert!(
            !has("schedule_trigger_handover", privilege).await,
            "handover {privilege}"
        );
    }
}

#[tokio::test]
async fn the_runtime_role_can_lock_the_row_of_a_schedule() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    let schedule = schedule(&db.pool, tenant, namespace, "locked").await;
    let runtime = db.runtime_pool().await;
    let mut tx = runtime.begin().await.expect("begin as the runtime role");
    let locked: Uuid =
        sqlx::query_scalar("SELECT id FROM schedules WHERE tenant_id = $1 AND id = $2 FOR UPDATE")
            .bind(tenant)
            .bind(schedule)
            .fetch_one(&mut *tx)
            .await
            .expect("a row lock needs UPDATE, which the role holds");
    assert_eq!(locked, schedule);
    let revision: i32 = sqlx::query_scalar(
        "SELECT revision FROM schedule_revisions WHERE tenant_id = $1 AND schedule_id = $2",
    )
    .bind(tenant)
    .bind(schedule)
    .fetch_one(&mut *tx)
    .await
    .expect("revision");
    assert_eq!(revision, 1);
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn a_rollback_is_refused_while_schedules_exist_and_removes_the_tables_when_they_do_not() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    schedule(&db.pool, tenant, namespace, "retained").await;

    let before: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE version < $1")
            .bind(MIGRATION)
            .fetch_one(&db.pool)
            .await
            .expect("previous version");
    let refused = undo_to(&db, before).await;
    assert!(refused.is_err(), "schedules are authority and evidence");
    assert!(
        table_exists(&db.pool, "schedules").await,
        "nothing was lost"
    );

    // An operator drains the rows first, in one transaction. The tables forbid deleting them, so the
    // drain switches their user triggers off on purpose; the deferred foreign key from the head
    // revision is satisfied at commit because nothing is left.
    let mut tx = db.pool.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE schedule_actions DISABLE TRIGGER USER",
        "ALTER TABLE schedule_revisions DISABLE TRIGGER USER",
        "ALTER TABLE schedules DISABLE TRIGGER USER",
        "DELETE FROM schedule_actions",
        "DELETE FROM schedule_revisions",
        "DELETE FROM schedules",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    tx.commit().await.expect("the drain commits");
    undo_to(&db, before)
        .await
        .expect("rollback with no schedules left");
    for table in TABLES {
        assert!(!table_exists(&db.pool, table).await, "{table} is gone");
    }
    for function in ["schedules_identity_guard", "schedule_actions_guard"] {
        let left: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = $1)")
                .bind(function)
                .fetch_one(&db.pool)
                .await
                .expect("pg_proc");
        assert!(!left, "{function} is dropped with its tables");
    }
    migrate(&db.pool).await.expect("forward again");
    let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover")
        .fetch_one(&db.pool)
        .await
        .expect("handover");
    assert_eq!(
        state, "LEGACY",
        "a re-applied migration starts from the beginning"
    );
}

#[tokio::test]
async fn a_rollback_is_refused_after_the_cron_handover() {
    let db = migrated_db().await;
    sqlx::query("UPDATE schedule_trigger_handover SET state = 'HANDED_OVER', changed_by = 'operator', changed_at = now()")
        .execute(&db.pool)
        .await
        .expect("handover");
    let before: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE version < $1")
            .bind(MIGRATION)
            .fetch_one(&db.pool)
            .await
            .expect("previous version");
    let refused = undo_to(&db, before).await.expect_err("handed over");
    assert!(
        refused.to_string().contains("handed over")
            || format!("{refused:?}").contains(NOT_IN_PREREQUISITE_STATE),
        "{refused}"
    );
    assert!(table_exists(&db.pool, "schedule_trigger_handover").await);
}

const QUOTA_MIGRATION: i64 = 20_261_230_000_100;

#[tokio::test]
async fn both_quota_tables_accept_the_schedule_resources_for_a_namespace() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    for resource in ["schedules", "schedule_backfills"] {
        sqlx::query(
            "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
             VALUES ($1, $2, $3, 5, 'admin')",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(resource)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("limit for {resource}: {e}"));
        sqlx::query(
            "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, $2, 10, 'operator')",
        )
        .bind(tenant)
        .bind(resource)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("ceiling for {resource}: {e}"));
    }
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
                 VALUES ($1, $2, 'schedule_nonsense', 5, 'admin')",
            )
            .bind(tenant)
            .bind(namespace)
            .execute(&db.pool)
            .await
        ),
        CHECK_VIOLATION,
        "a resource nobody named is still refused"
    );
}

#[tokio::test]
async fn the_quota_rollback_is_refused_while_a_schedule_limit_or_ceiling_is_set() {
    let db = migrated_db().await;
    let (tenant, namespace) = namespace(&db.pool).await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'schedules', 5, 'admin')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("limit");
    assert!(
        undo_to(&db, MIGRATION).await.is_err(),
        "an operator's limit is not dropped by a rollback"
    );
    sqlx::query("DELETE FROM quota_limits WHERE resource = 'schedules'")
        .execute(&db.pool)
        .await
        .expect("the operator removes the limit first");
    undo_to(&db, MIGRATION)
        .await
        .expect("rollback of the quota migration alone");
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
                 VALUES ($1, $2, 'schedules', 5, 'admin')",
            )
            .bind(tenant)
            .bind(namespace)
            .execute(&db.pool)
            .await
        ),
        CHECK_VIOLATION,
        "the previous list is back"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(QUOTA_MIGRATION)
            .fetch_one(&db.pool)
            .await
            .expect("recorded");
    assert_eq!(recorded, 0);
    migrate(&db.pool).await.expect("forward again");
}

#[tokio::test]
async fn migrating_an_empty_database_creates_the_same_schema_as_migrating_step_by_step() {
    // the migration must not depend on data that an earlier migration left behind
    let db = fresh_db().await;
    migrate(&db.pool).await.expect("migrate");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1 AND success")
            .bind(MIGRATION)
            .fetch_one(&db.pool)
            .await
            .expect("recorded");
    assert_eq!(count, 1);
}
