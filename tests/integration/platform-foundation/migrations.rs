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

/// Design 043 (D043-145, D043-153): from the migration of design 043 on, every CHECK is added `NOT VALID`, and none is
/// validated in the migration that added it: each migration runs in one transaction, so validating there would scan the
/// table under the `ACCESS EXCLUSIVE` lock of the `ADD`. The older migrations are immutable and not read.
#[test]
fn a_constraint_added_not_valid_is_validated_by_a_later_migration() {
    const FROM: i64 = 20_270_118_000_000;
    for migration in MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version >= FROM)
    {
        let sql = migration.sql.as_str().to_owned();
        let added: Vec<String> = sql
            .split("ADD CONSTRAINT")
            .skip(1)
            .filter_map(|rest| {
                let name = rest.split_whitespace().next()?.to_owned();
                let statement = rest.split(';').next().unwrap_or_default();
                statement.contains("NOT VALID").then_some(name)
            })
            .collect();
        for check in sql.split("ADD CONSTRAINT").skip(1) {
            let statement = check.split(';').next().unwrap_or_default();
            if statement.contains(" CHECK ") || statement.contains(" CHECK(") {
                let tail = statement.split("\n    DROP").next().unwrap_or(statement);
                assert!(
                    tail.contains("NOT VALID"),
                    "{}: a CHECK added without NOT VALID scans its table under the lock of the ADD: {}",
                    migration.version,
                    tail.lines().next().unwrap_or_default()
                );
            }
        }
        for name in added {
            assert!(
                !sql.contains(&format!("VALIDATE CONSTRAINT {name}")),
                "{}: {name} is added NOT VALID and validated in the same transaction",
                migration.version
            );
            // ... and a later migration validates it (architecture review of feature 041, m5).
            assert!(
                MIGRATOR
                    .iter()
                    .any(|later| later.migration_type.is_up_migration()
                        && later.version > migration.version
                        && later
                            .sql
                            .as_str()
                            .contains(&format!("VALIDATE CONSTRAINT {name}"))),
                "{}: {name} is added NOT VALID and no later migration validates it",
                migration.version
            );
        }
    }
}

/// Feature 041 (architecture review of 2026-11-19, M3): the migration with the highest version that restates a
/// resource list of the ceilings tables is the one in force on a fresh database, whatever order branches merged in.
/// Its list must name every resource the code knows, so a migration of another branch that restates the list
/// without the names of this one (or is stamped earlier and merged later) fails here.
#[test]
fn the_last_restatement_of_each_quota_resource_list_names_every_resource() {
    use sagat_domain::quota::{QuotaResource, QuotaScope};
    for constraint in [
        "quota_limits_resource_valid",
        "quota_ceilings_resource_valid",
        "quota_limits_tenant_scoped",
    ] {
        let marker = format!("ADD CONSTRAINT {constraint}");
        let last = MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_up_migration() && m.sql.as_str().contains(&marker))
            .max_by_key(|m| m.version)
            .expect("a migration restates the list");
        let statement = last
            .sql
            .as_str()
            .split(&marker)
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .unwrap_or_default();
        let named: std::collections::BTreeSet<&str> =
            statement.split('\'').skip(1).step_by(2).collect();
        let expected: std::collections::BTreeSet<&str> = QuotaResource::ALL
            .iter()
            .filter(|resource| match constraint {
                "quota_limits_tenant_scoped" => resource.scope() == QuotaScope::Namespace,
                // Iteration capacity is a tenant setting stored apart from quota_limits.
                "quota_limits_resource_valid" => **resource != QuotaResource::IterationConcurrency,
                _ => true,
            })
            .map(|resource| resource.as_str())
            .collect();
        assert_eq!(named, expected, "{constraint} in {}", last.version);
    }
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

/// The stamp of the migration before the one that records the blobs a lake attempt may read (feature 035, D035-144).
const BEFORE_LAKE_BLOB_PINS: i64 = 20_270_110_000_000;

