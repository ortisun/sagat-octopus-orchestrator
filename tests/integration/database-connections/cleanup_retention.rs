//! Current/grace versions and live/manual-review pins survive the operator cleanup path.
use super::world;
use super::{
    cleanup::{age_operations, runtime_store},
    support::engine::{Session, ctx, document, task_ref},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    definitions::Idempotency,
    error::{AppError, StoreError},
    execution::{Engine, RunRecord},
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    quotas::RunLimits,
    runs::{NewRun, ReviewAction, RunOperation, RunService, RunStore},
    secret_cleanup::{SecretCleanupService, SecretCleanupStore},
    secret_provider::SecretProvider,
    worker::Capabilities,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    execution::{ReviewOutcome, RunId},
    identity::{Role, Slug},
    quota::QuotaResource,
    secrets::{SecretName, SecretValue},
};
use sagat_infra_ir::{JsonDatabaseResultValidator, JsonEvaluator};
use sagat_infra_secrets::database::JsonDatabaseCredentials;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.workers = w
        .workers
        .with_secret_provider(provider.clone())
        .with_database_content(
            Arc::new(JsonDatabaseCredentials),
            Arc::new(JsonDatabaseResultValidator),
        );
    let store = runtime_store(&w).await;
    let cleanup = SecretCleanupService::new(store.clone(), provider.clone());
    let ns = w.namespace.id();
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let rotation = |expected_version| {
        ManagedSecretWrite { name:"retained-credential", expected_version, request_id:Uuid::new_v4(), value: SecretValue::new(serde_json::to_vec(&json!({"username":"reader", "password":format!("retention-password-{expected_version}")})).unwrap()).unwrap() }
    };
    secrets
        .put(&w.admin, ns, rotation(0), &ctx())
        .await
        .unwrap();
    ConnectionService::new(w.store.clone(), SystemClock)
        .put(
            &w.admin,
            ns,
            ConnectionWrite {
                name: Slug::parse("retained-db").unwrap(),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("retained-credential").unwrap(),
                    DatabaseLimits::default(),
                )
                .unwrap(),
            },
            &ctx(),
        )
        .await
        .unwrap();
    let definition = |name, operation, statement| {
        document(
            name,
            "db",
            vec![
                json!({"id":"db","type":"database","connectionRef":{"name":"retained-db"},"operation":operation,"statement":statement}),
            ],
        )
    };
    let (query, _) = w
        .publish(definition("retained-query", "query", "SELECT 1"))
        .await;
    let (write, _) = w
        .publish(definition(
            "retained-write",
            "execute",
            "INSERT INTO orders VALUES (1)",
        ))
        .await;
    let machine = w.member("cleanup-machine", Role::DatabaseRunner).await;
    w.bind_everything(&machine).await;
    let registration = w
        .workers
        .register(
            &machine,
            "payments",
            "cleanup-machine",
            &Capabilities {
                activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                languages: vec![],
                pools: vec![],
                max_inflight: 4,
                protocol_version: "1.6".into(),
            },
        )
        .await
        .unwrap();
    let session = Session {
        principal: machine.clone(),
        worker_id: registration.worker_id,
        token: registration.session_token.expose().to_owned(),
    };
    let live = w.start(query, &json!({})).await;
    let review = w.start(write, &json!({})).await;
    let terminal = w.start(query, &json!({})).await;
    let tasks = w.poll(&session, 4).await;
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        w.workers
            .resolve_database(&machine, &task_ref(task), None)
            .await
            .unwrap();
    }
    let pending = tasks.iter().find(|task| task.run_id == review).unwrap();
    w.fail_with(&session, pending, "OUTCOME_UNKNOWN", false)
        .await
        .unwrap();
    assert_eq!(w.run_status(review).await.as_str(), "MANUAL_REVIEW");
    let done = tasks.iter().find(|task| task.run_id == terminal).unwrap();
    w.complete(&session,done,&json!({"columns":[{"name":"value","type":"integer"}],"rows":[[{"type":"integer","value":"1"}]],"affectedRows":"1","truncated":false})).await.unwrap();
    for version in 1..8 {
        secrets
            .put(&w.admin, ns, rotation(version), &ctx())
            .await
            .unwrap();
    }
    age_operations(&w.db.pool).await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!(
        result.retired, 2,
        "only versions 2 and 3 are outside the retained five and unpinned"
    );
    assert_eq!(
        result.pins_released, 0,
        "recent terminal pin has a 24 hour grace period"
    );
    let age_terminal = || async {
        sqlx::query("UPDATE runs SET finished_at=clock_timestamp()-interval '25 hours' WHERE finished_at IS NOT NULL").execute(&w.db.pool).await.unwrap();
    };
    age_terminal().await;
    let source_credential: (i64, Uuid) = sqlx::query_as(
        "SELECT secret_version::bigint,provider_operation FROM step_database_credentials WHERE tenant_id=$1 AND run_id=$2",
    ).bind(w.admin.tenant_id().as_uuid()).bind(terminal.as_uuid())
        .fetch_one(&w.db.pool).await.unwrap();
    let current_credential: (i64, Uuid) = sqlx::query_as(
        "SELECT v.version::bigint,v.provider_operation FROM secrets s JOIN secret_versions v \
         ON v.tenant_id=s.tenant_id AND v.secret_id=s.id AND v.version=s.current_version \
         WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.name='retained-credential'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(ns.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_ne!(source_credential, current_credential);
    let rerun = Box::pin(rerun_source_and_cleanup_barrier(&w, &store, terminal)).await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!((result.pins_released, result.retired), (1, 0));
    let rerun_tasks = w.poll(&session, 1).await;
    assert_eq!(rerun_tasks.len(), 1);
    let rerun_task = &rerun_tasks[0];
    assert_eq!(rerun_task.run_id, rerun.info.run_id);
    w.workers
        .resolve_database(&machine, &task_ref(rerun_task), None)
        .await
        .unwrap();
    let rerun_credential: (i64, Uuid) = sqlx::query_as(
        "SELECT secret_version::bigint,provider_operation FROM step_database_credentials WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(rerun.info.run_id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(rerun_credential, current_credential);
    assert_ne!(rerun_credential, source_credential);
    w.complete(&session,rerun_task,&json!({"columns":[{"name":"value","type":"integer"}],"rows":[[{"type":"integer","value":"1"}]],"affectedRows":"1","truncated":false})).await.unwrap();
    store
        .verify_run(w.admin.tenant_id(), terminal)
        .await
        .unwrap();
    store
        .verify_run(w.admin.tenant_id(), rerun.info.run_id)
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET finished_at=clock_timestamp()-interval '25 hours' WHERE tenant_id=$1 AND id=$2 AND finished_at IS NOT NULL")
        .bind(w.admin.tenant_id().as_uuid()).bind(rerun.info.run_id.as_uuid()).execute(&w.db.pool).await.unwrap();
    assert_eq!(store.release_terminal_credential_pins(32).await.unwrap(), 1);
    assert!(
        store
            .provider_cleanup_candidates(32)
            .await
            .unwrap()
            .is_empty()
    );
    let pinned: i64 = sqlx::query_scalar("SELECT count(*) FROM step_database_credentials")
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(
        pinned, 2,
        "live and manual-review attempts keep the old provider object"
    );
    w.operate_run(live, RunOperation::Cancel).await;
    // Cancellation waits for the leased executor to drain/expire; until then its pin remains live.
    w.age_lease(live, "db").await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(live).await.as_str(), "CANCELLED");
    w.review(
        review,
        "db",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "database independently reconciled",
    )
    .await
    .unwrap();
    age_terminal().await;
    let result = cleanup.run_batch(32).await.unwrap();
    assert_eq!((result.pins_released, result.retired), (2, 1));
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations")
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(
        retained, 5,
        "current and four grace versions remain readable"
    );
    let references: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM secret_provider_operations")
        .fetch_all(&w.db.pool)
        .await
        .unwrap();
    for operation in references {
        assert!(
            provider
                .read(&sagat_application::secret_provider::ProviderReference {
                    tenant: w.admin.tenant_id(),
                    namespace: ns,
                    operation
                })
                .await
                .unwrap()
                .is_some()
        );
    }
}

