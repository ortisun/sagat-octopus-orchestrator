//! ADR-0027 shared-core proofs: generated retained-byte accounting, durable rate state and the
//! private accounting boundary. Consumer-specific admission remains covered by its feature suite.

use std::collections::BTreeSet;

use sagat_application::error::StoreError;
use sagat_application::identity::IdentityStore;
use sagat_application::quotas::{NamespaceApiAdmission, RateDecision};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{Slug, Workspace, WorkspaceId};
use sagat_domain::quota::QuotaResource;
use sagat_infra_postgres::{MIGRATOR, PgIdentityStore, PgRunStore, PoolConfig, connect, migrate};
use serde_json::Value;
use sqlx::{Connection, PgConnection};

use crate::support::engine::{activity, ctx, document, world};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, fresh_db, now};

async fn runtime_store(w: &crate::support::engine::World) -> PgRunStore {
    let pool = connect(&PoolConfig::new(
        w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD),
    ))
    .await
    .expect("runtime pool");
    PgRunStore::new(pool, w.cache.clone())
}

async fn usage(pool: &sqlx::PgPool, tenant: uuid::Uuid) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT content_bytes, content_reserved_bytes, log_bytes \
         FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("usage")
}

#[tokio::test]
async fn migration_backfills_preexisting_rows_exactly() {
    const BEFORE_DURABLE_QUOTAS: i64 = 20_261_023_000_000;
    let db = fresh_db().await;
    MIGRATOR.run(&db.pool).await.expect("initial migration");
    MIGRATOR
        .undo(&db.pool, BEFORE_DURABLE_QUOTAS)
        .await
        .expect("return to pre-quota schema");
    let tenant = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tenants (id, slug, created_at) VALUES ($1, 'legacy', clock_timestamp())",
    )
    .bind(tenant)
    .execute(&db.pool)
    .await
    .expect("legacy tenant");
    let workspace = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) \
         VALUES ($1, $2, 'abc', clock_timestamp())",
    )
    .bind(workspace)
    .bind(tenant)
    .execute(&db.pool)
    .await
    .expect("legacy workspace");
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) \
         VALUES ($1, 'éx', 'viewer', clock_timestamp())",
    )
    .bind(tenant)
    .execute(&db.pool)
    .await
    .expect("legacy membership");
    let namespace = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES ($1, $2, $3, 'ns1', 'dev', clock_timestamp())",
    )
    .bind(namespace)
    .bind(tenant)
    .bind(workspace)
    .execute(&db.pool)
    .await
    .expect("legacy namespace");
    let connector = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_connectors \
         (id, tenant_id, namespace_id, slug, kind, settings, rate_per_second, created_by) \
         VALUES ($1, $2, $3, 'con', 'NATS', 'null'::jsonb, 1, 'legacy')",
    )
    .bind(connector)
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("legacy JSON null");
    sqlx::query(
        "INSERT INTO egress_policies \
         (tenant_id, namespace_id, rules, ports, allow_http, updated_by) \
         VALUES ($1, $2, ARRAY['a']::text[], ARRAY[80]::integer[], true, 'legacy')",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("legacy arrays");
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) \
         VALUES ($1, decode(repeat('00', 32), 'hex'), decode('c3a9', 'hex'))",
    )
    .bind(tenant)
    .execute(&db.pool)
    .await
    .expect("legacy bytes");

    migrate(&db.pool).await.expect("durable quota migration");
    // UTF-8 text is measured in bytes, JSON null is canonical `null`, arrays use canonical JSON,
    // and bytea uses its raw size: 3+3+3+(3+4)+(5+4)+2 = 27.
    assert_eq!(usage(&db.pool, tenant).await, (27, 0, 0));

    sqlx::query("UPDATE event_connectors SET settings = '{}'::jsonb WHERE id = $1")
        .bind(connector)
        .execute(&db.pool)
        .await
        .expect("replace JSON null");
    assert_eq!(usage(&db.pool, tenant).await, (25, 0, 0));
    sqlx::query(
        "UPDATE egress_policies SET rules = ARRAY[]::text[], ports = ARRAY[]::integer[] \
         WHERE tenant_id = $1 AND namespace_id = $2",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&db.pool)
    .await
    .expect("replace arrays");
    assert_eq!(usage(&db.pool, tenant).await, (20, 0, 0));
    sqlx::query("DELETE FROM event_connectors WHERE id = $1")
        .bind(connector)
        .execute(&db.pool)
        .await
        .expect("delete JSON row");
    assert_eq!(usage(&db.pool, tenant).await, (15, 0, 0));
    sqlx::query("DELETE FROM egress_policies WHERE tenant_id = $1 AND namespace_id = $2")
        .bind(tenant)
        .bind(namespace)
        .execute(&db.pool)
        .await
        .expect("delete arrays");
    assert_eq!(usage(&db.pool, tenant).await, (11, 0, 0));
}