#[tokio::test]
async fn the_lake_blob_pins_roll_back_only_while_no_attempt_holds_one() {
    let db = migrated_db().await;
    assert!(table_exists(&db.pool, "step_lease_blobs").await);

    // A pin is written by the lease of a lake step, which needs a run and a blob of its own; the guard reads the table alone, so the
    // row is written with the foreign keys switched off for this transaction (the owner may) and the guard is what is tested.
    let mut tx = db.pool.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("foreign keys off for this transaction");
    sqlx::query(
        "INSERT INTO step_lease_blobs (tenant_id, namespace_id, run_id, node_id, attempt, generation, blob_id) \
         VALUES (gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'load', 1, 1, gen_random_uuid())",
    )
    .execute(&mut *tx)
    .await
    .expect("a pin");
    tx.commit().await.expect("committed");

    // sqlx takes an advisory lock for a rollback and does not release it when a revert fails: the refused rollback runs on a connection of
    // its own and lets the lock go (see the binding profile's test).
    let mut connection = db.pool.acquire().await.expect("a connection");
    let refused = MIGRATOR
        .undo(&mut *connection, BEFORE_LAKE_BLOB_PINS)
        .await
        .expect_err("a pin blocks the rollback");
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *connection)
        .await
        .expect("release the migration lock");
    drop(connection);
    let message = refused.to_string();
    assert!(
        message.contains("cannot roll back lake blob pins"),
        "{message}"
    );
    assert!(
        table_exists(&db.pool, "step_lease_blobs").await,
        "a refused rollback leaves the table, and the pin, in place"
    );
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM step_lease_blobs")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(kept, 1);

    // With no attempt holding one the rollback goes through, and forward brings the table back, empty.
    sqlx::query("DELETE FROM step_lease_blobs")
        .execute(&db.pool)
        .await
        .expect("the attempt's run was cleaned up");
    MIGRATOR
        .undo(&db.pool, BEFORE_LAKE_BLOB_PINS)
        .await
        .expect("rollback of the pins alone");
    assert!(!table_exists(&db.pool, "step_lease_blobs").await);
    let function: bool =
        sqlx::query_scalar("SELECT to_regproc('sagat_lake_blob_pin_immutable') IS NOT NULL")
            .fetch_one(&db.pool)
            .await
            .expect("function");
    assert!(!function, "the guard function goes with the table");
    migrate(&db.pool).await.expect("forward again");
    assert!(table_exists(&db.pool, "step_lease_blobs").await);
}

/// The stamp of the migration before the one that lets the runtime role delete the pins of lake blobs (feature 035, D035-147).
const BEFORE_LAKE_BLOB_PIN_RELEASE: i64 = 20_270_111_000_000;

#[tokio::test]
async fn the_release_of_the_lake_blob_pins_is_a_grant_and_rolls_back_as_one() {
    let db = migrated_db().await;
    let can_delete = |db: &crate::support::TestDb| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT has_table_privilege('sagat_app', 'step_lease_blobs', 'DELETE')",
            )
            .fetch_one(&pool)
            .await
            .expect("the privilege")
        }
    };
    assert!(
        can_delete(&db).await,
        "the retention job runs as the runtime role"
    );
    // Nothing is lost by rolling it back: the pins stay, the table stays, and only the grant goes.
    MIGRATOR
        .undo(&db.pool, BEFORE_LAKE_BLOB_PIN_RELEASE)
        .await
        .expect("the grant alone rolls back");
    assert!(table_exists(&db.pool, "step_lease_blobs").await);
    assert!(
        !can_delete(&db).await,
        "and the runtime role no longer deletes a pin"
    );
    let still: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('sagat_app', 'step_lease_blobs', 'SELECT') AND has_table_privilege('sagat_app', 'step_lease_blobs', 'INSERT')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the privileges");
    assert!(still, "it reads and writes them as before");
    migrate(&db.pool).await.expect("forward again");
    assert!(can_delete(&db).await);
}

/// The stamp of the migration before design 043's two (`20270118000000_run_input_validation` and its validation).
const BEFORE_RUN_INPUT_VALIDATION: i64 = 20_270_116_000_000;

