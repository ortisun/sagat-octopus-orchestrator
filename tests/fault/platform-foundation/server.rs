//! Connection-level abuse: slow clients, connection floods and stuck connections during shutdown.
//! These tests drive the real server loop with raw TCP sockets.

use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use sagat_api_http::{HttpLimits, ServerConfig, protect, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct Server {
    addr: std::net::SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    handle: JoinHandle<std::io::Result<()>>,
}

impl Server {
    async fn start(config: ServerConfig) -> Self {
        let app = protect(
            Router::new().route("/ok", get(|| async { "ok" })),
            HttpLimits::default(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stop, stopped) = oneshot::channel::<()>();
        let handle = tokio::spawn(serve(listener, app, config, async {
            let _ = stopped.await;
        }));
        Self {
            addr,
            stop: Some(stop),
            handle,
        }
    }

    async fn connect(&self) -> TcpStream {
        TcpStream::connect(self.addr).await.expect("connect")
    }

    fn shutdown(&mut self) {
        let _ = self.stop.take().expect("not stopped yet").send(());
    }
}

const REQUEST: &[u8] = b"GET /ok HTTP/1.1\r\nHost: test\r\n\r\n";

/// Reads until the peer closes or `limit` passes; returns what was read and whether it closed.
async fn read_until_closed(stream: &mut TcpStream, limit: Duration) -> (Vec<u8>, bool) {
    let mut collected = Vec::new();
    let deadline = Instant::now() + limit;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut chunk = [0u8; 1024];
        match tokio::time::timeout(remaining, stream.read(&mut chunk)).await {
            Err(_) => return (collected, false),
            Ok(Ok(0) | Err(_)) => return (collected, true),
            Ok(Ok(n)) => collected.extend_from_slice(&chunk[..n]),
        }
    }
}

fn quick() -> ServerConfig {
    ServerConfig {
        header_read_timeout: Duration::from_millis(500),
        max_connections: 64,
        shutdown_grace: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn a_client_that_never_finishes_its_request_head_is_disconnected() {
    let server = Server::start(quick()).await;
    let mut slow = server.connect().await;
    slow.write_all(b"GET /ok HTTP/1.1\r\nHost: test\r\nX-Slow: ")
        .await
        .expect("partial head");

    let started = Instant::now();
    let (_, closed) = read_until_closed(&mut slow, Duration::from_secs(5)).await;

    assert!(
        closed,
        "the server must hang up on a client that stalls mid-head"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_client_that_connects_and_says_nothing_is_disconnected() {
    let server = Server::start(quick()).await;
    let mut silent = server.connect().await;

    let (_, closed) = read_until_closed(&mut silent, Duration::from_secs(5)).await;

    assert!(closed);
}

#[tokio::test]
async fn an_idle_keep_alive_connection_is_closed_after_serving_its_request() {
    let server = Server::start(quick()).await;
    let mut client = server.connect().await;
    client.write_all(REQUEST).await.expect("request");

    let (bytes, closed) = read_until_closed(&mut client, Duration::from_secs(5)).await;

    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "the request itself was served: {text}"
    );
    assert!(closed, "and the idle connection was then reclaimed");
}

#[tokio::test]
async fn a_well_behaved_client_is_unaffected_by_slow_ones() {
    let server = Server::start(quick()).await;
    let mut slow: Vec<TcpStream> = Vec::new();
    for _ in 0..20 {
        let mut s = server.connect().await;
        s.write_all(b"GET /ok HTTP/1.1\r\n").await.expect("partial");
        slow.push(s);
    }

    let mut good = server.connect().await;
    good.write_all(b"GET /ok HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .expect("request");
    let (bytes, _) = read_until_closed(&mut good, Duration::from_secs(3)).await;

    assert!(String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn at_the_connection_cap_new_clients_wait_instead_of_exhausting_the_server() {
    let config = ServerConfig {
        header_read_timeout: Duration::from_secs(30), // the holders stay until we close them
        max_connections: 2,
        shutdown_grace: Duration::from_secs(5),
    };
    let server = Server::start(config).await;
    let holder_a = server.connect().await;
    let holder_b = server.connect().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The third connection is accepted by the kernel but not served while the cap is full.
    let mut waiting = server.connect().await;
    waiting.write_all(REQUEST).await.expect("request");
    let (early, _) = read_until_closed(&mut waiting, Duration::from_millis(500)).await;
    assert!(
        early.is_empty(),
        "no response while the cap is full: {}",
        String::from_utf8_lossy(&early)
    );

    // Freeing a slot lets it through.
    drop(holder_a);
    let (bytes, _) = read_until_closed(&mut waiting, Duration::from_secs(3)).await;
    assert!(
        String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 200"),
        "served once a slot freed"
    );
    drop(holder_b);
}

#[tokio::test]
async fn shutdown_does_not_wait_forever_for_a_connection_that_never_completes() {
    let config = ServerConfig {
        header_read_timeout: Duration::from_secs(60), // would hold the connection for a minute
        max_connections: 8,
        shutdown_grace: Duration::from_millis(600),
    };
    let mut server = Server::start(config).await;
    let mut stuck = server.connect().await;
    stuck
        .write_all(b"GET /ok HTTP/1.1\r\nHost: test\r\nX-Never: ")
        .await
        .expect("partial head");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let started = Instant::now();
    server.shutdown();
    let result = tokio::time::timeout(Duration::from_secs(5), server.handle).await;

    assert!(
        result.is_ok(),
        "the server must stop within the grace period, not hang"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}
