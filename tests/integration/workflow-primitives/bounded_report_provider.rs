//! Real OpenBao scan barriers for bounded inline reports; the wrapper only pauses real reads.
use std::{sync::Arc, time::Duration};

use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::ManagedSecretService,
    secret_provider::{ProviderFuture, ProviderReference, SecretProvider},
};
use sagat_domain::{identity::Permission, secrets::SecretValue};
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::{
    bounded_worker::runtime_workers,
    scoped_worker::fixture,
    support::engine::{ctx, task_ref},
};

struct PausedRead {
    inner: Arc<dyn SecretProvider>,
    entered: Notify,
    resume: Notify,
}

impl SecretProvider for PausedRead {
    fn create<'a>(
        &'a self,
        reference: &'a ProviderReference,
        value: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        self.inner.create(reference, value)
    }

    fn read<'a>(
        &'a self,
        reference: &'a ProviderReference,
    ) -> ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async move {
            let result = self.inner.read(reference).await?;
            self.entered.notify_one();
            self.resume.notified().await;
            Ok(result)
        })
    }

    fn delete<'a>(&'a self, reference: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        self.inner.delete(reference)
    }
}

#[tokio::test]
async fn role_revocation_commits_while_a_scoped_openbao_report_scan_is_paused() {
    assert_current_authority_after_scan(true, false).await;
}

#[tokio::test]
async fn binding_revocation_commits_while_a_scoped_openbao_report_scan_is_paused() {
    assert_current_authority_after_scan(false, false).await;
}

#[tokio::test]
async fn role_revocation_discards_secrets_after_a_scoped_openbao_read() {
    assert_current_authority_after_scan(true, true).await;
}

#[tokio::test]
async fn binding_revocation_discards_secrets_after_a_scoped_openbao_read() {
    assert_current_authority_after_scan(false, true).await;
}

async fn assert_current_authority_after_scan(revoke_role: bool, resolving: bool) {
    let fixture = fixture("bounded-report-provider-fence").await;
    let container = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", "sagat-test-bootstrap")
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .expect("real OpenBao");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("OpenBao port");
    let temp = tempfile::tempdir().expect("test token directory");
    let token_file = temp.path().join("token");
    std::fs::write(&token_file, "sagat-test-bootstrap").expect("test provider token");
    let provider = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            token_file,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .expect("OpenBao adapter"),
    );
    ManagedSecretService::new(
        fixture.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).expect("test intent binding key"),
    )
    .expect("managed secrets")
    .with_legacy_cipher(fixture.world.keys.cipher())
    .migrate(
        &fixture.world.admin,
        fixture.world.namespace.id(),
        "scoped-key",
        1,
        Uuid::now_v7(),
        &ctx(),
    )
    .await
    .expect("explicitly migrate the real declared secret to OpenBao");
    let paused = Arc::new(PausedRead {
        inner: provider,
        entered: Notify::new(),
        resume: Notify::new(),
    });
    let workers = Arc::new(
        runtime_workers(&fixture, 2)
            .await
            .with_secret_provider(paused.clone()),
    );
    let task = workers
        .poll(
            &fixture.session.principal,
            fixture.session.worker_id,
            &fixture.session.token,
            1,
            None,
        )
        .await
        .expect("bounded claim")
        .pop()
        .expect("one task");
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&fixture.world.db.pool)
        .await
        .expect("forbid full history on both scan phases");
    let caller = workers.clone();
    let principal = fixture.session.principal.clone();
    let report = tokio::spawn(async move {
        if resolving {
            caller
                .resolve_secrets(&principal, &task_ref(&task), None)
                .await
                .map(|_| ())
        } else {
            caller
                .complete(
                    &principal,
                    &task_ref(&task),
                    b"{}",
                    "provider-fenced-report",
                )
                .await
                .map(|_| ())
        }
    });
    tokio::time::timeout(Duration::from_secs(5), paused.entered.notified())
        .await
        .expect("the actual OpenBao read reached the barrier");
    // Both the run lock and current membership/binding locks must be released during provider I/O.
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut probe = fixture
            .world
            .db
            .pool
            .begin()
            .await
            .expect("independent run lock probe");
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(fixture.world.admin.tenant_id().as_uuid())
            .bind(fixture.run.as_uuid())
            .fetch_one(&mut *probe)
            .await
            .expect("the run lock is free while OpenBao is paused");
        probe
            .rollback()
            .await
            .expect("release the independent run lock");
        if revoke_role {
            sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
                .bind(fixture.session.principal.tenant_id().as_uuid())
                .bind(fixture.session.principal.subject().as_str())
                .execute(&fixture.world.db.pool)
                .await
                .expect("role withdrawal commits while scan is paused");
        } else {
            fixture
                .world
                .bindings
                .unbind(
                    &fixture.world.admin,
                    fixture.world.namespace.id(),
                    fixture.session.principal.subject().as_str(),
                    &ctx(),
                )
                .await
                .expect("binding withdrawal commits while scan is paused");
        }
    })
    .await
    .expect("no authority lock may be held over the provider read");
    paused.resume.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), report)
        .await
        .expect("report completes after provider release")
        .expect("report task");
    assert_eq!(
        result,
        Err(if revoke_role {
            AppError::Forbidden {
                permission: Permission::WorkerExecute,
            }
        } else {
            AppError::NotBound
        })
    );
    let completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.completed'",
    )
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("no committed completion");
    assert_eq!(completed, 0);
    let deliveries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='secret.resolved'",
    )
    .bind(fixture.session.principal.tenant_id().as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("no successful delivery was committed");
    assert_eq!(deliveries, 0);
}
