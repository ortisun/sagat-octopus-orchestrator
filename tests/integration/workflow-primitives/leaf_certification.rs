//! Dormant schema fixtures only: these values do not prove a successful source execution.

use crate::support::engine::{World, activity, document, end_node, world};
use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

const BEFORE_LEAF: i64 = 20_261_210_000_000;
const SOURCES: [&str; 8] = [
    "runs",
    "steps",
    "iteration_scopes",
    "collection_scope_items",
    "collection_iterations",
    "iteration_checkpoints",
    "scope_sagas",
    "scope_saga_entries",
];

async fn fixture() -> (World, Value) {
    let w = world().await;
    let mut node = activity("work");
    node["next"] = json!("done");
    let (workflow, version) = w
        .publish(document(
            "leaf-storage",
            "work",
            vec![node, end_node("done")],
        ))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let fact = json!({
        "tenant_id":w.admin.tenant_id().to_string(), "namespace_id":w.namespace.id().to_string(),
        "run_id":run.to_string(), "owner_node":"items", "iteration_index":0,
        "profile":"ORDINARY_LEAF_V1", "definition_version_id":version.to_string(),
        "scope_id":Uuid::new_v4().to_string(), "body_node":"line", "return_node":"returned",
        "admitted_sequence":2, "returned_sequence":8, "checkpoint_sequence":9,
        "checkpoint_result_digest":format!("\\x{}", "11".repeat(32)), "finished_sequence":12,
        "aggregate_result_digest":format!("\\x{}", "22".repeat(32)), "owner_routed_sequence":13,
        "certified_run_revision":20, "certified_at":"2026-10-18T12:00:00Z",
        "audit_id":Uuid::new_v4().to_string()
    });
    (w, fact)
}

async fn insert(conn: &mut PgConnection, fact: &Value) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO leaf_collection_success_certifications SELECT * FROM jsonb_populate_record(NULL::leaf_collection_success_certifications,$1)")
        .bind(fact).execute(conn).await.map(|_| ())
}

fn error_is(error: sqlx::Error, code: &str) {
    assert_eq!(crate::support::db_error(error).0, code);
}

#[tokio::test]
async fn dormant_snapshot_constraints_and_namespace_binding_are_real() {
    let (w, fact) = fixture().await;
    let mut conn = w.db.pool.acquire().await.expect("owner schema connection");
    insert(&mut conn, &fact)
        .await
        .expect("owner fixture is structurally valid, not source proof");
    let row = sqlx::query("SELECT profile,checkpoint_result_digest,aggregate_result_digest FROM leaf_collection_success_certifications")
        .fetch_one(&mut *conn).await.expect("canonical metadata");
    assert_eq!(row.get::<String, _>("profile"), "ORDINARY_LEAF_V1");
    assert_ne!(
        row.get::<Vec<u8>, _>("checkpoint_result_digest"),
        row.get::<Vec<u8>, _>("aggregate_result_digest")
    );
    let mut duplicate = fact.clone();
    duplicate["audit_id"] = json!(Uuid::new_v4().to_string());
    error_is(
        insert(&mut conn, &duplicate)
            .await
            .expect_err("canonical duplicate"),
        "23505",
    );
    let mut other = fact.clone();
    other["iteration_index"] = json!(1);
    error_is(
        insert(&mut conn, &other)
            .await
            .expect_err("audit cannot identify two facts"),
        "23505",
    );
    let same_tenant_namespace = Uuid::new_v4();
    sqlx::query("INSERT INTO namespaces(id,tenant_id,workspace_id,slug,environment,created_at) SELECT $1,tenant_id,workspace_id,'leaf-other','dev',now() FROM namespaces WHERE id=$2")
        .bind(same_tenant_namespace).bind(w.namespace.id().as_uuid()).execute(&mut *conn).await.expect("second namespace within original tenant");
    for (field, value, code) in [
        (
            "namespace_id",
            json!(same_tenant_namespace.to_string()),
            "23503",
        ),
        (
            "namespace_id",
            json!(w.other_namespace.id().to_string()),
            "23503",
        ),
        ("run_id", json!(Uuid::new_v4().to_string()), "23503"),
        ("profile", json!("ORDINARY_LEAF_V2"), "23514"),
        ("owner_node", json!("bad/owner"), "23514"),
        ("body_node", json!("x".repeat(65)), "23514"),
        ("return_node", json!(""), "23514"),
        ("iteration_index", json!(-1), "23514"),
        ("iteration_index", json!(10000), "23514"),
        ("checkpoint_sequence", json!(10), "23514"),
        ("returned_sequence", json!(i64::MAX), "23514"),
        ("admitted_sequence", json!(0), "23514"),
        ("finished_sequence", json!(9), "23514"),
        ("owner_routed_sequence", json!(14), "23514"),
        ("certified_run_revision", json!(12), "23514"),
        ("certified_at", json!("infinity"), "23514"),
        ("checkpoint_result_digest", json!("\\x11"), "23514"),
        ("aggregate_result_digest", json!("\\x22"), "23514"),
    ] {
        let mut invalid = fact.clone();
        invalid["owner_node"] = json!("probe");
        invalid["audit_id"] = json!(Uuid::new_v4().to_string());
        invalid[field] = value;
        error_is(insert(&mut conn, &invalid).await.expect_err(field), code);
    }
}

