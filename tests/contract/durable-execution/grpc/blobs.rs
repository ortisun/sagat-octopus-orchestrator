//! Blobs over the worker protocol (FR-002-008, ADR-0023): a payload too large for a message travels
//! as a blob, and what the run keeps is a small descriptor. Real server, PostgreSQL and identity
//! provider; the object store is in memory. `crates/infra-blobstore` separately exercises local
//! memory/filesystem backends and signed requests against an in-process S3-compatible server;
//! real AWS S3 and Cloudflare R2 checks remain ignored until credentials are supplied.

use std::time::Duration;

use bytes::Bytes;
use reqwest::Method;
use sagat_api_grpc::BlobTransfer;
use sagat_api_grpc::pb::{WriteBlobHeader, WriteBlobRequest, write_blob_request};
use sagat_application::blobs::{BlobAccess, BlobKey, BlobLimits};
use sagat_domain::blobs::BlobId;
use sagat_domain::identity::{NamespaceId, Role};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Code;

use crate::support::api::Options;
use crate::support::worker::{
    Platform, Worker, chain_ir, machine_code, platform, platform_with, remediation_of,
};

fn limits() -> BlobLimits {
    BlobLimits {
        max_bytes: 4 * 1024 * 1024,
        max_pending: 8,
        upload_ttl: Duration::from_secs(24 * 3600),
        abandoned_after: Duration::from_secs(3600),
    }
}

async fn with_blobs() -> Platform {
    platform_with(Options {
        blobs: Some(limits()),
        ..Options::default()
    })
    .await
}

/// `size` bytes that are not all alike, so a truncation or a swap would show.
fn content(size: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    (0..size)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (state >> 33) as u8
        })
        .collect()
}

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn json_header() -> WriteBlobHeader {
    WriteBlobHeader {
        content_type: "application/json".into(),
        ..WriteBlobHeader::default()
    }
}

/// The `{"$blob": {...}}` object somewhere inside a step input.
fn descriptor_in(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => map
            .get("$blob")
            .or_else(|| map.values().find_map(descriptor_in)),
        Value::Array(items) => items.iter().find_map(descriptor_in),
        _ => None,
    }
}

async fn count(p: &Platform, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count")
}

/// A started chain workflow and a registered worker holding the lease of its first step.
async fn first_lease(p: &Platform) -> (Worker<'_>, sagat_api_grpc::pb::TaskLease, String) {
    let wf = p.publish("big-output", &chain_ir("big-output")).await;
    let run = p.start(&wf, &json!({"orderId": "o-1"})).await;
    let mut w = p.worker("wendy").await;
    w.register_version(&["act.a", "act.b"], "1.3").await;
    let lease = w.one_task().await;
    (w, lease, run)
}

// ------------------------------------------------------------------------------ the flow

