//! The migration of task T041-5 as an object: its rollback, its grants and privileges, its guards, the source-text checks
//! of the quota-lease rules, the two architecture tests (ADR-0084 decisions 1 and 2) and the per-replica pool names.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use sagat_infra_postgres::{MIGRATOR, PoolConfig, PoolName, connect, migrate, step_log_wait_slots};
use sqlx::{Connection as _, PgConnection};
use uuid::Uuid;

use super::{Fx, LIMIT, lease_of, refusal};
use crate::support::{self, TestDb, bounded};

/// The migrations of this task and the newest before them.
const STEP_LOGS: i64 = 20_270_210_000_000;
const BEFORE: i64 = 20_270_201_000_100;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository")
        .to_path_buf()
}

/// Rolls the schema back to `target` on a session of its own, as `step_log_quotas_migration.rs` does: a refused
/// migration keeps its session advisory lock, so the lock is released and the session closed.
async fn undo_to(db: &TestDb, target: i64) -> Result<(), sqlx::migrate::MigrateError> {
    let mut connection = PgConnection::connect(&db.url)
        .await
        .expect("a session for the migrator");
    let result = MIGRATOR.undo(&mut connection, target).await;
    if result.is_err() {
        sqlx::migrate::Migrate::unlock(&mut connection)
            .await
            .expect("release the migration lock");
    }
    connection
        .close()
        .await
        .expect("close the migrator session");
    if result.is_err() {
        migrate(&db.pool)
            .await
            .expect("bring back what was undone before the refusal");
    }
    result
}

async fn exists(db: &TestDb, sql: &str) -> bool {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.pool)
        .await
        .expect("a catalog read")
}

/// The rollback refuses while a line exists; with none it folds the quota leases' used bytes into S (so S = 0, which the
/// rollback of task T041-4 requires), drops what it added and restores the four-argument `sagat_quota_add_delta` and the
/// bodies it replaced; migrating again brings everything back.
#[tokio::test]
async fn the_migration_rolls_back_only_without_lines_and_restores_what_it_replaced() {
    bounded::within("up and down", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000).await.expect("a line");
        w.conn.close().await.expect("close");
        assert!(undo_to(&fx.db, BEFORE).await.is_err(), "a stored line refuses the rollback");
        assert!(exists(&fx.db, "SELECT to_regclass('step_log_lines') IS NOT NULL").await, "nothing undone");
        sqlx::query("DELETE FROM step_log_lines").execute(fx.pool()).await.expect("the purge");
        let l = fx.ledger(fx.tenant).await;
        assert_eq!((l.usage(), l.s, l.used), (0, -1_000, 1_000), "S is negative while the quota lease holds the bytes");
        undo_to(&fx.db, BEFORE).await.expect("an empty rollback");
        for (gone, what) in [
            ("SELECT to_regclass('step_log_lines') IS NULL AND to_regclass('step_log_streams') IS NULL", "the tables"),
            ("SELECT to_regclass('step_log_quota_leases') IS NULL", "the quota leases"),
            ("SELECT to_regclass('step_log_quota_refills_refill_seq') IS NULL", "the counters"),
            (
                "SELECT NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'tenant_retained_usage'::regclass \
                   AND attname = 'step_log_leased_bytes' AND NOT attisdropped)",
                "G",
            ),
            (
                "SELECT NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'quota_transaction_deltas'::regclass \
                   AND attname = 'step_log_delta' AND NOT attisdropped)",
                "the step-log delta",
            ),
            ("SELECT NOT EXISTS (SELECT 1 FROM pg_proc WHERE proname LIKE 'sagat_quota_step_log%')", "the functions"),
            (
                "SELECT NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname LIKE '%step_log_reclaim')",
                "the limit-change triggers",
            ),
            (
                "SELECT to_regprocedure('sagat_quota_add_delta(uuid,bigint,bigint,bigint)') IS NOT NULL \
                   AND to_regprocedure('sagat_quota_add_delta(uuid,bigint,bigint,bigint,bigint)') IS NULL",
                "the four-argument delta",
            ),
        ] {
            assert!(exists(&fx.db, gone).await, "the rollback restored {what}");
        }
        let s: i64 = sqlx::query_scalar("SELECT step_log_bytes FROM tenant_retained_usage WHERE tenant_id = $1")
            .bind(fx.tenant)
            .fetch_one(fx.pool())
            .await
            .expect("S");
        assert_eq!(s, 0, "the quota leases' used bytes were folded into S");
        let unknown = sqlx::query("SELECT sagat_quota_measure('{\"level\": \"info\"}', 'fixed_octets', 'level', false)")
            .execute(fx.pool())
            .await
            .map(|_| ());
        assert!(unknown.is_err(), "the restored measurement knows no fixed charge");
        migrate(fx.pool()).await.expect("migrate again");
        let stream = super::stream(fx.pool(), fx.tenant).await;
        let mut w = fx.writer().await;
        fx.charge(&mut w.conn, fx.tenant, stream, 1_000).await.expect("charged again");
        assert_eq!(fx.ledger(fx.tenant).await.usage(), 1_000);
    })
    .await;
}