#[tokio::test]
async fn dormant_runtime_cannot_create_or_rewrite_certifications() {
    let (w, fact) = fixture().await;
    insert(
        &mut w.db.pool.acquire().await.expect("fixture owner"),
        &fact,
    )
    .await
    .expect("owner fixture");
    let runtime = w.db.runtime_pool().await;
    let row:(String,bool,bool,bool,bool,bool)=sqlx::query_as("SELECT current_user::text,has_table_privilege(current_user,'leaf_collection_success_certifications','SELECT'),has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'),has_any_column_privilege(current_user,'leaf_collection_success_certifications','UPDATE'),has_table_privilege(current_user,'leaf_collection_success_certifications','DELETE'),has_table_privilege(current_user,'leaf_collection_success_certifications','TRUNCATE')")
        .fetch_one(&runtime).await.expect("actual runtime privileges");
    assert_eq!(row, ("sagat_app".into(), true, false, false, false, false));
    let mut conn = runtime.acquire().await.expect("runtime connection");
    error_is(
        insert(&mut conn, &fact)
            .await
            .expect_err("no dormant writer"),
        "42501",
    );
    for sql in [
        "UPDATE leaf_collection_success_certifications SET profile=profile",
        "DELETE FROM leaf_collection_success_certifications",
        "TRUNCATE leaf_collection_success_certifications",
    ] {
        error_is(
            sqlx::query(sql)
                .execute(&mut *conn)
                .await
                .expect_err("no rewrite"),
            "42501",
        );
    }
}

