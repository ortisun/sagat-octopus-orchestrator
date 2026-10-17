//! Disclosure against the real HTTP stack, PostgreSQL catalog and retained blob bytes.
use super::support::worker::Platform;
use futures_util::StreamExt;
use reqwest::Method;
use sagat_application::blobs::{BackendFailure, BlobBackend, BlobKey, BoxFuture, ByteStream};
use sagat_domain::identity::Role;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Debug)]
pub struct ReadFaults {
    inner: Arc<dyn BlobBackend>,
    mode: AtomicU8,
    entered: Notify,
    release: Notify,
}
impl ReadFaults {
    pub fn new() -> Self {
        Self {
            inner: sagat_infra_blobstore::build_backend(
                &sagat_infra_blobstore::BlobStoreConfig::Memory,
                Default::default(),
            )
            .unwrap(),
            mode: AtomicU8::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}
impl BlobBackend for ReadFaults {
    fn provider(&self) -> &'static str {
        "memory"
    }
    fn put<'a>(
        &'a self,
        key: &'a BlobKey,
        body: ByteStream,
    ) -> BoxFuture<'a, Result<(), BackendFailure>> {
        self.inner.put(key, body)
    }
    fn delete<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<(), BackendFailure>> {
        self.inner.delete(key)
    }
    fn get<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<ByteStream, BackendFailure>> {
        Box::pin(async move {
            let stream = self.inner.get(key).await?;
            match self.mode.load(Ordering::SeqCst) {
                1 => {
                    self.entered.notify_one();
                    self.release.notified().await;
                    Ok(stream)
                }
                2 => Ok(Box::pin(stream.map(|chunk| {
                    chunk.map(|chunk| {
                        let mut bytes = chunk.to_vec();
                        if let Some(first) = bytes.first_mut() {
                            *first ^= 1;
                        }
                        bytes.into()
                    })
                })) as ByteStream),
                3 => {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(stream)
                }
                _ => Ok(stream),
            }
        })
    }
}