#[tokio::test]
async fn a_large_output_travels_as_a_blob_and_the_run_keeps_only_its_descriptor() {
    let p = with_blobs().await;
    let (mut w, lease_a, run) = first_lease(&p).await;
    let data = content(1_500_000);

    // The worker uploads in chunks, stating what it expects; the server measures it independently.
    let stored = w
        .write_blob(
            &lease_a,
            WriteBlobHeader {
                expected_size: i64::try_from(data.len()).expect("fits"),
                expected_sha256: hex_sha256(&data),
                ..json_header()
            },
            &data,
            100_000,
        )
        .await
        .expect("uploaded");
    assert_eq!(stored.size, 1_500_000);
    assert_eq!(stored.sha256, hex_sha256(&data));
    assert_eq!(stored.content_type, "application/json");
    assert!(stored.blob_id.parse::<BlobId>().is_ok());

    // Until something uses it the blob is merely uploaded.
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state = 'UPLOADED'").await,
        1
    );

    // The blob becomes the step's output.
    let done = w
        .complete_with_blob(&lease_a, &stored.blob_id)
        .await
        .expect("completed with the blob");
    assert_eq!(done.step_status, "SUCCEEDED");
    assert!(!done.already_applied);
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state = 'COMMITTED'").await,
        1,
        "committed in the transaction that completed the step"
    );

    // The run kept a small descriptor, not 1.5 MB.
    let largest: i32 = sqlx::query_scalar("SELECT max(octet_length(content)) FROM payloads")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("max");
    assert!(largest < 1024, "payloads stay small: {largest} bytes");

    // The next step receives that descriptor in its input, and reads the bytes with its own lease.
    let lease_b = w.one_task().await;
    assert_eq!(lease_b.activity_name, "act.b");
    let input: Value = serde_json::from_str(&lease_b.input_json).expect("input is JSON");
    let descriptor = descriptor_in(&input).unwrap_or_else(|| panic!("no descriptor in {input}"));
    assert_eq!(descriptor["id"], stored.blob_id.as_str());
    assert_eq!(descriptor["size"], 1_500_000);
    assert_eq!(descriptor["sha256"], hex_sha256(&data).as_str());
    assert_eq!(descriptor["contentType"], "application/json");

    let (header, read) = w.read_blob(&lease_b, &stored.blob_id).await.expect("read");
    assert_eq!(read, data, "the bytes come back whole");
    assert_eq!(header.size, 1_500_000);
    assert_eq!(header.sha256, hex_sha256(&data));
    assert_eq!(header.content_type, "application/json");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM audit_records WHERE action = 'blob.read'"
        )
        .await,
        1
    );
    w.read_blob(&lease_b, &stored.blob_id)
        .await
        .expect("repeat read");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM audit_records WHERE action = 'blob.read'"
        )
        .await,
        1,
        "the same actor, attempt and blob is audited once"
    );

    let finished = w.complete(&lease_b, &json!({"ok": true})).await.expect("b");
    assert_eq!(finished.run_status, "SUCCEEDED");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn a_read_returns_no_bytes_when_its_audit_cannot_be_appended() {
    let p = with_blobs().await;
    let (mut w, lease_a, _) = first_lease(&p).await;
    let stored = w
        .write_blob(&lease_a, json_header(), b"audit before bytes", 8)
        .await
        .expect("uploaded");
    w.complete_with_blob(&lease_a, &stored.blob_id)
        .await
        .expect("used");
    let lease_b = w.one_task().await;
    sqlx::query(
        "CREATE FUNCTION reject_blob_read_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action = 'blob.read' THEN RAISE EXCEPTION 'test audit failure'; \
         END IF; RETURN NEW; END $$",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("failure function");
    sqlx::query(
        "CREATE TRIGGER reject_blob_read_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_blob_read_audit()",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("failure trigger");

    let status = w
        .read_blob(&lease_b, &stored.blob_id)
        .await
        .expect_err("audit is required before bytes");

    assert_eq!(status.code(), Code::Internal, "{status:?}");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM audit_records WHERE action = 'blob.read'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn a_blob_belongs_to_one_step_and_a_retried_report_is_answered_again() {
    let p = with_blobs().await;
    let (mut w, lease_a, _) = first_lease(&p).await;
    let data = content(300_000);
    let stored = w
        .write_blob(&lease_a, json_header(), &data, 64 * 1024)
        .await
        .expect("uploaded");

    let first = w
        .complete_with_blob(&lease_a, &stored.blob_id)
        .await
        .expect("first");
    // The same report again (a lost response): the original answer, nothing applied twice.
    let again = w
        .complete_with_blob(&lease_a, &stored.blob_id)
        .await
        .expect("again");
    assert!(!first.already_applied && again.already_applied);
    assert_eq!(first.command_id, again.command_id);
    let mut forged = lease_a.clone();
    forged.step_id = uuid::Uuid::new_v4().to_string();
    let error = w
        .complete_with_blob(&forged, &stored.blob_id)
        .await
        .expect_err("forged replay target");
    assert_eq!(machine_code(&error), "STALE_ATTEMPT");

    // Another step cannot claim the same, already used, blob.
    let lease_b = w.one_task().await;
    let stolen = w.complete_with_blob(&lease_b, &stored.blob_id).await;
    let status = stolen.expect_err("a blob is used once");
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    // The refusal changed nothing: step b still holds its lease and can finish normally.
    let finished = w.complete(&lease_b, &json!({"ok": true})).await.expect("b");
    assert_eq!(finished.run_status, "SUCCEEDED");
}

// ------------------------------------------------------------------------- what is refused

#[tokio::test]
async fn a_stated_size_or_digest_that_is_wrong_is_refused_and_leaves_nothing() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let data = content(10_000);

    let wrong_size = w
        .write_blob(
            &lease,
            WriteBlobHeader {
                expected_size: 9_999,
                ..json_header()
            },
            &data,
            4096,
        )
        .await
        .expect_err("size");
    let wrong_digest = w
        .write_blob(
            &lease,
            WriteBlobHeader {
                expected_sha256: "ab".repeat(32),
                ..json_header()
            },
            &data,
            4096,
        )
        .await
        .expect_err("digest");
    let not_hex = w
        .write_blob(
            &lease,
            WriteBlobHeader {
                expected_sha256: "zz".into(),
                ..json_header()
            },
            &data,
            4096,
        )
        .await
        .expect_err("not hex");
    let not_a_type = w
        .write_blob(
            &lease,
            WriteBlobHeader {
                content_type: "not a media type".into(),
                ..json_header()
            },
            &data,
            4096,
        )
        .await
        .expect_err("type");

    for status in [&wrong_size, &wrong_digest, &not_hex, &not_a_type] {
        assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
        assert!(
            remediation_of(status).len() >= 10,
            "says what to do: {status:?}"
        );
    }
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0,
        "no reservation left"
    );
}

#[tokio::test]
async fn a_blob_over_the_maximum_is_refused_and_leaves_nothing() {
    let p = platform_with(Options {
        blobs: Some(BlobLimits {
            max_bytes: 1024 * 1024,
            ..limits()
        }),
        ..Options::default()
    })
    .await;
    let (mut w, lease, _) = first_lease(&p).await;

    let status = w
        .write_blob(&lease, json_header(), &content(1024 * 1024 + 1), 200_000)
        .await
        .expect_err("over the maximum");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    // A declared size over the maximum is refused before any byte is read.
    let declared = w
        .write_blob(
            &lease,
            WriteBlobHeader {
                expected_size: 5 * 1024 * 1024,
                ..json_header()
            },
            b"x",
            10,
        )
        .await
        .expect_err("declared over the maximum");
    assert_eq!(declared.code(), Code::InvalidArgument);

    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0
    );
    // Exactly at the maximum is fine.
    let ok = w
        .write_blob(&lease, json_header(), &content(1024 * 1024), 200_000)
        .await
        .expect("at the maximum");
    assert_eq!(ok.size, 1024 * 1024);
}

