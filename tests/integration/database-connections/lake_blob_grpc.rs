//! A lake runner reads the blobs of its step on the wire (feature 035, slice R, D035-144): a real control plane over real PostgreSQL and a
//! blob store, the REST API that creates the connection and publishes and starts the run, and the worker gRPC that leases the step and
//! serves `ReadBlob`. A gRPC client stands in for the lake runner.
//!
//! What the scenario of `lake_read.rs` cannot hold is the handler itself: that it asks the check that names the blob before the store is
//! touched, and that the bytes that come back are the bytes of the blob. It runs inside the lifecycle scenario of `main.rs`.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::append_blobs;
use crate::support::{
    api::Options,
    worker::{HUMAN, Platform, platform_with},
};
use bytes::Bytes;
use reqwest::Method;
use sagat_api_grpc::pb::WriteBlobHeader;
use sagat_application::{
    blobs::{BlobAccess, BlobBackend, BlobKey, BlobLimits},
    secret_provider::SecretProvider,
};
use sagat_domain::{blobs::BlobId, database::ICEBERG_WRITE_ACTIVITY, identity::Role};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const NDJSON: &str = "application/x-ndjson";

/// A committed output of the run's step, whose bytes are in the store: the row, as the owner writes it, and the object.
async fn stored(
    p: &Platform,
    backend: &Arc<dyn BlobBackend>,
    ns: &str,
    run: &str,
    content: &'static [u8],
    id: BlobId,
    object: bool,
) {
    let (tenant, namespace) = (p.tenant.as_uuid(), ns.parse::<Uuid>().expect("namespace"));
    let (step, attempt, generation): (Uuid, i32, i64) = sqlx::query_as(
        "SELECT step_id, attempt, generation FROM steps WHERE tenant_id = $1 AND run_id = $2::uuid",
    )
    .bind(tenant)
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the step of the run");
    sqlx::query(
        "INSERT INTO blobs (tenant_id, namespace_id, id, state, size_bytes, sha256, content_type, created_by, run_id, step_id, \
         attempt, generation, purpose, uploaded_at, committed_at) \
         VALUES ($1, $2, $3, 'COMMITTED', $4, $5, $6, 'lake-blob-wire', $7::uuid, $8, $9, $10, 'STEP_OUTPUT', now(), now())",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(id.as_uuid())
    .bind(i64::try_from(content.len()).expect("size"))
    .bind(Sha256::digest(content).to_vec())
    .bind(NDJSON)
    .bind(run)
    .bind(step)
    .bind(attempt)
    .bind(generation)
    .execute(&p.api.db.pool)
    .await
    .expect("the catalog row");
    let access = BlobAccess {
        tenant_id: p.tenant,
        namespace_id: ns.parse().expect("namespace id"),
    };
    if !object {
        return;
    }
    backend
        .put(
            &BlobKey::of(access, id),
            Box::pin(futures_util::stream::once(async move {
                Ok(Bytes::from_static(content))
            })),
        )
        .await
        .expect("the object");
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let backend: Arc<dyn BlobBackend> = sagat_infra_blobstore::build_backend(
        &sagat_infra_blobstore::BlobStoreConfig::Memory,
        Default::default(),
    )
    .expect("a memory blob store");
    let p = platform_with(Options {
        secret_provider: Some(provider),
        lake: true,
        blobs: Some(BlobLimits::default()),
        blob_backend: Some(backend.clone()),
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    for (name, value) in [
        (
            "blob-catalog",
            r#"{"clientId":"wire-client","clientSecret":"WIRE_CATALOG_CANARY"}"#,
        ),
        (
            "blob-storage",
            r#"{"accessKeyId":"WIRE_KEY","secretAccessKey":"WIRE_STORAGE_CANARY"}"#,
        ),
    ] {
        let (status, body) = p
            .rest(
                Method::PUT,
                &format!("/namespaces/{ns}/secrets/{name}"),
                Some(&json!({
                    "value": value, "requestId": Uuid::new_v4().to_string(),
                    "expectedVersion": 0, "requiredProvider": "openbao"
                })),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    // Every field of a blob is `pii` for the egress check, so the connection that takes blobs has that ceiling, and the administrator
    // who writes it holds the reveal grant of the namespace.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/payload-reveal-grants/{HUMAN}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/blob-warehouse"),
            Some(&json!({
                "requestId": Uuid::new_v4().to_string(), "expectedRevision": 0, "status": "ACTIVE",
                "config": {
                    "driver": "iceberg",
                    "catalog": {"type": "rest", "rest": {
                        "uri": "https://catalog.example/api", "warehouse": "analytics",
                        "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
                        "accessDelegation": "none", "remoteSigning": false
                    }},
                    "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
                    "warehouseLocations": ["s3://lake/warehouse/"],
                    "allowedModes": ["append"],
                    "egressCeiling": "pii",
                    "catalogSecret": "blob-catalog",
                    "storageSecret": "blob-storage",
                    "limits": {}
                }
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let workflow = p
        .publish(
            "lake-blob-wire",
            &append_blobs("lake-blob-wire", "blob-warehouse", &["/id", "/total"]),
        )
        .await;

    // The run names two blobs by their descriptors; the blobs are committed by the run's own step after it starts.
    static NAMED: &[u8] = b"{\"id\":\"r-1\",\"total\":3}\n{\"id\":\"r-2\",\"total\":4}\n";
    static OTHER: &[u8] = b"{\"id\":\"x-1\",\"total\":9}\n";
    let (named, unnamed) = (BlobId::generate(), BlobId::generate());
    let descriptor = Value::Array(vec![json!({"$blob": {
        "id": named.to_string(),
        "size": NAMED.len(),
        "sha256": hex(&Sha256::digest(NAMED)),
        "contentType": NDJSON,
    }})]);
    let run = p.start(&workflow, &json!({ "parts": descriptor })).await;
    stored(&p, &backend, &ns, &run, NAMED, named, true).await;
    // A blob of the same run that the step did not name, with bytes in the store: the lease did not pin it.
    stored(&p, &backend, &ns, &run, OTHER, unnamed, true).await;
    // And one whose object is gone from the store: asked for, the store would say so (a lost blob is not a missing one).
    let lost = BlobId::generate();
    stored(&p, &backend, &ns, &run, OTHER, lost, false).await;

    // The runner is an identity of the lake profile, bound to the lake activity.
    p.grant(p.tenant, "lake-blob-wire", Role::DatabaseRunner)
        .await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/lake-blob-wire"),
            Some(&json!({"activities": [ICEBERG_WRITE_ACTIVITY], "profile": "lake"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut worker = p.connect("lake-blob-wire", "acme").await;
    worker
        .register_version(&[ICEBERG_WRITE_ACTIVITY], "1.6")
        .await;
    let lease = worker.one_task().await;
    assert_eq!(lease.activity_name, ICEBERG_WRITE_ACTIVITY);
    assert_eq!(lease.run_id, run);
    let input: Value = serde_json::from_str(&lease.input_json).expect("the lease input");
    assert_eq!(
        input["records"]["blobs"],
        json!([{"id": named.to_string(), "format": "ndjson"}]),
        "the lease names the blob and nothing else"
    );

    // The named blob is read whole, and what comes back is what the catalog holds: the header and the bytes.
    let (header, content) = worker
        .read_blob(&lease, &named.to_string())
        .await
        .expect("a lake attempt reads the blob its lease named");
    assert_eq!(header.blob_id, named.to_string());
    assert_eq!(header.content_type, NDJSON);
    assert_eq!(header.size, i64::try_from(NAMED.len()).expect("size"));
    assert_eq!(header.sha256, hex(&Sha256::digest(NAMED)));
    assert_eq!(content, NAMED);

    // A blob of the run that the lease did not name has bytes and a row and is refused as one that does not exist, as is an id that
    // was never made: the same code, and the same words.
    let refused = worker
        .read_blob(&lease, &unnamed.to_string())
        .await
        .expect_err("a blob the lease did not name");
    let missing = worker
        .read_blob(&lease, &BlobId::generate().to_string())
        .await
        .expect_err("a blob that does not exist");
    assert_eq!(refused.code(), tonic::Code::NotFound, "{refused:?}");
    assert_eq!(
        (refused.code(), refused.message()),
        (missing.code(), missing.message()),
        "the answer says nothing of what exists"
    );
    // The decision is made before the store is touched: a blob the lease did not name whose object is lost is answered as the others
    // are, and not as the store would answer (which would tell that it exists).
    let unread = worker
        .read_blob(&lease, &lost.to_string())
        .await
        .expect_err("a blob the lease did not name, whose object is gone");
    assert_eq!(
        (unread.code(), unread.message()),
        (missing.code(), missing.message()),
        "the check comes before the store"
    );

    // The upload is the door that stays shut to a managed step, whatever it is given.
    let upload = worker
        .write_blob(
            &lease,
            WriteBlobHeader {
                content_type: NDJSON.into(),
                ..Default::default()
            },
            b"{}\n",
            1024,
        )
        .await
        .expect_err("a lake step cannot upload a blob");
    assert_ne!(upload.code(), tonic::Code::Ok, "{upload:?}");
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
