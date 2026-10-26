#![allow(clippy::unwrap_used, clippy::expect_used)]
mod ad_hoc;
mod admission;
mod cleanup;
mod cleanup_retention;
mod consumers;
mod executor;
mod key_rotation;
mod output;
mod output_blobs;
mod policies;
mod results;
mod signing_consumers;
#[path = "../../support/mod.rs"]
mod support;

use sagat_application::secret_provider::SecretProvider;
use sagat_application::worker::ClaimedTask;
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
};
use sagat_domain::secrets::SecretValue;
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use std::{sync::Arc, time::Duration};
use support::bounded::{self, LIFECYCLE_LIMIT, Progress};
use support::engine::{World, ctx, world_with};
use uuid::Uuid;

mod bounded_stages;

/// A world whose leases outlive the slowest machine this suite runs on.
///
/// The default lease lasts 30 s. A scenario that leases a task early and uses it at the end took
/// 25 s on a quiet laptop and more than 60 s on a loaded runner: the lease expired, the call was
/// refused before it reached the provider, and the barrier waiting for that read never opened. A
/// scenario that tests a lease expiring ages it on purpose (`age_lease`).
pub(crate) async fn world() -> World {
    world_with(sagat_application::worker::WorkerSettings {
        lease_ttl: Duration::from_secs(3600),
        ..Default::default()
    })
    .await
}

// This aggregate lifecycle contains deeply nested adapter futures. Keep its explicit test-only
// stack allocation local; the worker and control-plane runtimes keep their normal settings.
#[test]
fn durable_provider_intents_reconcile_without_plaintext_or_duplicate_rotation() {
    std::thread::Builder::new()
        .name("managed-secret-lifecycle".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(bounded::lifecycle(
                    "managed secret lifecycle",
                    LIFECYCLE_LIMIT,
                    managed_secret_lifecycle,
                ))
        })
        .unwrap()
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

