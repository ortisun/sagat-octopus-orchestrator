//! The three migrations of the 0.3.0 slice (T022-009, T022-070, T022-094, T022-007's refusal): rolled back and forward, a
//! down that refuses to destroy live flow-control state, counters kept by the database and closed to the runtime role,
//! and the key rotation refused while a row uses another version.

use sagat_infra_postgres::MIGRATOR;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::support::engine::{World, activity, document, world};
use crate::support::{db_error, migrated_db};

/// The last migration before this feature's.
const BEFORE: i64 = 20_270_118_000_100;
const CONCURRENCY: i64 = 20_270_130_000_000;
const RATE: i64 = 20_270_130_000_100;

const CONCURRENCY_TABLES: [&str; 4] = [
    "flow_control_key_versions",
    "concurrency_semaphores",
    "concurrency_holders",
    "concurrency_waiters",
];
const RATE_TABLES: [&str; 3] = [
    "rate_limit_windows",
    "rate_limit_grants",
    "rate_limit_waiters",
];

/// `MIGRATOR.undo` on a connection of its own, closed after: a refused rollback leaves the migrator's session lock on the
/// connection it ran on, and the next rollback would wait for it forever on a pooled one.
async fn undo(pool: &PgPool, target: i64) -> Result<(), sqlx::migrate::MigrateError> {
    let mut connection = pool.acquire().await.expect("a connection");
    let result = MIGRATOR.undo(&mut *connection, target).await;
    let _ = sqlx::Connection::close(connection.detach()).await;
    result
}

async fn exists(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

/// A run of the world's tenant, for the foreign keys: (tenant, namespace, run).
async fn a_run(w: &World) -> (Uuid, Uuid, Uuid) {
    let (workflow, _) = w
        .publish(document("flow-anchor", "a", vec![activity("a")]))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let row = sqlx::query("SELECT tenant_id, namespace_id FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the run");
    (row.get(0), row.get(1), run.as_uuid())
}

const DIGEST: [u8; 32] = [7; 32];

async fn activate_key(pool: &PgPool, version: i16) -> (String, i64) {
    let row =
        sqlx::query("SELECT out_outcome, out_rows FROM flow_control_key_activate($1, 'operator')")
            .bind(version)
            .fetch_one(pool)
            .await
            .expect("the operator's command");
    (row.get(0), row.get(1))
}

async fn semaphore(pool: &PgPool, (tenant, namespace, _): (Uuid, Uuid, Uuid), version: i16) {
    sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
         key_version, key_shard, updated_at) VALUES ($1, $2, 'ORDINARY', 'RUN', '=flow-anchor', $3, $4, 7, clock_timestamp())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(DIGEST.as_slice())
    .bind(version)
    .execute(pool)
    .await
    .expect("a semaphore");
}

async fn holder(
    pool: &PgPool,
    (tenant, namespace, run): (Uuid, Uuid, Uuid),
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, pool, group_name, \
         key_digest, declared_limit, acquired_at) \
         VALUES ($1, $2, $3, 'RUN', '', 0, 'ORDINARY', '=flow-anchor', $4, 1, clock_timestamp())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(run)
    .bind(DIGEST.as_slice())
    .execute(pool)
    .await
    .map(drop)
}

async fn counts(pool: &PgPool, tenant: Uuid) -> Option<(i32, i32)> {
    sqlx::query_as("SELECT holders, waiters FROM concurrency_semaphores WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_optional(pool)
        .await
        .expect("counts")
}

async fn window(pool: &PgPool, (tenant, namespace, _): (Uuid, Uuid, Uuid), version: i16) {
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
         max_period_ms, key_version, key_shard, updated_at) \
         VALUES ($1, $2, 'ORDINARY', 'STEP', 'provider-starts', $3, 1000, $4, 3, clock_timestamp())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(DIGEST.as_slice())
    .bind(version)
    .execute(pool)
    .await
    .expect("a window");
}

async fn grant(
    pool: &PgPool,
    (tenant, namespace, run): (Uuid, Uuid, Uuid),
    period_ms: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
         key_digest, declared_limit, period_ms, granted_at, shard) \
         VALUES ($1, $2, $3, 'a', 1, 'ORDINARY', 'STEP', 'provider-starts', $4, 10, $5, clock_timestamp(), 3)",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(run)
    .bind(DIGEST.as_slice())
    .bind(period_ms)
    .execute(pool)
    .await
    .map(drop)
}