/// A freshly migrated database's fallback refill waits 200 ms for the usage row (D041-119).
#[tokio::test]
async fn a_freshly_migrated_database_waits_200_milliseconds_for_the_usage_row() {
    let db = support::migrated_db().await;
    let budget: String = sqlx::query_scalar("SELECT sagat_quota_step_log_refill_wait()::text")
        .fetch_one(&db.pool)
        .await
        .expect("the budget");
    assert_eq!(budget, "00:00:00.2");
}

/// Every function the migration creates or replaces: its name and arguments, whether it is `SECURITY DEFINER`, its
/// volatility and whether the runtime role may execute it (ADR-0097 decision 1, data-model.md §4).
const FUNCTIONS: [(&str, bool, char, bool); 22] = [
    (
        "sagat_quota_measure(jsonb,text,text,boolean)",
        false,
        'i',
        false,
    ),
    (
        "sagat_quota_add_delta(uuid,bigint,bigint,bigint,bigint)",
        true,
        'v',
        false,
    ),
    ("sagat_quota_row_delta()", true, 'v', false),
    ("sagat_quota_flush_deltas()", true, 'v', false),
    ("sagat_quota_step_log_allowance(uuid)", true, 's', true),
    ("sagat_quota_step_log_insert_trigger()", true, 'v', false),
    ("sagat_quota_step_log_charge(uuid,bigint)", true, 'v', false),
    (
        "sagat_quota_step_log_refill(uuid,bigint,boolean)",
        true,
        'v',
        false,
    ),
    ("sagat_quota_step_log_refill_wait()", true, 's', false),
    ("sagat_quota_step_log_limit(uuid)", true, 's', false),
    ("sagat_quota_step_log_reclaim()", true, 'v', false),
    ("sagat_quota_step_log_leases_cascade()", true, 'v', false),
    ("sagat_quota_step_log_reserve(uuid,bigint)", true, 'v', true),
    ("sagat_quota_step_log_top_up(uuid)", true, 'v', true),
    ("sagat_quota_step_log_usage(uuid)", true, 's', true),
    ("sagat_quota_step_log_counters()", true, 's', true),
    (
        "sagat_quota_step_log_idle_tenants(integer)",
        true,
        'v',
        true,
    ),
    ("sagat_quota_step_log_reclaim_idle(uuid)", true, 'v', true),
    ("sagat_quota_step_log_top_up_mark()", false, 'i', true),
    ("sagat_quota_step_log_wait_slots()", false, 's', true),
    ("step_log_streams_guard()", false, 'v', false),
    ("step_log_lines_append_only()", false, 'v', false),
];

