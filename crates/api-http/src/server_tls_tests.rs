//! Real TLS handshakes, verified identity and expiry on an existing connection.
#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use axum::{
    Extension,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use sha2::Digest as _;

struct Authority {
    cert: rcgen::Certificate,
    key: KeyPair,
}
impl Authority {
    fn new() -> Self {
        let key = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(Vec::new()).expect("params");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        Self {
            cert: params.self_signed(&key).expect("CA"),
            key,
        }
    }
    fn issue(&self, expires: Option<time::OffsetDateTime>) -> (rcgen::Certificate, KeyPair) {
        let key = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(vec!["localhost".to_owned()]).expect("params");
        if let Some(expires) = expires {
            params.not_after = expires;
        }
        (
            params.signed_by(&key, &self.cert, &self.key).expect("leaf"),
            key,
        )
    }
}

async fn protected(
    identity: Option<Extension<VerifiedClientCertificate>>,
    _: HeaderMap,
) -> Result<String, StatusCode> {
    identity
        .map(|identity| identity.sha256.clone())
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn client(ca: &Authority, identity: Option<(&rcgen::Certificate, &KeyPair)>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .add_root_certificate(
            reqwest::Certificate::from_pem(ca.cert.pem().as_bytes()).expect("root"),
        )
        .timeout(Duration::from_secs(5));
    if let Some((cert, key)) = identity {
        builder = builder.identity(
            reqwest::Identity::from_pem(
                format!("{}{}", cert.pem(), key.serialize_pem()).as_bytes(),
            )
            .expect("identity"),
        );
    }
    builder.build().expect("client")
}

#[tokio::test]
async fn only_verified_native_certificates_produce_an_identity() {
    let ca = Authority::new();
    let (server, key) = ca.issue(None);
    let (sender, sender_key) = ca.issue(None);
    let tls = HttpTlsConfig::from_pem(
        server.pem().as_bytes(),
        key.serialize_pem().as_bytes(),
        ca.cert.pem().as_bytes(),
    )
    .expect("TLS");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "https://localhost:{}/",
        listener.local_addr().expect("address").port()
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_tls(
        listener,
        Router::new().route("/", get(protected)),
        ServerConfig::default(),
        tls,
        async {
            let _ = stopped.await;
        },
    ));
    let expected: String = sha2::Sha256::digest(sender.der().as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let response = client(&ca, Some((&sender, &sender_key)))
        .get(&url)
        .send()
        .await
        .expect("valid handshake");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.expect("body"), expected);
    let response = client(&ca, None)
        .get(&url)
        .header("x-forwarded-client-cert", expected)
        .send()
        .await
        .expect("anonymous TLS");
    assert_eq!(
        response.status(),
        401,
        "forwarded headers confer no identity"
    );
    let alien = Authority::new();
    let (untrusted, untrusted_key) = alien.issue(None);
    assert!(
        client(&ca, Some((&untrusted, &untrusted_key)))
            .get(&url)
            .send()
            .await
            .is_err()
    );
    let (expired, expired_key) = ca.issue(Some(
        time::OffsetDateTime::now_utc() - time::Duration::seconds(1),
    ));
    assert!(
        client(&ca, Some((&expired, &expired_key)))
            .get(&url)
            .send()
            .await
            .is_err()
    );
    stop.send(()).expect("stop");
    task.await.expect("join").expect("serve");
}

#[tokio::test]
async fn certificate_expiry_is_rechecked_on_a_keep_alive_connection() {
    let ca = Authority::new();
    let (server, key) = ca.issue(None);
    let expiry = time::OffsetDateTime::now_utc() + time::Duration::seconds(3);
    let (sender, sender_key) = ca.issue(Some(expiry));
    let tls = HttpTlsConfig::from_pem(
        server.pem().as_bytes(),
        key.serialize_pem().as_bytes(),
        ca.cert.pem().as_bytes(),
    )
    .expect("TLS");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "https://localhost:{}/",
        listener.local_addr().expect("address").port()
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_tls(
        listener,
        Router::new().route("/", get(protected)),
        ServerConfig::default(),
        tls,
        async {
            let _ = stopped.await;
        },
    ));
    let client = client(&ca, Some((&sender, &sender_key)));
    let first = client.get(&url).send().await.expect("initial handshake");
    assert_eq!(first.status(), 200);
    first
        .bytes()
        .await
        .expect("drain body for keep-alive reuse");
    tokio::time::sleep(Duration::from_secs(4)).await;
    let later = client
        .get(&url)
        .send()
        .await
        .expect("existing TLS connection");
    assert_eq!(
        later.status(),
        401,
        "the connection cannot retain an expired identity"
    );
    later.bytes().await.expect("body");
    stop.send(()).expect("stop");
    task.await.expect("join").expect("serve");
}
