//! One contract, every provider (FR-002-008, ADR-0023).
//!
//! The same assertions run against memory, a local directory and a real S3-protocol server that
//! verifies AWS Signature V4 (`s3s-fs`, in this process), configured once the way an AWS bucket is
//! and once the way a Cloudflare R2 bucket is (region `auto`, path-style, an explicit endpoint).
//! The in-process server receives the 12 MiB case as one signed request: `s3s-fs` stores multipart
//! metadata in a single filename and cannot represent the production tenant/namespace key without
//! exceeding common filename limits. Multipart remains covered only by the ignored real-provider
//! checks; this suite does not claim they have run.
//!
//! What this does NOT prove: that real AWS S3 or real Cloudflare R2 accept these requests. That needs
//! their credentials; the `#[ignore]`d tests at the end do it when `SAGAT_TEST_S3_*` /
//! `SAGAT_TEST_R2_*` are set, and nobody has run them in this repository yet.
#![allow(clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use hyper_util::rt::TokioIo;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use sagat_application::blobs::{
    BackendFailure, BlobBackend, BlobIoError, BlobKey, ByteStream, byte_stream,
};
use sagat_domain::blobs::BlobId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_infra_blobstore::{
    BlobStoreConfig, Credentials, PartSize, R2Config, S3Config, Secret, build_backend,
};
use tokio::net::TcpListener;

const ACCESS_KEY: &str = "AKIATESTKEY0000000";
const SECRET_KEY: &str = "test-secret-key-0123456789";
const BUCKET: &str = "sagat-blobs";
const LOCAL_PROTOCOL_PART_BYTES: usize = 64 * 1024 * 1024;

/// Logs of the adapter in the test output, for the tests that fail: `RUST_LOG=warn cargo test`.
fn logs() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}

fn key() -> BlobKey {
    logs();
    BlobKey::of(
        sagat_application::blobs::BlobAccess {
            tenant_id: TenantId::generate(),
            namespace_id: NamespaceId::generate(),
        },
        BlobId::generate(),
    )
}

fn body(chunks: Vec<Result<Bytes, BlobIoError>>) -> ByteStream {
    byte_stream(chunks)
}

fn bytes_of(size: usize, seed: u8) -> Vec<u8> {
    (0..size)
        .map(|i| seed.wrapping_add((i % 251) as u8))
        .collect()
}

fn chunks_of(data: &[u8], chunk: usize) -> Vec<Result<Bytes, BlobIoError>> {
    data.chunks(chunk)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect()
}