/// The privilege audit of the migration: each function keeps or has `SECURITY DEFINER` and the pinned `search_path`,
/// none is executable by `PUBLIC`, and the runtime role executes exactly the accessor, the six definer functions and the
/// two invoker helpers; `sagat_quota_step_log_usage` is `STABLE` (`provolatile = 's'`, decision D041-141), the counter
/// sequences and the quota-lease table are revoked from the runtime role, and the tables' grants are data-model.md §4's.
#[tokio::test]
async fn the_quota_lease_functions_and_tables_have_exactly_their_privileges() {
    let db = support::migrated_db().await;
    for (signature, definer, volatility, executable) in FUNCTIONS {
        let (secdef, provolatile, config, public, app): (bool, String, Option<Vec<String>>, bool, bool) = sqlx::query_as(
            "SELECT p.prosecdef, p.provolatile::text, p.proconfig, \
                    EXISTS (SELECT 1 FROM aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a \
                             WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'), \
                    has_function_privilege('sagat_app', p.oid, 'EXECUTE') \
               FROM pg_proc p WHERE p.oid = $1::regprocedure",
        )
        .bind(signature)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{signature}: {e}"));
        assert_eq!(secdef, definer, "{signature}: SECURITY DEFINER");
        assert_eq!(
            provolatile,
            volatility.to_string(),
            "{signature}: volatility"
        );
        assert_eq!(
            config,
            Some(vec!["search_path=pg_catalog, pg_temp".to_owned()]),
            "{signature}: search_path"
        );
        assert!(!public, "{signature}: executable by PUBLIC");
        assert_eq!(app, executable, "{signature}: executable by sagat_app");
    }
    let listed: BTreeSet<String> = FUNCTIONS
        .iter()
        .filter(|(name, ..)| name.starts_with("sagat_quota_step_log"))
        .map(|(name, ..)| (*name).to_owned())
        .collect();
    let present: BTreeSet<String> = sqlx::query_scalar(
        "SELECT replace(oid::regprocedure::text, ' ', '') FROM pg_proc WHERE proname LIKE 'sagat_quota_step_log%'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("the step-log functions")
    .into_iter()
    .collect();
    assert_eq!(
        present, listed,
        "the audit lists every step-log quota function"
    );
    for sequence in [
        "step_log_quota_refills_refill_seq",
        "step_log_quota_refills_reclaim_seq",
        "step_log_quota_refills_idle_seq",
        "step_log_quota_refills_ended_seq",
    ] {
        let any: bool = sqlx::query_scalar(
            "SELECT has_sequence_privilege('sagat_app', $1, 'USAGE, SELECT, UPDATE')",
        )
        .bind(sequence)
        .fetch_one(&db.pool)
        .await
        .expect("sequence privilege");
        assert!(!any, "{sequence} is revoked from sagat_app");
    }
    for (table, privilege, granted) in [
        (
            "step_log_quota_leases",
            "SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER",
            false,
        ),
        ("step_log_lines", "SELECT", true),
        ("step_log_lines", "INSERT", true),
        ("step_log_lines", "DELETE", true),
        ("step_log_lines", "UPDATE, TRUNCATE", false),
        ("step_log_streams", "SELECT", true),
        ("step_log_streams", "INSERT", true),
        ("step_log_streams", "UPDATE", true),
        ("step_log_streams", "DELETE, TRUNCATE", false),
        ("tenant_retained_usage", "UPDATE, INSERT, DELETE", false),
    ] {
        let has: bool = sqlx::query_scalar("SELECT has_table_privilege('sagat_app', $1, $2)")
            .bind(table)
            .bind(privilege)
            .fetch_one(&db.pool)
            .await
            .expect("table privilege");
        assert_eq!(has, granted, "sagat_app {privilege} on {table}");
    }
}

/// The runtime role reaches the quota-lease objects only through the functions it may execute: a direct read or write
/// of the quota leases, of G or of a counter, and a call of the charge, the refill or their helpers, are refused.
#[tokio::test]
async fn the_runtime_role_reaches_the_quota_leases_only_through_its_functions() {
    let fx = Fx::new(1 << 30).await;
    let mut runtime = PgConnection::connect(
        &fx.db
            .url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD),
    )
    .await
    .expect("a runtime connection");
    for statement in [
        "SELECT * FROM step_log_quota_leases",
        "DELETE FROM step_log_quota_leases",
        "UPDATE tenant_retained_usage SET step_log_leased_bytes = 0",
        "UPDATE tenant_retained_usage SET step_log_bytes = 0",
        "SELECT sagat_quota_step_log_charge($1, 128)",
        "SELECT sagat_quota_step_log_refill($1, 128, true)",
        "SELECT sagat_quota_step_log_limit($1)",
        "SELECT sagat_quota_step_log_refill_wait()",
        "SELECT sagat_quota_add_delta($1, 0, 0, 0, -128)",
        "SELECT nextval('step_log_quota_refills_refill_seq')",
        "SELECT last_value FROM step_log_quota_refills_refill_seq",
    ] {
        let query = sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()));
        let query = if statement.contains("$1") {
            query.bind(fx.tenant)
        } else {
            query
        };
        let result = query.execute(&mut runtime).await.map(|_| ());
        assert_eq!(
            refusal(&result).map(|(code, _)| code).as_deref(),
            Some("42501"),
            "{statement}: {result:?}"
        );
    }
    for statement in [
        "SELECT sagat_quota_step_log_usage($1)",
        "SELECT used FROM sagat_quota_step_log_allowance($1)",
        "SELECT reserved FROM sagat_quota_step_log_reserve($1, 128)",
        "SELECT outcome FROM sagat_quota_step_log_top_up($1)",
        "SELECT reclaimed FROM sagat_quota_step_log_reclaim_idle($1)",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .bind(fx.tenant)
            .execute(&mut runtime)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    for statement in [
        "SELECT refills FROM sagat_quota_step_log_counters()",
        "SELECT sagat_quota_step_log_idle_tenants(10)",
        "SELECT sagat_quota_step_log_top_up_mark()",
        "SELECT sagat_quota_step_log_wait_slots()",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&mut runtime)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    let mark: i64 = sqlx::query_scalar("SELECT sagat_quota_step_log_top_up_mark()")
        .fetch_one(&mut runtime)
        .await
        .expect("M");
    assert_eq!(mark, super::MARK);
}