#[tokio::test]
async fn retained_fact_and_original_run_survive_owner_replication_role_bypass() {
    let (w, fact) = fixture().await;
    insert(
        &mut w.db.pool.acquire().await.expect("fixture owner"),
        &fact,
    )
    .await
    .expect("owner fixture");
    for sql in [
        "UPDATE leaf_collection_success_certifications SET profile=profile",
        "DELETE FROM leaf_collection_success_certifications",
        "TRUNCATE leaf_collection_success_certifications",
        "DELETE FROM runs",
        "DELETE FROM steps",
        "UPDATE runs SET id=gen_random_uuid()",
    ] {
        let mut tx = w.db.pool.begin().await.expect("privileged replica probe");
        sqlx::query("SET LOCAL session_replication_role=replica")
            .execute(&mut *tx)
            .await
            .expect("actual FK bypass mode");
        error_is(
            sqlx::query(sql)
                .execute(&mut *tx)
                .await
                .expect_err("ALWAYS guard retains identity"),
            "23001",
        );
        tx.rollback().await.expect("probe rollback");
    }
    seed_retained_sources(&w, &fact).await;
    for table in SOURCES {
        let before = source_count(&w, table).await;
        assert!(before > 0, "{table} must be populated before DELETE probe");
        let mut tx =
            w.db.pool
                .begin()
                .await
                .expect("populated source DELETE probe");
        sqlx::query("SET LOCAL session_replication_role=replica")
            .execute(&mut *tx)
            .await
            .expect("replica FK bypass");
        let error = sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(&mut *tx)
            .await
            .expect_err("actual source row cannot be deleted");
        assert_eq!(
            error
                .as_database_error()
                .expect("database rejection")
                .message(),
            format!("{table} is append-only (DELETE attempted)")
        );
        error_is(error, "23001");
        tx.rollback().await.expect("source probe rollback");
        assert_eq!(
            source_count(&w, table).await,
            before,
            "{table} rows retained"
        );
        let mut tx =
            w.db.pool
                .begin()
                .await
                .expect("retained source TRUNCATE probe");
        sqlx::query("SET LOCAL session_replication_role=replica")
            .execute(&mut *tx)
            .await
            .expect("replica source probe");
        error_is(
            sqlx::query(sqlx::AssertSqlSafe(format!("TRUNCATE {table} CASCADE")))
                .execute(&mut *tx)
                .await
                .expect_err("source cannot be truncated"),
            "23001",
        );
        tx.rollback().await.expect("truncate probe rollback");
        // CASCADE rejection can originate at a descendant: independently bind both exact guards
        // to the shared rejection function and their actual trigger kinds in PostgreSQL's catalog.
        let guards:Vec<(String,i16,String)>=sqlx::query_as("SELECT tgname::text,tgtype,tgenabled::text FROM pg_trigger WHERE tgrelid=$1::regclass AND tgname IN ('leaf_source_no_delete','leaf_source_no_truncate') AND tgfoid='execution_reject_mutation()'::regprocedure ORDER BY tgname")
            .bind(table).fetch_all(&w.db.pool).await.expect("exact source retention guards");
        assert_eq!(
            guards,
            vec![
                ("leaf_source_no_delete".into(), 11, "A".into()),
                ("leaf_source_no_truncate".into(), 34, "A".into())
            ],
            "{table}"
        );
    }
    // Deliberately remove only the parent DELETE guard within a rollback-only owner fault probe.
    // This reaches an actual ordinary-mode FK cascade, which a retained child must still reject.
    let before = source_count(&w, "iteration_checkpoints").await;
    let mut tx = w.db.pool.begin().await.expect("cascade isolation probe");
    sqlx::query("ALTER TABLE iteration_scopes DISABLE TRIGGER leaf_source_no_delete")
        .execute(&mut *tx)
        .await
        .expect("test-only parent guard bypass");
    let error = sqlx::query("DELETE FROM iteration_scopes")
        .execute(&mut *tx)
        .await
        .expect_err("actual FK cascade cannot erase retained children");
    let message = error
        .as_database_error()
        .expect("cascade rejection")
        .message();
    assert!(
        ["collection_scope_items", "collection_iterations"]
            .into_iter()
            .any(|table| message == format!("{table} is append-only (DELETE attempted)")),
        "{message}"
    );
    error_is(error, "23001");
    tx.rollback().await.expect("restore parent guard");
    assert_eq!(source_count(&w, "iteration_checkpoints").await, before);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM leaf_collection_success_certifications")
            .fetch_one(&w.db.pool)
            .await
            .expect("fact retained");
    assert_eq!(rows, 1);
}

async fn source_count(w: &World, table: &str) -> i64 {
    assert!(SOURCES.contains(&table));
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(&w.db.pool)
        .await
        .expect("retained source count")
}

