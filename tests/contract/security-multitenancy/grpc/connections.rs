//! Who may hold a connection of the worker API, and for how long, over real sockets (security
//! review of feature 009, the availability finding): sockets that never speak, one address taking
//! every place, and connections that outlive their usefulness.

use std::time::Duration;

use sagat_api_grpc::pb::PollTasksRequest;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::{GrpcConfig, serve, worker_service};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tonic::transport::Endpoint;

use crate::stub::Stub;

struct Server {
    addr: std::net::SocketAddr,
    _stop: oneshot::Sender<()>,
}

async fn serve_stub(config: GrpcConfig) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = serve(listener, worker_service(Stub), config, async {
            let _ = stopped.await;
        })
        .await;
    });
    Server { addr, _stop: stop }
}

/// A real call on a new connection; `Err` when it was not served within `within`.
async fn call(server: &Server, within: Duration) -> Result<(), String> {
    let channel = Endpoint::from_shared(format!("http://{}", server.addr))
        .expect("endpoint")
        .connect_timeout(within)
        .connect_lazy();
    let mut client = WorkerServiceClient::new(channel);
    tokio::time::timeout(within, client.poll_tasks(PollTasksRequest::default()))
        .await
        .map_err(|_| "no answer in time".to_owned())?
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// True when the server closed `socket` within `within`. What the server sends before that (an
/// HTTP/2 server speaks first, with its settings) is read and ignored.
async fn closed_within(socket: &mut TcpStream, within: Duration) -> bool {
    let wait = async {
        let mut buffer = [0_u8; 256];
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    };
    tokio::time::timeout(within, wait).await.is_ok()
}

#[tokio::test]
async fn sockets_that_never_speak_are_closed_and_give_their_places_back() {
    let server = serve_stub(GrpcConfig {
        max_connections: 2,
        max_connections_per_peer: 100,
        first_byte_timeout: Duration::from_millis(300),
        ..GrpcConfig::default()
    })
    .await;

    // Two sockets that connect and say nothing take both places.
    let mut mute = vec![
        TcpStream::connect(server.addr).await.expect("connect"),
        TcpStream::connect(server.addr).await.expect("connect"),
    ];
    assert!(
        call(&server, Duration::from_millis(150)).await.is_err(),
        "served although both places were taken"
    );

    // They are closed for their silence, and the places are free again.
    for socket in &mut mute {
        assert!(
            closed_within(socket, Duration::from_secs(3)).await,
            "a silent socket was still open after 3 s"
        );
    }
    let mut served = Err(String::new());
    for _ in 0..20 {
        served = call(&server, Duration::from_millis(500)).await;
        if served.is_ok() {
            break;
        }
    }
    assert_eq!(served, Ok(()));
}

#[tokio::test]
async fn one_address_cannot_take_every_place() {
    let server = serve_stub(GrpcConfig {
        max_connections: 8,
        max_connections_per_peer: 2,
        first_byte_timeout: Duration::from_secs(60),
        ..GrpcConfig::default()
    })
    .await;

    let first = TcpStream::connect(server.addr).await.expect("connect");
    let second = TcpStream::connect(server.addr).await.expect("connect");
    // The address holds its two places: the next connection from it is turned away at once, although
    // six places are free...
    let mut third = TcpStream::connect(server.addr).await.expect("connect");
    assert!(
        closed_within(&mut third, Duration::from_secs(2)).await,
        "a third connection of the same address was kept"
    );
    // ...and so is a real client from the same address.
    assert!(call(&server, Duration::from_millis(600)).await.is_err());

    // When the address gives a place back it can use it.
    drop(first);
    let mut served = Err(String::new());
    for _ in 0..20 {
        served = call(&server, Duration::from_millis(500)).await;
        if served.is_ok() {
            break;
        }
    }
    assert_eq!(served, Ok(()));
    drop(second);
}

#[tokio::test]
async fn a_peer_that_says_a_little_and_then_nothing_is_closed_for_being_idle() {
    let server = serve_stub(GrpcConfig {
        // The first byte is allowed to take a minute, so what ends this connection is the idle limit.
        first_byte_timeout: Duration::from_secs(60),
        idle_timeout: Duration::from_millis(400),
        ..GrpcConfig::default()
    })
    .await;
    let mut socket = TcpStream::connect(server.addr).await.expect("connect");
    // The first bytes of an HTTP/2 connection preface, and then silence.
    socket.write_all(b"PRI * HT").await.expect("write");
    assert!(
        closed_within(&mut socket, Duration::from_secs(3)).await,
        "an idle connection was still open after 3 s"
    );
}

#[tokio::test]
async fn a_connection_that_makes_calls_is_not_taken_for_idle() {
    let server = serve_stub(GrpcConfig {
        first_byte_timeout: Duration::from_millis(300),
        idle_timeout: Duration::from_millis(600),
        ..GrpcConfig::default()
    })
    .await;
    let channel = Endpoint::from_shared(format!("http://{}", server.addr))
        .expect("endpoint")
        .connect()
        .await
        .expect("connect");
    let mut client = WorkerServiceClient::new(channel);
    // Calls spread over longer than either limit: each one is traffic, so the connection lives.
    for _ in 0..6 {
        client
            .poll_tasks(PollTasksRequest::default())
            .await
            .expect("served");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn a_connection_that_reaches_the_maximum_age_is_closed_even_if_it_only_answers_pings() {
    let server = serve_stub(GrpcConfig {
        first_byte_timeout: Duration::from_secs(60),
        idle_timeout: Duration::from_secs(600),
        max_connection_age: Duration::from_secs(1),
        // The grace is the request timeout.
        request_timeout: Duration::from_secs(1),
        ..GrpcConfig::default()
    })
    .await;
    // A bare HTTP/2 client: it completes the handshake, answers whatever the server sends, and
    // makes no call. Nothing but the age can end it.
    let tcp = TcpStream::connect(server.addr).await.expect("connect");
    let (_requests, connection) = h2::client::handshake(tcp).await.expect("handshake");
    let ended = tokio::time::timeout(Duration::from_secs(8), connection).await;
    assert!(
        ended.is_ok(),
        "the connection was still open long after its maximum age"
    );
}
