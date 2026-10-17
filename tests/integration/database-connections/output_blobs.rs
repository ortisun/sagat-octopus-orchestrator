//! Managed result bytes are scanned before staging and again before fenced completion.
use super::support::{
    api::Options,
    engine::document,
    worker::{Worker, platform_with},
};
use reqwest::Method;
use sagat_api_grpc::pb::WriteBlobHeader;
use sagat_application::{
    blobs::BlobLimits,
    secret_provider::{ProviderError, ProviderFuture, ProviderReference, SecretProvider},
};
use sagat_domain::{database::MANAGED_DATABASE_ACTIVITY, identity::Role, secrets::SecretValue};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

struct Faults {
    inner: Arc<dyn SecretProvider>,
    down: AtomicBool,
    delay_ms: AtomicU64,
}
impl SecretProvider for Faults {
    fn create<'a>(
        &'a self,
        r: &'a ProviderReference,
        v: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        self.inner.create(r, v)
    }
    fn delete<'a>(&'a self, r: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        self.inner.delete(r)
    }
    fn read<'a>(&'a self, r: &'a ProviderReference) -> ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async move {
            if self.down.load(Ordering::SeqCst) {
                return Err(ProviderError::Unavailable);
            }
            tokio::time::sleep(Duration::from_millis(self.delay_ms.load(Ordering::SeqCst))).await;
            self.inner.read(r).await
        })
    }
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let faults = Arc::new(Faults {
        inner: provider,
        down: AtomicBool::new(false),
        delay_ms: AtomicU64::new(0),
    });
    let p = platform_with(Options {
        secret_provider: Some(faults.clone()),
        blobs: Some(BlobLimits::default()),
        blob_transfer: sagat_api_grpc::BlobTransfer {
            timeout: Duration::from_secs(1),
            ..Default::default()
        },
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    let secret = json!({"username":"blob_reader","password":"blob-password-canary"}).to_string();
    assert_eq!(p.rest(Method::PUT, &format!("/namespaces/{ns}/secrets/blob-credential"),Some(&json!({
        "value":secret,"requestId":Uuid::new_v4().to_string(),"expectedVersion":0,"requiredProvider":"openbao"
    }))).await.0,200);
    assert_eq!(p.rest(Method::PUT, &format!("/namespaces/{ns}/connections/blob-db"),Some(&json!({
        "requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"status":"ACTIVE","config":{
            "driver":"postgresql","host":"db.example.com","port":5432,"database":"test",
            "tls":{"mode":"verify-full"},"secretName":"blob-credential"
        }
    }))).await.0,200);
    let mut workflows = Vec::new();
    for (name, operation, statement) in [
        ("blob-query", "query", "SELECT 1"),
        ("blob-write", "execute", "INSERT INTO orders VALUES (1)"),
    ] {
        workflows.push(p.publish(name,&document(name,"database",vec![json!({"id":"database","type":"database","connectionRef":{"name":"blob-db"},"operation":operation,"statement":statement})])).await);
    }
    p.grant(p.tenant, "database-blobs", Role::DatabaseRunner)
        .await;
    p.bind_everything("database-blobs").await;
    let mut worker = p.connect("database-blobs", "acme").await;
    worker
        .register_version(&[MANAGED_DATABASE_ACTIVITY], "1.5")
        .await;
    let result = |value: String| {
        serde_json::to_vec(&json!({"columns":[{"name":"v","type":"text"}],"rows":[[{"type":"text","value":value}]],"affectedRows":"1","truncated":false})).unwrap()
    };
    let header = || WriteBlobHeader {
        content_type: "application/json".into(),
        ..Default::default()
    };

    let run = p.start(&workflows[0], &json!({})).await;
    let lease = worker.one_task().await;
    let clean = result("x".repeat(350_000));
    assert!(
        worker
            .write_blob(&lease, header(), &clean, 50_000)
            .await
            .is_err(),
        "no credential pin, no upload"
    );
    let request = worker.req(Worker::resolve_secrets_message(&lease));
    worker.client.resolve_database(request).await.unwrap();
    assert!(
        worker
            .write_blob(
                &lease,
                WriteBlobHeader {
                    content_type: "application/blob-password-canary".into(),
                    ..Default::default()
                },
                &clean,
                50_000
            )
            .await
            .is_err()
    );
    let stored = worker
        .write_blob(&lease, header(), &clean, 50_000)
        .await
        .expect("bounded scanned upload");
    assert!(stored.size > 256 * 1024);
    faults.down.store(true, Ordering::SeqCst);
    assert!(
        worker
            .complete_with_blob(&lease, &stored.blob_id)
            .await
            .is_err()
    );
    assert_eq!(p.status(&run).await, "RUNNING");
    faults.down.store(false, Ordering::SeqCst);
    let ack = worker
        .complete_with_blob(&lease, &stored.blob_id)
        .await
        .unwrap();
    assert_eq!(ack.step_status, "SUCCEEDED");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    faults.down.store(true, Ordering::SeqCst);
    assert!(
        worker
            .complete_with_blob(&lease, &stored.blob_id)
            .await
            .unwrap()
            .already_applied
    );
    faults.down.store(false, Ordering::SeqCst);
    // Application callers get the same receipt even with no provider or codecs configured.
    let reports = sagat_application::worker::WorkerService::new(
        p.api.executions.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        Default::default(),
    );
    let principal = sagat_domain::identity::Principal::new(
        p.tenant,
        sagat_domain::identity::Subject::parse("database-blobs").unwrap(),
        Role::DatabaseRunner,
    );
    let node = sagat_domain::workflow::NodeId::parse("database").unwrap();
    let task = sagat_application::worker::TaskRef {
        run_id: lease.run_id.parse().unwrap(),
        node: &node,
        step_id: Some(lease.step_id.parse().unwrap()),
        fence: sagat_domain::execution::Fence {
            attempt: lease.attempt as u32,
            generation: lease.generation as u64,
        },
        lease_token: &lease.lease_token,
    };
    let descriptor = sagat_domain::blobs::BlobDescriptor::new(
        stored.blob_id.parse().unwrap(),
        stored.size as u64,
        sagat_domain::workflow::Digest::from_hex(&stored.sha256).unwrap(),
        sagat_domain::blobs::ContentType::parse("application/json").unwrap(),
    );
    let key = Worker::complete_message(&lease, "").idempotency_key;
    assert!(
        reports
            .complete_with_blob(
                &principal,
                &task,
                sagat_application::worker::BlobOutput {
                    descriptor: &descriptor,
                    expected_digest: "",
                    content: None,
                },
                &key
            )
            .await
            .unwrap()
            .already_applied
    );
    p.grant(p.tenant, "another-database-machine", Role::DatabaseRunner)
        .await;
    p.bind_everything("another-database-machine").await;
    let mut other = p.connect("another-database-machine", "acme").await;
    assert!(
        other
            .complete_with_blob(&lease, &stored.blob_id)
            .await
            .is_err(),
        "receipt belongs to the original machine"
    );

    for (workflow, expected) in [(&workflows[0], "FAILED"), (&workflows[1], "MANUAL_REVIEW")] {
        let run = p.start(workflow, &json!({})).await;
        let lease = worker.one_task().await;
        let request = worker.req(Worker::resolve_secrets_message(&lease));
        worker.client.resolve_database(request).await.unwrap();
        let prefix = result("blob-password-canary".into())
            .windows(20)
            .position(|w| w == b"blob-password-canary")
            .unwrap();
        let leak = result(format!(
            "{}blob-password-canary{}",
            "x".repeat(5 * 65536 - prefix - 7),
            "y".repeat(30_000)
        ));
        if expected == "FAILED" {
            sqlx::raw_sql("CREATE FUNCTION refuse_result_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='secret.denied' THEN RAISE EXCEPTION 'injected audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_result_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_result_audit();")
                .execute(&p.api.db.pool).await.unwrap();
            assert!(
                worker
                    .write_blob(&lease, header(), &leak, 65536)
                    .await
                    .is_err()
            );
            assert_eq!(
                p.status(&run).await,
                "RUNNING",
                "failed audit rolls back the rejection transition"
            );
            sqlx::raw_sql("DROP TRIGGER refuse_result_audit ON audit_records; DROP FUNCTION refuse_result_audit();").execute(&p.api.db.pool).await.unwrap();
        }
        assert!(
            worker
                .write_blob(&lease, header(), &leak, 65536)
                .await
                .is_err()
        );
        assert_eq!(p.status(&run).await, expected);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
        .fetch_one(&p.api.db.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "rejected bytes never reserve or reach the object store"
    );

    let run = p.start(&workflows[0], &json!({})).await;
    let lease = worker.one_task().await;
    let request = worker.req(Worker::resolve_secrets_message(&lease));
    worker.client.resolve_database(request).await.unwrap();
    let oversize = result("z".repeat(1_048_576));
    assert!(
        worker
            .write_blob(&lease, header(), &oversize, 100_000)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
        .fetch_one(&p.api.db.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(p.status(&run).await, "RUNNING");
    faults.delay_ms.store(1500, Ordering::SeqCst);
    let timed_out = worker
        .write_blob(&lease, header(), &clean, 50_000)
        .await
        .unwrap_err();
    assert_eq!(timed_out.code(), tonic::Code::Unavailable);
    faults.delay_ms.store(0, Ordering::SeqCst);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
        .fetch_one(&p.api.db.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "provider scan timeout occurs before storage reservation"
    );

    // Fault injection bypasses the public upload route to prove that completion never trusts an
    // uploaded descriptor as evidence that its content/metadata was scanned.
    use sagat_application::blobs::{BlobAccess, BlobScope, WriteRequest};
    use sagat_domain::{blobs::ContentType, identity::NamespaceId};
    let scope = BlobScope {
        access: BlobAccess {
            tenant_id: p.tenant,
            namespace_id: NamespaceId::new(ns.parse().unwrap()),
        },
        run_id: run.parse().unwrap(),
        step_id: lease.step_id.parse().unwrap(),
        attempt: lease.attempt as u32,
        generation: lease.generation as u64,
    };
    let service = p.api.blobs.as_ref().unwrap();
    let metadata = service
        .write(
            scope,
            "database-blobs",
            WriteRequest {
                content_type: ContentType::parse("application/blob-password-canary").unwrap(),
                expected_size: None,
                expected_digest: None,
            },
            Box::pin(tokio_stream::iter([Ok(bytes::Bytes::copy_from_slice(
                &clean,
            ))])),
        )
        .await
        .unwrap();
    assert!(
        worker
            .complete_with_blob(&lease, &metadata.id().to_string())
            .await
            .is_err()
    );
    assert_eq!(p.status(&run).await, "RUNNING");
    let leak = result("blob-password-canary".into());
    let raw = service
        .write(
            scope,
            "database-blobs",
            WriteRequest {
                content_type: ContentType::parse("application/json").unwrap(),
                expected_size: None,
                expected_digest: None,
            },
            Box::pin(tokio_stream::iter([Ok(bytes::Bytes::from(leak))])),
        )
        .await
        .unwrap();
    let ack = worker
        .complete_with_blob(&lease, &raw.id().to_string())
        .await
        .unwrap();
    assert_eq!(ack.step_status, "FAILED");
    let payloads: Vec<Vec<u8>> = sqlx::query_scalar("SELECT content FROM payloads")
        .fetch_all(&p.api.db.pool)
        .await
        .unwrap();
    assert!(
        payloads
            .iter()
            .all(|p| !String::from_utf8_lossy(p).contains("blob-password-canary"))
    );
}
