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
        .publish(document(
            &format!("flow-anchor-{}", &Uuid::now_v7().simple().to_string()[24..]),
            "a",
            vec![activity("a")],
        ))
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

async fn holder<'e, E: sqlx::PgExecutor<'e>>(
    pool: E,
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
         key_version, key_shard, updated_at) \
         VALUES ($1, $2, 'ORDINARY', 'STEP', 'provider-starts', $3, $4, 3, clock_timestamp())",
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

/// T022-095, FR-022-056 (per-key rotation, replacing T022-007's refusal): a new version is activated with live rows,
/// the old one is RETIRING and in force while a row uses it, out of force the moment its last row goes and recorded
/// RETIRED by the next activation; a third version waits for that; a version with no row is retired at once; at most
/// one version is RETIRING; the role that ran the command is recorded (G1190).
#[tokio::test]
async fn a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row() {
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
        ("ACTIVATED".to_owned(), 2),
        "a semaphore and a window of version 1 rotate with it"
    );
    assert_eq!(
        activate_key(pool, 3).await,
        ("ROTATION_IN_PROGRESS".to_owned(), 2),
        "one rotation at a time"
    );
    let states = || async {
        sqlx::query_as::<_, (i16, String, bool)>(
            "SELECT version, state, retired_at IS NOT NULL FROM flow_control_key_versions ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .expect("versions")
    };
    assert_eq!(
        states().await,
        [
            (1, "RETIRING".to_owned(), false),
            (2, "ACTIVE".to_owned(), false)
        ]
    );
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(
        states().await[0],
        (1, "RETIRING".to_owned(), false),
        "the window still uses version 1"
    );
    assert_eq!(
        activate_key(pool, 3).await,
        ("ROTATION_IN_PROGRESS".to_owned(), 1)
    );
    let in_force = || async {
        sqlx::query_as::<_, (Option<i16>, Option<i16>)>(
            "SELECT active, retiring FROM flow_control_key_versions_in_force()",
        )
        .fetch_one(pool)
        .await
        .expect("in force")
    };
    assert_eq!(in_force().await, (Some(2), Some(1)));
    sqlx::query("DELETE FROM rate_limit_grants")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(
        in_force().await,
        (Some(2), None),
        "out of force with its last row: no decider needs its key any more"
    );
    let second_retiring = sqlx::query(
        "INSERT INTO flow_control_key_versions (version, state, activated_at, activated_by) \
         VALUES (9, 'RETIRING', clock_timestamp(), 'forged')",
    )
    .execute(pool)
    .await
    .map_err(db_error);
    assert_eq!(
        second_retiring.map(drop).unwrap_err().0,
        "23505",
        "at most one version is retiring"
    );
    assert_eq!(
        activate_key(pool, 3).await,
        ("ACTIVATED".to_owned(), 0),
        "the unused retiring version is retired, and no row uses version 2"
    );
    assert_eq!(
        states().await,
        [
            (1, "RETIRED".to_owned(), true),
            (2, "RETIRED".to_owned(), true),
            (3, "ACTIVE".to_owned(), false)
        ],
        "a version no row uses is retired at once"
    );
    assert_eq!(activate_key(pool, 2).await.0, "RETIRED");
    let role: Option<String> = sqlx::query_scalar(
        "SELECT activated_role FROM flow_control_key_versions WHERE version = 3",
    )
    .fetch_one(pool)
    .await
    .expect("the role");
    assert!(role.is_some_and(|role| !role.is_empty()), "G1190");
    let two_active = sqlx::query(
        "UPDATE flow_control_key_versions SET state = 'ACTIVE', retired_at = NULL WHERE version = 1",
    )
    .execute(pool)
    .await
    .map_err(db_error);
    assert_eq!(
        two_active.map(drop).unwrap_err().0,
        "23505",
        "at most one version is active"
    );
}