async fn read_all(backend: &dyn BlobBackend, key: &BlobKey) -> Result<Vec<u8>, BackendFailure> {
    let mut stream = backend.get(key).await?;
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.map_err(BackendFailure::Body)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------- the contract itself

async fn contract(backend: &dyn BlobBackend) {
    // A small blob: one request.
    let small = bytes_of(300 * 1024, 1);
    let k = key();
    backend
        .put(&k, body(chunks_of(&small, 64 * 1024)))
        .await
        .expect("put small");
    assert_eq!(read_all(backend, &k).await.expect("get small"), small);

    // A blob larger than a part: several, still streamed, still intact.
    let large = bytes_of(12 * 1024 * 1024 + 17, 9);
    let k_large = key();
    backend
        .put(&k_large, body(chunks_of(&large, 1024 * 1024)))
        .await
        .expect("put large");
    assert_eq!(read_all(backend, &k_large).await.expect("get large"), large);

    // The same key written again replaces the object (a retry of an upload is harmless).
    let replacement = bytes_of(1000, 77);
    backend
        .put(&k, body(chunks_of(&replacement, 1000)))
        .await
        .expect("overwrite");
    assert_eq!(read_all(backend, &k).await.expect("get"), replacement);

    // An empty body is a valid, empty object.
    let k_empty = key();
    backend
        .put(&k_empty, body(Vec::new()))
        .await
        .expect("put empty");
    assert!(
        read_all(backend, &k_empty)
            .await
            .expect("get empty")
            .is_empty()
    );

    // A key that was never written is not found, and says so with the right failure.
    assert_eq!(
        backend.get(&key()).await.err(),
        Some(BackendFailure::NotFound)
    );

    // Deleting removes the object, and deleting what is not there is not an error.
    backend.delete(&k).await.expect("delete");
    backend.delete(&k).await.expect("delete again");
    backend.delete(&key()).await.expect("delete unknown");
    assert_eq!(backend.get(&k).await.err(), Some(BackendFailure::NotFound));
}

async fn failed_uploads_leave_nothing(backend: &dyn BlobBackend) {
    // A body that fails after more than a part was sent: the upload is abandoned, no object appears,
    // and the producer's failure (not a store failure) is what comes back.
    let data = bytes_of(7 * 1024 * 1024, 3);
    let mut chunks = chunks_of(&data, 1024 * 1024);
    chunks.push(Err(BlobIoError::Interrupted));
    let k = key();

    let result = backend.put(&k, body(chunks)).await;

    assert_eq!(result, Err(BackendFailure::Body(BlobIoError::Interrupted)));
    assert_eq!(backend.get(&k).await.err(), Some(BackendFailure::NotFound));

    // The same for a body that fails before the first part is full.
    let k2 = key();
    let result = backend
        .put(
            &k2,
            body(vec![
                Ok(Bytes::from_static(b"some")),
                Err(BlobIoError::TooLarge),
            ]),
        )
        .await;
    assert_eq!(result, Err(BackendFailure::Body(BlobIoError::TooLarge)));
    assert_eq!(backend.get(&k2).await.err(), Some(BackendFailure::NotFound));
}

#[tokio::test]
async fn memory_honours_the_contract() {
    let backend = build_backend(&BlobStoreConfig::Memory, PartSize::default()).expect("memory");
    assert_eq!(backend.provider(), "memory");
    contract(backend.as_ref()).await;
    failed_uploads_leave_nothing(backend.as_ref()).await;
}

#[tokio::test]
async fn a_local_directory_honours_the_contract() {
    let root = scratch_dir("fs");
    let backend = build_backend(
        &BlobStoreConfig::Fs { root: root.clone() },
        PartSize::default(),
    )
    .expect("fs");
    assert_eq!(backend.provider(), "fs");
    contract(backend.as_ref()).await;
    failed_uploads_leave_nothing(backend.as_ref()).await;
    let _ = std::fs::remove_dir_all(root);
}

// ------------------------------------------------------------------ an S3-protocol server

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sagat-blobstore-{label}-{}", BlobId::generate()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A real S3 server over a directory that checks AWS Signature V4 on every request.
struct S3Server {
    addr: SocketAddr,
    root: PathBuf,
}

impl S3Server {
    async fn start() -> Self {
        let root = scratch_dir("s3");
        std::fs::create_dir_all(root.join(BUCKET)).expect("bucket");
        let fs = s3s_fs::FileSystem::new(&root).expect("file system");
        let mut builder = S3ServiceBuilder::new(fs);
        builder.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
        let service = builder.build();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let service = service.clone();
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self { addr, root }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Objects the server really holds, to check that a failed upload left none.
    fn object_count(&self) -> usize {
        fn walk(dir: &std::path::Path) -> usize {
            std::fs::read_dir(dir).map_or(0, |entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| {
                        let path = e.path();
                        if path.is_dir() {
                            walk(&path)
                        } else if path.file_name().is_some_and(|n| {
                            n.to_string_lossy().starts_with(".")
                                || n.to_string_lossy().ends_with(".metadata.json")
                        }) {
                            0
                        } else {
                            1
                        }
                    })
                    .sum()
            })
        }
        walk(&self.root.join(BUCKET))
    }
}

impl Drop for S3Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn credentials(secret: &str) -> Credentials {
    Credentials {
        access_key_id: ACCESS_KEY.to_owned(),
        secret_access_key: Secret::new(secret),
        session_token: None,
    }
}

