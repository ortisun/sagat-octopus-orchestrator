//! TLS and mutual TLS on the worker API, with generated certificates and real handshakes.
//!
//! The service behind the listener is a stub that answers every call: what is under test is who
//! gets to talk to it, not what it says.

use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use sagat_api_grpc::pb::PollTasksRequest;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::{GrpcConfig, TlsConfig, serve, worker_service};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::stub::Stub;

/// A certificate authority and what it signed.
struct Authority {
    cert: rcgen::Certificate,
    key: KeyPair,
}

impl Authority {
    fn new(name: &str) -> Self {
        let key = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(Vec::new()).expect("params");
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).expect("ca");
        Self { cert, key }
    }

    fn pem(&self) -> String {
        self.cert.pem()
    }

    /// A certificate for `localhost` signed by this authority, as `(certificate, private key)`.
    fn issue(&self) -> (String, String) {
        let key = KeyPair::generate().expect("key");
        let params = CertificateParams::new(vec!["localhost".to_owned()]).expect("params");
        let cert = params.signed_by(&key, &self.cert, &self.key).expect("leaf");
        (cert.pem(), key.serialize_pem())
    }
}

struct Server {
    url: String,
    _stop: oneshot::Sender<()>,
}

async fn serve_stub(tls: TlsConfig) -> Server {
    serve_stub_with(GrpcConfig {
        tls: Some(tls),
        ..GrpcConfig::default()
    })
    .await
}

async fn serve_stub_with(config: GrpcConfig) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = serve(listener, worker_service(Stub), config, async {
            let _ = stopped.await;
        })
        .await;
    });
    Server {
        // The certificate is for `localhost`, so that is the name the client dials.
        url: format!("https://localhost:{port}"),
        _stop: stop,
    }
}

async fn call(channel: Result<Channel, tonic::transport::Error>) -> Result<(), String> {
    let channel = channel.map_err(|e| format!("connect: {e}"))?;
    let mut client = WorkerServiceClient::new(channel);
    tokio::time::timeout(
        Duration::from_secs(5),
        client.poll_tasks(PollTasksRequest::default()),
    )
    .await
    .map_err(|_| "timed out".to_owned())?
    .map(|_| ())
    .map_err(|e| format!("call: {e}"))
}

async fn tls_client(
    url: &str,
    trusting: &str,
    identity: Option<(&str, &str)>,
) -> Result<Channel, tonic::transport::Error> {
    let mut tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(trusting))
        .domain_name("localhost");
    if let Some((cert, key)) = identity {
        tls = tls.identity(Identity::from_pem(cert, key));
    }
    Endpoint::from_shared(url.to_owned())
        .expect("endpoint")
        .tls_config(tls)
        .expect("tls config")
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
}

fn server_tls(ca: &Authority, client_ca: Option<&Authority>) -> TlsConfig {
    let (cert, key) = ca.issue();
    TlsConfig {
        certificate_pem: cert.into_bytes(),
        private_key_pem: key.into_bytes(),
        client_ca_pem: client_ca.map(|c| c.pem().into_bytes()),
    }
}

#[tokio::test]
async fn a_client_that_trusts_the_server_certificate_is_served_over_an_encrypted_channel() {
    let ca = Authority::new("sagat test server ca");
    let server = serve_stub(server_tls(&ca, None)).await;
    assert_eq!(
        call(tls_client(&server.url, &ca.pem(), None).await).await,
        Ok(())
    );
}

#[tokio::test]
async fn plaintext_and_untrusting_clients_cannot_talk_to_a_tls_listener() {
    let ca = Authority::new("sagat test server ca");
    let impostor = Authority::new("some other ca");
    let server = serve_stub(server_tls(&ca, None)).await;

    // Plaintext HTTP/2 to a TLS port goes nowhere.
    let plaintext = Endpoint::from_shared(server.url.replace("https", "http"))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await;
    assert!(call(plaintext).await.is_err(), "plaintext was served");

    // A client that does not trust the CA that signed the server refuses it (no silent downgrade).
    let untrusting = tls_client(&server.url, &impostor.pem(), None).await;
    assert!(
        call(untrusting).await.is_err(),
        "an untrusted server was accepted"
    );
}

#[tokio::test]
async fn mutual_tls_serves_only_clients_with_a_certificate_signed_by_the_client_ca() {
    let server_ca = Authority::new("sagat test server ca");
    let client_ca = Authority::new("sagat test client ca");
    let other_ca = Authority::new("an unrelated ca");
    let server = serve_stub(server_tls(&server_ca, Some(&client_ca))).await;

    // A valid client certificate: served.
    let (cert, key) = client_ca.issue();
    assert_eq!(
        call(tls_client(&server.url, &server_ca.pem(), Some((&cert, &key))).await).await,
        Ok(())
    );

    // No certificate: the handshake or the first call fails.
    assert!(
        call(tls_client(&server.url, &server_ca.pem(), None).await)
            .await
            .is_err(),
        "a client without a certificate was served"
    );

    // A certificate from a CA the server does not trust: refused.
    let (cert, key) = other_ca.issue();
    assert!(
        call(tls_client(&server.url, &server_ca.pem(), Some((&cert, &key))).await)
            .await
            .is_err(),
        "a client with a foreign certificate was served"
    );
}

#[tokio::test]
async fn the_tls_configuration_never_prints_the_private_key() {
    let ca = Authority::new("sagat test server ca");
    let tls = server_tls(&ca, None);
    let printed = format!(
        "{:?}",
        GrpcConfig {
            tls: Some(tls.clone()),
            ..GrpcConfig::default()
        }
    );
    let key_body = String::from_utf8(tls.private_key_pem).expect("pem");
    let secret_line = key_body.lines().nth(1).expect("a line of key material");
    assert!(!printed.contains(secret_line), "{printed}");
    assert!(printed.contains("<redacted>"));
}

#[tokio::test]
async fn a_handshake_that_starts_and_stalls_is_cut_off_and_gives_its_place_back() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let ca = Authority::new("sagat test server ca");
    // One place only, and nothing else that could close the stalled connection in time: the
    // silence limits are far away, so what ends it is the handshake timeout.
    let server = serve_stub_with(GrpcConfig {
        tls: Some(server_tls(&ca, None)),
        max_connections: 1,
        first_byte_timeout: Duration::from_secs(60),
        idle_timeout: Duration::from_secs(60),
        handshake_timeout: Duration::from_millis(300),
        ..GrpcConfig::default()
    })
    .await;
    let port = server
        .url
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .expect("port");

    // A handshake record type, and then nothing: the shape of a client that began and stopped.
    let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    stalled.write_all(&[0x16]).await.expect("one byte");
    let mut buffer = [0_u8; 16];
    let closed = tokio::time::timeout(Duration::from_secs(3), stalled.read(&mut buffer)).await;
    assert!(
        matches!(closed, Ok(Ok(0) | Err(_))),
        "the stalled handshake was still open after 3 s: {closed:?}"
    );

    // Its place is free again: a real client is served although the cap is one connection.
    assert_eq!(
        call(tls_client(&server.url, &ca.pem(), None).await).await,
        Ok(())
    );
}