/// T022-095's evidence: a rotation with live holders, waiters and grants never splits a key. A deciding transaction (as
/// the runtime role) reads the versions in force, digests the key under both, and files it on the row that exists under
/// the retiring version, else under the active one; two transactions that file a new key at once share one row; the
/// old version retires when its last row goes.
#[tokio::test]
async fn a_rotation_with_live_holders_waiters_and_grants_never_splits_a_key() {
    use sagat_application::keyed_digest::{KeyPurpose, KeyRing};
    use sagat_domain::execution::GatePool;
    use sagat_domain::identity::{NamespaceId, TenantId};
    use sagat_domain::workflow::{GateLevel, ResolvedGroup};
    use sagat_infra_postgres::{KeyTable, KeyToFile, file_keys, key_versions};

    let w = world().await;
    let pool = &w.db.pool;
    let runtime = w.db.runtime_pool().await;
    let first = a_run(&w).await;
    let second = a_run(&w).await;
    let (tenant, namespace) = (TenantId::new(first.0), NamespaceId::new(first.1));
    let ring =
        KeyRing::parse(&format!("1:{},2:{}", "11".repeat(32), "22".repeat(32))).expect("ring");
    let key = |purpose, value: &str, versions| KeyToFile {
        tenant,
        namespace,
        pool: GatePool::Ordinary,
        level: GateLevel::Run,
        group: ResolvedGroup::parse(if purpose == KeyPurpose::Concurrency {
            "accounts"
        } else {
            "providers"
        })
        .expect("group"),
        candidates: ring
            .candidates(versions, purpose, tenant, namespace, value.as_bytes())
            .expect("digests"),
        shard: 5,
    };
    assert!(
        key_versions(&mut runtime.acquire().await.expect("conn"))
            .await
            .is_err(),
        "no version was ever activated: nothing is decided"
    );
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");

    // before the rotation: a holder of acct-1 and a grant of prov-1, both under version 1
    let mut tx = runtime.begin().await.expect("tx");
    let versions = key_versions(&mut tx).await.expect("versions");
    assert_eq!((versions.active, versions.retiring), (1, None));
    let acct = file_keys(
        &mut tx,
        KeyTable::Semaphores,
        &[key(KeyPurpose::Concurrency, "acct-1", versions)],
    )
    .await
    .expect("filed");
    assert!(acct[0].created);
    let v1_acct = acct[0].digest;
    sqlx::query(
        "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, pool, group_name, \
         key_digest, declared_limit, acquired_at) \
         VALUES ($1, $2, $3, 'RUN', '', 0, 'ORDINARY', 'accounts', $4, 2, clock_timestamp())",
    )
    .bind(first.0)
    .bind(first.1)
    .bind(first.2)
    .bind(v1_acct.as_slice())
    .execute(&mut *tx)
    .await
    .expect("a holder");
    let prov = file_keys(
        &mut tx,
        KeyTable::Windows,
        &[key(KeyPurpose::RateLimit, "prov-1", versions)],
    )
    .await
    .expect("filed");
    sqlx::query(
        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
         key_digest, declared_limit, period_ms, granted_at, shard) \
         VALUES ($1, $2, $3, '', 0, 'ORDINARY', 'RUN', 'providers', $4, 10, 1000, clock_timestamp(), 5)",
    )
    .bind(first.0)
    .bind(first.1)
    .bind(first.2)
    .bind(prov[0].digest.as_slice())
    .execute(&mut *tx)
    .await
    .expect("a grant");
    tx.commit().await.expect("commit");

    assert_eq!(activate_key(pool, 2).await, ("ACTIVATED".to_owned(), 2));

    // during the rotation: acct-1 is found on its version-1 row and gains a waiter there; acct-2 is new under version 2
    let mut tx = runtime.begin().await.expect("tx");
    let versions = key_versions(&mut tx).await.expect("versions");
    assert_eq!((versions.active, versions.retiring), (2, Some(1)));
    let filed = file_keys(
        &mut tx,
        KeyTable::Semaphores,
        &[
            key(KeyPurpose::Concurrency, "acct-2", versions),
            key(KeyPurpose::Concurrency, "acct-1", versions),
        ],
    )
    .await
    .expect("filed");
    let old = filed.iter().find(|f| f.index == 1).expect("acct-1");
    let new = filed.iter().find(|f| f.index == 0).expect("acct-2");
    assert_eq!(
        (old.digest, old.version, old.created),
        (v1_acct, 1, false),
        "the existing row wins"
    );
    assert_eq!((new.version, new.created), (2, true));
    let mut order = filed.iter().map(|f| f.digest).collect::<Vec<_>>();
    order.sort_unstable();
    assert_eq!(
        order,
        filed.iter().map(|f| f.digest).collect::<Vec<_>>(),
        "locked in digest order"
    );
    sqlx::query(
        "INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
         group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
         VALUES ($1, $2, $3, '', 'RUN', 0, 'ORDINARY', 'accounts', $4, 2, clock_timestamp(), \
                 clock_timestamp() + interval '1 minute', 1)",
    )
    .bind(second.0)
    .bind(second.1)
    .bind(second.2)
    .bind(v1_acct.as_slice())
    .execute(&mut *tx)
    .await
    .expect("a waiter on the old row");
    tx.commit().await.expect("commit");

    // two transactions file a new key at once: the second waits for the first's row and shares it
    let mut holder_tx = runtime.begin().await.expect("tx");
    let versions = key_versions(&mut holder_tx).await.expect("versions");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder_tx)
        .await
        .expect("pid");
    let first_filing = file_keys(
        &mut holder_tx,
        KeyTable::Semaphores,
        &[key(KeyPurpose::Concurrency, "acct-3", versions)],
    )
    .await
    .expect("filed");
    assert!(first_filing[0].created);
    let contender = {
        let runtime = runtime.clone();
        let wanted = key(KeyPurpose::Concurrency, "acct-3", versions);
        tokio::spawn(async move {
            let mut tx = runtime.begin().await.expect("tx");
            let filed = file_keys(&mut tx, KeyTable::Semaphores, &[wanted])
                .await
                .expect("filed");
            tx.commit().await.expect("commit");
            filed
        })
    };
    loop {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(holder_pid)
        .fetch_one(pool)
        .await
        .expect("activity");
        if blocked > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    holder_tx.commit().await.expect("commit");
    let second_filing = contender.await.expect("the contender");
    assert_eq!(second_filing[0].digest, first_filing[0].digest);
    assert!(
        !second_filing[0].created,
        "it found the row the first one made"
    );

    // no key is ever split: each key has exactly one row, under one of its two digests
    for value in ["acct-1", "acct-2", "acct-3"] {
        let candidates = ring
            .candidates(
                versions,
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                value.as_bytes(),
            )
            .expect("digests");
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM concurrency_semaphores WHERE tenant_id = $1 AND key_digest = ANY($2)",
        )
        .bind(first.0)
        .bind(vec![
            candidates.active.as_bytes().to_vec(),
            candidates.retiring.expect("rotating").as_bytes().to_vec(),
        ])
        .fetch_one(pool)
        .await
        .expect("rows");
        assert_eq!(rows, 1, "{value}");
    }
    assert_eq!(activate_key(pool, 3).await.0, "ROTATION_IN_PROGRESS");

    // the old version leaves force with its last row: the grant of prov-1, then the holder and waiter of acct-1
    sqlx::query("DELETE FROM rate_limit_grants")
        .execute(pool)
        .await
        .expect("drain");
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    let retiring = || async {
        let mut conn = runtime.acquire().await.expect("conn");
        key_versions(&mut conn).await.expect("versions").retiring
    };
    assert_eq!(
        retiring().await,
        Some(1),
        "the waiter of acct-1 still uses it"
    );
    sqlx::query("DELETE FROM concurrency_waiters")
        .execute(&runtime)
        .await
        .expect("the runtime role drains it");
    assert_eq!(retiring().await, None, "out of force with its last row");
    assert_eq!(
        activate_key(pool, 3).await,
        ("ACTIVATED".to_owned(), 2),
        "acct-2 and acct-3 rotate with version 2"
    );
    let state: String =
        sqlx::query_scalar("SELECT state FROM flow_control_key_versions WHERE version = 1")
            .fetch_one(pool)
            .await
            .expect("state");
    assert_eq!(state, "RETIRED", "the next activation recorded it");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM concurrency_semaphores WHERE key_version = 1")
            .fetch_one(pool)
            .await
            .expect("rows");
    assert_eq!(left, 0);
}