#[tokio::test]
async fn an_attempt_with_declared_secrets_cannot_upload_until_stream_scanning_is_available() {
    let p = with_blobs().await;
    let namespace = p.namespace_id().await;
    let (stored_status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/secrets/payments-key"),
            Some(&json!({"value": "secret-value-long-enough-0001"})),
        )
        .await;
    assert_eq!(stored_status, 200);
    let (granted_status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/secrets/payments-key/access/alice"),
            None,
        )
        .await;
    assert_eq!(granted_status, 204);

    let mut ir = chain_ir("secret-blob-output");
    ir["nodes"][0]["secrets"] = json!(["payments-key"]);
    let workflow = p.publish("secret-blob-output", &ir).await;
    p.start(&workflow, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register_version(&["act.a", "act.b"], "1.3").await;
    let lease = w.one_task().await;

    let status = w
        .write_blob(&lease, json_header(), b"ordinary output", 5)
        .await
        .expect_err("fail closed before any bytes are stored");

    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(status.message().contains("content scanning"), "{status:?}");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn only_the_holder_of_a_current_lease_can_move_blobs() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let stored = w
        .write_blob(&lease, json_header(), &content(50_000), 8192)
        .await
        .expect("uploaded");

    let mut forged = lease.clone();
    forged.lease_token = "0".repeat(64);
    let write = w
        .write_blob(&forged, json_header(), b"data", 4)
        .await
        .expect_err("forged token");
    let read = w
        .read_blob(&forged, &stored.blob_id)
        .await
        .expect_err("forged token");
    for status in [&write, &read] {
        assert_eq!(status.code(), Code::Aborted, "{status:?}");
        assert_eq!(machine_code(status), "STALE_ATTEMPT");
    }

    let mut stale_attempt = lease.clone();
    stale_attempt.attempt += 1;
    let status = w
        .write_blob(&stale_attempt, json_header(), b"data", 4)
        .await
        .expect_err("forged attempt");
    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert_eq!(machine_code(&status), "STALE_ATTEMPT");
    let mut stale_generation = lease.clone();
    stale_generation.generation += 1;
    stale_generation.task_id = format!(
        "{}/{}",
        lease.task_id.rsplit_once('/').unwrap().0,
        stale_generation.generation
    );
    let status = w
        .write_blob(&stale_generation, json_header(), b"data", 4)
        .await
        .expect_err("forged generation");
    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert_eq!(machine_code(&status), "STALE_ATTEMPT");

    // Once the attempt is over its lease is no longer a capability.
    w.complete(&lease, &json!({"ok": true}))
        .await
        .expect("complete");
    let late = w
        .read_blob(&lease, &stored.blob_id)
        .await
        .expect_err("finished attempt");
    assert_eq!(late.code(), Code::Aborted, "{late:?}");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        1,
        "the forged upload stored nothing"
    );
}