fn as_aws_s3(server: &S3Server) -> BlobStoreConfig {
    BlobStoreConfig::S3(S3Config {
        bucket: BUCKET.to_owned(),
        region: "eu-west-1".to_owned(),
        endpoint: Some(server.endpoint()),
        credentials: Some(credentials(SECRET_KEY)),
        path_style: true,
        allow_http: true,
        prefix: None,
    })
}

fn as_cloudflare_r2(server: &S3Server) -> BlobStoreConfig {
    BlobStoreConfig::R2(R2Config {
        account_id: "0123456789abcdef0123456789abcdef".to_owned(),
        bucket: BUCKET.to_owned(),
        credentials: credentials(SECRET_KEY),
        jurisdiction: None,
        // Only a test points R2 at another address; production derives it from the account.
        endpoint_override: Some(server.endpoint()),
        prefix: None,
    })
}

#[tokio::test]
async fn s3_honours_the_contract_with_signed_requests() {
    let server = S3Server::start().await;
    let backend = build_backend(
        &as_aws_s3(&server),
        PartSize::new(LOCAL_PROTOCOL_PART_BYTES).expect("valid test part size"),
    )
    .expect("s3");
    assert_eq!(backend.provider(), "s3");

    contract(backend.as_ref()).await;
    let kept = server.object_count();
    assert!(
        kept >= 2,
        "the contract leaves the large and the empty object: {kept}"
    );
    failed_uploads_leave_nothing(backend.as_ref()).await;

    assert_eq!(
        server.object_count(),
        kept,
        "an abandoned upload must not leave an object (or a part) in the bucket"
    );
}

#[tokio::test]
async fn cloudflare_r2_honours_the_contract_with_signed_requests() {
    let server = S3Server::start().await;
    let backend = build_backend(
        &as_cloudflare_r2(&server),
        PartSize::new(LOCAL_PROTOCOL_PART_BYTES).expect("valid test part size"),
    )
    .expect("r2");
    assert_eq!(backend.provider(), "r2");

    contract(backend.as_ref()).await;
    failed_uploads_leave_nothing(backend.as_ref()).await;
}

#[tokio::test]
async fn a_prefix_keeps_two_deployments_apart_inside_one_bucket() {
    let server = S3Server::start().await;
    let mut staging = as_aws_s3(&server);
    let mut production = as_aws_s3(&server);
    if let BlobStoreConfig::S3(config) = &mut staging {
        config.prefix = Some("staging".to_owned());
    }
    if let BlobStoreConfig::S3(config) = &mut production {
        config.prefix = Some("production".to_owned());
    }
    let staging = build_backend(&staging, PartSize::default()).expect("staging");
    let production = build_backend(&production, PartSize::default()).expect("production");
    let k = key();

    staging
        .put(&k, body(chunks_of(b"staging data", 100)))
        .await
        .expect("put");

    assert_eq!(
        read_all(staging.as_ref(), &k).await.expect("get"),
        b"staging data"
    );
    assert_eq!(
        production.get(&k).await.err(),
        Some(BackendFailure::NotFound),
        "the same key under another prefix is another object"
    );
}

#[tokio::test]
async fn wrong_credentials_are_refused_not_retried_forever() {
    let server = S3Server::start().await;
    let mut config = as_aws_s3(&server);
    if let BlobStoreConfig::S3(s3) = &mut config {
        s3.credentials = Some(credentials("not-the-secret"));
    }
    let backend = build_backend(&config, PartSize::default()).expect("builds");

    let put = backend.put(&key(), body(chunks_of(b"data", 10))).await;
    let get = backend.get(&key()).await.err();

    assert_eq!(
        put,
        Err(BackendFailure::Refused),
        "a retry cannot cure this"
    );
    assert_eq!(get, Some(BackendFailure::Refused));
}