async fn managed_secret_lifecycle(progress: Progress) {
    progress.enter("start PostgreSQL and platform");
    let mut w = world().await;
    progress.enter("start OpenBao");
    let container = bounded::start_openbao("sagat-test-bootstrap").await;
    let port = container.get_host_port_ipv4(8200).await.unwrap();
    progress.enter("managed secrets");
    let temp = tempfile::tempdir().unwrap();
    let token_file = temp.path().join("token");
    std::fs::write(&token_file, "sagat-test-bootstrap").unwrap();
    let provider = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            token_file,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .unwrap(),
    );
    let service = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let ns = w.namespace.id();
    let key = Uuid::new_v4();
    let write = || ManagedSecretWrite {
        name: "db-credentials",
        value: SecretValue::new(b"credential-canary-017".to_vec()).unwrap(),
        expected_version: 0,
        request_id: key,
    };
    let before: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    let intent = service
        .prepare(&w.admin, ns, &write(), &ctx())
        .await
        .unwrap();
    let after: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(after - before, write().value.len() as i64);
    // Intent committed, process dies before any provider write: a value is needed again.
    assert!(matches!(
        service
            .reconcile(&w.admin, ns, intent.reference.operation, &ctx())
            .await,
        Err(AppError::StateConflict(_))
    ));
    // Provider accepted the write, but Sagat lost the response before activation.
    provider
        .create(&intent.reference, &write().value)
        .await
        .unwrap();
    let stored = service
        .reconcile(&w.admin, ns, intent.reference.operation, &ctx())
        .await
        .unwrap();
    assert_eq!(stored.version, 1);
    assert_eq!(
        service
            .put(&w.admin, ns, write(), &ctx())
            .await
            .unwrap()
            .version,
        1
    );
    let changed = ManagedSecretWrite {
        value: SecretValue::new(b"different-canary".to_vec()).unwrap(),
        ..write()
    };
    assert!(matches!(
        service.put(&w.admin, ns, changed, &ctx()).await,
        Err(AppError::IdempotencyKeyReused)
    ));
    assert!(matches!(
        service.put(&w.other_admin, ns, write(), &ctx()).await,
        Err(AppError::NotFound)
    ));
    let rotation = ManagedSecretWrite {
        expected_version: 1,
        request_id: Uuid::new_v4(),
        value: SecretValue::new(b"rotated-canary-017".to_vec()).unwrap(),
        ..write()
    };
    assert_eq!(
        service
            .put(&w.admin, ns, rotation, &ctx())
            .await
            .unwrap()
            .version,
        2
    );
    let listed = w.secrets.list(&w.admin, ns, &ctx()).await.unwrap();
    assert_eq!(listed[0].version, 2);
    w.workers = w.workers.with_secret_provider(provider.clone());
    let (workflow, _) = w
        .try_publish_as(
            &w.admin,
            support::engine::document(
                "managed-secrets",
                "a",
                vec![support::engine::with(
                    support::engine::activity("a"),
                    "secrets",
                    serde_json::json!(["db-credentials"]),
                )],
            ),
        )
        .await
        .unwrap();
    w.start(workflow, &serde_json::json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    let resolved = w
        .workers
        .resolve_secrets(&session.principal, &support::engine::task_ref(&task), None)
        .await
        .unwrap();
    assert_eq!(resolved[0].value.expose(), b"rotated-canary-017");
    w.complete(&session, &task, &serde_json::json!({}))
        .await
        .unwrap();
    assert!(matches!(
        w.workers
            .resolve_secrets(&session.principal, &support::engine::task_ref(&task), None)
            .await,
        Err(AppError::StaleAttempt)
    ));
    // A legacy writer cannot silently switch provider authority back to local ciphertext.
    assert!(
        w.secrets
            .put(
                &w.admin,
                ns,
                "db-credentials",
                b"local-canary".to_vec(),
                &ctx()
            )
            .await
            .is_err()
    );
    let canary_found: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM secret_provider_operations o WHERE to_jsonb(o)::text LIKE '%canary%') OR EXISTS (SELECT 1 FROM secret_versions v WHERE to_jsonb(v)::text LIKE '%canary%')")
        .fetch_one(&w.db.pool).await.unwrap();
    assert!(!canary_found);
    let sealed_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_versions WHERE ciphertext IS NOT NULL")
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(sealed_count, 0);
    w.put_secret("legacy-credential", "legacy-migration-canary")
        .await;
    w.secrets
        .grant(
            &w.admin,
            ns,
            "legacy-credential",
            "developer-original",
            &ctx(),
        )
        .await
        .unwrap();
    let service = service.with_legacy_cipher(w.keys.cipher());
    let migrated = service
        .migrate(&w.admin, ns, "legacy-credential", 1, Uuid::new_v4(), &ctx())
        .await
        .unwrap();
    assert_eq!(migrated.version, 2);
    assert_eq!(
        w.secrets
            .access(&w.admin, ns, "legacy-credential", &ctx())
            .await
            .unwrap(),
        vec!["developer-original"]
    );
    let authority: String = sqlx::query_scalar("SELECT authority FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name='legacy-credential'")
        .bind(w.admin.tenant_id().as_uuid()).bind(ns.as_uuid()).fetch_one(&w.db.pool).await.unwrap();
    assert_eq!(authority, "openbao");
    progress.enter("managed HTTP facade");
    Box::pin(managed_http_facade(provider.clone())).await;
    progress.enter("connection catalog");
    connection_catalog(&w).await;
    progress.enter("ad_hoc::run");
    Box::pin(ad_hoc::run(&w)).await;
    progress.enter("admission::run");
    Box::pin(admission::run(&w, &service)).await;
    progress.enter("cleanup::run");
    Box::pin(cleanup::run(provider.clone())).await;
    progress.enter("cleanup_retention::run");
    Box::pin(cleanup_retention::run(provider.clone())).await;
    progress.enter("key_rotation::run");
    Box::pin(key_rotation::run(provider.clone())).await;
    progress.enter("consumers::run");
    Box::pin(consumers::run(provider.clone())).await;
    progress.enter("signing_consumers::run");
    Box::pin(signing_consumers::run(provider.clone())).await;
    progress.enter("policies::run");
    Box::pin(policies::run(provider.clone())).await;
    progress.enter("executor::run");
    Box::pin(executor::run(provider.clone())).await;
    progress.enter("executor::grpc");
    Box::pin(executor::grpc(provider.clone())).await;
    progress.enter("output::run");
    Box::pin(output::run(provider.clone())).await;
    progress.enter("output_blobs::run");
    Box::pin(output_blobs::run(provider.clone())).await;

    progress.enter("membership change during a provider read");
    w.start(workflow, &serde_json::json!({})).await;
    let active_task = w.one_task(&session).await;
    let gated = Arc::new(ReadBarrier::new(provider.clone()));
    w.workers = w.workers.with_secret_provider(gated.clone());
    revoke_during_provider_read(&w, &gated, &session, &active_task).await;
    // Stale in-memory admin principals cannot replay a management receipt either.
    assert!(service.put(&w.admin, ns, write(), &ctx()).await.is_err());
    let catalog = sagat_application::database::ConnectionService::new(w.store.clone(), SystemClock);
    // The current viewer may still inspect metadata, but cannot use the stale admin object to write.
    assert!(catalog.list(&w.admin, ns, &ctx()).await.is_ok());
    let stale_write = sagat_application::database::ConnectionWrite {
        name: sagat_domain::identity::Slug::parse("stale-access").unwrap(),
        expected_revision: 0,
        request_id: Uuid::new_v4(),
        status: sagat_domain::database::ConnectionStatus::Paused,
        config: sagat_domain::database::ConnectionConfig::postgres(
            "db.example.com",
            5432,
            "orders",
            sagat_domain::database::TlsPolicy::VerifyFull { ca_pem: None },
            sagat_domain::secrets::SecretName::parse("db-credentials").unwrap(),
            sagat_domain::database::DatabaseLimits::default(),
        )
        .unwrap(),
    };
    assert!(
        catalog
            .put(&w.admin, ns, stale_write, &ctx())
            .await
            .is_err()
    );
}