#[tokio::test]
async fn an_upload_that_is_not_header_then_chunks_is_refused() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let header = |h: WriteBlobHeader| WriteBlobRequest {
        part: Some(write_blob_request::Part::Header(WriteBlobHeader {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            ..h
        })),
    };
    let chunk = |n: usize| WriteBlobRequest {
        part: Some(write_blob_request::Part::Chunk(Bytes::from(vec![7u8; n]))),
    };

    let cases: Vec<(&str, Vec<WriteBlobRequest>)> = vec![
        ("no message at all", vec![]),
        ("a chunk first", vec![chunk(10), header(json_header())]),
        (
            "an empty message first",
            vec![WriteBlobRequest { part: None }],
        ),
        (
            "a second header",
            vec![header(json_header()), chunk(10), header(json_header())],
        ),
        (
            "a chunk over the agreed size",
            vec![
                header(json_header()),
                chunk(sagat_api_grpc::BLOB_CHUNK_BYTES + 1),
            ],
        ),
        (
            "an empty message after the header",
            vec![header(json_header()), WriteBlobRequest { part: None }],
        ),
    ];
    for (label, messages) in cases {
        let status = w.write_blob_messages(messages).await.expect_err(label);
        assert_eq!(status.code(), Code::InvalidArgument, "{label}: {status:?}");
    }
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn a_chunk_of_exactly_the_agreed_size_is_accepted() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let data = content(sagat_api_grpc::BLOB_CHUNK_BYTES * 2);

    let stored = w
        .write_blob(
            &lease,
            json_header(),
            &data,
            sagat_api_grpc::BLOB_CHUNK_BYTES,
        )
        .await
        .expect("uploaded");

    assert_eq!(stored.size, i64::try_from(data.len()).expect("fits"));
}

#[tokio::test]
async fn an_empty_blob_is_a_blob() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;

    let stored = w
        .write_blob(&lease, json_header(), b"", 1)
        .await
        .expect("uploaded");
    let (header, read) = w.read_blob(&lease, &stored.blob_id).await.expect("read");

    assert_eq!((stored.size, header.size, read.len()), (0, 0, 0));
}

#[tokio::test]
async fn blobs_are_refused_where_none_is_configured() {
    let p = platform().await;
    let (mut w, lease, _) = first_lease(&p).await;

    let write = w
        .write_blob(&lease, json_header(), b"data", 4)
        .await
        .expect_err("not configured");
    let read = w
        .read_blob(&lease, &BlobId::generate().to_string())
        .await
        .expect_err("not configured");
    let complete = w
        .complete_with_blob(&lease, &BlobId::generate().to_string())
        .await
        .expect_err("not configured");

    for status in [&write, &read, &complete] {
        assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
        assert!(
            status.message().contains("not configured"),
            "says why: {status:?}"
        );
    }
}

