//! Negotiated worker capabilities are durable admission constraints, not response decoration.

use reqwest::Method;
use sagat_api_grpc::pb::WriteBlobHeader;
use sagat_application::blobs::BlobLimits;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::api::{Options, json_of};
use crate::support::worker::{Platform, Worker, platform_with, single_ir};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn platform() -> Platform {
    platform_with(Options {
        blobs: Some(BlobLimits {
            max_bytes: 4 * 1024 * 1024,
            max_pending: 8,
            ..BlobLimits::default()
        }),
        ..Options::default()
    })
    .await
}

async fn register(worker: &mut Worker<'_>, worker_name: &str, protocol: &str, accepted: &str) {
    let mut message = Worker::register_message(&["act.a"], 100);
    message.worker_name = worker_name.to_owned();
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = protocol.to_owned();
    let request = worker.req(message);
    let response = worker
        .client
        .register_worker(request)
        .await
        .expect("register")
        .into_inner();
    assert_eq!(response.accepted_protocol_version, accepted);
    worker.worker_id = response.worker_id;
    worker.session = response.session_token;
}

async fn staged_input(platform: &Platform, bytes: &[u8]) -> String {
    let response = platform
        .api
        .as_user(Method::POST, "/run-input-blobs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("x-octopus-blob-size", bytes.len())
        .header("x-octopus-blob-sha256", sha256_hex(bytes))
        .body(bytes.to_vec())
        .send()
        .await
        .expect("upload input");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("blob id").to_owned()
}

async fn start_with_blob(platform: &Platform, workflow: &str, blob: &str) -> String {
    let response = platform
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", format!("blob-run-{}", Uuid::now_v7()))
        .json(&json!({"workflowId": workflow, "inputBlobRef": blob}))
        .send()
        .await
        .expect("start blob run");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 202, "{body}");
    body["runId"].as_str().expect("run id").to_owned()
}