// ------------------------------------------------------------------ the review of 2026-11-15

/// Security review M1, architecture review M1: the runtime role cannot set a counter when it inserts a key row either.
#[tokio::test]
async fn the_runtime_role_cannot_set_a_counter_when_it_inserts_a_key() {
    let w = world().await;
    let runtime = w.db.runtime_pool().await;
    let (tenant, namespace, _) = a_run(&w).await;
    assert_eq!(activate_key(&w.db.pool, 1).await.0, "ACTIVATED");
    for statement in [
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
         holders, key_version, key_shard, updated_at) \
         VALUES ($1, $2, 'ORDINARY', 'RUN', 'forged', $3, 10000, 1, 0, clock_timestamp())",
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
         live_grants, grants_total, max_period_ms, key_version, key_shard, updated_at) \
         VALUES ($1, $2, 'ORDINARY', 'RUN', 'forged', $3, 100, 100, 1000, 1, 0, clock_timestamp())",
    ] {
        let forged = sqlx::query(statement)
            .bind(tenant)
            .bind(namespace)
            .bind(DIGEST.as_slice())
            .execute(&runtime)
            .await
            .map_err(db_error);
        assert_eq!(forged.map(drop).unwrap_err().0, "42501", "{statement}");
    }
}

/// Security review (suspicion), architecture review M2: a key row is written only under the active key version, which
/// a deciding transaction reads through a function that takes it shared; a retired version is never active again.
#[tokio::test]
async fn a_key_row_is_written_only_under_the_active_version_and_a_retired_one_stays_retired() {
    let w = world().await;
    let pool = &w.db.pool;
    let runtime = w.db.runtime_pool().await;
    let run = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    assert_eq!(activate_key(pool, 2).await.0, "ACTIVATED");
    let stale = sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
         key_version, key_shard, updated_at) VALUES ($1, $2, 'ORDINARY', 'RUN', '=flow-anchor', $3, 1, 7, clock_timestamp())",
    )
    .bind(run.0)
    .bind(run.1)
    .bind(DIGEST.as_slice())
    .execute(&runtime)
    .await
    .map_err(db_error);
    assert_eq!(
        stale.map(drop).unwrap_err().0,
        "23001",
        "version 1 is no longer the active one"
    );
    let mut tx = runtime.begin().await.expect("tx");
    let active: i16 = sqlx::query_scalar("SELECT flow_control_active_key_version()")
        .fetch_one(&mut *tx)
        .await
        .expect("the runtime role reads the active version, shared");
    assert_eq!(active, 2);
    tx.rollback().await.expect("rollback");
    semaphore(&runtime, run, 2).await;
    sqlx::query("UPDATE flow_control_key_versions SET state = 'RETIRED', retired_at = clock_timestamp() WHERE version = 1")
        .execute(pool)
        .await
        .expect("the operator retires version 1");
    sqlx::query("DELETE FROM concurrency_semaphores")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(
        activate_key(pool, 1).await.0,
        "RETIRED",
        "a retired version is never active again"
    );
}

/// Correctness review m6: a key emptied and filled again in one transaction keeps its row (the empty row is deleted at
/// commit, not at the delete), and a waiter row is refused an update with the guard's own error.
#[tokio::test]
async fn a_key_emptied_and_filled_in_one_transaction_keeps_its_row() {
    let w = world().await;
    let pool = &w.db.pool;
    let first = a_run(&w).await;
    let second = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    semaphore(pool, first, 1).await;
    holder(pool, first).await.expect("a holder");
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("DELETE FROM concurrency_holders WHERE run_id = $1")
        .bind(first.2)
        .execute(&mut *tx)
        .await
        .expect("released");
    holder(&mut *tx, second)
        .await
        .expect("the next holder of the same key");
    tx.commit().await.expect("commit");
    assert_eq!(counts(pool, first.0).await, Some((1, 0)));

    window(pool, first, 1).await;
    sqlx::query(
        "INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
         key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
         VALUES ($1, $2, $3, 'a', 1, 'ORDINARY', 'STEP', 'provider-starts', $4, 10, 1000, clock_timestamp(), \
                 clock_timestamp() + interval '1 minute', 3)",
    )
    .bind(first.0)
    .bind(first.1)
    .bind(first.2)
    .bind(DIGEST.as_slice())
    .execute(pool)
    .await
    .expect("a waiter");
    let changed = sqlx::query("UPDATE rate_limit_waiters SET declared_limit = 2")
        .execute(pool)
        .await
        .map_err(db_error);
    assert_eq!(
        changed.map(drop).unwrap_err().0,
        "23001",
        "a waiter is never changed"
    );
}