#[tokio::test]
async fn a_report_names_the_output_inline_or_as_a_blob_never_both() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let stored = w
        .write_blob(&lease, json_header(), &content(1000), 500)
        .await
        .expect("uploaded");

    let mut both = Worker::complete_message(&lease, "{\"a\":1}");
    both.output_blob_ref = stored.blob_id.clone();
    let status = w.client.complete_step(w.req(both)).await.expect_err("both");
    assert_eq!(status.code(), Code::InvalidArgument);

    let mut wrong_digest = Worker::complete_message(&lease, "");
    wrong_digest.output_blob_ref = stored.blob_id.clone();
    wrong_digest.output_sha256 = "cd".repeat(32);
    let status = w
        .client
        .complete_step(w.req(wrong_digest))
        .await
        .expect_err("digest");
    assert_eq!(status.code(), Code::InvalidArgument);

    let mut not_an_id = Worker::complete_message(&lease, "");
    not_an_id.output_blob_ref = "blob://x".into();
    let status = w
        .client
        .complete_step(w.req(not_an_id))
        .await
        .expect_err("id");
    assert_eq!(status.code(), Code::InvalidArgument);

    // The right digest is accepted, and none of the refusals used the blob.
    let mut right = Worker::complete_message(&lease, "");
    right.output_blob_ref = stored.blob_id;
    right.output_sha256 = hex_sha256(&content(1000)).to_uppercase();
    assert!(w.client.complete_step(w.req(right)).await.is_ok());
}

// ---------------------------------------------------------------------------- isolation

/// A worker of the second tenant holding the lease of a step of its own.
async fn other_tenants_lease(p: &Platform) -> (Worker<'_>, sagat_api_grpc::pb::TaskLease) {
    let api = &p.api;
    let ws = api.create_workspace("bob", "globex", "platform").await;
    let ns = api
        .create_namespace("bob", "globex", &ws, "payments", "dev")
        .await;
    // One REST call as bob of globex.
    let call = |n: &'static str, method: Method, path: String, body: Value| {
        let request = api
            .as_user(method, &path, "bob", "globex")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", format!("globex-{n}-0001"))
            .json(&body);
        async move {
            let response = request.send().await.expect("send");
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.expect("json"))
        }
    };
    let (status, created) = call(
        "create",
        Method::POST,
        "/workflows".into(),
        json!({"name": "theirs", "ir": chain_ir("theirs")}),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();
    let (status, published) = call(
        "publish",
        Method::POST,
        format!("/workflows/{id}/publish"),
        json!({"draftRevision": 1}),
    )
    .await;
    assert_eq!(status, 201, "{published}");
    let (status, started) = call(
        "start",
        Method::POST,
        "/runs".into(),
        json!({"workflowId": id, "input": {}}),
    )
    .await;
    assert_eq!(status, 202, "{started}");

    p.grant(p.other_tenant, "gwen", Role::Worker).await;
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
         VALUES ($1, $2::text::uuid, 'gwen', '*', 'test')",
    )
    .bind(p.other_tenant.as_uuid())
    .bind(ns)
    .execute(&api.db.pool)
    .await
    .expect("bind");
    let mut w = p.connect("gwen", "globex").await;
    w.register_version(&["act.a", "act.b"], "1.3").await;
    let lease = w.one_task().await;
    (w, lease)
}