async fn rerun_source_and_cleanup_barrier(
    w: &super::support::engine::World,
    store: &sagat_infra_postgres::PgRunStore,
    source: RunId,
) -> RunRecord {
    let tenant = w.admin.tenant_id();
    let namespace = w.namespace.id();
    let secrets_key = format!("quota:{}:{tenant}:{namespace}", QuotaResource::Secrets);
    let quota_key = format!("quota:{}:{tenant}:{namespace}", QuotaResource::ActiveRuns);
    let checkpoint = || async {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM runs WHERE tenant_id=$1), \
             (SELECT count(*) FROM run_events WHERE tenant_id=$1), \
             (SELECT count(*) FROM event_outbox WHERE tenant_id=$1), \
             (SELECT count(*) FROM audit_records WHERE tenant_id=$1), \
             (SELECT count(*) FROM payloads WHERE tenant_id=$1), \
             (SELECT count(*) FROM step_database_credentials WHERE tenant_id=$1)",
        )
        .bind(tenant.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("atomic rerun effects")
    };
    // Native invalid reference keeps the existing FK error class; no metadata or values leave.
    let before = checkpoint().await;
    let mut info = w.run_record(source).await.unwrap().info;
    info.run_id = RunId::generate();
    info.root = info.run_id;
    info.rerun_of = Some(RunId::generate());
    let new = NewRun {
        schedule: None,
        retained: None,
        limits: RunLimits::of(store, tenant, namespace).await.unwrap(),
        definition: w.cache.get(info.version_id).unwrap(),
        input: b"{}".to_vec(),
        input_blob: None,
        audit: AuditRecord::by(
            &w.admin,
            AuditAction::RunRerun,
            ResourceType::Run,
            info.run_id.to_string(),
            None,
            time::OffsetDateTime::now_utc(),
        ),
        idempotency: Idempotency {
            scope: "missing-rerun-source".into(),
            key: Uuid::new_v4().to_string(),
            fingerprint: vec![1],
        },
        info,
    };
    assert!(matches!(
        store.create(new).await,
        Err(StoreError::MissingReference)
    ));
    assert_eq!(
        checkpoint().await,
        before,
        "missing source leaves no effects"
    );

    let runs = RunService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    );
    let context = ctx();
    let reason = "repeat retained managed query";
    let key = "retention-rerun-lock-order";
    // Crash/rollback before admission: source U blocks the real public rerun. While it waits,
    // Secrets must remain free. Cancel the future before releasing the unchanged source tuple.
    let mut source_guard = w.db.pool.begin().await.unwrap();
    let source_backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *source_guard)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant.as_uuid())
        .bind(source.as_uuid())
        .execute(&mut *source_guard)
        .await
        .unwrap();
    let mut abandoned = Box::pin(runs.rerun(&w.admin, source, reason, key, &context));
    let abandoned_backend: i32 = tokio::select! {
        _ = &mut abandoned => panic!("rerun must wait on its referenced source"),
        pid = super::support::eventually("actual app rerun waits on source identity", || async {
            sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND usename='sagat_app' \
                 AND (query LIKE '%FROM runs%' OR query LIKE '%INSERT INTO runs%') LIMIT 1",
            ).bind(source_backend).fetch_optional(&w.db.pool).await.unwrap()
        }) => pid,
    };
    let mut probe = w.db.pool.begin().await.unwrap();
    let secrets_free: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,0))")
            .bind(&secrets_key)
            .fetch_one(&mut *probe)
            .await
            .unwrap();
    probe.rollback().await.unwrap();
    drop(abandoned);
    source_guard.rollback().await.unwrap();
    super::support::eventually("abandoned rerun transaction rolled back", || async {
        sqlx::query_scalar::<_,bool>(
            "SELECT NOT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND xact_start IS NOT NULL)",
        ).bind(abandoned_backend).fetch_one(&w.db.pool).await.unwrap().then_some(())
    }).await;
    assert_eq!(
        checkpoint().await,
        before,
        "abandoned admission rolls back all effects"
    );

    // Retry the same real request. Pause after catalog authority at active-run quota so the
    // actual native cleanup port can reach its conflicting source/catalog edge on old code.
    let mut quota_gate = w.db.pool.begin().await.unwrap();
    let quota_backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *quota_gate)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&quota_key)
        .execute(&mut *quota_gate)
        .await
        .unwrap();
    let deadlocks_before: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    let mut rerunning = Box::pin(runs.rerun(&w.admin, source, reason, key, &context));
    let rerun_backend: i32 = tokio::select! {
        _ = &mut rerunning => panic!("managed rerun must reach active-run quota"),
        pid = super::support::eventually("actual app rerun waits at active-run quota", || async {
            sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND usename='sagat_app' \
                 AND query LIKE '%pg_advisory_xact_lock%' LIMIT 1",
            ).bind(quota_backend).fetch_optional(&w.db.pool).await.unwrap()
        }) => pid,
    };
    // Use pin release alone: full batch also visits unrelated provider operations and their
    // Secrets locks. SKIP LOCKED may immediately return0 when rerun holds source KS.
    let mut cleaning = Box::pin(store.release_terminal_credential_pins(32));
    let mut cleanup_backend = None;
    let early_cleanup = tokio::select! {
        result = &mut cleaning => Some(result),
        pid = super::support::eventually("cleanup waits on rerun-held Secrets on old code", || async {
            sqlx::query_scalar::<_,i32>(
                "SELECT pid FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND usename='sagat_app' \
                 AND query LIKE '%pg_advisory_xact_lock%' LIMIT 1",
            ).bind(rerun_backend).fetch_optional(&w.db.pool).await.unwrap()
        }) => { cleanup_backend = Some(pid); None }
    };
    let protected =
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
            .bind(tenant.as_uuid())
            .bind(source.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect_err("source guard remains retained");
    assert_eq!(super::support::db_error(protected).0, "55P03");
    quota_gate.rollback().await.unwrap();
    let (rerun_result, cleanup_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(&mut rerunning, async {
            match early_cleanup {
                Some(result) => result,
                None => (&mut cleaning).await,
            }
        })
    })
    .await
    .expect("rerun and native cleanup finish");
    drop(rerunning);
    drop(cleaning);
    let deadlocks_after: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    let rerun_ok = rerun_result.is_ok();
    let cleanup_ok = cleanup_result.is_ok();
    let rerun_unavailable = matches!(&rerun_result, Err(AppError::Unavailable));
    let cleanup_unavailable = matches!(&cleanup_result, Err(StoreError::Unavailable { .. }));
    assert!(
        secrets_free && deadlocks_after == deadlocks_before && rerun_ok && cleanup_ok,
        "rerun source guard must precede catalog; secrets_free={secrets_free}, \
         deadlocks={deadlocks_before}->{deadlocks_after}, rerun_ok={rerun_ok}, cleanup_ok={cleanup_ok}, \
         rerun_unavailable={rerun_unavailable}, cleanup_unavailable={cleanup_unavailable}, \
         rerun_backend={rerun_backend}, cleanup_backend={cleanup_backend:?}"
    );
    assert_eq!(
        cleanup_result.unwrap(),
        0,
        "first cleanup skips the retained source identity"
    );
    let rerun = rerun_result.unwrap();
    assert_eq!(rerun.info.rerun_of, Some(source));
    rerun
}