/// An ACL/membership change during provider I/O must discard the value already read.
///
/// The change is made while the provider read is held at a barrier. If the call ends before it reaches
/// the provider (the lease expired, the worker binding went away), no read ever arrives and the
/// barrier would wait forever, so that outcome is a failure that says what the call returned.
///
/// `w.workers` must already read through `gated`.
async fn revoke_during_provider_read(
    w: &World,
    gated: &ReadBarrier,
    session: &support::engine::Session,
    task: &ClaimedTask,
) {
    let task_ref = support::engine::task_ref(task);
    let mut resolving = Box::pin(
        w.workers
            .resolve_secrets(&session.principal, &task_ref, None),
    );
    tokio::select! {
        () = async { gated.entered.wait().await; } => {}
        early = &mut resolving => panic!(
            "resolve_secrets returned before it read the provider ({:?}): the lease or the binding was gone, \
             so there is no read to hold",
            early.as_ref().map(Vec::len)
        ),
    }
    let revoking = async {
        sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(w.admin.subject().as_str())
            .execute(&w.db.pool)
            .await
            .unwrap();
        gated.resume.wait().await;
    };
    let (result, ()) = tokio::join!(resolving, revoking);
    assert!(matches!(result, Err(AppError::SecretUnavailable { .. })));
}

struct ReadBarrier {
    inner: Arc<dyn SecretProvider>,
    entered: tokio::sync::Barrier,
    resume: tokio::sync::Barrier,
}
impl ReadBarrier {
    fn new(inner: Arc<dyn SecretProvider>) -> Self {
        Self {
            inner,
            entered: tokio::sync::Barrier::new(2),
            resume: tokio::sync::Barrier::new(2),
        }
    }
}
impl SecretProvider for ReadBarrier {
    fn create<'a>(
        &'a self,
        reference: &'a sagat_application::secret_provider::ProviderReference,
        value: &'a SecretValue,
    ) -> sagat_application::secret_provider::ProviderFuture<'a, ()> {
        self.inner.create(reference, value)
    }
    fn read<'a>(
        &'a self,
        reference: &'a sagat_application::secret_provider::ProviderReference,
    ) -> sagat_application::secret_provider::ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async move {
            let value = self.inner.read(reference).await?;
            self.entered.wait().await;
            self.resume.wait().await;
            Ok(value)
        })
    }
    fn delete<'a>(
        &'a self,
        reference: &'a sagat_application::secret_provider::ProviderReference,
    ) -> sagat_application::secret_provider::ProviderFuture<'a, ()> {
        self.inner.delete(reference)
    }
}