#[tokio::test]
async fn another_tenants_blob_is_exactly_as_missing_as_one_that_does_not_exist() {
    let p = with_blobs().await;
    let (mut acme, acme_lease, _) = first_lease(&p).await;
    let secret = acme
        .write_blob(&acme_lease, json_header(), &content(20_000), 4096)
        .await
        .expect("uploaded");
    let (mut globex, globex_lease) = other_tenants_lease(&p).await;

    let foreign = globex
        .read_blob(&globex_lease, &secret.blob_id)
        .await
        .expect_err("not theirs");
    let missing = globex
        .read_blob(&globex_lease, &BlobId::generate().to_string())
        .await
        .expect_err("not there");
    let claimed = globex
        .complete_with_blob(&globex_lease, &secret.blob_id)
        .await
        .expect_err("cannot become their output");

    assert_eq!(foreign.code(), Code::NotFound);
    assert_eq!(
        (foreign.code(), foreign.message()),
        (missing.code(), missing.message()),
        "indistinguishable"
    );
    assert_eq!(claimed.code(), Code::NotFound);
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state = 'UPLOADED'").await,
        1,
        "the blob is still acme's and still unused"
    );
}

// ----------------------------------------------------------------------------- integrity

#[tokio::test]
async fn content_that_changed_in_the_store_ends_the_download_with_an_error() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let data = content(700_000);
    let stored = w
        .write_blob(&lease, json_header(), &data, 100_000)
        .await
        .expect("uploaded");

    // Someone with access to the bucket changes one byte, and nothing else.
    let key = BlobKey::of(
        BlobAccess {
            tenant_id: p.tenant,
            namespace_id: p
                .namespace_id()
                .await
                .parse::<NamespaceId>()
                .expect("namespace"),
        },
        stored.blob_id.parse().expect("id"),
    );
    let mut tampered = data.clone();
    tampered[350_000] ^= 0xff;
    let backend = p.api.blob_backend.as_ref().expect("blobs on");
    backend
        .put(
            &key,
            sagat_application::blobs::byte_stream(vec![Ok(Bytes::from(tampered))]),
        )
        .await
        .expect("replaced");

    let status = w
        .read_blob(&lease, &stored.blob_id)
        .await
        .expect_err("detected");

    assert_eq!(status.code(), Code::DataLoss, "{status:?}");
    assert_eq!(machine_code(&status), "BLOB_CORRUPT");
    assert!(remediation_of(&status).len() >= 10);
}

// ---------------------------------------------------------------------------- limits

#[tokio::test]
async fn a_tenant_cannot_pile_up_unused_blobs() {
    let p = platform_with(Options {
        blobs: Some(BlobLimits {
            max_pending: 2,
            ..limits()
        }),
        ..Options::default()
    })
    .await;
    let (mut w, lease, _) = first_lease(&p).await;
    for n in 0..2 {
        w.write_blob(&lease, json_header(), format!("blob {n}").as_bytes(), 16)
            .await
            .expect("within the budget");
    }

    let status = w
        .write_blob(&lease, json_header(), b"one too many", 16)
        .await
        .expect_err("over the budget");

    assert_eq!(status.code(), Code::ResourceExhausted, "{status:?}");
    assert_eq!(machine_code(&status), "QUOTA_EXCEEDED");
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        2
    );
}

/// An upload that has sent its header and then waits for the test to go on.
async fn held_upload(
    w: &mut Worker<'_>,
    lease: &sagat_api_grpc::pb::TaskLease,
) -> (
    mpsc::Sender<WriteBlobRequest>,
    tokio::task::JoinHandle<Result<sagat_api_grpc::pb::WriteBlobResponse, tonic::Status>>,
) {
    let (tx, rx) = mpsc::channel(4);
    tx.send(WriteBlobRequest {
        part: Some(write_blob_request::Part::Header(WriteBlobHeader {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            ..json_header()
        })),
    })
    .await
    .expect("header");
    let request = w.req(ReceiverStream::new(rx));
    let mut client = w.client.clone();
    let call = tokio::spawn(async move {
        client
            .write_blob(request)
            .await
            .map(tonic::Response::into_inner)
    });
    (tx, call)
}

