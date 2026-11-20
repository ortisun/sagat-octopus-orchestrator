//! Owned loopback uploads and deterministic sources; no external service or source replay.
#![allow(clippy::expect_used)]
use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
use sagat_octopus_orchestrator_sdk::{Client, ClientOptions, TokenSource, UploadRunInputOptions};
use sha2::{Digest, Sha256};
use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, ReadBuf};
const ID: &str = "019b5d6e-0000-7000-8000-000000000001";
struct Server {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(router: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let mut options = ClientOptions::new(
        format!("http://{}/v1", listener.local_addr().expect("address")),
        "payments",
        TokenSource::bearer("fixture-token"),
    );
    options.timeout = Duration::from_secs(1);
    let client = Client::new(options).expect("client");
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("owned server");
    });
    Server { client, task }
}
#[derive(Default)]
struct Probe {
    bytes: AtomicUsize,
    eof: AtomicBool,
    max_read: AtomicUsize,
}
struct Counted {
    source: std::io::Cursor<Vec<u8>>,
    probe: Arc<Probe>,
}
impl AsyncRead for Counted {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.source).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            let count = buf.filled().len() - before;
            this.probe.bytes.fetch_add(count, Ordering::AcqRel);
            this.probe.max_read.fetch_max(count, Ordering::AcqRel);
            if count == 0 {
                this.probe.eof.store(true, Ordering::Release);
            }
        }
        result
    }
}
#[tokio::test]
async fn a_large_source_is_measured_once_in_bounded_chunks_with_precondition_headers() {
    let headers = Arc::new(Mutex::new(None));
    let captured = headers.clone();
    let router=Router::new().route("/v1/run-input-blobs",post(move |incoming:HeaderMap,bytes:Bytes|{let captured=captured.clone();async move {
        *captured.lock().expect("headers")=Some(incoming);
        (axum::http::StatusCode::CREATED,axum::Json(serde_json::json!({"id":ID,"size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),"contentType":"application/octet-stream"})))
    }}));
    let server = serve(router).await;
    let payload = vec![7; 500_000];
    let digest = format!("{:x}", Sha256::digest(&payload));
    let probe = Arc::new(Probe::default());
    let source = Counted {
        source: std::io::Cursor::new(payload),
        probe: probe.clone(),
    };
    let descriptor = server
        .client
        .upload_run_input(
            source,
            UploadRunInputOptions {
                expected_size: Some(500_000),
                expected_sha256: Some(digest.clone()),
                ..UploadRunInputOptions::default()
            },
        )
        .await
        .expect("verified upload");
    assert_eq!(descriptor.size, 500_000);
    assert_eq!(descriptor.sha256, digest);
    assert_eq!(probe.bytes.load(Ordering::Acquire), 500_000);
    assert!(probe.eof.load(Ordering::Acquire));
    assert!(probe.max_read.load(Ordering::Acquire) <= 256 * 1024);
    let headers = headers.lock().expect("headers");
    let headers = headers.as_ref().expect("request");
    assert_eq!(headers["x-octopus-blob-size"], "500000");
    assert_eq!(headers["x-octopus-blob-sha256"], digest);
}
struct HeldAtEof {
    sent: bool,
}
impl AsyncRead for HeldAtEof {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.sent {
            return Poll::Pending;
        }
        self.sent = true;
        buf.put_slice(b"abc");
        Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn an_early_success_cannot_accept_a_descriptor_before_source_eof() {
    let router=Router::new().route("/v1/run-input-blobs",post(||async{
        (axum::http::StatusCode::CREATED,axum::Json(serde_json::json!({"id":ID,"size":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","contentType":"application/octet-stream"})))
    }));
    let server = serve(router).await;
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        server
            .client
            .upload_run_input(HeldAtEof { sent: false }, UploadRunInputOptions::default()),
    )
    .await
    .expect("bounded upload")
    .expect_err("EOF was never reached");
    assert!(matches!(error.code, "PROTOCOL" | "TRANSPORT"));
}
struct FailedSource(Arc<AtomicUsize>);
impl AsyncRead for FailedSource {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Poll::Ready(Err(std::io::Error::other("private-source-canary")))
    }
}
#[tokio::test]
async fn source_failure_does_not_replay_or_expose_its_error_text() {
    let router = Router::new().route(
        "/v1/run-input-blobs",
        post(|_: Bytes| async { axum::http::StatusCode::CREATED }),
    );
    let server = serve(router).await;
    let reads = Arc::new(AtomicUsize::new(0));
    let error = server
        .client
        .upload_run_input(
            FailedSource(reads.clone()),
            UploadRunInputOptions::default(),
        )
        .await
        .expect_err("failed source");
    assert_eq!(reads.load(Ordering::Acquire), 1);
    assert!(!format!("{error:?} {error}").contains("private-source-canary"));
}
