//! Retained-pin content checks use provider I/O outside the engine transaction and fail closed.
use super::{
    ReadBarrier,
    support::engine::{Session, ctx, document, task_ref, world},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    secret_provider::{ProviderError, ProviderFuture, ProviderReference, SecretProvider},
    worker::Capabilities,
};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    identity::{Role, Slug},
    secrets::{SecretName, SecretValue},
};
use sagat_infra_ir::JsonDatabaseResultValidator;
use sagat_infra_secrets::database::JsonDatabaseCredentials;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;
struct Down(Arc<dyn SecretProvider>);
impl SecretProvider for Down {
    fn create<'a>(
        &'a self,
        r: &'a ProviderReference,
        v: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        self.0.create(r, v)
    }
    fn read<'a>(&'a self, _: &'a ProviderReference) -> ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async { Err(ProviderError::Unavailable) })
    }
    fn delete<'a>(&'a self, r: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        self.0.delete(r)
    }
}
pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.workers = w
        .workers
        .with_secret_provider(provider.clone())
        .with_database_content(
            Arc::new(JsonDatabaseCredentials),
            Arc::new(JsonDatabaseResultValidator),
        );
    let ns = w.namespace.id();
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let credential = |version| {
        ManagedSecretWrite{name:"result-credentials",expected_version:version,request_id:Uuid::new_v4(),value:SecretValue::new(serde_json::to_vec(&json!({"username":"managed_reader","password":format!("result-password-{version}")})).unwrap()).unwrap()}
    };
    secrets
        .put(&w.admin, ns, credential(0), &ctx())
        .await
        .unwrap();
    ConnectionService::new(w.store.clone(), SystemClock)
        .put(
            &w.admin,
            ns,
            ConnectionWrite {
                name: Slug::parse("result-db").unwrap(),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("result-credentials").unwrap(),
                    DatabaseLimits::default(),
                )
                .unwrap(),
            },
            &ctx(),
        )
        .await
        .unwrap();
    let (query,_)=w.publish(document("result-query","database",vec![json!({"id":"database","type":"database","connectionRef":{"name":"result-db"},"operation":"query","statement":"SELECT 1"})])).await;
    let (write,_)=w.publish(document("result-write","database",vec![json!({"id":"database","type":"database","connectionRef":{"name":"result-db"},"operation":"execute","statement":"INSERT INTO orders VALUES (1) RETURNING id"})])).await;
    let machine = w.member("result-machine", Role::DatabaseRunner).await;
    w.bind_everything(&machine).await;
    let registration = w
        .workers
        .register(
            &machine,
            "payments",
            "result-machine",
            &Capabilities {
                activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                languages: vec![],
                pools: vec![],
                max_inflight: 4,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .unwrap();
    let session = Session {
        principal: machine.clone(),
        worker_id: registration.worker_id,
        token: registration.session_token.expose().to_owned(),
    };
    let clean=serde_json::to_vec(&json!({"columns":[{"name":"answer","type":"integer"}],"rows":[[{"type":"integer","value":"42"}]],"affectedRows":"1","truncated":false})).unwrap();
    // A capability is required even when a worker claims its output is harmless.
    let clean_run = w.start(query, &json!({})).await;
    let tasks = w.poll(&session, 1).await;
    let task = &tasks[0];
    assert_eq!(task.run_id, clean_run);
    assert!(
        w.workers
            .complete(&machine, &task_ref(task), &clean, "result-clean")
            .await
            .is_err()
    );
    w.workers
        .resolve_database(&machine, &task_ref(task), None)
        .await
        .unwrap();
    w.workers = w
        .workers
        .with_secret_provider(Arc::new(Down(provider.clone())));
    assert!(
        w.workers
            .complete(&machine, &task_ref(task), &clean, "result-clean")
            .await
            .is_err()
    );
    assert_eq!(
        w.step_status(clean_run, "database").await.unwrap().as_str(),
        "LEASED"
    );
    w.workers = w.workers.with_secret_provider(provider.clone());
    let ack = w
        .workers
        .complete(&machine, &task_ref(task), &clean, "result-clean")
        .await
        .unwrap();
    assert_eq!(ack.step_status.as_str(), "SUCCEEDED");
    w.workers = w
        .workers
        .with_secret_provider(Arc::new(Down(provider.clone())));
    assert!(
        w.workers
            .complete(&machine, &task_ref(task), &clean, "result-clean")
            .await
            .unwrap()
            .already_applied
    );
    w.workers = w.workers.with_secret_provider(provider.clone());
    // Rotation must not remove the old password from this attempt's output guard.
    let leak_run = w.start(query, &json!({})).await;
    let tasks = w.poll(&session, 1).await;
    let task = &tasks[0];
    w.workers
        .resolve_database(&machine, &task_ref(task), None)
        .await
        .unwrap();
    secrets
        .put(&w.admin, ns, credential(1), &ctx())
        .await
        .unwrap();
    use base64::Engine as _;
    let leaked=serde_json::to_vec(&json!({"columns":[{"name":"bytes","type":"binary"}],"rows":[[{"type":"binary","value":base64::engine::general_purpose::STANDARD.encode(b"prefix-result-password-0-suffix")}]],"affectedRows":"1","truncated":false})).unwrap();
    let ack = w
        .workers
        .complete(&machine, &task_ref(task), &leaked, "result-leak")
        .await
        .unwrap();
    assert_eq!(ack.step_status.as_str(), "FAILED");
    let output: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT output_digest FROM steps WHERE run_id=$1 AND node='database'")
            .bind(leak_run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert!(output.is_none());
    // A write whose reported output cannot be retained requires reconciliation/manual review.
    let write_run = w.start(write, &json!({})).await;
    let tasks = w.poll(&session, 1).await;
    let task = &tasks[0];
    w.workers
        .resolve_database(&machine, &task_ref(task), None)
        .await
        .unwrap();
    let leaked=serde_json::to_vec(&json!({"columns":[{"name":"v","type":"text"}],"rows":[[{"type":"text","value":"result-password-1"}]],"affectedRows":"1","truncated":false})).unwrap();
    let ack = w
        .workers
        .complete(&machine, &task_ref(task), &leaked, "write-leak")
        .await
        .unwrap();
    assert_eq!(ack.step_status.as_str(), "WAITING");
    assert_eq!(w.run_status(write_run).await.as_str(), "MANUAL_REVIEW");
    assert!(
        w.journal(write_run)
            .await
            .contains(&"step.needs_review".to_owned())
    );
    w.assert_consistent(write_run).await;
    // Cancellation commits while output scanning waits on OpenBao; the subsequent fence rejects it.
    let cancelled_run = w.start(query, &json!({})).await;
    let tasks = w.poll(&session, 1).await;
    let task = &tasks[0];
    w.workers
        .resolve_database(&machine, &task_ref(task), None)
        .await
        .unwrap();
    let barrier = Arc::new(ReadBarrier {
        inner: provider.clone(),
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(barrier.clone());
    let reference = task_ref(task);
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers
                .complete(&machine, &reference, &clean, "cancelled-output"),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(2),
                    w.operate(cancelled_run, sagat_application::runs::RunOperation::Cancel),
                )
                .await
                .unwrap()
                .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::StaleAttempt)));
    let stored: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .unwrap();
    for bytes in stored {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("result-password"));
        assert!(!text.contains("managed_reader"));
    }
    for run in [clean_run, leak_run, cancelled_run] {
        w.assert_consistent(run).await;
    }
    // Failure to retain an already performed write is ambiguous, even at a full content quota.
    w.workers = w.workers.with_secret_provider(provider);
    let quota_run = w.start(write, &json!({})).await;
    let tasks = w.poll(&session, 1).await;
    let task = &tasks[0];
    w.workers
        .resolve_database(&machine, &task_ref(task), None)
        .await
        .unwrap();
    let tenant = w.admin.tenant_id().as_uuid();
    let mut tx = w.db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota-policy:tenant_content_bytes:{tenant}"))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) SELECT tenant_id, NULL, 'tenant_content_bytes', content_bytes + content_reserved_bytes, 'database-result-test' FROM tenant_retained_usage WHERE tenant_id=$1")
        .bind(tenant).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true")
        .execute(&w.db.pool)
        .await
        .unwrap();
    // A different digest from the earlier clean output must allocate retained bytes.
    let output = serde_json::to_vec(&json!({"columns":[{"name":"quota","type":"text"}],"rows":[[{"type":"text","value":"new result cannot fit"}]],"affectedRows":"1","truncated":false})).unwrap();
    let ack = w
        .workers
        .complete(&machine, &task_ref(task), &output, "quota-output")
        .await
        .unwrap();
    assert_eq!(ack.step_status.as_str(), "WAITING");
    assert_eq!(ack.run_status.as_str(), "MANUAL_REVIEW");
    assert!(
        w.journal(quota_run)
            .await
            .contains(&"step.needs_review".to_owned())
    );
    assert!(
        w.workers
            .complete(&machine, &task_ref(task), &output, "quota-output")
            .await
            .unwrap()
            .already_applied
    );
    w.assert_consistent(quota_run).await;
}