#[tokio::test]
async fn only_so_many_transfers_run_at_once() {
    let p = platform_with(Options {
        blobs: Some(limits()),
        blob_transfer: BlobTransfer {
            max_concurrent: 1,
            timeout: Duration::from_secs(60),
        },
        ..Options::default()
    })
    .await;
    let (mut w, lease, _) = first_lease(&p).await;
    let (tx, first) = held_upload(&mut w, &lease).await;
    // Let the server take the header and the place.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let second = w
        .write_blob(&lease, json_header(), b"data", 4)
        .await
        .expect_err("no place");
    assert_eq!(second.code(), Code::ResourceExhausted, "{second:?}");
    assert_eq!(machine_code(&second), "RATE_LIMITED");
    assert!(second.metadata().get("retry-after-ms").is_some());

    // The first one finishes, and its place is given back.
    tx.send(WriteBlobRequest {
        part: Some(write_blob_request::Part::Chunk(Bytes::from_static(
            b"hello",
        ))),
    })
    .await
    .expect("chunk");
    drop(tx);
    let done = first.await.expect("joined").expect("the first upload");
    assert_eq!(done.size, 5);
    let third = w.write_blob(&lease, json_header(), b"data", 4).await;
    assert!(third.is_ok(), "{third:?}");
}

#[tokio::test]
async fn a_transfer_that_stalls_is_cut_and_leaves_nothing() {
    let p = platform_with(Options {
        blobs: Some(limits()),
        blob_transfer: BlobTransfer {
            max_concurrent: 4,
            timeout: Duration::from_secs(1),
        },
        ..Options::default()
    })
    .await;
    let (mut w, lease, _) = first_lease(&p).await;
    let (tx, call) = held_upload(&mut w, &lease).await;
    tx.send(WriteBlobRequest {
        part: Some(write_blob_request::Part::Chunk(Bytes::from_static(b"part"))),
    })
    .await
    .expect("chunk");

    // The client then says nothing: without a time limit it would hold its place for ever.
    let status = tokio::time::timeout(Duration::from_secs(20), call)
        .await
        .expect("the server cut it")
        .expect("joined")
        .expect_err("timed out");

    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    drop(tx);
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0,
        "no reservation left"
    );
}

// ---------------------------------------------------------------------------- housekeeping

