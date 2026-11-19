//! The disposable PostgreSQL of the test support (design 043, the canary failure of 2026-11-16 phase 3): a host port that
//! another server of this machine answers is not taken for the database.

use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::support;

/// On Docker Desktop for macOS a container's port is published by a proxy that listens on every address with
/// `SO_REUSEADDR`, so it can share its number with a listener a test already holds on `127.0.0.1`; a connection to
/// `127.0.0.1` then reaches the test's listener, and PostgreSQL's client reads `HTTP/1.1 ...` as `unexpected response
/// from SSLRequest: 0x48`. The wait for the database tells that apart from a cold start and gives up at once, so the
/// support starts the container again on another port instead of waiting 60 s and failing the test.
#[tokio::test]
async fn a_port_an_http_server_answers_is_told_apart_and_not_waited_on() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        }
    });
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let answer =
        tokio::time::timeout(Duration::from_secs(20), support::wait_until_queryable(&url)).await;
    assert_eq!(
        answer,
        Ok(Err(support::AnotherServer)),
        "an HTTP answer is another server, at once"
    );
}