#[tokio::test]
async fn migration_backfills_legacy_finalized_blobs_exactly_and_unknown_uploads_conservatively() {
    const BEFORE_DURABLE_QUOTAS: i64 = 20_261_023_000_000;
    const LEGACY_UPLOAD_RESERVATION: i64 = 5 * 1024 * 1024 * 1024;
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "legacy-blob-backfill",
            "store",
            vec![activity("store")],
        ))
        .await;
    w.start(workflow, &serde_json::json!({})).await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE_DURABLE_QUOTAS)
        .await
        .expect("return to pre-quota schema");
    let (run_id, step_id, attempt, generation): (uuid::Uuid, uuid::Uuid, i32, i64) =
        sqlx::query_as(
            "SELECT run_id, step_id, attempt, generation FROM steps \
             WHERE tenant_id = $1 AND namespace_id = $2 LIMIT 1",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(w.namespace.id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("seed step");
    let uploading = uuid::Uuid::now_v7();
    let uploaded = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO blobs \
         (tenant_id, namespace_id, id, state, content_type, created_by, run_id, step_id, \
          attempt, generation, purpose) \
         VALUES ($1, $2, $3, 'UPLOADING', 'application/octet-stream', 'legacy', $5, $6, $7, $8, \
                 'STEP_OUTPUT'), \
                ($1, $2, $4, 'UPLOADING', 'application/octet-stream', 'legacy', $5, $6, $7, $8, \
                 'STEP_OUTPUT')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(uploading)
    .bind(uploaded)
    .bind(run_id)
    .bind(step_id)
    .bind(attempt)
    .bind(generation)
    .execute(&w.db.pool)
    .await
    .expect("legacy blobs");
    sqlx::query(
        "UPDATE blobs SET state = 'UPLOADED', size_bytes = 7, \
         sha256 = decode(repeat('ab', 32), 'hex'), uploaded_at = clock_timestamp() WHERE id = $1",
    )
    .bind(uploaded)
    .execute(&w.db.pool)
    .await
    .expect("legacy finalized measurement");

    migrate(&w.db.pool).await.expect("durable quota migration");
    let reservations: Vec<(uuid::Uuid, i64)> =
        sqlx::query_as("SELECT id, reservation_bytes FROM blobs WHERE id IN ($1, $2) ORDER BY id")
            .bind(uploading)
            .bind(uploaded)
            .fetch_all(&w.db.pool)
            .await
            .expect("persisted reservations");
    assert!(reservations.contains(&(uploading, LEGACY_UPLOAD_RESERVATION)));
    assert!(reservations.contains(&(uploaded, 0)));
    let before_delete = usage(&w.db.pool, w.admin.tenant_id().as_uuid()).await;
    assert_eq!(before_delete.1, LEGACY_UPLOAD_RESERVATION);
    sqlx::query("DELETE FROM blobs WHERE id = $1")
        .bind(uploaded)
        .execute(&w.db.pool)
        .await
        .expect("delete finalized legacy blob");
    assert_eq!(
        usage(&w.db.pool, w.admin.tenant_id().as_uuid()).await.0,
        before_delete.0 - 7,
        "the exact measured legacy bytes were released"
    );
    sqlx::query("DELETE FROM blobs WHERE id = $1")
        .bind(uploading)
        .execute(&w.db.pool)
        .await
        .expect("delete legacy unknown upload");
    assert_eq!(usage(&w.db.pool, w.admin.tenant_id().as_uuid()).await.1, 0);
}

#[tokio::test]
async fn manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers()
{
    let w = world().await;
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../crates/infra-postgres/storage-classification.json"
    ))
    .expect("classification manifest");
    let entries = manifest["columns"].as_array().expect("columns array");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests crate is in the repository");
    for entry in entries {
        assert!(
            matches!(
                entry["class"].as_str(),
                Some("content" | "log" | "protected_system")
            ),
            "unknown classification: {entry}"
        );
        let evidence = entry["measurement"]["evidence"]
            .as_str()
            .unwrap_or_default();
        if entry["class"].as_str() == Some("protected_system") {
            assert!(
                !evidence.is_empty() && !evidence.contains("closed schema, bounded"),
                "protected field lacks concrete evidence: {entry}"
            );
        }
        for owner in entry["owner_paths"].as_array().expect("owner paths") {
            assert!(
                repository
                    .join(owner.as_str().expect("owner path"))
                    .is_file(),
                "classification cites a missing owner: {entry}"
            );
        }
    }
    let classified: BTreeSet<_> = entries
        .iter()
        .map(|entry| {
            (
                entry["table"].as_str().expect("table").to_owned(),
                entry["column"].as_str().expect("column").to_owned(),
                entry["sql_type"].as_str().expect("type").to_owned(),
            )
        })
        .collect();
    assert_eq!(classified.len(), entries.len(), "duplicate manifest entry");

    let catalog: BTreeSet<(String, String, String)> =
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT c.relname, a.attname, base.typname FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_type declared ON declared.oid = a.atttypid \
         JOIN pg_type base ON base.oid = CASE WHEN declared.typtype = 'd' \
              THEN declared.typbasetype ELSE declared.oid END \
         WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p') \
           AND c.relname <> '_sqlx_migrations' AND a.attnum > 0 AND NOT a.attisdropped \
           AND base.typlen = -1 ORDER BY c.relname, a.attnum",
        )
        .fetch_all(&w.db.pool)
        .await
        .expect("catalog columns")
        .into_iter()
        .collect();
    assert_eq!(
        classified, catalog,
        "classification drifted from the schema"
    );

    let accounted_tables: BTreeSet<String> = entries
        .iter()
        .filter(|entry| matches!(entry["class"].as_str(), Some("content" | "log")))
        .map(|entry| entry["table"].as_str().expect("table").to_owned())
        .collect();
    for table in accounted_tables {
        let triggers: Vec<(String, String, bool, bool)> = sqlx::query_as(
            "SELECT tgname, tgenabled::text, tgdeferrable, tginitdeferred FROM pg_trigger \
             WHERE tgrelid = $1::regclass AND NOT tgisinternal ORDER BY tgname",
        )
        .bind(&table)
        .fetch_all(&w.db.pool)
        .await
        .expect("accounting triggers");
        assert!(
            triggers
                .iter()
                .any(
                    |(name, enabled, deferred, initially)| name.ends_with("_quota_delta")
                        && enabled == "A"
                        && !deferred
                        && !initially
                ),
            "{table} lacks its immediate ALWAYS delta trigger: {triggers:?}"
        );
        assert!(
            triggers
                .iter()
                .any(
                    |(name, enabled, deferred, initially)| name.ends_with("_quota_flush")
                        && enabled == "A"
                        && *deferred
                        && *initially
                ),
            "{table} lacks its deferred ALWAYS finalizer: {triggers:?}"
        );
        let actual: Vec<u8> = sqlx::query_scalar(
            "SELECT tgargs FROM pg_trigger WHERE tgrelid = $1::regclass \
             AND tgname = $1 || '_quota_delta'",
        )
        .bind(&table)
        .fetch_one(&w.db.pool)
        .await
        .expect("trigger arguments");
        let actual: Vec<String> = actual
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8(part.to_vec()).expect("UTF-8 trigger argument"))
            .collect();
        let expected: Vec<String> = entries
            .iter()
            .filter(|entry| {
                entry["table"].as_str() == Some(table.as_str())
                    && matches!(entry["class"].as_str(), Some("content" | "log"))
            })
            .flat_map(|entry| {
                [
                    entry["class"].as_str().expect("class").to_owned(),
                    entry["measurement"]["kind"]
                        .as_str()
                        .expect("measurement kind")
                        .to_owned(),
                    entry["column"].as_str().expect("column").to_owned(),
                ]
            })
            .collect();
        assert_eq!(actual, expected, "{table} trigger/manifest column drift");
    }
    let blob_triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger WHERE tgrelid = 'blobs'::regclass \
         AND tgname IN ('blobs_quota_delta', 'blobs_quota_flush') AND tgenabled = 'A'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("blob accounting triggers");
    assert_eq!(blob_triggers, 2);
}