// Relational fixtures are intentionally not a valid execution/source certificate.
async fn seed_retained_sources(w: &World, fact: &Value) {
    let run = Uuid::parse_str(fact["run_id"].as_str().expect("run UUID")).expect("UUID");
    let digest: Vec<u8> = sqlx::query_scalar("SELECT input_digest FROM runs WHERE id=$1")
        .bind(run)
        .fetch_one(&w.db.pool)
        .await
        .expect("existing payload reference");
    let hex = format!(
        "\\x{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let mut base = json!({"tenant_id":fact["tenant_id"],"run_id":fact["run_id"],"scope_id":fact["scope_id"],"created_at":"2026-10-18T12:00:00Z","updated_at":"2026-10-18T12:00:00Z"});
    let rows = [
        (
            "iteration_scopes",
            json!({"namespace_id":fact["namespace_id"],"owner_path_nodes":[],"owner_path_indices":[],"owner_node":"items","kind":"COLLECTION","lifecycle":"OPEN","snapshot_digest":hex,"item_count":1,"max_items":1,"max_concurrency":1,"next_admission_index":1,"admitted_count":1,"completed_count":0,"active_count":1}),
        ),
        (
            "collection_scope_items",
            json!({"iteration_index":0,"item_digest":hex}),
        ),
        (
            "collection_iterations",
            json!({"iteration_index":0,"entry_path_nodes":["items"],"entry_path_indices":[0],"entry_node":"work","entry_instance_id":Uuid::new_v4().to_string(),"activity":"act.work","item_digest":hex,"lifecycle":"ADMITTED","slot_held":true,"reserved_instances":2,"remaining_reservations":1,"admitted_sequence":2,"outbox_id":Uuid::new_v4().to_string()}),
        ),
        (
            "iteration_checkpoints",
            json!({"iteration_index":0,"return_instance_id":Uuid::new_v4().to_string(),"result_digest":hex,"journal_sequence":9,"outbox_id":Uuid::new_v4().to_string()}),
        ),
        (
            "scope_sagas",
            json!({"owner_path_nodes":[],"owner_path_indices":[],"owner_node":"items","phase":"COMPENSATING","cause_code":"failure","during_stop":false,"planned_sequence":1}),
        ),
        (
            "scope_saga_entries",
            json!({"owner_path_nodes":[],"owner_path_indices":[],"owner_node":"items","iteration_index":0,"node":"work","kind":"IRREVERSIBLE","after_iterations":[],"after_nodes":[]}),
        ),
    ];
    let mut tx =
        w.db.pool
            .begin()
            .await
            .expect("retained-source owner fixtures");
    for (table, fields) in rows {
        assert!(SOURCES.contains(&table));
        let old = base.clone();
        base.as_object_mut()
            .expect("base fields")
            .extend(fields.as_object().expect("source fields").clone());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} SELECT * FROM jsonb_populate_record(NULL::{table},$1)"
        )))
        .bind(&base)
        .execute(&mut *tx)
        .await
        .expect(table);
        base = old;
    }
    tx.commit().await.expect("relational fixtures commit");
}

#[tokio::test]
async fn downgrade_waits_on_actual_own_run_prefix_before_retained_fact_check() {
    let (w, fact) = fixture().await;
    let mut writer = w.db.pool.begin().await.expect("uncommitted owner fixture");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .expect("writer backend");
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(Uuid::parse_str(fact["run_id"].as_str().expect("run UUID")).expect("UUID"))
        .fetch_one(&mut *writer)
        .await
        .expect("actual owning run guard");
    let pool = w.db.pool.clone();
    let down = tokio::spawn(async move { MIGRATOR.undo(&pool, BEFORE_LEAF).await });
    crate::support::eventually("down waits on own run before certificate relation", || async {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%LOCK TABLE runs%')")
            .bind(pid).fetch_one(&w.db.pool).await.expect("actual migration wait").then_some(())
    }).await;
    insert(&mut writer, &fact)
        .await
        .expect("guarded fixture can insert before down");
    writer.commit().await.expect("fixture commits first");
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), down)
        .await
        .expect("no lock cycle")
        .expect("down task")
        .expect_err("retained fact refuses down");
    match error {
        sqlx::migrate::MigrateError::ExecuteMigration(error, _) => error_is(error, "55000"),
        other => panic!("expected retained refusal, got {other:?}"),
    }
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM leaf_collection_success_certifications")
            .fetch_one(&w.db.pool)
            .await
            .expect("committed fact survives");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn empty_schema_reverses_but_any_retained_fact_refuses_down() {
    let w = world().await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE_LEAF)
        .await
        .expect("empty leaf storage reverses");
    let absent: bool =
        sqlx::query_scalar("SELECT to_regclass('leaf_collection_success_certifications') IS NULL")
            .fetch_one(&w.db.pool)
            .await
            .expect("removed table");
    assert!(absent);
    migrate(&w.db.pool).await.expect("reapply dormant storage");
    let (w, fact) = fixture().await;
    insert(&mut w.db.pool.acquire().await.expect("owner"), &fact)
        .await
        .expect("retained fact");
    let error = MIGRATOR
        .undo(&w.db.pool, BEFORE_LEAF)
        .await
        .expect_err("nonempty retained snapshot blocks down");
    match error {
        sqlx::migrate::MigrateError::ExecuteMigration(error, _) => error_is(error, "55000"),
        other => panic!("expected retained schema refusal, got {other:?}"),
    }
    let max: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&w.db.pool)
        .await
        .expect("ledger retained");
    assert_eq!(max, 20_261_211_000_000);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM leaf_collection_success_certifications")
            .fetch_one(&w.db.pool)
            .await
            .expect("fact retained");
    assert_eq!(count, 1);
}
