//! `serve_unix`: the same connection-level protections as the TCP loop, with the peer's user checked at accept (feature 035,
//! ADR-0079 rules 3 and 4). A node-local socket is a trust boundary of its own: the kernel says who connects, and a peer that is not
//! allowed is dropped before a byte of its request is read.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use sagat_api_http::{ServerConfig, serve_unix};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const REQUEST: &[u8] = b"GET /ping HTTP/1.1\r\nHost: node\r\nConnection: close\r\n\r\n";
const BOUND: Duration = Duration::from_secs(10);

struct Server {
    path: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
    _directory: tempfile::TempDir,
}

impl Server {
    fn start(allow: impl Fn(u32) -> bool + Send + Sync + 'static, config: ServerConfig) -> Self {
        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("api.sock");
        let listener = UnixListener::bind(&path).expect("a socket");
        let (stop, stopped) = oneshot::channel::<()>();
        let app = Router::new().route("/ping", get(|| async { "pong" }));
        let task = tokio::spawn(serve_unix(listener, allow, app, config, async move {
            let _ = stopped.await;
        }));
        Self {
            path,
            stop: Some(stop),
            task,
            _directory: directory,
        }
    }

    async fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.path)
            .await
            .expect("the socket accepts a connection")
    }

    async fn stop(mut self) -> std::io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        tokio::time::timeout(BOUND, self.task)
            .await
            .expect("the server stops")
            .expect("the server task")
    }
}

/// The user this test runs as, as the kernel reports it to the peer of a socket.
async fn own_uid() -> u32 {
    let (a, _b) = UnixStream::pair().expect("a pair");
    a.peer_cred().expect("credentials").uid()
}

fn quick() -> ServerConfig {
    ServerConfig {
        header_read_timeout: Duration::from_secs(5),
        max_connections: 8,
        write_stall_timeout: Duration::from_secs(5),
        shutdown_grace: Duration::from_millis(500),
    }
}

async fn read_all(stream: &mut UnixStream) -> String {
    let mut got = Vec::new();
    tokio::time::timeout(BOUND, stream.read_to_end(&mut got))
        .await
        .expect("the server answers or closes")
        .expect("a read");
    String::from_utf8_lossy(&got).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_the_check_allows_is_served() {
    let uid = own_uid().await;
    let server = Server::start(move |peer| peer == uid, quick());
    let mut stream = server.connect().await;
    stream.write_all(REQUEST).await.expect("a request");
    let answer = read_all(&mut stream).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("pong"), "{answer}");
    server.stop().await.expect("a clean stop");
}

/// The check is made on the connection, before the request: a peer that is not allowed gets nothing, not even a refusal it
/// could read, and the application never sees its bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_the_check_refuses_gets_nothing_and_is_closed() {
    let uid = own_uid().await;
    // allowed is any user but this one
    let server = Server::start(move |peer| peer != uid, quick());
    let mut stream = server.connect().await;
    let _ = stream.write_all(REQUEST).await;
    let answer = read_all(&mut stream).await;
    assert_eq!(answer, "", "a refused peer was answered");
    server.stop().await.expect("a clean stop");
}

/// The check decides for each connection: a refusal does not make the server stop, nor remember the peer.
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_does_not_stop_the_server_and_the_next_peer_is_judged_again() {
    let uid = own_uid().await;
    let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&allowed);
    let server = Server::start(
        move |peer| peer == uid && flag.load(std::sync::atomic::Ordering::SeqCst),
        quick(),
    );
    let mut refused = server.connect().await;
    let _ = refused.write_all(REQUEST).await;
    assert_eq!(read_all(&mut refused).await, "");

    allowed.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut served = server.connect().await;
    served.write_all(REQUEST).await.expect("a request");
    assert!(read_all(&mut served).await.starts_with("HTTP/1.1 200"));
    server.stop().await.expect("a clean stop");
}

/// A client that opens a connection and says nothing is let go (the slow-client protection of the TCP loop, on the socket).
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_sends_no_request_is_disconnected() {
    let uid = own_uid().await;
    let config = ServerConfig {
        header_read_timeout: Duration::from_millis(300),
        ..quick()
    };
    let server = Server::start(move |peer| peer == uid, config);
    let mut stream = server.connect().await;
    let mut byte = [0_u8; 1];
    let read = tokio::time::timeout(BOUND, stream.read(&mut byte))
        .await
        .expect("the server closes the idle connection")
        .expect("a read");
    assert_eq!(read, 0, "a silent peer was answered");
    server.stop().await.expect("a clean stop");
}

/// At the cap the server stops accepting, and the next peer waits in the backlog until a connection ends. (An absence has to
/// be looked for for some time; the second half of the case is the proof that it was waiting and not lost.)
#[tokio::test(flavor = "multi_thread")]
async fn at_the_cap_the_next_peer_waits_until_a_connection_ends() {
    let uid = own_uid().await;
    let config = ServerConfig {
        max_connections: 1,
        ..quick()
    };
    let server = Server::start(move |peer| peer == uid, config);
    let first = server.connect().await; // holds the only place, saying nothing

    let mut second = server.connect().await;
    second.write_all(REQUEST).await.expect("a request");
    let mut head = [0_u8; 16];
    let early = tokio::time::timeout(Duration::from_millis(400), second.read(&mut head)).await;
    assert!(early.is_err(), "served beyond the cap: {early:?}");

    drop(first);
    let answer = read_all(&mut second).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    server.stop().await.expect("a clean stop");
}

/// Shutdown returns even with a connection that never says anything: the grace is a ceiling, not a wait for the client.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_returns_with_a_connection_still_open() {
    let uid = own_uid().await;
    let server = Server::start(move |peer| peer == uid, quick());
    let _idle = server.connect().await;
    server.stop().await.expect("a clean stop");
}