const QUOTA_RESOURCES: i64 = 20_270_130_000_300;
const ROTATION: i64 = 20_270_130_000_500;
const DECLARATIONS: i64 = 20_270_130_000_400;

/// T022-013 and T022-069 (D022-040, D022-041): rolling back the quota resources refuses while an administrator's limit
/// or an operator's ceiling of one exists, and rolling back the group declarations refuses while a published version
/// declares one; drained, both roll back and go forward again.
#[tokio::test]
async fn the_quota_and_declaration_migrations_roll_back_only_once_their_rows_are_drained() {
    let w = world().await;
    let pool = &w.db.pool;
    let (tenant, namespace, run) = a_run(&w).await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'queued_runs', 5, 'admin')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(pool)
    .await
    .expect("a limit of a new resource");
    let (workflow, version): (Uuid, Uuid) =
        sqlx::query_as("SELECT r.workflow_id, r.workflow_version_id FROM runs r WHERE r.id = $1")
            .bind(run)
            .fetch_one(pool)
            .await
            .expect("the run's version");
    sqlx::query(
        "INSERT INTO flow_control_group_declarations \
         (tenant_id, namespace_id, workflow_id, version_id, gate, node, group_name, declared_limit, period_ms) \
         VALUES ($1, $2, $3, $4, 'RATE_LIMIT', 'a', 'provider-api', 10, 1000)",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(workflow)
    .bind(version)
    .execute(pool)
    .await
    .expect("a declaration");
    for statement in [
        "UPDATE flow_control_group_declarations SET declared_limit = 11",
        "DELETE FROM flow_control_group_declarations",
    ] {
        let error = sqlx::query(statement)
            .execute(pool)
            .await
            .expect_err(statement);
        assert_eq!(
            db_error(error).0,
            "23001",
            "{statement}: a declaration never changes"
        );
    }

    let refused = undo(pool, QUOTA_RESOURCES).await;
    assert!(
        format!("{refused:?}").contains("published versions declare flow-control groups"),
        "{refused:?}"
    );
    assert!(exists(pool, "flow_control_group_declarations").await);
    sqlx::query("ALTER TABLE flow_control_group_declarations DISABLE TRIGGER flow_control_group_declarations_immutable")
        .execute(pool)
        .await
        .expect("the operator drains it on purpose");
    sqlx::query("DELETE FROM flow_control_group_declarations")
        .execute(pool)
        .await
        .expect("drain");
    let refused = undo(pool, CONCURRENCY + 200).await;
    assert!(
        format!("{refused:?}")
            .contains("limits or ceilings of flow-control resources are retained"),
        "{refused:?}"
    );
    assert!(!exists(pool, "flow_control_group_declarations").await);
    sqlx::query("DELETE FROM quota_limits WHERE resource = 'queued_runs'")
        .execute(pool)
        .await
        .expect("drain");
    undo(pool, CONCURRENCY + 200)
        .await
        .expect("rolled back past both");
    let old_check = sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'concurrency_live_keys', 5, 'admin')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(pool)
    .await;
    assert!(old_check.is_err(), "the list is the old one again");
    MIGRATOR.run(pool).await.expect("forward again");
    assert!(exists(pool, "flow_control_group_declarations").await);
    let _ = DECLARATIONS;
}