/// The guard trigger refuses, as the runtime role, every change it guards, and lets the counters, a higher sequence and
/// a higher order guard through; a stored line never changes.
#[tokio::test]
async fn the_guard_triggers_refuse_every_change_they_guard_as_the_runtime_role() {
    let fx = Fx::new(1 << 30).await;
    let mut w = fx.writer().await;
    fx.charge_committed(&mut w.conn, 1_000)
        .await
        .expect("a line");
    sqlx::query(
        "UPDATE step_log_streams SET last_sequence = 10, final_at = now(), removed_at = now(), deleted_at = now(), \
           purged_at = now(), last_stored_xid = pg_current_xact_id() WHERE stream_id = $1",
    )
    .bind(fx.stream)
    .execute(fx.pool())
    .await
    .expect("a stream with every mark set");
    let mut runtime = PgConnection::connect(
        &fx.db
            .url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD),
    )
    .await
    .expect("a runtime connection");
    for change in [
        "tenant_id = gen_random_uuid()",
        "public_id = gen_random_uuid()",
        "namespace_id = gen_random_uuid()",
        "run_id = gen_random_uuid()",
        "step_id = gen_random_uuid()",
        "node = 'b'",
        "instance_path_nodes = '{x}', instance_path_indices = '{0}'",
        "attempt = 2",
        "generation = 1",
        "retention_days = 15",
        "source = 'script'",
        "last_sequence = 9",
        "final_at = NULL",
        "deleted_at = NULL",
        "removed_at = NULL, deleted_at = NULL, purged_at = NULL",
        "purged_at = NULL",
        "last_stored_xid = NULL",
        "last_stored_xid = '1'::xid8",
    ] {
        let result = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE step_log_streams SET {change} WHERE stream_id = $1"
        )))
        .bind(fx.stream)
        .execute(&mut runtime)
        .await
        .map(|_| ());
        assert_eq!(
            refusal(&result),
            Some(("23001".into(), Some("step_log_streams_guard".into()))),
            "{change}"
        );
    }
    sqlx::query(
        "UPDATE step_log_streams SET records = records + 1, bytes = bytes + 1, last_sequence = 11, \
           last_stored_xid = pg_current_xact_id() WHERE stream_id = $1",
    )
    .bind(fx.stream)
    .execute(&mut runtime)
    .await
    .expect("the counters, a higher sequence and a later order guard");
    // A stored line never changes: the runtime role holds no `UPDATE` on the lines (data-model.md §4), and the owner,
    // who does, meets the append-only trigger.
    async fn change_a_line(conn: &mut PgConnection, stream: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE step_log_lines SET truncated = true WHERE stream_id = $1")
            .bind(stream)
            .execute(conn)
            .await
            .map(|_| ())
    }
    assert_eq!(
        refusal(&change_a_line(&mut runtime, fx.stream).await),
        Some(("42501".into(), None)),
        "the runtime role may not update a line"
    );
    let mut owner = fx.pool().acquire().await.expect("an owner connection");
    assert_eq!(
        refusal(&change_a_line(&mut owner, fx.stream).await),
        Some(("23001".into(), Some("step_log_lines_append_only".into()))),
        "the owner meets the append-only trigger"
    );
}