async fn managed_http_facade(provider: Arc<dyn SecretProvider>) {
    let api = support::api::start_with(support::api::Options {
        secret_provider: Some(provider),
        runtime_role: true,
        ..Default::default()
    })
    .await;
    for statement in [
        "UPDATE memberships SET role=role WHERE false",
        "UPDATE namespaces SET environment=environment WHERE false",
        "UPDATE secret_access SET subject=subject WHERE false",
    ] {
        let error = sqlx::query(statement)
            .execute(&api.api_pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501"),
            "read-lock functions must not grant identity/ACL mutation"
        );
    }
    let public_execute: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_proc p CROSS JOIN LATERAL aclexplode(p.proacl) a WHERE p.proname IN ('sagat_lock_memberships','sagat_lock_namespace','sagat_lock_secret_access') AND a.grantee=0 AND a.privilege_type='EXECUTE'")
        .fetch_one(&api.db.pool).await.unwrap();
    assert_eq!(public_execute, 0);
    api.tenant("bao-http", "owner").await;
    let workspace = api.create_workspace("owner", "bao-http", "workspace").await;
    let namespace = api
        .create_namespace("owner", "bao-http", &workspace, "database", "dev")
        .await;
    let path = format!("/namespaces/{namespace}/secrets/postgres-password");
    let key = Uuid::new_v4();
    let body = serde_json::json!({"value":"http-credential-canary", "requestId": key.to_string(), "expectedVersion":0});
    for _ in 0..2 {
        let response = api
            .as_user(reqwest::Method::PUT, &path, "owner", "bao-http")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let metadata: serde_json::Value = response.json().await.unwrap();
        assert_eq!(metadata["provider"], "openbao");
        assert_eq!(metadata["version"], 1);
        assert!(metadata["keyId"].is_null());
        assert!(!metadata.to_string().contains("canary"));
    }
    let response = api
        .post_as(
            &format!("/namespaces/{namespace}/secret-operations/{key}/reconcile"),
            "owner",
            "bao-http",
            &serde_json::json!({}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let connection_path = format!("/namespaces/{namespace}/connections/orders-db");
    let connection_body = serde_json::json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"status":"PAUSED","config":{
        "driver":"postgresql","host":"db.example.com","port":5432,"database":"orders","tls":{"mode":"verify-full"},"secretName":"postgres-password"
    }});
    let contract = support::contract::OpenApi::load();
    for _ in 0..2 {
        let response = api
            .as_user(reqwest::Method::PUT, &connection_path, "owner", "bao-http")
            .json(&connection_body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let metadata: serde_json::Value = response.json().await.unwrap();
        assert_eq!(metadata["revision"], 1);
        assert_eq!(metadata["config"]["limits"]["maxRows"], 1000);
        assert!(!metadata.to_string().contains("canary"));
        contract
            .validate_body(
                "PUT",
                "/namespaces/{namespaceId}/connections/{connectionName}",
                200,
                &metadata,
            )
            .unwrap()
            .unwrap();
    }
    ad_hoc::http(&api, &namespace, connection_body.clone()).await;
    let mut bad = connection_body.clone();
    bad["config"]["password"] = serde_json::json!("must-not-accept");
    assert_eq!(
        api.as_user(reqwest::Method::PUT, &connection_path, "owner", "bao-http")
            .json(&bad)
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    let changed = serde_json::json!({"value":"changed-http-canary", "requestId":key.to_string(), "expectedVersion":0});
    let response = api
        .as_user(reqwest::Method::PUT, &path, "owner", "bao-http")
        .json(&changed)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    let response = api
        .as_user(reqwest::Method::PUT, &path, "owner", "bao-http")
        .json(&serde_json::json!({"value":"missing-receipt-canary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
}

async fn connection_catalog(w: &support::engine::World) {
    use sagat_application::database::{ConnectionService, ConnectionWrite};
    use sagat_domain::{
        database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy},
        identity::Slug,
        secrets::SecretName,
    };
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let ns = w.namespace.id();
    let name = Slug::parse("orders-db").unwrap();
    let config = ConnectionConfig::postgres(
        "db.example.com",
        5432,
        "orders",
        TlsPolicy::VerifyFull { ca_pem: None },
        SecretName::parse("db-credentials").unwrap(),
        DatabaseLimits::default(),
    )
    .unwrap();
    let write = ConnectionWrite {
        name: name.clone(),
        expected_revision: 0,
        request_id: Uuid::new_v4(),
        config: config.clone(),
        status: ConnectionStatus::Active,
    };
    let before: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    let first = catalog
        .put(&w.admin, ns, write.clone(), &ctx())
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(
        catalog
            .put(&w.admin, ns, write.clone(), &ctx())
            .await
            .unwrap(),
        first
    );
    let after: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert!(after > before);
    assert!(matches!(
        catalog.put(&w.other_admin, ns, write.clone(), &ctx()).await,
        Err(AppError::NotFound)
    ));
    let changed = ConnectionWrite {
        status: ConnectionStatus::Paused,
        ..write.clone()
    };
    assert!(matches!(
        catalog.put(&w.admin, ns, changed.clone(), &ctx()).await,
        Err(AppError::IdempotencyKeyReused)
    ));
    let second = ConnectionWrite {
        expected_revision: 1,
        request_id: Uuid::new_v4(),
        ..changed
    };
    let competing = ConnectionWrite {
        request_id: Uuid::new_v4(),
        ..second.clone()
    };
    let request_context = ctx();
    let (left, right) = tokio::join!(
        catalog.put(&w.admin, ns, second, &request_context),
        catalog.put(&w.admin, ns, competing, &request_context)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let historical = catalog
        .get(&w.admin, ns, &name, Some(1), &ctx())
        .await
        .unwrap();
    assert_eq!(historical.config, config);
    assert_eq!(historical.status, ConnectionStatus::Paused);
    assert_eq!(historical.revision, 1);
    assert!(
        sqlx::query("UPDATE database_connection_revisions SET config='{}' WHERE connection_id=$1")
            .bind(first.id.as_uuid())
            .execute(&w.db.pool)
            .await
            .is_err()
    );
    assert_eq!(catalog.list(&w.admin, ns, &ctx()).await.unwrap().len(), 1);
    let deleted = catalog
        .put(
            &w.admin,
            ns,
            ConnectionWrite {
                expected_revision: 2,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Deleted,
                ..write.clone()
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status, ConnectionStatus::Deleted);
    assert!(matches!(
        catalog.put(&w.admin, ns, write, &ctx()).await,
        Err(AppError::NotFound)
    ));
    assert!(catalog.list(&w.admin, ns, &ctx()).await.unwrap().is_empty());
}

#[tokio::test]
async fn requiring_openbao_never_silently_creates_a_local_secret() {
    let api = support::api::start().await;
    api.tenant("local-only", "owner").await;
    let ws = api
        .create_workspace("owner", "local-only", "workspace")
        .await;
    let ns = api
        .create_namespace("owner", "local-only", &ws, "database", "dev")
        .await;
    let response=api.as_user(reqwest::Method::PUT,&format!("/namespaces/{ns}/secrets/database-key"),"owner","local-only")
        .json(&serde_json::json!({"value":"no-fallback-canary","requiredProvider":"openbao","expectedVersion":0,"requestId":Uuid::new_v4().to_string()})).send().await.unwrap();
    assert_eq!(response.status(), 503);
    let response = api
        .as_user(
            reqwest::Method::GET,
            &format!("/namespaces/{ns}/secrets"),
            "owner",
            "local-only",
        )
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["items"].as_array().unwrap().is_empty());
}