#[tokio::test]
async fn retained_bytes_apply_net_transaction_delta_enforce_the_last_byte_and_rollback_refusals() {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let baseline = usage(&w.db.pool, tenant).await;

    // A state-only update on an accounted table must not serialize behind the tenant usage row.
    let mut usage_lock = w.db.pool.begin().await.expect("usage lock transaction");
    sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
        .bind(tenant)
        .execute(&mut *usage_lock)
        .await
        .expect("usage row lock");
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        sqlx::query("UPDATE workspaces SET created_at = created_at WHERE tenant_id = $1")
            .bind(tenant)
            .execute(&w.db.pool),
    )
    .await
    .expect("unchanged measurement waited on the tenant usage lock")
    .expect("state-only update");
    usage_lock.rollback().await.expect("release usage lock");

    // A transient insert and purge has no retained growth and must not be falsely refused.
    let mut tx = w.db.pool.begin().await.expect("transaction");
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) \
         VALUES ($1, 'éx', 'viewer', clock_timestamp())",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("insert");
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = 'éx'")
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .expect("purge");
    tx.commit().await.expect("net-zero commit");
    assert_eq!(usage(&w.db.pool, tenant).await, baseline);

    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::TenantContentBytes,
            u64::try_from(baseline.0).expect("nonnegative") + 1,
            &ctx(),
        )
        .await
        .expect("one remaining byte");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled = true")
        .execute(&w.db.pool)
        .await
        .expect("enable test enforcement");
    let insert = |subject: &'static str| {
        let pool = w.db.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO memberships (tenant_id, subject, role, created_at) \
                 VALUES ($1, $2, 'viewer', clock_timestamp())",
            )
            .bind(tenant)
            .bind(subject)
            .execute(&pool)
            .await
        }
    };
    let (x, y) = tokio::join!(insert("x"), insert("y"));
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert_eq!(usage(&w.db.pool, tenant).await.0, baseline.0 + 1);
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject IN ('x', 'y')")
        .bind(tenant)
        .execute(&w.db.pool)
        .await
        .expect("release raced byte");
    assert_eq!(usage(&w.db.pool, tenant).await, baseline);

    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::TenantContentBytes,
            u64::try_from(baseline.0).expect("nonnegative") + 3,
            &ctx(),
        )
        .await
        .expect("set exact byte limit");
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) \
         VALUES ($1, 'éx', 'viewer', clock_timestamp())",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("exact last three UTF-8 bytes");
    let full = usage(&w.db.pool, tenant).await;
    assert_eq!(full.0, baseline.0 + 3);

    let refused = sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) \
         VALUES ($1, 'z', 'viewer', clock_timestamp())",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await;
    assert!(refused.is_err(), "one byte above the quota committed");
    assert_eq!(
        usage(&w.db.pool, tenant).await,
        full,
        "refusal leaked usage"
    );

    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = 'éx'")
        .bind(tenant)
        .execute(&w.db.pool)
        .await
        .expect("release bytes");
    assert_eq!(usage(&w.db.pool, tenant).await, baseline);

    let timestamp = now();
    let workspace = Workspace::create(
        &w.admin,
        WorkspaceId::generate(),
        Slug::parse("typed-refusal").expect("slug"),
        timestamp,
    )
    .expect("workspace");
    let audit = AuditRecord::by(
        &w.admin,
        AuditAction::WorkspaceCreated,
        ResourceType::Workspace,
        workspace.id().to_string(),
        ctx().trace_id,
        timestamp,
    );
    let refused = PgIdentityStore::new(w.db.pool.clone())
        .create_workspace(&workspace, &audit)
        .await;
    assert!(matches!(
        refused,
        Err(StoreError::QuotaExceeded { what })
            if what == QuotaResource::TenantContentBytes.limit_name()
    ));
    assert_eq!(
        usage(&w.db.pool, tenant).await,
        baseline,
        "typed refusal was not atomic"
    );
}

