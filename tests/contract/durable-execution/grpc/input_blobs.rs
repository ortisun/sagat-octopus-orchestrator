//! A raw REST upload becomes a lease-readable claim-check only through an atomic run start.

use crate::support::api::{Options, json_of};
use crate::support::worker::{Platform, chain_ir, platform_with};
use reqwest::{Method, RequestBuilder};
use sagat_application::blobs::BlobLimits;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tonic::Code;

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

fn upload(p: &Platform, who: &str, namespace: &str, data: Vec<u8>) -> RequestBuilder {
    p.api
        .as_user(Method::POST, "/run-input-blobs", who, "acme")
        .header("x-octopus-namespace", namespace)
        .body(data)
}

async fn staged(p: &Platform, data: &[u8]) -> Value {
    let response = upload(p, "alice", "payments", data.to_vec())
        .header("x-octopus-blob-size", data.len())
        .header(
            "x-octopus-blob-sha256",
            sagat_domain::workflow::Digest::new(Sha256::digest(data).into()).to_hex(),
        )
        .send()
        .await
        .expect("upload");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 201, "{body}");
    body
}

fn start(p: &Platform, who: &str, workflow: &str, id: &str, key: &str) -> RequestBuilder {
    p.api
        .as_user(Method::POST, "/runs", who, "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", key)
        .json(&json!({"workflowId":workflow,"inputBlobRef":id}))
}

async fn count(p: &Platform, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn large_input_is_private_until_start_then_lease_readable_and_deduplicated() {
    let p = platform().await;
    let wf = p.publish("blob-input", &chain_ir("blob-input")).await;
    p.start(&wf, &json!({})).await;
    let mut worker = p.worker("reader").await;
    worker.register(&["act.a", "act.b"]).await;
    let unrelated = worker.one_task().await;
    let bytes = vec![b'z'; 1_500_000];
    let blob = staged(&p, &bytes).await;
    let id = blob["id"].as_str().expect("id");
    assert_eq!(
        worker
            .read_blob(&unrelated, id)
            .await
            .expect_err("unbound input")
            .code(),
        Code::NotFound
    );
    let response = start(&p, "alice", &wf, id, "start-input-0001")
        .send()
        .await
        .expect("start");
    let status = response.status();
    let run = json_of(response).await;
    assert_eq!(status, 202, "{run}");
    let lease = worker.one_task().await;
    assert_eq!(lease.input_blob_ref, id);
    let rendered: Value = serde_json::from_str(&lease.input_json).expect("json");
    assert!(rendered.to_string().contains(id));
    assert!(lease.input_json.len() < 1024);
    assert_eq!(
        worker.read_blob(&lease, id).await.expect("bound input").1,
        bytes
    );
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE purpose='RUN_INPUT' AND state='COMMITTED'"
        )
        .await,
        1
    );
    let replay = start(&p, "alice", &wf, id, "start-input-0001")
        .send()
        .await
        .expect("replay");
    assert_eq!(replay.status(), 202);
    assert_eq!(json_of(replay).await["runId"], run["runId"]);
    assert_eq!(
        start(&p, "alice", &wf, id, "another-input-key")
            .send()
            .await
            .expect("single use")
            .status(),
        404
    );
    // A finished lease never becomes a lasting read capability.
    worker.complete(&lease, &json!({})).await.expect("complete");
    assert!(worker.read_blob(&lease, id).await.is_err());
    let next = worker.one_task().await;
    assert_eq!(next.input_blob_ref, id);
    assert_eq!(
        worker.read_blob(&next, id).await.expect("next lease").1,
        bytes
    );
    worker.complete(&next, &json!({})).await.expect("finish");
    let run_id = run["runId"].as_str().expect("run");
    let (status, rerun) = p
        .rest(
            Method::POST,
            &format!("/runs/{run_id}/operations"),
            Some(&json!({"action":"RERUN_AS_NEW","reason":"verify inherited blob"})),
        )
        .await;
    assert_eq!(status, 202, "{rerun}");
    let rerun_lease = worker.one_task().await;
    assert_eq!(rerun_lease.input_blob_ref, id);
    assert_eq!(
        worker
            .read_blob(&rerun_lease, id)
            .await
            .expect("rerun read")
            .1,
        bytes
    );
}

#[tokio::test]
async fn replay_precedes_active_version_and_expired_deadline_and_catalog_checks() {
    let p = platform().await;
    let wf = p.publish("stable-start", &chain_ir("stable-start")).await;
    let blob = staged(&p, b"stable").await;
    let id = blob["id"].as_str().expect("id");
    let deadline = (time::OffsetDateTime::now_utc() + time::Duration::seconds(2))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("deadline");
    let request = json!({"workflowId":wf,"inputBlobRef":id,"deadline":deadline});
    let call = || {
        p.api
            .as_user(Method::POST, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", "stable-input-key")
            .json(&request)
    };
    let first = call().send().await.expect("first");
    assert_eq!(first.status(), 202);
    let first = json_of(first).await;
    sqlx::query("UPDATE workflows SET active_version_id=NULL WHERE id=$1")
        .bind(uuid::Uuid::parse_str(&wf).expect("workflow"))
        .execute(&p.api.db.pool)
        .await
        .expect("deactivate");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let replay = call().send().await.expect("replay");
    let status = replay.status();
    let replay = json_of(replay).await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["runId"], first["runId"]);
}