async fn step_state(platform: &Platform, run: &str, node: &str) -> (String, i32) {
    sqlx::query_as(
        "SELECT status, attempt FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(Uuid::parse_str(run).expect("run uuid"))
    .bind(node)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("step state")
}

#[tokio::test]
async fn mixed_workers_only_lease_run_input_blobs_to_protocol_1_3() {
    let platform = platform().await;
    let workflow = platform
        .publish("protocol-input", &single_ir("protocol-input", None))
        .await;
    let blob = staged_input(&platform, b"opaque run input").await;
    let blob_run = start_with_blob(&platform, &workflow, &blob).await;
    let inline_run = platform.start(&workflow, &json!({"ordinary": true})).await;

    let mut old = platform.worker("old-input-worker").await;
    register(&mut old, "old-input", "1.2", "1.2").await;
    let old_minor: i32 =
        sqlx::query_scalar("SELECT accepted_protocol_minor FROM worker_sessions WHERE id = $1")
            .bind(Uuid::parse_str(&old.worker_id).expect("worker uuid"))
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("stored minor");
    assert_eq!(old_minor, 2);

    let old_tasks = old.poll(64).await;
    assert_eq!(old_tasks.len(), 1);
    assert_eq!(old_tasks[0].run_id, inline_run);
    assert_eq!(
        step_state(&platform, &blob_run, "a").await,
        ("READY".into(), 0)
    );

    let mut current = platform.worker("current-input-worker").await;
    register(&mut current, "current-input", "1.3", "1.3").await;
    let blob_lease = current.one_task().await;
    assert_eq!(blob_lease.run_id, blob_run);
    assert_eq!(blob_lease.input_blob_ref, blob);

    register(&mut old, "old-input", "1.999", "1.6").await;
    let replaced_minor: i32 =
        sqlx::query_scalar("SELECT accepted_protocol_minor FROM worker_sessions WHERE id = $1")
            .bind(Uuid::parse_str(&old.worker_id).expect("worker uuid"))
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("updated minor");
    assert_eq!(
        replaced_minor, 6,
        "a reconnect persists the negotiated minor"
    );
}

#[tokio::test]
async fn committed_step_output_provenance_gates_the_downstream_task() {
    let platform = platform().await;
    let workflow = platform
        .publish(
            "protocol-output",
            &json!({
                "apiVersion": "sagat-octopus-orchestrator.io/v1",
                "kind": "Workflow",
                "metadata": {"name": "protocol-output"},
                "entrypoint": "a",
                "nodes": [
                    {"id": "a", "type": "activity", "activity": "act.a", "next": "b"},
                    {"id": "b", "type": "activity", "activity": "act.a"}
                ]
            }),
        )
        .await;
    let run = platform.start(&workflow, &json!({"ordinary": true})).await;
    let mut current = platform.worker("current-output-worker").await;
    register(&mut current, "current-output", "1.3", "1.3").await;
    let first = current.one_task().await;
    let bytes = br#"{"external":true}"#;
    let uploaded = current
        .write_blob(
            &first,
            WriteBlobHeader {
                content_type: "application/json".into(),
                expected_size: i64::try_from(bytes.len()).expect("fits"),
                expected_sha256: sha256_hex(bytes),
                ..WriteBlobHeader::default()
            },
            bytes,
            5,
        )
        .await
        .expect("upload output");
    current
        .complete_with_blob(&first, &uploaded.blob_id)
        .await
        .expect("commit output");

    let mut old = platform.worker("old-output-worker").await;
    register(&mut old, "old-output", "1.2", "1.2").await;
    assert!(old.poll(64).await.is_empty());
    assert_eq!(step_state(&platform, &run, "b").await, ("READY".into(), 0));

    let next = current.one_task().await;
    assert_eq!(next.run_id, run);
    assert_eq!(next.step_id, step_id(&platform, &run, "b").await);
    assert!(
        next.input_blob_ref.is_empty(),
        "only run input uses this field"
    );
    let rendered: Value = serde_json::from_str(&next.input_json).expect("rendered input");
    assert!(rendered.to_string().contains(&uploaded.blob_id));
}

async fn step_id(platform: &Platform, run: &str, node: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "SELECT step_id FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(Uuid::parse_str(run).expect("run uuid"))
    .bind(node)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("step id");
    id.to_string()
}

async fn mark_catalog_blob_provenance(platform: &Platform, run: &str) {
    sqlx::query(
        "INSERT INTO blobs (tenant_id, namespace_id, id, state, size_bytes, sha256, \
             content_type, created_by, run_id, step_id, attempt, generation, uploaded_at, \
             committed_at, purpose) \
         SELECT r.tenant_id, r.namespace_id, $3, 'COMMITTED', 1, $4, \
             'application/octet-stream', 'protocol-compatibility-test', r.id, s.step_id, 0, 0, \
             clock_timestamp(), clock_timestamp(), 'STEP_OUTPUT' \
         FROM runs r JOIN steps s ON s.tenant_id = r.tenant_id AND s.run_id = r.id \
         WHERE r.tenant_id = $1 AND r.id = $2 AND s.node = 'a'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(Uuid::parse_str(run).expect("run uuid"))
    .bind(Uuid::now_v7())
    .bind(vec![7_u8; 32])
    .execute(&platform.api.db.pool)
    .await
    .expect("catalog provenance");
}

#[tokio::test]
async fn incompatible_backlog_does_not_hide_later_inline_work_from_an_old_worker() {
    let platform = platform().await;
    let workflow = platform
        .publish("protocol-cap", &single_ir("protocol-cap", None))
        .await;
    let mut incompatible = Vec::new();
    for n in 0..65 {
        let run = platform.start(&workflow, &json!({"backlog": n})).await;
        mark_catalog_blob_provenance(&platform, &run).await;
        incompatible.push(Uuid::parse_str(&run).expect("run uuid"));
    }
    // Descriptor-shaped JSON has no catalog provenance and remains valid inline work for 1.2.
    let inline = platform
        .start(
            &workflow,
            &json!({"$blob": {"id": Uuid::now_v7().to_string(), "size": 1}}),
        )
        .await;

    let mut old = platform.worker("old-backlog-worker").await;
    register(&mut old, "old-backlog", "1.2", "1.2").await;
    let tasks = old.poll(64).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].run_id, inline);

    let untouched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = ANY($2) \
         AND status = 'READY' AND attempt = 0",
    )
    .bind(platform.tenant.as_uuid())
    .bind(&incompatible)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("untouched backlog");
    assert_eq!(untouched, 65, "incompatible work consumes no attempt");
}