#[tokio::test]
async fn a_blob_nobody_used_is_swept_with_its_bytes() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let unused = w
        .write_blob(&lease, json_header(), &content(30_000), 8192)
        .await
        .expect("uploaded");
    let used = w
        .write_blob(&lease, json_header(), &content(40_000), 8192)
        .await
        .expect("uploaded");
    w.complete_with_blob(&lease, &used.blob_id)
        .await
        .expect("used");
    sqlx::query("UPDATE blobs SET created_at = created_at - interval '2 days', uploaded_at = uploaded_at - interval '2 days'")
        .execute(&p.api.db.pool)
        .await
        .expect("aged");

    let swept = p
        .api
        .blobs
        .as_ref()
        .expect("blobs")
        .sweep(100)
        .await
        .expect("swept");

    assert_eq!(swept, 1);
    let backend = p.api.blob_backend.as_ref().expect("blobs on");
    let access = BlobAccess {
        tenant_id: p.tenant,
        namespace_id: p
            .namespace_id()
            .await
            .parse::<NamespaceId>()
            .expect("namespace"),
    };
    let gone = BlobKey::of(access, unused.blob_id.parse().expect("id"));
    let kept = BlobKey::of(access, used.blob_id.parse().expect("id"));
    assert!(
        backend.get(&gone).await.is_err(),
        "the bytes of the unused blob are gone"
    );
    assert!(
        backend.get(&kept).await.is_ok(),
        "the used blob is untouched"
    );
    let lease_b = w.one_task().await;
    assert!(w.read_blob(&lease_b, &used.blob_id).await.is_ok());
    assert_eq!(
        w.read_blob(&lease_b, &unused.blob_id)
            .await
            .expect_err("swept")
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn inline_descriptor_and_blob_completion_cannot_share_an_idempotency_key() {
    let p = with_blobs().await;
    let (mut w, lease, _) = first_lease(&p).await;
    let stored = w
        .write_blob(&lease, json_header(), b"{}", 2)
        .await
        .expect("upload");
    let descriptor = format!(
        "{{\"$blob\":{{\"id\":\"{}\",\"size\":{},\"sha256\":\"{}\",\"contentType\":\"application/json\"}}}}",
        stored.blob_id, stored.size, stored.sha256,
    );
    let request = w.req(Worker::complete_message(&lease, &descriptor));
    w.client
        .complete_step(request)
        .await
        .expect("inline completion");
    let error = w
        .complete_with_blob(&lease, &stored.blob_id)
        .await
        .expect_err("different operation");
    assert_eq!(error.code(), Code::Aborted);
    assert_eq!(machine_code(&error), "IDEMPOTENCY_KEY_REUSED");
    assert_eq!(
        count(&p, "SELECT count(*) FROM blobs WHERE state = 'COMMITTED'").await,
        0
    );
}

#[tokio::test]
async fn rejected_blob_completion_replays_after_unused_blob_collection() {
    let p = with_blobs().await;
    let nodes: Vec<Value> = (0..17)
        .map(|i| {
            let mut node = json!({"id": format!("s{i}"), "type": "activity", "activity": "act.a"});
            if i < 16 {
                node["next"] = json!(format!("s{}", i + 1));
            }
            node
        })
        .collect();
    let definition = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "blob-budget"}, "entrypoint": "s0", "nodes": nodes
    });
    let wf = p.publish("blob-budget", &definition).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    w.client = w.client.clone().max_decoding_message_size(8 * 1024 * 1024);
    let output = json!({"p": "x".repeat(262_132)});
    assert_eq!(serde_json::to_vec(&output).unwrap().len(), 262_140);
    for _ in 0..16 {
        let lease = w.one_task().await;
        w.complete(&lease, &output)
            .await
            .expect("within retained budget");
    }
    let lease = w.one_task().await;
    let stored = w
        .write_blob(&lease, json_header(), b"{}", 2)
        .await
        .expect("upload");
    let first = w
        .complete_with_blob(&lease, &stored.blob_id)
        .await
        .expect("accepted failure");
    assert_eq!(first.run_status, "FAILED");
    sqlx::query("UPDATE blobs SET uploaded_at = uploaded_at - interval '2 days'")
        .execute(&p.api.db.pool)
        .await
        .expect("age unused blob");
    assert_eq!(p.api.blobs.as_ref().unwrap().sweep(100).await.unwrap(), 1);
    assert_eq!(
        count(
            &p,
            "SELECT count(*) FROM blobs WHERE deletion_confirmed_at IS NULL"
        )
        .await,
        0
    );
    let replay = w
        .complete_with_blob(&lease, &stored.blob_id)
        .await
        .expect("durable replay");
    assert!(replay.already_applied);
    assert_eq!(first.command_id, replay.command_id);
    assert_eq!(replay.run_status, "FAILED");
}

#[tokio::test]
async fn stalled_download_releases_tenant_and_transfer_slots_at_its_deadline() {
    let p = platform_with(Options {
        blobs: Some(limits()),
        blob_transfer: BlobTransfer {
            max_concurrent: 2,
            timeout: Duration::from_secs(2),
        },
        limits: sagat_api_http::HttpLimits {
            max_in_flight_per_tenant: 1,
            ..Default::default()
        },
        ..Options::default()
    })
    .await;
    let (mut w, lease, _) = first_lease(&p).await;
    let stored = w
        .write_blob(&lease, json_header(), &content(4 * 1024 * 1024), 64 * 1024)
        .await
        .expect("upload");
    let request = w.req(Worker::read_blob_message(&lease, &stored.blob_id));
    let mut stalled = w
        .client
        .read_blob(request)
        .await
        .expect("download")
        .into_inner();
    stalled
        .message()
        .await
        .expect("header transport")
        .expect("header");
    let refused = w
        .read_blob(&lease, &stored.blob_id)
        .await
        .expect_err("tenant slot held by stream");
    assert_eq!(refused.code(), Code::ResourceExhausted);
    // No message() calls: HTTP/2 receive flow control stops body polling on the server.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let (_, bytes) = w
        .read_blob(&lease, &stored.blob_id)
        .await
        .expect("deadline releases slot");
    assert_eq!(bytes.len(), 4 * 1024 * 1024);
    drop(stalled);
}