/// Architecture test of ADR-0084 decision 1: no foreign key leads from the log tables to `runs` or `steps`; the only
/// one is the lines' to their stream.
#[tokio::test]
async fn no_foreign_key_leads_from_the_log_tables_to_runs_or_steps() {
    let db = support::migrated_db().await;
    let keys: Vec<(String, String)> = sqlx::query_as(
        "SELECT conrelid::regclass::text, confrelid::regclass::text FROM pg_constraint \
          WHERE contype = 'f' AND conrelid IN ('step_log_lines'::regclass, 'step_log_streams'::regclass) ORDER BY 1, 2",
    )
    .fetch_all(&db.pool)
    .await
    .expect("the foreign keys");
    assert_eq!(
        keys,
        vec![("step_log_lines".to_owned(), "step_log_streams".to_owned())]
    );
}

/// Decision D041-143: the quota leases have no foreign key (whose check would lock the usage row on a second charge of a
/// transaction), and a tenant's deletion still removes them, through the usage row's cascade trigger.
#[tokio::test]
async fn the_quota_leases_have_no_foreign_key_and_go_with_their_tenant() {
    let fx = Fx::new(1 << 30).await;
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE contype = 'f' AND conrelid = 'step_log_quota_leases'::regclass",
    )
    .fetch_one(fx.pool())
    .await
    .expect("the foreign keys");
    assert_eq!(keys, 0, "no foreign key leads from the quota leases");
    let mut w = fx.writer().await;
    fx.charge_committed(&mut w.conn, 1_000)
        .await
        .expect("a quota lease");
    let (other, other_stream) = fx.another_tenant(1 << 30).await;
    fx.charge(&mut w.conn, other, other_stream, 1_000)
        .await
        .expect("the other tenant's quota lease");
    sqlx::query("DELETE FROM step_log_lines WHERE tenant_id = $1")
        .bind(fx.tenant)
        .execute(fx.pool())
        .await
        .expect("the tenant's lines released");
    sqlx::query("DELETE FROM step_log_streams WHERE tenant_id = $1")
        .bind(fx.tenant)
        .execute(fx.pool())
        .await
        .expect("the tenant's streams");
    sqlx::query("DELETE FROM tenant_retained_usage WHERE tenant_id = $1")
        .bind(fx.tenant)
        .execute(fx.pool())
        .await
        .expect("the tenant's usage row deleted, as its tenant's deletion cascades to it");
    assert_eq!(
        lease_of(fx.pool(), fx.tenant, w.pid).await,
        None,
        "the tenant's quota lease went with it"
    );
    assert_eq!(
        lease_of(fx.pool(), other, w.pid)
            .await
            .map(|(_, used)| used),
        Some(1_000),
        "another tenant's quota lease is kept"
    );
}