async fn window_counts(pool: &PgPool, tenant: Uuid) -> Option<(i32, i32, i64, i32)> {
    sqlx::query_as(
        "SELECT waiters, live_grants, grants_total, max_period_ms FROM rate_limit_windows WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_optional(pool)
    .await
    .expect("window counts")
}

/// T022-009, T022-070, T022-094; the migration skill: rolling back removes what the slice added, refuses while a row of
/// live flow-control state exists, and going forward again restores it.
#[tokio::test]
async fn the_migrations_roll_back_only_once_their_rows_are_drained_and_go_forward_again() {
    let w = world().await;
    let pool = &w.db.pool;
    for table in CONCURRENCY_TABLES.iter().chain(&RATE_TABLES) {
        assert!(exists(pool, table).await, "{table}");
    }
    let run = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    semaphore(pool, run, 1).await;
    holder(pool, run).await.expect("a holder");
    window(pool, run, 1).await;
    grant(pool, run, 1000).await.expect("a grant");

    let refused = undo(pool, CONCURRENCY).await;
    assert!(
        refused.is_err(),
        "a live grant refuses the rollback: {refused:?}"
    );
    assert!(
        format!("{refused:?}")
            .contains("rate-limit windows, grants, waiters or their journal events are retained"),
        "{refused:?}"
    );
    assert!(
        exists(pool, "rate_limit_grants").await,
        "nothing of the rate tables was dropped"
    );
    sqlx::query("DELETE FROM rate_limit_grants")
        .execute(pool)
        .await
        .expect("drain");
    assert!(
        window_counts(pool, run.0).await.is_none(),
        "the drained window went with its last grant"
    );
    undo(pool, CONCURRENCY)
        .await
        .expect("rolled back past the rate tables");
    for table in RATE_TABLES {
        assert!(!exists(pool, table).await, "{table} is gone");
    }

    let refused = undo(pool, BEFORE).await;
    assert!(
        format!("{refused:?}").contains(
            "concurrency permits, waiters, key versions or their journal events are retained"
        ),
        "{refused:?}"
    );
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    assert!(
        counts(pool, run.0).await.is_none(),
        "the emptied key went with its last holder"
    );
    let still = undo(pool, BEFORE).await;
    assert!(
        still.is_err(),
        "a recorded key version is retained too: {still:?}"
    );
    sqlx::query("DELETE FROM flow_control_key_versions")
        .execute(pool)
        .await
        .expect("drain");
    undo(pool, BEFORE)
        .await
        .expect("rolled back past the slice");
    for table in CONCURRENCY_TABLES {
        assert!(!exists(pool, table).await, "{table} is gone");
    }
    MIGRATOR.run(pool).await.expect("forward again");
    for table in CONCURRENCY_TABLES.iter().chain(&RATE_TABLES) {
        assert!(exists(pool, table).await, "{table} is back");
    }
}

/// The registry rollback refuses while a flow-control capability is recorded, and restores the closed list otherwise.
#[tokio::test]
async fn the_registry_rollback_refuses_while_a_flow_control_capability_is_recorded() {
    let db = migrated_db().await;
    let pool = &db.pool;
    sqlx::query(
        "SELECT instance_register(gen_random_uuid(), '\\x00112233445566778899aabbccddeeff'::bytea, 'EXECUTION_ENGINE', \
         NULL, '0.3.0', ARRAY['flow-control-rate-v1'])",
    )
    .execute(pool)
    .await
    .expect("a process that lists the capability");
    let refused = undo(pool, RATE).await;
    assert!(
        format!("{refused:?}")
            .contains("a flow-control capability is listed, required or recorded"),
        "{refused:?}"
    );
    sqlx::query("DELETE FROM serving_instances")
        .execute(pool)
        .await
        .expect("drain");
    undo(pool, RATE).await.expect("rolled back");
    let narrowed = sqlx::query(
        "SELECT instance_register(gen_random_uuid(), '\\x00112233445566778899aabbccddeeff'::bytea, 'EXECUTION_ENGINE', \
         NULL, '0.3.0', ARRAY['flow-control-rate-v1'])",
    )
    .execute(pool)
    .await;
    assert!(
        narrowed.is_err(),
        "the closed list of the release before: {narrowed:?}"
    );
    MIGRATOR.run(pool).await.expect("forward again");
}

/// FR-022-058, `data-model.md` "Grants and counters": the counters follow the rows in the same transaction, the runtime
/// role cannot write one, and an emptied key is deleted by the transaction that empties it.
#[tokio::test]
async fn the_counters_follow_the_rows_and_the_runtime_role_cannot_write_them() {
    let w = world().await;
    let owner = &w.db.pool;
    let runtime = w.db.runtime_pool().await;
    let run = a_run(&w).await;
    assert_eq!(activate_key(owner, 1).await.0, "ACTIVATED");

    semaphore(&runtime, run, 1).await;
    holder(&runtime, run)
        .await
        .expect("the runtime role writes a holder");
    assert_eq!(counts(&runtime, run.0).await, Some((1, 0)));
    let forged = sqlx::query("UPDATE concurrency_semaphores SET holders = 0 WHERE tenant_id = $1")
        .bind(run.0)
        .execute(&runtime)
        .await
        .map_err(db_error);
    assert_eq!(
        forged.map(drop).unwrap_err().0,
        "42501",
        "a counter is not the runtime role's to write"
    );
    let hint = sqlx::query(
        "UPDATE concurrency_semaphores SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
         WHERE tenant_id = $1",
    )
    .bind(run.0)
    .execute(&runtime)
    .await
    .expect("the hint is");
    assert_eq!(hint.rows_affected(), 1);
    let mut tx = runtime.begin().await.expect("tx");
    sqlx::query("SELECT holders FROM concurrency_semaphores WHERE tenant_id = $1 FOR UPDATE")
        .bind(run.0)
        .fetch_one(&mut *tx)
        .await
        .expect("the runtime role locks a key");
    tx.rollback().await.expect("rollback");
    let changed =
        sqlx::query("UPDATE concurrency_holders SET declared_limit = 2 WHERE tenant_id = $1")
            .bind(run.0)
            .execute(owner)
            .await
            .map_err(db_error);
    assert_eq!(
        changed.map(drop).unwrap_err().0,
        "23001",
        "a holder is never changed, not even by the owner"
    );
    sqlx::query("DELETE FROM concurrency_holders WHERE tenant_id = $1")
        .bind(run.0)
        .execute(&runtime)
        .await
        .expect("released");
    assert_eq!(counts(owner, run.0).await, None, "the emptied key is gone");

    window(&runtime, run, 1).await;
    grant(&runtime, run, 60_000).await.expect("a grant");
    assert_eq!(
        window_counts(&runtime, run.0).await,
        Some((0, 1, 1, 60_000))
    );
    sqlx::query("UPDATE rate_limit_grants SET leased_at = clock_timestamp() WHERE tenant_id = $1")
        .bind(run.0)
        .execute(&runtime)
        .await
        .expect("the lease instant is recorded once");
    let again = sqlx::query(
        "UPDATE rate_limit_grants SET leased_at = clock_timestamp() WHERE tenant_id = $1",
    )
    .bind(run.0)
    .execute(&runtime)
    .await
    .map_err(db_error);
    assert_eq!(again.map(drop).unwrap_err().0, "23001", "and never again");
    let moved = sqlx::query(
        "UPDATE rate_limit_grants SET granted_at = clock_timestamp() WHERE tenant_id = $1",
    )
    .bind(run.0)
    .execute(&runtime)
    .await
    .map_err(db_error);
    assert_eq!(
        moved.map(drop).unwrap_err().0,
        "42501",
        "a grant's instant is not the runtime role's to move"
    );
    let forged = sqlx::query("UPDATE rate_limit_windows SET live_grants = 0 WHERE tenant_id = $1")
        .bind(run.0)
        .execute(&runtime)
        .await
        .map_err(db_error);
    assert_eq!(forged.map(drop).unwrap_err().0, "42501");
    sqlx::query("DELETE FROM rate_limit_grants WHERE tenant_id = $1")
        .bind(run.0)
        .execute(&runtime)
        .await
        .expect("purged");
    assert_eq!(
        window_counts(owner, run.0).await,
        None,
        "the emptied window is gone"
    );

    for statement in [
        "INSERT INTO flow_control_key_versions (version, state, activated_at, activated_by) \
         VALUES (9, 'ACTIVE', clock_timestamp(), 'x')",
        "SELECT * FROM flow_control_key_activate(2::smallint, 'x')",
    ] {
        let denied = sqlx::query(statement)
            .execute(&runtime)
            .await
            .map_err(db_error);
        assert_eq!(denied.map(drop).unwrap_err().0, "42501", "{statement}");
    }
}

/// T022-007, T022-070, ADR-0056 decision 7, ADR-0088 decision 8: a new digest key version is not activated while any
/// semaphore or window row uses another; once the keys drain it is, and the old one is retiring.
#[tokio::test]
async fn a_new_key_version_is_refused_while_a_row_uses_another() {
    let w = world().await;
    let pool = &w.db.pool;
    let run = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await, ("ACTIVATED".to_owned(), 0));
    assert_eq!(activate_key(pool, 1).await.0, "ALREADY_ACTIVE");
    semaphore(pool, run, 1).await;
    holder(pool, run).await.expect("a holder");
    window(pool, run, 1).await;
    grant(pool, run, 1000).await.expect("a grant");
    assert_eq!(
        activate_key(pool, 2).await,
        ("ROWS_UNDER_OTHER_VERSION".to_owned(), 2),
        "a semaphore and a window of version 1 refuse version 2"
    );
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(
        activate_key(pool, 2).await,
        ("ROWS_UNDER_OTHER_VERSION".to_owned(), 1),
        "the window still uses version 1"
    );
    sqlx::query("DELETE FROM rate_limit_grants")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(activate_key(pool, 2).await.0, "ACTIVATED");
    let states: Vec<(i16, String)> =
        sqlx::query_as("SELECT version, state FROM flow_control_key_versions ORDER BY version")
            .fetch_all(pool)
            .await
            .expect("versions");
    assert_eq!(
        states,
        [(1, "RETIRING".to_owned()), (2, "ACTIVE".to_owned())]
    );
    let two_active =
        sqlx::query("UPDATE flow_control_key_versions SET state = 'ACTIVE' WHERE version = 1")
            .execute(pool)
            .await
            .map_err(db_error);
    assert_eq!(
        two_active.map(drop).unwrap_err().0,
        "23505",
        "at most one version is active"
    );
}