/// T022-013: the usage the quota view reads for the three counts of the gate. A run waiting at both gates counts once in
/// `queued_runs` and once in each key's waiters; each key row is a live key.
#[tokio::test]
async fn the_counts_of_the_gate_read_the_waiter_and_key_rows() {
    let w = world().await;
    let pool = &w.db.pool;
    let first = a_run(&w).await;
    let second = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    let (tenant, namespace, _) = first;
    sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
         key_version, key_shard, updated_at) VALUES ($1, $2, 'ORDINARY', 'RUN', 'accounts', $3, 1, 7, clock_timestamp())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(DIGEST.as_slice())
    .execute(pool)
    .await
    .expect("a semaphore");
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
         key_version, key_shard, updated_at) VALUES ($1, $2, 'ORDINARY', 'RUN', 'provider-api', $3, 1, 3, clock_timestamp())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(DIGEST.as_slice())
    .execute(pool)
    .await
    .expect("a window");
    for (_, _, run) in [first, second] {
        sqlx::query(
            "INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
             group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
             VALUES ($1, $2, $3, '', 'RUN', 0, 'ORDINARY', 'accounts', $4, 1, clock_timestamp(), \
                     clock_timestamp() + interval '1 minute', 1)",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(run)
        .bind(DIGEST.as_slice())
        .execute(pool)
        .await
        .expect("a concurrency waiter");
    }
    // the first run waits at both gates
    sqlx::query(
        "INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
         key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
         VALUES ($1, $2, $3, '', 0, 'ORDINARY', 'RUN', 'provider-api', $4, 10, 1000, clock_timestamp(), \
                 clock_timestamp() + interval '1 minute', 1)",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(first.2)
    .bind(DIGEST.as_slice())
    .execute(pool)
    .await
    .expect("a rate-limit waiter");

    let used = |resource| {
        let w = &w;
        async move {
            w.quotas
                .list(
                    &w.admin,
                    Some(w.namespace.id()),
                    &crate::support::engine::ctx(),
                )
                .await
                .expect("the quota view")
                .into_iter()
                .find(|view| view.resource == resource)
                .and_then(|view| view.used)
        }
    };
    use sagat_domain::quota::QuotaResource;
    assert_eq!(
        used(QuotaResource::QueuedRuns).await,
        Some(2),
        "two runs wait, one of them at both gates"
    );
    assert_eq!(
        used(QuotaResource::ConcurrencyLiveKeys).await,
        Some(1),
        "the semaphore; a window is not a concurrency key"
    );
    assert_eq!(
        used(QuotaResource::FlowControlKeyWaiters).await,
        Some(2),
        "the busiest key holds two run waiters"
    );
}

/// Correctness review F1, architecture review M3 (2026-11-16): a key whose retiring row goes away between its resolution
/// and its lock is not filed out of the one lock order; the deciding transaction gets a retryable error, and its retry
/// files the key under the active version.
#[tokio::test]
async fn a_retiring_row_that_goes_away_while_it_is_filed_is_a_retryable_refusal() {
    use sagat_application::error::StoreError;
    use sagat_application::keyed_digest::{KeyPurpose, KeyRing};
    use sagat_domain::execution::GatePool;
    use sagat_domain::identity::{NamespaceId, TenantId};
    use sagat_domain::workflow::{GateLevel, ResolvedGroup};
    use sagat_infra_postgres::{KEY_ROW_VANISHED, KeyTable, KeyToFile, file_keys, key_versions};

    let w = world().await;
    let pool = &w.db.pool;
    let runtime = w.db.runtime_pool().await;
    let run = a_run(&w).await;
    let (tenant, namespace) = (TenantId::new(run.0), NamespaceId::new(run.1));
    let ring =
        KeyRing::parse(&format!("1:{},2:{}", "11".repeat(32), "22".repeat(32))).expect("ring");
    let key = |versions| KeyToFile {
        tenant,
        namespace,
        pool: GatePool::Ordinary,
        level: GateLevel::Run,
        group: ResolvedGroup::parse("accounts").expect("group"),
        candidates: ring
            .candidates(
                versions,
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                b"acct-9",
            )
            .expect("digests"),
        shard: 2,
    };
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    let mut tx = runtime.begin().await.expect("tx");
    let versions = key_versions(&mut tx).await.expect("versions");
    let filed = file_keys(&mut tx, KeyTable::Semaphores, &[key(versions)])
        .await
        .expect("filed");
    sqlx::query(
        "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, pool, group_name, \
         key_digest, declared_limit, acquired_at) \
         VALUES ($1, $2, $3, 'RUN', '', 0, 'ORDINARY', 'accounts', $4, 1, clock_timestamp())",
    )
    .bind(run.0)
    .bind(run.1)
    .bind(run.2)
    .bind(filed[0].digest.as_slice())
    .execute(&mut *tx)
    .await
    .expect("a holder");
    tx.commit().await.expect("commit");
    assert_eq!(activate_key(pool, 2).await.0, "ACTIVATED");

    // the release that empties the old row holds its lock; the decider resolves to it and waits for it
    let mut release = pool.begin().await.expect("the release");
    let release_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *release)
        .await
        .expect("pid");
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(&mut *release)
        .await
        .expect("the last holder goes");
    let rotating = {
        let mut conn = runtime.acquire().await.expect("conn");
        key_versions(&mut conn).await.expect("versions")
    };
    assert_eq!(rotating.retiring, Some(1), "the release has not committed");
    let decider = {
        let runtime = runtime.clone();
        let wanted = key(rotating);
        tokio::spawn(async move {
            let mut tx = runtime.begin().await.expect("tx");
            file_keys(&mut tx, KeyTable::Semaphores, &[wanted]).await
        })
    };
    loop {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(release_pid)
        .fetch_one(pool)
        .await
        .expect("activity");
        if blocked > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    release
        .commit()
        .await
        .expect("the old row is pruned at commit");
    match decider.await.expect("the decider") {
        Err(StoreError::Unavailable { detail }) => assert_eq!(detail, KEY_ROW_VANISHED),
        other => panic!("expected the retryable refusal, got {other:?}"),
    }
    let mut tx = runtime.begin().await.expect("the retry");
    let versions = key_versions(&mut tx).await.expect("versions");
    assert_eq!(versions.retiring, None, "nothing uses version 1 any more");
    let retried = file_keys(&mut tx, KeyTable::Semaphores, &[key(versions)])
        .await
        .expect("filed");
    assert_eq!((retried[0].version, retried[0].created), (2, true));
    tx.rollback().await.expect("rollback");
}

async fn versions_in_force(pool: &PgPool) -> (i16, Option<i16>) {
    let (active, retiring): (Option<i16>, Option<i16>) =
        sqlx::query_as("SELECT active, retiring FROM flow_control_key_versions_in_force()")
            .fetch_one(pool)
            .await
            .expect("in force");
    (active.expect("active"), retiring)
}

/// Security review S2, correctness review F3 (2026-11-16): the earlier command left every replaced version RETIRING with
/// no row; the rotation migration retires them, so at most one version is ever retiring and none is in force unused; its
/// down migration refuses while a rotation is in progress.
#[tokio::test]
async fn the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation()
 {
    let w = world().await;
    let pool = &w.db.pool;
    let run = a_run(&w).await;
    undo(pool, ROTATION - 100)
        .await
        .expect("back to the earlier command");
    for version in [1, 2, 3] {
        assert_eq!(
            activate_key(pool, version).await.0,
            "ACTIVATED",
            "the earlier command"
        );
    }
    let retiring: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM flow_control_key_versions WHERE state = 'RETIRING'",
    )
    .fetch_one(pool)
    .await
    .expect("count");
    assert_eq!(retiring, 2, "what the earlier command leaves behind");
    MIGRATOR.run(pool).await.expect("forward");
    let states: Vec<(i16, String)> =
        sqlx::query_as("SELECT version, state FROM flow_control_key_versions ORDER BY version")
            .fetch_all(pool)
            .await
            .expect("versions");
    assert_eq!(
        states,
        [
            (1, "RETIRED".to_owned()),
            (2, "RETIRED".to_owned()),
            (3, "ACTIVE".to_owned())
        ]
    );
    assert_eq!(versions_in_force(pool).await, (3, None));

    semaphore(pool, run, 3).await;
    holder(pool, run).await.expect("a holder");
    assert_eq!(activate_key(pool, 4).await.0, "ACTIVATED");
    let refused = undo(pool, ROTATION - 100).await;
    assert!(
        format!("{refused:?}").contains("a digest key rotation is in progress"),
        "{refused:?}"
    );
    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    undo(pool, ROTATION - 100)
        .await
        .expect("rolled back once the rotation is over");
    MIGRATOR.run(pool).await.expect("forward again");
}