/// Round 2 of 2026-11-16 (architecture 6): the rollback of design 043 restores the old CHECKs only while nothing names what
/// they do not know. Each of its four guards refuses, with its own message, while a row of its kind exists, and leaves the
/// new CHECKs in place. With no action or process naming it the rollback goes through, the old CHECK refuses `INPUT_INVALID`
/// again, and forward restores the new ones. A recorded requirement blocks it, and so does its history after its removal:
/// the history is append-only evidence, so the rollback then waits for a compatible reader.
#[tokio::test]
async fn the_run_input_validation_rolls_back_only_while_nothing_names_it() {
    let db = migrated_db().await;
    let constraint = |name: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = $1)",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .expect("constraint")
        }
    };
    let refused_rollback = || {
        let pool = db.pool.clone();
        async move {
            // sqlx takes an advisory lock for a rollback and does not release it when a revert fails: the refused rollback
            // runs on a connection of its own and lets the lock go (see the binding profile's test).
            let mut connection = pool.acquire().await.expect("a connection");
            let refused = MIGRATOR
                .undo(&mut *connection, BEFORE_RUN_INPUT_VALIDATION)
                .await
                .expect_err("a row blocks the rollback");
            sqlx::query("SELECT pg_advisory_unlock_all()")
                .execute(&mut *connection)
                .await
                .expect("release the migration lock");
            refused.to_string()
        }
    };

    // An action and a process: written, and removed, with the foreign keys off for their transaction (the owner may); the
    // guard reads the table alone, and the guard is what is tested.
    for (insert, delete, says) in [
        (
            "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
             VALUES (gen_random_uuid(), gen_random_uuid(), 'SPEC', now(), 'SCHEDULED', 1, 'INPUT_INVALID', now())",
            "DELETE FROM schedule_actions WHERE outcome = 'INPUT_INVALID'",
            "schedule actions recorded INPUT_INVALID are retained",
        ),
        (
            "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
             VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', '0.3.0', ARRAY['run-input-validation-v1'], now(), now(), sha256('t'::bytea))",
            "DELETE FROM serving_instances WHERE 'run-input-validation-v1' = ANY (capabilities)",
            "a registered process lists run-input-validation-v1",
        ),
    ] {
        for statement in [insert, delete] {
            let mut tx = db.pool.begin().await.expect("a transaction");
            sqlx::query("SET LOCAL session_replication_role = replica")
                .execute(&mut *tx)
                .await
                .expect("foreign keys off for this transaction");
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|error| panic!("{statement}: {error}"));
            tx.commit().await.expect("committed");
            if statement == insert {
                let message = refused_rollback().await;
                assert!(message.contains(says), "{says}: {message}");
                assert!(
                    constraint("schedule_actions_outcome_known").await,
                    "{says}: a refused rollback leaves the new CHECKs in place"
                );
            }
        }
    }

    MIGRATOR
        .undo(&db.pool, BEFORE_RUN_INPUT_VALIDATION)
        .await
        .expect("nothing names run-input validation: the rollback goes through");
    assert!(constraint("schedule_actions_outcome_valid").await);
    assert!(!constraint("schedule_actions_outcome_known").await);
    assert!(constraint("serving_instances_capabilities_closed").await);
    let refused = sqlx::query(
        "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         VALUES (gen_random_uuid(), gen_random_uuid(), 'SPEC', now(), 'SCHEDULED', 1, 'INPUT_INVALID', now())",
    )
    .execute(&db.pool)
    .await
    .expect_err("the old CHECK does not know INPUT_INVALID");
    assert!(
        refused
            .to_string()
            .contains("schedule_actions_outcome_valid"),
        "{refused}"
    );
    migrate(&db.pool).await.expect("forward again");
    assert!(constraint("schedule_actions_outcome_known").await);
    assert!(!constraint("schedule_actions_outcome_valid").await);

    // A requirement, recorded the only way there is (no live process lacks it): the rollback refuses while it is recorded,
    // and after its removal the history that names it still refuses it.
    let recorded: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('run-input-validation-v1', ARRAY['EXECUTION_ENGINE'], 'operator')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("activate");
    assert_eq!(recorded, "RECORDED");
    let message = refused_rollback().await;
    assert!(
        message.contains("run-input-validation-v1 is a recorded requirement"),
        "{message}"
    );
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('run-input-validation-v1', 'operator')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("remove");
    assert_eq!(removed, "REMOVED");
    let message = refused_rollback().await;
    assert!(
        message.contains("the requirement history names run-input-validation-v1"),
        "{message}"
    );
    assert!(constraint("platform_requirement_history_capability_known").await);
}

/// The stamp of the migration before feature 006's three (`20270208000000_lease_withdrawal`, its validation and
/// `20270208000200_lease_withdrawal_floor`).
const BEFORE_LEASE_WITHDRAWAL: i64 = 20_270_119_000_000;

