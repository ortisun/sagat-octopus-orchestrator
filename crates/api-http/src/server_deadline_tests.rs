//! Native socket delivery must expire even when the client makes periodic progress.
#![allow(clippy::unwrap_used)]
use super::*;
use axum::{
    Extension,
    body::{Body, Bytes},
    routing::get,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpSocket,
};

struct RetainedBody {
    bytes: Vec<u8>,
    _slot: OwnedSemaphorePermit,
    _deadline: DeliveryDeadline,
}
impl AsRef<[u8]> for RetainedBody {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[tokio::test]
async fn absolute_deadline_drops_buffered_bytes_and_recovers_slot_for_a_slow_reader() {
    let slots = Arc::new(Semaphore::new(1));
    let held = slots.clone();
    let app = Router::new().route(
        "/result",
        get(move |Extension(connection): Extension<ConnectionAbort>| {
            let slots = held.clone();
            async move {
                let slot = slots.try_acquire_owned().unwrap();
                let body = RetainedBody {
                    bytes: vec![b'x'; 4 * 1024 * 1024],
                    _slot: slot,
                    _deadline: connection.deadline(Instant::now() + Duration::from_millis(500)),
                };
                Body::from(Bytes::from_owner(body))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve(
        listener,
        app,
        ServerConfig {
            write_stall_timeout: Duration::from_secs(10),
            ..Default::default()
        },
        async {
            let _ = stopped.await;
        },
    ));
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(1024).unwrap();
    let mut client = socket.connect(address).await.unwrap();
    client
        .write_all(b"GET /result HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    let mut chunk = [0u8; 256];
    let first = client.read(&mut chunk).await.unwrap();
    assert!(first > 0);
    assert_eq!(slots.available_permits(), 0);
    let started = Instant::now();
    let mut received = first;
    while started.elapsed() < Duration::from_millis(900) {
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(50), client.read(&mut chunk)).await
        {
            received += n;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(received < 4 * 1024 * 1024);
    assert_eq!(
        slots.available_permits(),
        1,
        "deadline releases buffered owner and permit independently of polling"
    );
    drop(client);
    let _ = stop.send(());
    server.await.unwrap().unwrap();
}