// ------------------------------------------------------------------ the force of 2026-11-19 (OD-2026-11-19-2)

/// The migration before the force's (`20270202000000`, ADR-0093 decision 5): the last flow-control migration.
const BEFORE_FORCE: i64 = 20_270_130_000_700;

async fn force_retire(
    pool: &PgPool,
    version: i16,
    reason: &str,
) -> Result<(String, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT out_outcome, out_rows FROM flow_control_key_force_retire($1, 'operator:incident', $2)",
    )
    .bind(version)
    .bind(reason)
    .fetch_one(pool)
    .await
}

/// OD-2026-11-19-2: the force retires only a retiring version, is the operator's alone, needs a reason, lists the runs
/// that hold the version's keys, is recorded in an append-only history, lets the next rotation start at once, and its
/// down migration refuses while a forced retirement is on the record.
#[tokio::test]
async fn the_force_is_the_operators_audited_and_retires_only_a_retiring_version() {
    let w = world().await;
    let pool = &w.db.pool;
    let runtime = w.db.runtime_pool().await;
    let run = a_run(&w).await;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    assert_eq!(
        force_retire(pool, 1, "leaked").await.expect("answered"),
        ("ACTIVE".to_owned(), 0),
        "the active version is retired by activating a successor first"
    );
    assert_eq!(
        force_retire(pool, 9, "leaked").await.expect("answered"),
        ("UNKNOWN".to_owned(), 0)
    );
    semaphore(pool, run, 1).await;
    holder(pool, run).await.expect("a holder");
    assert_eq!(activate_key(pool, 2).await, ("ACTIVATED".to_owned(), 1));
    assert_eq!(activate_key(pool, 3).await.0, "ROTATION_IN_PROGRESS");

    // the reason is bounded as typed, padding included (security review of 2026-11-20, finding 1)
    let padded = format!("x{}", " ".repeat(1_000));
    for reason in ["", "   ", "line\nbreak", padded.as_str()] {
        let refused = force_retire(pool, 1, reason).await.expect_err("no reason");
        assert_eq!(db_error(refused).0, "22023", "{reason:?}");
    }
    // the operator's name is one line, as the reason (security review of 2026-11-20, finding 3)
    for actor in [None, Some(""), Some("alice\nFORCED by bob")] {
        let refused =
            sqlx::query("SELECT * FROM flow_control_key_force_retire(1::smallint, $1, 'leaked')")
                .bind(actor)
                .fetch_one(pool)
                .await
                .expect_err("no such operator");
        assert_eq!(db_error(refused).0, "22023", "{actor:?}");
    }
    let refused = force_retire(&runtime, 1, "leaked")
        .await
        .expect_err("not the runtime role's");
    assert_eq!(db_error(refused).0, "42501");
    for statement in [
        "SELECT * FROM flow_control_key_version_runs(1::smallint)",
        "SELECT * FROM flow_control_key_forced_retirements",
    ] {
        let refused = sqlx::query(statement)
            .fetch_all(&runtime)
            .await
            .expect_err("the operator's");
        assert_eq!(db_error(refused).0, "42501", "{statement}");
    }

    // what the operator waits for (or cancels) before an ordinary rotation can end
    let holding: Vec<(Uuid, Uuid, Uuid, i64, i64, i64)> =
        sqlx::query_as("SELECT * FROM flow_control_key_version_runs(1::smallint)")
            .fetch_all(pool)
            .await
            .expect("the runs of version 1");
    assert_eq!(holding, vec![(run.0, run.1, run.2, 1, 0, 0)]);

    assert_eq!(
        force_retire(pool, 1, "version 1 leaked")
            .await
            .expect("forced"),
        ("FORCED".to_owned(), 1)
    );
    assert_eq!(versions_in_force(pool).await, (2, None));
    assert_eq!(
        counts(pool, run.0).await,
        Some((1, 0)),
        "its row lives on until it drains"
    );
    assert_eq!(
        force_retire(pool, 1, "again").await.expect("answered"),
        ("ALREADY_RETIRED".to_owned(), 1),
        "with the rows it still has"
    );
    assert_eq!(
        activate_key(pool, 1).await.0,
        "RETIRED",
        "never active again"
    );
    // a key row under 2, then the next rotation, which starts at once
    sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
         key_version, key_shard, updated_at) VALUES ($1, $2, 'ORDINARY', 'RUN', '=flow-anchor', $3, 2, 7, clock_timestamp())",
    )
    .bind(run.0)
    .bind(run.1)
    .bind([8_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("a key under 2");
    assert_eq!(
        activate_key(pool, 3).await,
        ("ACTIVATED".to_owned(), 1),
        "the next rotation starts at once"
    );
    // a second force while the first one's rows drain would split a key into three rows (reviews of 2026-11-20)
    assert_eq!(
        force_retire(pool, 2, "version 2 leaked too")
            .await
            .expect("answered"),
        ("FORCED_VERSION_DRAINING".to_owned(), 1),
        "the rows a force left under 1 drain first"
    );

    let recorded: Vec<(i16, i64, String, String)> = sqlx::query_as(
        "SELECT version, rows_left, actor, reason FROM flow_control_key_forced_retirements ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("history");
    assert_eq!(
        recorded,
        vec![(
            1,
            1,
            "operator:incident".to_owned(),
            "version 1 leaked".to_owned()
        )]
    );
    for statement in [
        "UPDATE flow_control_key_forced_retirements SET reason = 'nothing'",
        "DELETE FROM flow_control_key_forced_retirements",
        "TRUNCATE flow_control_key_forced_retirements",
    ] {
        let refused = sqlx::query(statement)
            .execute(pool)
            .await
            .expect_err("append-only");
        assert_eq!(db_error(refused).0, "23001", "{statement}");
    }

    sqlx::query("DELETE FROM concurrency_holders")
        .execute(pool)
        .await
        .expect("drain");
    assert_eq!(
        force_retire(pool, 2, "version 2 leaked too")
            .await
            .expect("forced"),
        ("FORCED".to_owned(), 1),
        "once 1's rows are gone"
    );
    let forced: i64 =
        sqlx::query_scalar("SELECT count(*) FROM flow_control_key_forced_retirements")
            .fetch_one(pool)
            .await
            .expect("history");
    assert_eq!(forced, 2);
    let refused = undo(pool, BEFORE_FORCE).await;
    assert!(
        format!("{refused:?}")
            .contains("a forced retirement of a digest key version is on the record"),
        "{refused:?}"
    );
}

/// The migration before the reset's slot refusal (task T022-086).
const BEFORE_RESET_SLOT: i64 = 20_270_207_000_100;

/// Task T022-086 (D022-069 item 4; migration `20270209000000_reset_rate_slot`): the reset register accepts the reason
/// `RATE_LIMIT_SLOT_UNAVAILABLE`, and the rollback refuses while an entry carries it (older code could not keep it under
/// the reasons it restores); once the operator has removed the entry, it rolls back to the old list and forward again.
#[tokio::test]
async fn the_reset_slot_reason_rolls_back_only_once_no_entry_carries_it() {
    let w = world().await;
    let pool = &w.db.pool;
    let (tenant, namespace, run) = a_run(&w).await;
    let insert = |id: Uuid, reason: &'static str| async move {
        sqlx::query(
            "INSERT INTO run_resets (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, \
             reapply, business_id_policy, source_policy, state, failure_reason, actor, reason, created_at, completed_at) \
             VALUES ($1, $2, $3, $4, 1, $5, 'NONE', 'KEEP', 'CANCEL_SOURCE', 'FAILED', $6, 'admin', \
                     'a reset the key refused', now(), now())",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(id)
        .bind(run)
        .bind(DIGEST.as_slice())
        .bind(reason)
        .execute(pool)
        .await
    };
    let entry = Uuid::now_v7();
    insert(entry, "RATE_LIMIT_SLOT_UNAVAILABLE")
        .await
        .expect("the new reason is accepted");
    let refused = undo(pool, BEFORE_RESET_SLOT).await;
    assert!(
        format!("{refused:?}").contains("resets a rate-limit key refused are retained"),
        "{refused:?}"
    );
    // an entry that failed for another reason is not in the way; the one the key refused is removed by the operator (a
    // completed entry never changes)
    insert(Uuid::now_v7(), "ADMISSION_REFUSED")
        .await
        .expect("an entry that failed for another reason");
    sqlx::query("DELETE FROM run_resets WHERE id = $1")
        .bind(entry)
        .execute(pool)
        .await
        .expect("the operator removes the entry");
    undo(pool, BEFORE_RESET_SLOT)
        .await
        .expect("rolled back once no entry carries it");
    let old = insert(Uuid::now_v7(), "RATE_LIMIT_SLOT_UNAVAILABLE").await;
    assert_eq!(
        db_error(old.expect_err("the old list")).0,
        "23514",
        "the list is the old one again"
    );
    MIGRATOR.run(pool).await.expect("forward again");
    insert(Uuid::now_v7(), "RATE_LIMIT_SLOT_UNAVAILABLE")
        .await
        .expect("accepted again");
    let validated: bool = sqlx::query_scalar(
        "SELECT convalidated FROM pg_constraint WHERE conname = 'run_resets_failure_reason_valid'",
    )
    .fetch_one(pool)
    .await
    .expect("the constraint");
    assert!(validated, "20270209000100 validates it");
}

/// The migration before the waiters' declarations (decision D022-078 item 1). They were renumbered after the latest
/// migration of every branch (their first number was taken by feature 041), and the queued-runs counters and the alert
/// kinds after them (review of 2026-11-25, correctness minor 8), so this is still the reset slot's validation.
const BEFORE_PINNED_LIMIT: i64 = 20_270_209_000_100;

/// Decisions D022-076 item 7 and D022-078 item 1 (migration `20270213000000_flow_control_waiter_pinned_limit`): the waiter
/// rows of both gates carry the declaration their version pins (`pinned_limit`), never below the filed limit; the
/// rollback refuses while a waiter's declaration is above its filed limit (no older binary could recover it) and goes
/// through once the rows agree or are gone; forward again, a row written before has none and the CHECKs are validated.
#[tokio::test]
async fn the_waiters_declaration_rolls_back_only_once_no_waiter_would_lose_it() {
    let w = world().await;
    let pool = &w.db.pool;
    let anchor = a_run(&w).await;
    let (tenant, namespace, run) = anchor;
    assert_eq!(activate_key(pool, 1).await.0, "ACTIVATED");
    window(pool, anchor, 1).await;
    let waiter = |filed: i32, pinned: Option<i32>| async move {
        sqlx::query(
            "INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
             group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard, pinned_limit) \
             VALUES ($1, $2, $3, 'a', 1, 'ORDINARY', 'STEP', 'provider-starts', $4, $5, 1000, clock_timestamp(), \
                     clock_timestamp() + interval '1 minute', 3, $6)",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(run)
        .bind(DIGEST.as_slice())
        .bind(filed)
        .bind(pinned)
        .execute(pool)
        .await
        .map(drop)
    };
    let below = waiter(5, Some(4)).await;
    assert_eq!(
        db_error(below.expect_err("a declaration below its filed limit")).0,
        "23514",
        "the filed limit is never above the declaration"
    );
    waiter(2, Some(5))
        .await
        .expect("a waiter filed under a lowered ceiling");
    let refused = undo(pool, BEFORE_PINNED_LIMIT).await;
    assert!(
        format!("{refused:?}").contains("bounded by a declaration above their filed limit"),
        "{refused:?}"
    );
    assert!(
        exists_column(pool, "rate_limit_waiters").await,
        "a refused rollback changes nothing"
    );
    sqlx::query("DELETE FROM rate_limit_waiters WHERE run_id = $1")
        .bind(run)
        .execute(pool)
        .await
        .expect("the waiter drains");
    // the emptied window went with its last waiter
    window(pool, anchor, 1).await;
    waiter(3, Some(3))
        .await
        .expect("a waiter whose two limits agree");
    undo(pool, BEFORE_PINNED_LIMIT)
        .await
        .expect("rolled back: a waiter whose limits agree loses nothing");
    assert!(!exists_column(pool, "rate_limit_waiters").await);
    assert!(!exists_column(pool, "concurrency_waiters").await);
    MIGRATOR.run(pool).await.expect("forward again");
    let kept: Option<i32> =
        sqlx::query_scalar("SELECT pinned_limit FROM rate_limit_waiters WHERE run_id = $1")
            .bind(run)
            .fetch_one(pool)
            .await
            .expect("the older row");
    assert_eq!(kept, None, "a row written before the column has none");
    let validated: Vec<bool> = sqlx::query_scalar(
        "SELECT convalidated FROM pg_constraint \
          WHERE conname IN ('concurrency_waiters_pinned_limit_valid', 'rate_limit_waiters_pinned_limit_valid')",
    )
    .fetch_all(pool)
    .await
    .expect("the constraints");
    assert_eq!(validated, [true, true], "20270213000100 validates both");
    sqlx::query("DELETE FROM rate_limit_waiters WHERE run_id = $1")
        .bind(run)
        .execute(pool)
        .await
        .expect("cleaned");
}

/// Whether `table` has the column `pinned_limit`.
async fn exists_column(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
                         WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'pinned_limit')",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .expect("query")
}

/// Task T022-105 (migration `20270213000200`; decision D022-084): the queued-runs counters are filled from the waiter rows
/// present when they are created, a dual-gated run once and a step waiter not at all; rolling them back loses nothing
/// (the waiter rows stay, the counters are derived), and migrating forward again counts the same. The alert kinds of task
/// T022-082 (`20270213000300`) roll back while no alert of theirs is kept, and refuse while one is.
#[tokio::test]
async fn the_queued_runs_counters_are_filled_from_the_waiters_and_roll_back_without_loss() {
    // the waiters' declarations, so the rollback takes the counters and the alert kinds only (correctness minor 8)
    const BEFORE_COUNTS: i64 = 20_270_213_000_100;
    let w = crate::support::flow_control::gated_world().await;
    let pool = &w.db.pool;
    let mut dual = document("counts-dual", "g", vec![activity("g")]);
    dual["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": 1, "per": "hour"});
    dual["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let dual = crate::support::flow_control::publish(&w, dual).await;
    let input = json!({"accountId": "acct-1"});
    let _holder = w.start(dual, &input).await;
    let _first = w.start(dual, &input).await;
    let _second = w.start(dual, &input).await;
    let queued = |pool: &PgPool| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(sum(queued_runs), 0)::bigint FROM flow_control_counts",
            )
            .fetch_one(&pool)
            .await
            .expect("counted")
        }
    };
    assert_eq!(queued(pool).await, 2);
    undo(pool, BEFORE_COUNTS).await.expect("rolled back");
    assert!(!exists(pool, "flow_control_counts").await);
    let waiters: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM concurrency_waiters) + (SELECT count(*) FROM rate_limit_waiters)",
    )
    .fetch_one(pool)
    .await
    .expect("waiters");
    assert_eq!(waiters, 4, "the waiter rows stay");
    MIGRATOR.run(pool).await.expect("forward again");
    assert_eq!(
        queued(pool).await,
        2,
        "filled from the rows present, a dual-gated run once"
    );

    // an alert of a flow-control kind is retained: its rollback refuses
    sqlx::query(
        "INSERT INTO operational_alerts (id, tenant_id, namespace_id, kind, subject, state, severity, first_seen_at, \
                last_seen_at, fired_count, item_count, oldest_seconds) \
         VALUES (gen_random_uuid(), $1, $2, 'RATE_LIMIT_QUEUE_SATURATED', 'accounts', 'FIRING', 'warning', now(), now(), 1, 1, 0)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(pool)
    .await
    .expect("an alert");
    let refused = undo(pool, BEFORE_COUNTS).await;
    assert!(
        format!("{refused:?}").contains("flow-control alerts are retained"),
        "{refused:?}"
    );
    MIGRATOR.run(pool).await.expect("forward");
}