/// Architecture Major 4 of the 2026-11-21 review (decision D006-032, ADR-0098 "Rollback"): the rollback of the lease
/// withdrawal refuses, each guard with its own message, while a journal holds `step.lease_withdrawn`, while a registered
/// process lists `lease-withdrawal-v1`, while the requirement is recorded, and, after its removal, while its history names
/// it. A refused rollback leaves the new checks and functions in place. With nothing naming it, it goes through and the
/// old checks refuse the capability again; forward restores everything.
#[tokio::test]
async fn the_lease_withdrawal_rolls_back_only_while_nothing_names_it() {
    let db = migrated_db().await;
    let census_exists = || {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'lease_withdrawal_required')",
            )
            .fetch_one(&pool)
            .await
            .expect("function")
        }
    };
    let refused_rollback = || {
        let pool = db.pool.clone();
        async move {
            let mut connection = pool.acquire().await.expect("a connection");
            let refused = MIGRATOR
                .undo(&mut *connection, BEFORE_LEASE_WITHDRAWAL)
                .await
                .expect_err("a row blocks the rollback");
            sqlx::query("SELECT pg_advisory_unlock_all()")
                .execute(&mut *connection)
                .await
                .expect("release the migration lock");
            refused.to_string()
        }
    };
    // The journal is append-only, so its case runs on a database of its own that is then left as it is.
    {
        let journal = migrated_db().await;
        let mut tx = journal.pool.begin().await.expect("a transaction");
        sqlx::query("SET LOCAL session_replication_role = replica")
            .execute(&mut *tx)
            .await
            .expect("foreign keys off for this transaction");
        sqlx::query(
            "INSERT INTO run_events (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, occurred_at) \
             VALUES (gen_random_uuid(), gen_random_uuid(), 1, 'step.lease_withdrawn', 'a', 1, \
                     '{\"attempt\": 1, \"generation\": 1, \"cause\": \"NOT_SENT\"}', 'lease-withdrawal', now())",
        )
        .execute(&mut *tx)
        .await
        .expect("a withdrawal in a journal");
        tx.commit().await.expect("committed");
        let mut connection = journal.pool.acquire().await.expect("a connection");
        let refused = MIGRATOR
            .undo(&mut *connection, BEFORE_LEASE_WITHDRAWAL)
            .await
            .expect_err("the journal blocks the rollback")
            .to_string();
        assert!(
            refused.contains("a journal holds step.lease_withdrawn"),
            "{refused}"
        );
    }
    for (insert, delete, says) in [(
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
             VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', '0.3.0', ARRAY['lease-withdrawal-v1'], now(), now(), sha256('t'::bytea))",
        "DELETE FROM serving_instances WHERE 'lease-withdrawal-v1' = ANY (capabilities)",
        "a registered process lists lease-withdrawal-v1",
    )] {
        for statement in [insert, delete] {
            let mut tx = db.pool.begin().await.expect("a transaction");
            sqlx::query("SET LOCAL session_replication_role = replica")
                .execute(&mut *tx)
                .await
                .expect("foreign keys off for this transaction");
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|error| panic!("{statement}: {error}"));
            tx.commit().await.expect("committed");
            if statement == insert {
                let message = refused_rollback().await;
                assert!(message.contains(says), "{says}: {message}");
                assert!(
                    census_exists().await,
                    "{says}: a refused rollback leaves the functions in place"
                );
            }
        }
    }

    let recorded: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
         ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE'], 'operator')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("activate");
    assert_eq!(recorded, "RECORDED");
    let message = refused_rollback().await;
    assert!(
        message.contains("lease-withdrawal-v1 is a recorded requirement"),
        "{message}"
    );
    // no journal holds the event, so the replay floor lets the requirement go
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'operator')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("remove");
    assert_eq!(removed, "REMOVED");
    let message = refused_rollback().await;
    assert!(
        message.contains("the requirement history names lease-withdrawal-v1"),
        "{message}"
    );

    // A fresh database with nothing naming it rolls back and forward.
    let clean = migrated_db().await;
    MIGRATOR
        .undo(&clean.pool, BEFORE_LEASE_WITHDRAWAL)
        .await
        .expect("nothing names the lease withdrawal: the rollback goes through");
    let index = || {
        let pool = clean.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_class WHERE relname = 'run_events_lease_withdrawn_idx')",
            )
            .fetch_one(&pool)
            .await
            .expect("index")
        }
    };
    assert!(!index().await, "the rollback drops the partial index");
    let refused = sqlx::query(
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
         VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', '0.3.0', ARRAY['lease-withdrawal-v1'], now(), now(), sha256('t'::bytea))",
    )
    .execute(&clean.pool)
    .await
    .expect_err("the old check does not know the capability");
    assert!(
        refused
            .to_string()
            .contains("serving_instances_capabilities_known"),
        "{refused}"
    );
    migrate(&clean.pool).await.expect("forward again");
    let lists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'lease_withdrawal_required')",
    )
    .fetch_one(&clean.pool)
    .await
    .expect("function");
    assert!(lists, "forward restores the functions");
    assert!(index().await, "forward restores the partial index");
}