/// The production code (outside `#[cfg(test)]`) of a source file: everything before its first `#[cfg(test)]` item.
fn production(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.split("#[cfg(test)]")
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn sources(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap_or_else(|e| panic!("{}: {e}", root.display())) {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Architecture test of ADR-0084 decisions 1 and 2: the hot path's production code never names the log tables or the
/// step-log transaction port (task T041-36 restates it so that a report reaches them only through `StepLogCarryTx`).
#[test]
fn the_hot_path_never_names_the_step_log_tables_or_their_transaction_port() {
    let repository = repository();
    let mut files = Vec::new();
    for dir in [
        "crates/application/src/worker",
        "crates/infra-postgres/src/execution",
    ] {
        sources(&repository.join(dir), &mut files);
    }
    for file in [
        "crates/application/src/worker.rs",
        "crates/infra-postgres/src/execution.rs",
    ] {
        files.push(repository.join(file));
    }
    for entry in std::fs::read_dir(repository.join("crates/application/src"))
        .expect("the application sources")
    {
        let path = entry.expect("an entry").path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("execution"))
        {
            if path.is_dir() {
                sources(&path, &mut files);
            } else {
                files.push(path);
            }
        }
    }
    assert!(
        files.len() > 4,
        "the hot path's files were found: {files:?}"
    );
    let names = Regex::new(r"\b(step_log_lines|step_log_streams|StepLogTx)\b").expect("a pattern");
    for file in files {
        let code = production(&file);
        assert!(
            !names.is_match(&code),
            "{} names the step-log storage",
            file.display()
        );
    }
}

/// The step-log migrations' up files, each split into its function bodies (name, body) and the rest.
fn migration_parts() -> Vec<(String, Vec<(String, String)>, String)> {
    let dir = repository().join("crates/infra-postgres/migrations");
    let mut parts = Vec::new();
    let function =
        Regex::new(r"(?s)CREATE (?:OR REPLACE )?FUNCTION (?:public\.)?(\w+)\(.*?AS \$\$(.*?)\$\$")
            .expect("a pattern");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the migrations")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.to_str().is_some_and(|p| p.ends_with(".up.sql")))
        .collect();
    files.sort();
    for path in files {
        let sql = std::fs::read_to_string(&path).expect("a migration");
        let mut bodies = Vec::new();
        let mut rest = String::new();
        let mut at = 0;
        for capture in function.captures_iter(&sql) {
            let whole = capture.get(0).expect("the match");
            rest.push_str(&sql[at..whole.start()]);
            at = whole.end();
            bodies.push((capture[1].to_owned(), capture[2].to_owned()));
        }
        rest.push_str(&sql[at..]);
        parts.push((
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned(),
            bodies,
            rest,
        ));
    }
    parts
}

/// Source-text check of the migrations (ADR-0097): only the top-up, the refill, the release branch of the finalizer, the
/// idle reclaim and the limit-change trigger lock or write `tenant_retained_usage` among the functions of the step-log
/// migrations; the charge, the reservation and the readers never do.
#[test]
fn only_the_top_up_the_refill_the_release_the_idle_reclaim_and_the_limit_change_lock_the_usage_row()
{
    let allowed: BTreeSet<&str> = [
        "sagat_quota_step_log_top_up",
        "sagat_quota_step_log_refill",
        "sagat_quota_flush_deltas",
        "sagat_quota_step_log_reclaim_idle",
        "sagat_quota_step_log_reclaim",
    ]
    .into_iter()
    .collect();
    let mut locking = BTreeSet::new();
    for (file, bodies, _) in migration_parts() {
        if !file.contains("_step_log") {
            continue;
        }
        for (name, body) in bodies {
            for statement in body.split(';') {
                let statement: String = statement
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("--"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let usage = statement.contains("public.tenant_retained_usage");
                let locks = statement.contains("FOR UPDATE")
                    || statement.contains("UPDATE public.tenant_retained_usage")
                    || statement.contains("INSERT INTO public.tenant_retained_usage");
                if usage && locks {
                    locking.insert(name.clone());
                }
            }
        }
    }
    assert_eq!(
        locking,
        allowed.iter().map(|n| (*n).to_owned()).collect(),
        "the functions that lock the usage row"
    );
}

/// Source-text check (data-model.md §4): no Rust source under `crates/` or `apps/` and no SQL of an up migration outside
/// the functions of data-model.md §3b and the finalizer reads `step_log_bytes` or `step_log_leased_bytes` but through
/// `sagat_quota_step_log_usage`; outside a function, a migration names them only to add or validate the columns.
#[test]
fn no_code_reads_the_folded_usage_or_the_leased_room_but_through_the_usage_function() {
    let columns = Regex::new(r"\bstep_log_(leased_)?bytes\b").expect("a pattern");
    let mut files = Vec::new();
    sources(&repository().join("crates"), &mut files);
    sources(&repository().join("apps"), &mut files);
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a source");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            assert!(
                !columns.is_match(code),
                "{}:{} reads a step-log usage column",
                file.display(),
                n + 1
            );
        }
    }
    let allowed: BTreeSet<&str> = [
        "sagat_quota_step_log_usage",
        "sagat_quota_step_log_charge",
        "sagat_quota_step_log_refill",
        "sagat_quota_step_log_reserve",
        "sagat_quota_step_log_top_up",
        "sagat_quota_step_log_reclaim_idle",
        "sagat_quota_step_log_reclaim",
        "sagat_quota_flush_deltas",
    ]
    .into_iter()
    .collect();
    let ddl = Regex::new(r"^\s*(ALTER TABLE tenant_retained_usage (ADD|VALIDATE)|CHECK \(step_log_leased_bytes >= 0\))").expect("a pattern");
    for (file, bodies, rest) in migration_parts() {
        for (name, body) in bodies {
            if columns.is_match(&body) {
                assert!(
                    allowed.contains(name.as_str()),
                    "{file}: {name} reads a step-log usage column"
                );
            }
        }
        for line in rest.lines() {
            let code = line.split("--").next().unwrap_or_default();
            if columns.is_match(code) {
                assert!(ddl.is_match(code), "{file}: {line}");
            }
        }
    }
}