#[tokio::test]
async fn child_inherits_catalog_provenance_only_when_it_inherits_the_input() {
    let p = platform().await;
    p.publish("input-child", &chain_ir("input-child")).await;
    let ir = json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
        "metadata":{"name":"input-parent"},"entrypoint":"child","nodes":[
            {"id":"child","type":"child","workflowId":"input-child"}]});
    let wf = p.publish("input-parent", &ir).await;
    let blob = staged(&p, b"child input").await;
    let id = blob["id"].as_str().expect("id");
    let response = start(&p, "alice", &wf, id, "child-input-key")
        .send()
        .await
        .expect("start");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 202, "{body}");
    let report = p.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    let mut worker = p.worker("child-reader").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    assert_eq!(lease.input_blob_ref, id);
    assert_eq!(
        worker.read_blob(&lease, id).await.expect("child read").1,
        b"child input"
    );
}

#[tokio::test]
async fn uploader_scope_schema_and_mode_fail_without_binding_or_creating_a_run() {
    let p = platform().await;
    let wf = p.publish("boundaries", &chain_ir("boundaries")).await;
    p.grant(p.tenant, "other-admin", Role::Admin).await;
    p.grant(p.tenant, "viewer", Role::Viewer).await;
    assert_eq!(
        upload(&p, "viewer", "payments", vec![1])
            .send()
            .await
            .expect("denied")
            .status(),
        403
    );
    let blob = staged(&p, b"private").await;
    let id = blob["id"].as_str().expect("id");
    assert_eq!(
        start(&p, "other-admin", &wf, id, "wrong-uploader")
            .send()
            .await
            .expect("denied")
            .status(),
        404
    );
    let ws = p.api.create_workspace("alice", "acme", "second").await;
    p.api
        .create_namespace("alice", "acme", &ws, "other", "dev")
        .await;
    let wrong_scope = upload(&p, "alice", "other", b"elsewhere".to_vec())
        .send()
        .await
        .expect("upload");
    let wrong_scope = json_of(wrong_scope).await;
    assert_eq!(
        start(
            &p,
            "alice",
            &wf,
            wrong_scope["id"].as_str().expect("id"),
            "wrong-namespace"
        )
        .send()
        .await
        .expect("denied")
        .status(),
        404
    );
    let other_ws = p.api.create_workspace("bob", "globex", "second").await;
    p.api
        .create_namespace("bob", "globex", &other_ws, "payments", "dev")
        .await;
    let response = p
        .api
        .as_user(Method::POST, "/runs", "bob", "globex")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "wrong-tenant")
        .json(&json!({"workflowId":wf,"inputBlobRef":id}))
        .send()
        .await
        .expect("tenant");
    assert_eq!(response.status(), 404);
    for body in [
        json!({"workflowId":wf,"input":null,"inputBlobRef":id}),
        json!({"workflowId":wf,"input":{},"inputBlobRef":null}),
        json!({"workflowId":wf}),
    ] {
        assert_eq!(p.rest(Method::POST, "/runs", Some(&body)).await.0, 422);
    }
    let mut ir = chain_ir("schema-input");
    ir["inputSchema"] = json!({"type":"object","required":["customerId"],"properties":{"customerId":{"type":"string"}}});
    let schema_wf = p.publish("schema-input", &ir).await;
    assert_eq!(
        start(&p, "alice", &schema_wf, id, "schema-failed-01")
            .send()
            .await
            .expect("schema")
            .status(),
        422
    );
    assert_eq!(count(&p, "SELECT count(*) FROM runs").await, 0);
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state='UPLOADED'").await,
        2
    );
    let mut good = chain_ir("schema-descriptor");
    good["inputSchema"] = json!({"type":"object","required":["$blob"],"properties":{"$blob":{"type":"object","required":["id","size","sha256","contentType"]}}});
    let good = p.publish("schema-descriptor", &good).await;
    assert_eq!(
        start(&p, "alice", &good, id, "schema-accept-01")
            .send()
            .await
            .expect("schema")
            .status(),
        202
    );
}