/// Records `lease-withdrawal-v1` of both kinds, as the operator's activation does.
async fn record_lease_withdrawal(pool: &sqlx::PgPool) {
    let recorded: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
         ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE'], 'operator')",
    )
    .fetch_one(pool)
    .await
    .expect("activate");
    assert_eq!(recorded, "RECORDED");
}

/// The removal of `lease-withdrawal-v1`, on a connection of its own, run until it ends; its error as text.
fn removal_of_lease_withdrawal(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
) -> tokio::task::JoinHandle<Result<String, String>> {
    tokio::spawn(async move {
        sqlx::query_scalar::<_, String>(
            "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'operator')",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())
    })
}

/// Waits until the backend `pid` waits for a lock (the barrier: `pg_stat_activity`, never a sleep).
async fn until_waiting_for_a_lock(pool: &sqlx::PgPool, pid: i32) {
    crate::support::eventually("the backend waits for a lock", || async {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')",
        )
        .bind(pid)
        .fetch_one(pool)
        .await
        .expect("pg_stat_activity");
        waiting.then_some(())
    })
    .await;
}

async fn backend_pid(connection: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(connection)
        .await
        .expect("pid")
}

/// Decision D006-084, the first order: a withdrawal holds the share lock of the requirement and writes its event; the
/// removal waits for it; once the withdrawal commits the guard sees the event and refuses. The requirement stays.
#[tokio::test]
async fn a_removal_that_waits_for_a_withdrawal_sees_its_event_and_is_refused() {
    let db = migrated_db().await;
    // the order of the two is what is tested, not the bound (it has its own test): on this database only, the removal may
    // wait long enough that the clock never decides
    sqlx::query("ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '60s'")
        .execute(&db.pool)
        .await
        .expect("the bound raised on this database");
    record_lease_withdrawal(&db.pool).await;
    let mut withdrawal = db.pool.begin().await.expect("a transaction");
    let required: bool = sqlx::query_scalar("SELECT lease_withdrawal_required()")
        .fetch_one(&mut *withdrawal)
        .await
        .expect("the requirement under its lock");
    assert!(required);
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *withdrawal)
        .await
        .expect("foreign keys off for this transaction");
    sqlx::query(
        "INSERT INTO run_events (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, occurred_at) \
         VALUES (gen_random_uuid(), gen_random_uuid(), 1, 'step.lease_withdrawn', 'a', 1, \
                 '{\"attempt\": 1, \"generation\": 1, \"cause\": \"NOT_SENT\"}', 'lease-withdrawal', now())",
    )
    .execute(&mut *withdrawal)
    .await
    .expect("the withdrawal's event");
    let mut connection = db.pool.acquire().await.expect("a connection");
    let pid = backend_pid(&mut connection).await;
    let removal = removal_of_lease_withdrawal(connection);
    until_waiting_for_a_lock(&db.pool, pid).await;
    withdrawal.commit().await.expect("the withdrawal commits");
    let refused = removal
        .await
        .expect("the removal ended")
        .expect_err("the removal is refused");
    assert!(
        refused.contains("cannot be removed while a journal holds step.lease_withdrawn"),
        "{refused}"
    );
    let still: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'lease-withdrawal-v1')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("requirement");
    assert!(still, "the requirement stays");
}

