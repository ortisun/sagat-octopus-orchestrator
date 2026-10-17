#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../../support/mod.rs"]
mod support;

use sagat_application::secret_provider::SecretProvider;
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
};
use sagat_domain::secrets::SecretValue;
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use std::{sync::Arc, time::Duration};
use support::engine::{ctx, world};
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
};
use uuid::Uuid;

#[tokio::test]
async fn durable_provider_intents_reconcile_without_plaintext_or_duplicate_rotation() {
    let mut w = world().await;
    let container = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", "sagat-test-bootstrap")
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(8200).await.unwrap();
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
    managed_http_facade(provider.clone()).await;

    // An ACL/membership change during provider I/O must discard the value already read.
    let gated = Arc::new(ReadBarrier {
        inner: provider.clone(),
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(gated.clone());
    let task_ref = support::engine::task_ref(&task);
    let resolving = w
        .workers
        .resolve_secrets(&session.principal, &task_ref, None);
    let revoking = async {
        gated.entered.wait().await;
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
    // Stale in-memory admin principals cannot replay a management receipt either.
    assert!(service.put(&w.admin, ns, write(), &ctx()).await.is_err());
}

struct ReadBarrier {
    inner: Arc<dyn SecretProvider>,
    entered: tokio::sync::Barrier,
    resume: tokio::sync::Barrier,
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
        ..Default::default()
    })
    .await;
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