pub async fn check(
    p: &Platform,
    namespace: &str,
    run: &str,
    expected: &[u8],
    backend: &ReadFaults,
) {
    let path = format!("/namespaces/{namespace}/runs/{run}/database-results/database");
    let spec = super::support::contract::OpenApi::load();
    let value: serde_json::Value = serde_json::from_slice(expected).unwrap();
    spec.validate_body(
        "GET",
        "/namespaces/{namespaceId}/runs/{runId}/database-results/{nodeId}",
        200,
        &value,
    )
    .unwrap()
    .unwrap();
    let response = p.api.get_as(&path, "alice", "acme").await;
    assert_eq!(
        response.status(),
        200,
        "authorized result even while OpenBao is unavailable"
    );
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
    assert_eq!(audit_count(p, run, "database.result_read").await, 1);
    for (subject, role) in [
        ("db-developer", Role::Developer),
        ("db-viewer", Role::Viewer),
        ("db-operator", Role::Operator),
        ("db-machine", Role::DatabaseRunner),
    ] {
        p.grant(p.tenant, subject, role).await;
        let response = p.api.get_as(&path, subject, "acme").await;
        assert_eq!(response.status(), 403, "{subject}");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert!(!response.text().await.unwrap().contains(&"x".repeat(100)));
    }
    assert!(audit_count(p, run, "database.denied").await >= 4);
    for inaccessible in [
        path.replace(namespace, &Uuid::new_v4().to_string()),
        path.replace(run, &Uuid::new_v4().to_string()),
        path.replace("/database-results/database", "/database-results/missing"),
    ] {
        let response = p.api.get_as(&inaccessible, "alice", "acme").await;
        assert_eq!(response.status(), 404);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let workspace = p
        .api
        .create_workspace("alice", "acme", "result-boundary")
        .await;
    let other_namespace = p
        .api
        .create_namespace("alice", "acme", &workspace, "result-boundary", "dev")
        .await;
    assert_eq!(
        p.api
            .get_as(&path.replace(namespace, &other_namespace), "alice", "acme")
            .await
            .status(),
        404
    );
    assert_eq!(p.api.get_as(&path, "bob", "globex").await.status(), 404);

    // Generic payload inspection still returns a descriptor/structure, never the rows.
    let seq: i64 = sqlx::query_scalar(
        "SELECT seq FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='step.completed'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.parse::<Uuid>().unwrap())
    .fetch_one(&p.api.db.pool)
    .await
    .unwrap();
    let (status, structural) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/payload-inspections"),
            Some(&json!({"source":"journal","sequence":seq})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(structural["kind"], "external");
    assert!(!structural.to_string().contains(&"x".repeat(100)));

    sqlx::raw_sql("CREATE FUNCTION refuse_disclosure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='database.result_read' THEN RAISE EXCEPTION 'injected disclosure audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_disclosure BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_disclosure();")
        .execute(&p.api.db.pool).await.unwrap();
    let response = p.api.get_as(&path, "alice", "acme").await;
    assert!(response.status().is_server_error());
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert!(!response.text().await.unwrap().contains(&"x".repeat(100)));
    sqlx::raw_sql(
        "DROP TRIGGER refuse_disclosure ON audit_records; DROP FUNCTION refuse_disclosure();",
    )
    .execute(&p.api.db.pool)
    .await
    .unwrap();

    // Revocation can commit while the external object reader is blocked. The final check refuses
    // the buffered result, even though the request began as an administrator.
    p.grant(p.tenant, "result-race", Role::Admin).await;
    backend.mode.store(1, Ordering::SeqCst);
    let request = p.api.as_user(Method::GET, &path, "result-race", "acme");
    let pending = tokio::spawn(async move { request.send().await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), backend.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query(
            "UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject='result-race'",
        )
        .bind(p.tenant.as_uuid())
        .execute(&p.api.db.pool),
    )
    .await
    .expect("no store transaction across object I/O")
    .unwrap();
    backend.release.notify_one();
    let response = pending.await.unwrap();
    assert_eq!(response.status(), 404);
    assert!(!response.text().await.unwrap().contains(&"x".repeat(100)));
    assert_eq!(audit_count(p, run, "database.result_read").await, 1);

    backend.mode.store(2, Ordering::SeqCst);
    let response = p.api.get_as(&path, "alice", "acme").await;
    assert_eq!(response.status(), 503, "corrupt object never disclosed");
    assert!(!response.text().await.unwrap().contains(&"x".repeat(100)));
    backend.mode.store(3, Ordering::SeqCst);
    let response = p.api.get_as(&path, "alice", "acme").await;
    assert_eq!(response.status(), 408, "bounded object read duration");
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(audit_count(p, run, "database.result_read").await, 1);
    backend.mode.store(0, Ordering::SeqCst);
    assert_eq!(p.api.get_as(&path, "alice", "acme").await.status(), 200);
    // A lookalike output from an ordinary activity is never eligible for this view.
    let workflow = p
        .publish(
            "ordinary-result",
            &super::support::engine::document(
                "ordinary-result",
                "database",
                vec![json!({"id":"database","type":"activity","activity":"ordinary.result"})],
            ),
        )
        .await;
    let ordinary_run = p.start(&workflow, &json!({})).await;
    p.grant(p.tenant, "ordinary-results-worker", Role::Worker)
        .await;
    p.bind_everything("ordinary-results-worker").await;
    let mut ordinary = p.connect("ordinary-results-worker", "acme").await;
    ordinary.register(&["ordinary.result"]).await;
    let task = ordinary.one_task().await;
    ordinary
        .complete(
            &task,
            &json!({"columns":[],"rows":[],"affectedRows":"0","truncated":false}),
        )
        .await
        .unwrap();
    assert_eq!(
        p.api
            .get_as(&path.replace(run, &ordinary_run), "alice", "acme")
            .await
            .status(),
        404
    );
}
async fn audit_count(p: &Platform, run: &str, action: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 AND action=$3",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .bind(action)
    .fetch_one(&p.api.db.pool)
    .await
    .unwrap()
}

#[test]
fn openapi_and_retained_database_result_contracts_are_identical() {
    let api: serde_json::Value = serde_saphyr::from_str(include_str!(
        "../../../specs/001-platform-foundation/contracts/openapi.yaml"
    ))
    .unwrap();
    let mut retained: serde_json::Value = serde_json::from_str(include_str!(
        "../../../specs/017-database-connections/contracts/database-result.schema.json"
    ))
    .unwrap();
    let cell = retained.as_object_mut().unwrap().remove("$defs").unwrap()["cell"].clone();
    retained.as_object_mut().unwrap().remove("$schema");
    retained.as_object_mut().unwrap().remove("$id");
    retained["properties"]["rows"]["items"]["items"]["$ref"] =
        json!("#/components/schemas/DatabaseResultCell");
    assert_eq!(api["components"]["schemas"]["DatabaseResult"], retained);
    assert_eq!(api["components"]["schemas"]["DatabaseResultCell"], cell);
}