/// Decision D006-084, the other order: a removal that holds the deleted row makes the withdrawal's read wait, and once it
/// commits the withdrawal finds no requirement and so writes nothing.
#[tokio::test]
async fn a_withdrawal_that_waits_for_a_removal_finds_no_requirement() {
    let db = migrated_db().await;
    record_lease_withdrawal(&db.pool).await;
    let mut removal = db.pool.begin().await.expect("a transaction");
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'operator')",
    )
    .fetch_one(&mut *removal)
    .await
    .expect("remove");
    assert_eq!(removed, "REMOVED");
    let mut connection = db.pool.acquire().await.expect("a connection");
    let pid = backend_pid(&mut connection).await;
    let read = tokio::spawn(async move {
        sqlx::query_scalar::<_, bool>("SELECT lease_withdrawal_required()")
            .fetch_one(&mut *connection)
            .await
            .expect("the requirement under its lock")
    });
    until_waiting_for_a_lock(&db.pool, pid).await;
    removal.commit().await.expect("the removal commits");
    assert!(
        !read.await.expect("the read ended"),
        "no requirement: the withdrawal writes nothing"
    );
}

/// Decision D006-084: outside READ COMMITTED the removal is refused, even with no withdrawal in any journal, because a
/// stricter level keeps a snapshot that cannot see one committed while it waits.
#[tokio::test]
async fn the_requirement_is_removed_only_under_read_committed() {
    let db = migrated_db().await;
    record_lease_withdrawal(&db.pool).await;
    for (level, statement) in [
        (
            "REPEATABLE READ",
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
        ),
        (
            "SERIALIZABLE",
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
        ),
    ] {
        let mut tx = db.pool.begin().await.expect("a transaction");
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("isolation level");
        let refused = sqlx::query_scalar::<_, String>(
            "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'operator')",
        )
        .fetch_one(&mut *tx)
        .await
        .expect_err("refused outside READ COMMITTED")
        .to_string();
        assert!(
            refused.contains("is removed only under READ COMMITTED"),
            "{level}: {refused}"
        );
    }
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'operator')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("remove under READ COMMITTED");
    assert_eq!(removed, "REMOVED");
}

/// Decision D006-085: the removal waits at most one second for a lock (a registration queues behind its epoch lock with
/// two), and the look at the journal is a probe of the partial index. A withdrawal that holds the share lock past the bound
/// makes the removal fail with `lock_not_available`, and nothing is removed.
#[tokio::test]
async fn the_removal_waits_for_a_lock_at_most_one_second_and_probes_an_index() {
    let db = migrated_db().await;
    let (setting, valid): (Vec<String>, bool) = sqlx::query_as(
        "SELECT (SELECT coalesce(p.proconfig, '{}') FROM pg_proc p WHERE p.proname = 'platform_requirement_remove'), \
                (SELECT i.indisvalid FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid \
                  WHERE c.relname = 'run_events_lease_withdrawn_idx')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the bound and the index");
    assert!(
        setting.iter().any(|entry| entry == "lock_timeout=1s"),
        "{setting:?}"
    );
    assert!(valid, "the partial index is valid");
    // the guard's look at the journal is a probe of the partial index, not a scan
    let mut planner = db.pool.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *planner)
        .await
        .expect("no sequential scan");
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT 1 FROM public.run_events e WHERE e.event_type = 'step.lease_withdrawn' LIMIT 1",
    )
    .fetch_all(&mut *planner)
    .await
    .expect("plan");
    assert!(
        plan.iter()
            .any(|line| line.contains("run_events_lease_withdrawn_idx")),
        "{plan:#?}"
    );
    planner.rollback().await.expect("rolled back");
    record_lease_withdrawal(&db.pool).await;
    let mut withdrawal = db.pool.begin().await.expect("a transaction");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT lease_withdrawal_required()")
            .fetch_one(&mut *withdrawal)
            .await
            .expect("the requirement under its lock")
    );
    let connection = db.pool.acquire().await.expect("a connection");
    // ten times the bound: a removal with no bound would wait for the withdrawal for ever
    let refused = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        removal_of_lease_withdrawal(connection),
    )
    .await
    .expect("the removal gives up within its bound")
    .expect("the removal ended")
    .expect_err("the removal gives up");
    assert!(refused.contains("lock timeout"), "{refused}");
    withdrawal.rollback().await.expect("rolled back");
    let still: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'lease-withdrawal-v1')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("requirement");
    assert!(still, "a removal that gave up removed nothing");
}