#[tokio::test]
async fn a_store_that_cannot_be_reached_is_unavailable() {
    // Nothing listens on this port: take one and close it.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let config = BlobStoreConfig::S3(S3Config {
        bucket: BUCKET.to_owned(),
        region: "eu-west-1".to_owned(),
        endpoint: Some(format!("http://127.0.0.1:{port}")),
        credentials: Some(credentials(SECRET_KEY)),
        path_style: true,
        allow_http: true,
        prefix: None,
    });
    let backend = build_backend(&config, PartSize::default()).expect("builds");

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        backend.put(&key(), body(chunks_of(b"data", 10))),
    )
    .await
    .expect("does not hang");

    assert_eq!(result, Err(BackendFailure::Unavailable));
}

#[tokio::test]
async fn a_backend_is_usable_from_many_tasks_at_once() {
    let server = S3Server::start().await;
    let backend: Arc<dyn BlobBackend> =
        build_backend(&as_aws_s3(&server), PartSize::default()).expect("s3");

    let mut tasks = Vec::new();
    for n in 0..8u8 {
        let backend = Arc::clone(&backend);
        tasks.push(tokio::spawn(async move {
            let data = bytes_of(200_000 + usize::from(n), n);
            let k = key();
            backend
                .put(&k, body(chunks_of(&data, 50_000)))
                .await
                .expect("put");
            assert_eq!(read_all(backend.as_ref(), &k).await.expect("get"), data);
        }));
    }
    for task in tasks {
        task.await.expect("joined");
    }
}

// ---------------------------------------------- real clouds, only when someone supplies credentials

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Needs `SAGAT_TEST_S3_BUCKET`, `SAGAT_TEST_S3_REGION` and the usual AWS credentials in the
/// environment. Writes and deletes a few objects under `sagat-contract-test/`.
#[tokio::test]
#[ignore = "needs real AWS credentials (SAGAT_TEST_S3_BUCKET, SAGAT_TEST_S3_REGION): never run in CI"]
async fn real_aws_s3_honours_the_contract() {
    let config = BlobStoreConfig::S3(S3Config {
        bucket: env("SAGAT_TEST_S3_BUCKET").expect("SAGAT_TEST_S3_BUCKET"),
        region: env("SAGAT_TEST_S3_REGION").expect("SAGAT_TEST_S3_REGION"),
        endpoint: None,
        credentials: None,
        path_style: false,
        allow_http: false,
        prefix: Some("sagat-contract-test".to_owned()),
    });
    let backend = build_backend(&config, PartSize::default()).expect("s3");
    contract(backend.as_ref()).await;
    failed_uploads_leave_nothing(backend.as_ref()).await;
}

/// Needs `SAGAT_TEST_R2_ACCOUNT_ID`, `SAGAT_TEST_R2_BUCKET`, `SAGAT_TEST_R2_ACCESS_KEY_ID` and
/// `SAGAT_TEST_R2_SECRET_ACCESS_KEY` (an R2 API token with object read and write on that bucket).
#[tokio::test]
#[ignore = "needs real Cloudflare R2 credentials (SAGAT_TEST_R2_*): never run in CI"]
async fn real_cloudflare_r2_honours_the_contract() {
    let config = BlobStoreConfig::R2(R2Config {
        account_id: env("SAGAT_TEST_R2_ACCOUNT_ID").expect("SAGAT_TEST_R2_ACCOUNT_ID"),
        bucket: env("SAGAT_TEST_R2_BUCKET").expect("SAGAT_TEST_R2_BUCKET"),
        credentials: Credentials {
            access_key_id: env("SAGAT_TEST_R2_ACCESS_KEY_ID").expect("SAGAT_TEST_R2_ACCESS_KEY_ID"),
            secret_access_key: Secret::new(
                env("SAGAT_TEST_R2_SECRET_ACCESS_KEY").expect("SAGAT_TEST_R2_SECRET_ACCESS_KEY"),
            ),
            session_token: None,
        },
        jurisdiction: None,
        endpoint_override: None,
        prefix: Some("sagat-contract-test".to_owned()),
    });
    let backend = build_backend(&config, PartSize::default()).expect("r2");
    contract(backend.as_ref()).await;
    failed_uploads_leave_nothing(backend.as_ref()).await;
}