#[tokio::test]
async fn two_stores_race_for_one_durable_rate_token_and_database_time_refills_it() {
    let w = world().await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ApiRequestsPerSecond,
            1,
            &ctx(),
        )
        .await
        .expect("one request per second");
    assert_eq!(
        w.store
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("first token"),
        RateDecision::Admitted
    );
    let runtime = runtime_store(&w).await;
    let (a, b) = tokio::join!(
        w.store
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id()),
        runtime.admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
    );
    let decisions = [a.expect("first store"), b.expect("second store")];
    assert_eq!(
        decisions
            .iter()
            .filter(|d| **d == RateDecision::Admitted)
            .count(),
        1
    );
    assert_eq!(
        decisions
            .iter()
            .filter(|d| matches!(
                d,
                RateDecision::Refused {
                    retry_after_secs: 1
                }
            ))
            .count(),
        1
    );
    assert!(matches!(
        runtime
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("durable refusal"),
        RateDecision::Refused { .. }
    ));

    sqlx::query(
        "UPDATE quota_rate_buckets SET microtokens = 0, \
             updated_at = clock_timestamp() - interval '1 second' \
         WHERE tenant_id = $1 AND namespace_id = $2 AND resource = 'api_requests_per_second'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("age using database time");
    assert_eq!(
        runtime
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("refilled token"),
        RateDecision::Admitted
    );
    let future: time::OffsetDateTime = sqlx::query_scalar(
        "UPDATE quota_rate_buckets SET microtokens = 1000000, \
             updated_at = clock_timestamp() + interval '1 second' \
         WHERE tenant_id = $1 AND namespace_id = $2 AND resource = 'api_requests_per_second' \
         RETURNING updated_at",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("future bucket timestamp");
    assert_eq!(
        runtime
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
            .await
            .expect("token with future timestamp"),
        RateDecision::Admitted
    );
    let stored: time::OffsetDateTime = sqlx::query_scalar(
        "SELECT updated_at FROM quota_rate_buckets WHERE tenant_id = $1 AND namespace_id = $2 \
         AND resource = 'api_requests_per_second'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("stored bucket timestamp");
    assert!(
        stored >= future,
        "a backward database clock moved bucket time backwards"
    );

    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ApiRequestsPerSecond,
            0,
            &ctx(),
        )
        .await
        .expect("disable API admission");
    assert!(matches!(
        w.store
            .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
            .await,
        Err(StoreError::QuotaExceeded { .. })
    ));
}

#[tokio::test]
async fn a_waiter_samples_database_time_after_the_bucket_row_lock() {
    let w = world().await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ApiRequestsPerSecond,
            1,
            &ctx(),
        )
        .await
        .expect("rate");
    w.store
        .admit_namespace_request(w.admin.tenant_id(), w.namespace.id())
        .await
        .expect("create bucket");

    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let mut blocker = w.db.pool.begin().await.expect("blocker");
    sqlx::query(
        "SELECT 1 FROM quota_rate_buckets WHERE tenant_id = $1 AND namespace_id = $2 \
         AND resource = 'api_requests_per_second' FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .execute(&mut *blocker)
    .await
    .expect("row lock");

    let store = w.store.clone();
    let tenant_id = w.admin.tenant_id();
    let namespace_id = w.namespace.id();
    let waiter =
        tokio::spawn(async move { store.admit_namespace_request(tenant_id, namespace_id).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock' \
               AND query LIKE '%SELECT microtokens, updated_at FROM quota_rate_buckets%')",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("blocked waiter");
        if blocked {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "admission did not reach the row lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let owner_time: time::OffsetDateTime = sqlx::query_scalar(
        "UPDATE quota_rate_buckets SET microtokens = 1000000, updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND namespace_id = $2 AND resource = 'api_requests_per_second' \
         RETURNING updated_at",
    )
    .bind(tenant)
    .bind(namespace)
    .fetch_one(&mut *blocker)
    .await
    .expect("advance locked state");
    blocker.commit().await.expect("release row");
    assert_eq!(
        waiter.await.expect("waiter task").expect("admission"),
        RateDecision::Admitted
    );
    let waiter_time: time::OffsetDateTime = sqlx::query_scalar(
        "SELECT updated_at FROM quota_rate_buckets WHERE tenant_id = $1 AND namespace_id = $2 \
         AND resource = 'api_requests_per_second'",
    )
    .bind(tenant)
    .bind(namespace)
    .fetch_one(&w.db.pool)
    .await
    .expect("waiter timestamp");
    assert!(
        waiter_time >= owner_time,
        "the waiter moved bucket time backwards"
    );
}

#[tokio::test]
async fn runtime_role_cannot_mutate_or_bypass_private_accounting() {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let before = usage(&w.db.pool, tenant).await;
    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::TenantLogBytes,
            u64::try_from(before.2).expect("nonnegative") + 3,
            &ctx(),
        )
        .await
        .expect("three remaining log bytes");
    sqlx::query("UPDATE quota_accounting_control SET log_enforcement_enabled = true")
        .execute(&w.db.pool)
        .await
        .expect("enable defensive log crosscheck");
    let mut runtime = PgConnection::connect(&w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD))
        .await
        .expect("runtime connection");

    assert!(
        sqlx::query("UPDATE tenant_retained_usage SET content_bytes = 0 WHERE tenant_id = $1")
            .bind(tenant)
            .execute(&mut runtime)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT sagat_quota_add_delta($1, 1, 0, 0)")
            .bind(tenant)
            .execute(&mut runtime)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO quota_transaction_deltas \
             (transaction_id, tenant_id, content_delta, content_reserved_delta, log_delta) \
             VALUES (pg_current_xact_id(), $1, 1, 0, 0)",
        )
        .bind(tenant)
        .execute(&mut runtime)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled = false")
            .execute(&mut runtime)
            .await
            .is_err()
    );
    // Each denial aborts a transaction only at the statement level in autocommit, so later probes
    // still exercise distinct privilege boundaries.
    assert!(
        sqlx::query("SET session_replication_role = replica")
            .execute(&mut runtime)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("ALTER TABLE memberships DISABLE TRIGGER memberships_quota_delta")
            .execute(&mut runtime)
            .await
            .is_err()
    );
    let can_create_public: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user, 'public', 'CREATE')")
            .fetch_one(&mut runtime)
            .await
            .expect("schema privilege");
    assert!(!can_create_public);
    for statement in [
        "CREATE OR REPLACE FUNCTION public.sagat_quota_measure(jsonb, text, text, boolean) \
         RETURNS bigint LANGUAGE sql AS 'SELECT 0'",
        "CREATE FUNCTION public.sagat_quota_attack_probe() RETURNS bigint \
         LANGUAGE sql AS 'SELECT 0'",
    ] {
        let error = sqlx::query(statement)
            .execute(&mut runtime)
            .await
            .expect_err("runtime substituted a quota helper");
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501"),
            "unexpected substitution error: {error}"
        );
    }

    let slug = "runtime-accounted";
    sqlx::query(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) \
         VALUES ($1, $2, $3, clock_timestamp())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(tenant)
    .bind(slug)
    .execute(&mut runtime)
    .await
    .expect("ordinary runtime write");
    let after = usage(&w.db.pool, tenant).await;
    assert_eq!(
        after.0 - before.0,
        i64::try_from(slug.len()).expect("small")
    );

    let projected: i64 = sqlx::query_scalar("SELECT sagat_quota_lock_log_usage($1)")
        .bind(tenant)
        .fetch_one(&mut runtime)
        .await
        .expect("narrow runtime log helper");
    assert_eq!(projected, after.2);
    let connector = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_connectors \
         (id, tenant_id, namespace_id, slug, kind, settings, rate_per_second, created_by) \
         VALUES ($1, $2, $3, 'quota-log', 'NATS', '{}'::jsonb, 1, 'runtime')",
    )
    .bind(connector)
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .execute(&mut runtime)
    .await
    .expect("runtime connector");
    sqlx::query("UPDATE event_connectors SET last_error = 'éx' WHERE id = $1")
        .bind(connector)
        .execute(&mut runtime)
        .await
        .expect("store UTF-8 log");
    assert_eq!(usage(&w.db.pool, tenant).await.2, after.2 + 3);
    let refusal = sqlx::query("UPDATE event_connectors SET last_error = 'éxz' WHERE id = $1")
        .bind(connector)
        .execute(&mut runtime)
        .await
        .expect_err("defensive log ceiling allowed growth");
    let database = refusal.as_database_error().expect("database refusal");
    assert_eq!(database.code().as_deref(), Some("54000"));
    assert_eq!(database.constraint(), Some("tenant_log_quota_exceeded"));
    assert_eq!(usage(&w.db.pool, tenant).await.2, after.2 + 3);
    sqlx::query("UPDATE event_connectors SET last_error = NULL WHERE id = $1")
        .bind(connector)
        .execute(&mut runtime)
        .await
        .expect("release log bytes");
    assert_eq!(usage(&w.db.pool, tenant).await.2, after.2);
}