/// Two pools built for two instances show two names in `pg_stat_activity`, and so do two in-process test replicas; a
/// store over a pool of 20 computes *Q* = 5 (2 over pools of 10 and 8), and a transaction that sends it reads it back.
#[tokio::test]
async fn each_replica_names_its_connections_and_computes_its_wait_slots_from_its_pool() {
    let w = crate::support::engine::world().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let mut names = Vec::new();
    for instance in [a, b] {
        let mut config = PoolConfig::new(w.db.url.clone()).named(PoolName::Instance(instance));
        config.max_connections = 1;
        let pool = connect(&config).await.expect("a pool");
        let (name, seen): (String, String) = sqlx::query_as(
            "SELECT current_setting('application_name'), \
                    (SELECT application_name FROM pg_stat_activity WHERE pid = pg_backend_pid())",
        )
        .fetch_one(&pool)
        .await
        .expect("the pool's name");
        assert_eq!(name, seen);
        assert_eq!(name, format!("sagat-octopus-orchestrator/{instance}"));
        names.push(name);
        pool.close().await;
    }
    assert_ne!(names[0], names[1]);
    let (first, second) = (w.replica().await, w.replica().await);
    let mut replicas = BTreeSet::new();
    for replica in [&first, &second] {
        let name: String = sqlx::query_scalar("SELECT current_setting('application_name')")
            .fetch_one(&replica.execution_pool)
            .await
            .expect("the replica's name");
        assert!(name.starts_with("sagat-octopus-orchestrator/"), "{name}");
        replicas.insert(name);
    }
    assert_eq!(replicas.len(), 2, "two test replicas, two names");
    for (size, slots) in [(20, 5), (10, 2), (8, 2)] {
        let mut config = PoolConfig::new(w.db.url.clone());
        config.max_connections = size;
        let pool = connect(&config).await.expect("a pool");
        let q = step_log_wait_slots(&pool);
        assert_eq!(q, slots, "a pool of {size}");
        let mut tx = pool.begin().await.expect("begin");
        sqlx::query("SELECT set_config('sagat.step_log_wait_slots', $1, true)")
            .bind(q.to_string())
            .execute(&mut *tx)
            .await
            .expect("the store's setting");
        let read: i32 = sqlx::query_scalar("SELECT sagat_quota_step_log_wait_slots()")
            .fetch_one(&mut *tx)
            .await
            .expect("Q");
        assert_eq!(i64::from(read), i64::from(q));
        tx.rollback().await.expect("rollback");
        pool.close().await;
    }
}