#[tokio::test]
async fn concurrent_same_start_returns_one_run_and_audit_failure_rolls_back_the_binding() {
    let p = platform().await;
    let wf = p.publish("atomic-input", &chain_ir("atomic-input")).await;
    let blob = staged(&p, b"atomic").await;
    let id = blob["id"].as_str().expect("id");
    sqlx::query("CREATE FUNCTION reject_run_start_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='run.started' THEN RAISE EXCEPTION 'test audit failure'; END IF; RETURN NEW; END $$")
        .execute(&p.api.db.pool).await.expect("function");
    sqlx::query("CREATE TRIGGER reject_run_start_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION reject_run_start_audit()")
        .execute(&p.api.db.pool).await.expect("trigger");
    let response = start(&p, "alice", &wf, id, "atomic-input-01")
        .send()
        .await
        .expect("failed");
    assert!(response.status().is_server_error(), "{}", response.status());
    assert_eq!(count(&p, "SELECT count(*) FROM runs").await, 0);
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE state='UPLOADED' AND run_id IS NULL"
        )
        .await,
        1
    );
    sqlx::query("DROP TRIGGER reject_run_start_audit ON audit_records")
        .execute(&p.api.db.pool)
        .await
        .expect("drop");
    let (a, b) = tokio::join!(
        start(&p, "alice", &wf, id, "atomic-input-01").send(),
        start(&p, "alice", &wf, id, "atomic-input-01").send()
    );
    let a = a.expect("a");
    let b = b.expect("b");
    let sa = a.status();
    let sb = b.status();
    let a = json_of(a).await;
    let b = json_of(b).await;
    assert_eq!(sa, 202, "{a}");
    assert_eq!(sb, 202, "{b}");
    assert_eq!(a["runId"], b["runId"]);
    assert_eq!(count(&p, "SELECT count(*) FROM runs").await, 1);
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM audit_records WHERE action='run.started'"
        )
        .await,
        1
    );
    let alias = p
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "atomic-input-01")
        .json(&json!({"workflowId":wf,"input":{"$blob":blob}}))
        .send()
        .await
        .expect("alias");
    assert_eq!(alias.status(), 409);
}

#[tokio::test]
async fn uploads_enforce_size_digest_normal_body_limit_and_expire_unused_inputs() {
    let p = platform().await;
    let mismatch = upload(&p, "alice", "payments", b"bytes".to_vec())
        .header("x-octopus-blob-size", "6")
        .send()
        .await
        .expect("mismatch");
    assert_eq!(mismatch.status(), 422);
    let mismatch = upload(&p, "alice", "payments", b"bytes".to_vec())
        .header("x-octopus-blob-sha256", "00".repeat(32))
        .send()
        .await
        .expect("mismatch");
    assert_eq!(mismatch.status(), 422);
    assert_eq!(
        upload(&p, "alice", "payments", vec![0; 4 * 1024 * 1024 + 1])
            .send()
            .await
            .expect("limit")
            .status(),
        413
    );
    let wf = p
        .publish("ordinary-limit", &chain_ir("ordinary-limit"))
        .await;
    assert_eq!(
        p.rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId":wf,"input":{"large":"x".repeat(300_000)}}))
        )
        .await
        .0,
        422
    );
    let blob = staged(&p, b"unused").await;
    let id = blob["id"].as_str().expect("id");
    sqlx::query("UPDATE blobs SET uploaded_at=clock_timestamp()-interval '2 days' WHERE id=$1")
        .bind(uuid::Uuid::parse_str(id).expect("uuid"))
        .execute(&p.api.db.pool)
        .await
        .expect("age");
    p.api
        .blobs
        .as_ref()
        .expect("blobs")
        .sweep(32)
        .await
        .expect("sweep");
    assert_eq!(
        start(&p, "alice", &wf, id, "expired-input")
            .send()
            .await
            .expect("expired")
            .status(),
        404
    );
    // An ordinary JSON descriptor is ordinary JSON, not catalog-backed input provenance.
    p.start(&wf, &json!({"$blob":blob})).await;
    let mut worker = p.worker("ordinary-worker").await;
    worker.register(&["act.a"]).await;
    assert!(worker.one_task().await.input_blob_ref.is_empty());
}

#[tokio::test]
async fn stalled_chunked_upload_times_out_and_releases_capacity_and_oversize_is_typed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let p = platform_with(Options {
        blobs: Some(BlobLimits {
            max_bytes: 64 * 1024,
            ..BlobLimits::default()
        }),
        blob_transfer: sagat_api_grpc::BlobTransfer {
            max_concurrent: 1,
            timeout: std::time::Duration::from_secs(2),
        },
        limits: sagat_api_http::HttpLimits {
            body_idle_timeout: std::time::Duration::from_millis(100),
            ..sagat_api_http::HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    let address = p
        .api
        .base
        .strip_prefix("http://")
        .expect("http")
        .split('/')
        .next()
        .expect("address");
    let token = p.api.idp.token("alice", "acme").build();
    let headers = format!(
        "POST /v1/run-input-blobs HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nX-Octopus-Namespace: payments\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    let mut socket = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    socket.write_all(headers.as_bytes()).await.expect("headers");
    socket.write_all(b"1\r\nx\r\n").await.expect("first chunk");
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(4),
        socket.read_to_end(&mut response),
    )
    .await
    .expect("bounded")
    .expect("response");
    let response = String::from_utf8(response).expect("utf8");
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state='COMMITTED'").await,
        0
    );
    staged(&p, b"slot released").await;
    let mut socket = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    socket.write_all(headers.as_bytes()).await.expect("headers");
    let bytes = vec![b'x'; 64 * 1024 + 1];
    socket
        .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
        .await
        .expect("chunk length");
    socket.write_all(&bytes).await.expect("bytes");
    let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(4),
        socket.read_to_end(&mut response),
    )
    .await
    .expect("bounded")
    .expect("response");
    let response = String::from_utf8(response).expect("utf8");
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
}
