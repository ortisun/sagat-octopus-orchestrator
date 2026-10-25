//! Real PostgreSQL/OpenBao: current policy, capacity and two-phase capability fencing.
use super::world;
use super::{
    ReadBarrier,
    support::engine::{Session, ctx, document, task_ref},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    secret_provider::SecretProvider,
    worker::Capabilities,
};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionStatus, DatabaseLimits, MANAGED_DATABASE_ACTIVITY, TlsPolicy,
    },
    identity::{Role, Slug},
    quota::{QuotaKind, QuotaResource as R},
    secrets::{SecretName, SecretValue},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.workers = w.workers.with_secret_provider(provider.clone());
    let ns = w.namespace.id();
    let policy_ctx = ctx();
    let set = |resource, value| {
        w.quotas
            .set(&w.admin, Some(ns), resource, value, &policy_ctx)
    };
    // Every new resource is persisted with matching scope/range constraints and accurate views.
    for resource in R::ALL.into_iter().filter(|r| r.is_database()) {
        let scope = if resource == R::TenantDatabaseConcurrency {
            None
        } else {
            Some(ns)
        };
        let view = w
            .quotas
            .set(&w.admin, scope, resource, resource.default_limit(), &ctx())
            .await
            .unwrap();
        assert_eq!(
            view.used,
            if resource.kind() == QuotaKind::Policy {
                None
            } else {
                Some(0)
            }
        );
        assert!(
            w.quotas
                .set(&w.admin, scope, resource, resource.maximum() + 1, &ctx())
                .await
                .is_err()
        );
        if resource.kind() == QuotaKind::Policy {
            assert!(
                w.quotas
                    .set(&w.admin, scope, resource, 0, &ctx())
                    .await
                    .is_err()
            );
            assert!(
                sqlx::query(
                    "UPDATE quota_limits SET max_value=0 WHERE tenant_id=$1 AND resource=$2"
                )
                .bind(w.admin.tenant_id().as_uuid())
                .bind(resource.as_str())
                .execute(&w.db.pool)
                .await
                .is_err()
            );
        }
    }
    assert!(
        w.quotas
            .set(&w.admin, Some(ns), R::TenantDatabaseConcurrency, 1, &ctx())
            .await
            .is_err()
    );
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let write_secret = |name| ManagedSecretWrite {
        name,
        value: SecretValue::new(b"policy-credential-canary".to_vec()).unwrap(),
        expected_version: 0,
        request_id: Uuid::new_v4(),
    };
    set(R::SecretProviderOperations, 1).await.unwrap();
    let pending = write_secret("policy-credentials");
    let intent = secrets
        .prepare(&w.admin, ns, &pending, &ctx())
        .await
        .unwrap();
    assert!(matches!(
        secrets
            .prepare(&w.admin, ns, &write_secret("other-credential"), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    set(R::SecretProviderOperations, 0).await.unwrap();
    provider
        .create(&intent.reference, &pending.value)
        .await
        .unwrap();
    secrets
        .reconcile(&w.admin, ns, intent.reference.operation, &ctx())
        .await
        .unwrap();
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    set(R::DatabaseConnections, 1).await.unwrap();
    let mut write = ConnectionWrite {
        name: Slug::parse("policy-db").unwrap(),
        request_id: Uuid::new_v4(),
        expected_revision: 0,
        status: ConnectionStatus::Paused,
        config: ConnectionConfig::postgres(
            "db.example.com",
            5432,
            "app",
            TlsPolicy::VerifyFull { ca_pem: None },
            SecretName::parse("policy-credentials").unwrap(),
            DatabaseLimits::MAX,
        )
        .unwrap(),
    };
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    let other = ConnectionWrite {
        name: Slug::parse("other-db").unwrap(),
        request_id: Uuid::new_v4(),
        ..write.clone()
    };
    assert!(matches!(
        catalog.put(&w.admin, ns, other, &ctx()).await,
        Err(AppError::QuotaExceeded(_))
    ));
    set(R::DatabaseConnections, 0).await.unwrap();
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap(); // receipt replay at zero
    write.expected_revision = 1;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Active;
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap(); // edit at zero
    let flow = document(
        "policy-flow",
        "database",
        vec![
            json!({"id":"database","type":"database","connectionRef":{"name":"policy-db"},"operation":"query","statement":"SELECT 1"}),
        ],
    );
    let (workflow, _) = w.publish(flow.clone()).await;
    set(R::DatabaseStatementBytes, 7).await.unwrap();
    assert!(
        w.try_publish_as(
            &w.admin,
            document(
                "policy-refused",
                "database",
                flow["nodes"].as_array().unwrap().clone()
            )
        )
        .await
        .is_err()
    );
    assert!(
        w.start_with(workflow, &json!({}), "policy-refused-start", None)
            .await
            .is_err()
    );
    set(R::DatabaseStatementBytes, 8).await.unwrap();
    // The operator's current ceiling narrows a previously stored namespace override.
    set(R::DatabaseRows, 20).await.unwrap();
    w.quotas
        .grant_ceiling(w.admin.tenant_id(), R::DatabaseRows, Some(3))
        .await
        .unwrap();
    assert!(set(R::DatabaseRows, 4).await.is_err());
    set(R::DatabaseTimeoutMs, 2000).await.unwrap();
    set(R::DatabaseConnectTimeoutMs, 250).await.unwrap();
    set(R::DatabaseResultBytes, 4096).await.unwrap();
    set(R::DatabaseParameterBytes, 512).await.unwrap();
    set(R::DatabaseParameters, 2).await.unwrap();
    set(R::DatabasePoolSize, 1).await.unwrap();
    set(R::DatabaseIdleTimeoutMs, 1000).await.unwrap();
    w.start(workflow, &json!({})).await;
    w.start(workflow, &json!({})).await;
    let runner = w.member("policy-runner", Role::DatabaseRunner).await;
    w.bind_everything(&runner).await;
    let registered = w
        .workers
        .register(
            &runner,
            "payments",
            "policy-runner",
            &Capabilities {
                activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                languages: vec![],
                pools: vec![],
                max_inflight: 8,
                protocol_version: "1.6".into(),
            },
        )
        .await
        .unwrap();
    let session = Session {
        principal: runner.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    set(R::DatabaseConcurrency, 0).await.unwrap();
    assert!(w.poll(&session, 8).await.is_empty());
    set(R::DatabaseConcurrency, 1).await.unwrap();
    w.quotas
        .set(&w.admin, None, R::TenantDatabaseConcurrency, 0, &ctx())
        .await
        .unwrap();
    assert!(w.poll(&session, 8).await.is_empty());
    w.quotas
        .set(&w.admin, None, R::TenantDatabaseConcurrency, 1, &ctx())
        .await
        .unwrap();
    let tasks = w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 1);
    let reference = task_ref(&tasks[0]);
    let resolved = w
        .workers
        .resolve_database(&runner, &reference, None)
        .await
        .unwrap();
    assert_eq!(
        resolved.operation.limits,
        DatabaseLimits {
            max_statement_bytes: 8,
            max_parameters: 2,
            max_parameter_bytes: 512,
            max_rows: 3,
            max_result_bytes: 4096,
            timeout_ms: 2000,
            connect_timeout_ms: 250,
            pool_size: 1,
            idle_timeout_ms: 1000
        }
    );
    assert!(w.poll(&session, 8).await.is_empty());
    // Tightening commits while provider I/O is in flight; the old capability never leaves.
    let barrier = Arc::new(ReadBarrier {
        inner: provider,
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(barrier.clone());
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            w.workers.resolve_database(&runner, &reference, None),
            async {
                barrier.entered.wait().await;
                tokio::time::timeout(Duration::from_secs(2), set(R::DatabaseRows, 1))
                    .await
                    .unwrap()
                    .unwrap();
                barrier.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::Unavailable)));
    // Policy writer waits for a live admission fence, then becomes visible after commit.
    let mut tx = w.db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1,0))")
        .bind(format!("database-policy:{}:{ns}", w.admin.tenant_id()))
        .execute(&mut *tx)
        .await
        .unwrap();
    let change = set(R::DatabaseTimeoutMs, 1500);
    tokio::pin!(change);
    assert!(
        tokio::time::timeout(Duration::from_millis(75), &mut change)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), change)
        .await
        .unwrap()
        .unwrap();
    write.expected_revision = 2;
    write.request_id = Uuid::new_v4();
    write.status = ConnectionStatus::Deleted;
    catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    let views = w.quotas.list(&w.admin, Some(ns), &ctx()).await.unwrap();
    assert_eq!(
        views
            .iter()
            .find(|v| v.resource == R::DatabaseConnections)
            .unwrap()
            .used,
        Some(0)
    );
    assert_eq!(
        views
            .iter()
            .find(|v| v.resource == R::DatabaseConcurrency)
            .unwrap()
            .used,
        Some(1)
    );
    assert_eq!(
        views
            .iter()
            .find(|v| v.resource == R::SecretProviderOperations)
            .unwrap()
            .used,
        Some(0)
    );
    // The last ordinary revision must still leave room for the terminal tombstone.
    let saturated = Uuid::new_v4();
    let mut tx = w.db.pool.begin().await.unwrap();
    sqlx::query("INSERT INTO database_connections(id,tenant_id,namespace_id,name,current_revision,status) VALUES($1,$2,$3,'saturated-db',1000,'PAUSED')")
        .bind(saturated).bind(w.admin.tenant_id().as_uuid()).bind(ns.as_uuid()).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO database_connection_revisions(tenant_id,namespace_id,connection_id,revision,config,status,actor,request_id) SELECT tenant_id,namespace_id,$3,1000,config,'PAUSED',actor,$4 FROM database_connection_revisions WHERE tenant_id=$1 AND namespace_id=$2 AND connection_id=(SELECT id FROM database_connections WHERE tenant_id=$1 AND namespace_id=$2 AND name='policy-db') AND revision=1")
        .bind(w.admin.tenant_id().as_uuid()).bind(ns.as_uuid()).bind(saturated).bind(Uuid::new_v4()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    write.name = Slug::parse("saturated-db").unwrap();
    write.expected_revision = 1000;
    write.request_id = Uuid::new_v4();
    let deleted = catalog.put(&w.admin, ns, write, &ctx()).await.unwrap();
    assert_eq!(deleted.revision, 1001);
    assert_eq!(deleted.status, ConnectionStatus::Deleted);
}
