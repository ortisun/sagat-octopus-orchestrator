//! Owned TLS sockets prove native trust, verified names and client authentication.
#![allow(clippy::expect_used)]
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
use sagat_octopus_orchestrator_sdk::{
    CallOptions, Client, ClientOptions, TlsIdentity, TokenSource,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
const ID: &str = "019b5d6e-0000-7000-8000-000000000001";
struct Authority {
    cert: rcgen::Certificate,
    key: KeyPair,
}
impl Authority {
    fn new() -> Self {
        let key = KeyPair::generate().expect("CA key");
        let mut params = CertificateParams::new(Vec::new()).expect("params");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        Self {
            cert: params.self_signed(&key).expect("CA"),
            key,
        }
    }
    fn issue(&self, client: bool) -> (String, String) {
        let key = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(if client {
            Vec::new()
        } else {
            vec!["localhost".to_owned()]
        })
        .expect("params");
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![if client {
            ExtendedKeyUsagePurpose::ClientAuth
        } else {
            ExtendedKeyUsagePurpose::ServerAuth
        }];
        (
            params
                .signed_by(&key, &self.cert, &self.key)
                .expect("leaf")
                .pem(),
            key.serialize_pem(),
        )
    }
}
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(authority: &Authority, require_client: bool) -> Server {
    let (certificate, key) = authority.issue(false);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("TLS versions");
    let builder = if require_client {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(authority.cert.der().clone())
            .expect("client root");
        builder.with_client_cert_verifier(
            rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .expect("client verification"),
        )
    } else {
        builder.with_no_client_auth()
    };
    let certs = CertificateDer::pem_slice_iter(certificate.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("certs");
    let config = builder
        .with_single_cert(
            certs,
            PrivateKeyDer::from_pem_slice(key.as_bytes()).expect("key"),
        )
        .expect("server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned listener");
    let url = format!("https://{}/v1", listener.local_addr().expect("address"));
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.expect("accept");
            let acceptor = acceptor.clone();
            let seen = seen.clone();
            children.spawn(async move {
                let Ok(mut tls)=acceptor.accept(socket).await else{return;};
                let mut input=Vec::new(); let mut buffer=[0;4096];
                while input.len()<16*1024 && !input.windows(4).any(|bytes|bytes==b"\r\n\r\n") {
                    let Ok(count)=tls.read(&mut buffer).await else{return;};if count==0{return;}input.extend_from_slice(&buffer[..count]);
                }
                let request=String::from_utf8(input).expect("HTTP headers");
                assert!(request.to_ascii_lowercase().contains("authorization: bearer fixture-token"));
                seen.fetch_add(1,Ordering::SeqCst);
                let body=serde_json::to_vec(&serde_json::json!({"runId":ID,"workflowId":ID,"versionId":ID,"status":"RUNNING","version":1,"createdAt":"2026-10-27T12:00:00Z"})).expect("response");
                let header=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
                tls.write_all(header.as_bytes()).await.expect("header");tls.write_all(&body).await.expect("body");
                let _=tls.shutdown().await;
            });
            while let Some(result) = children.try_join_next() {
                result.expect("owned connection");
            }
        }
    });
    Server {
        url,
        requests,
        task,
    }
}
fn options(server: &Server) -> ClientOptions {
    let mut config = ClientOptions::new(
        &server.url,
        "payments",
        TokenSource::bearer("fixture-token"),
    );
    config.retry.max_attempts = 1;
    config.timeout = Duration::from_secs(5);
    config
}
#[tokio::test]
async fn custom_roots_and_configured_names_are_verified_instead_of_skipped() {
    let ca = Authority::new();
    let server = server(&ca, false).await;
    let mut config = options(&server);
    config.tls.server_name = Some("localhost".to_owned());
    let untrusted = Client::new(config).expect("config");
    assert_eq!(
        untrusted
            .get(ID, CallOptions::default())
            .await
            .expect_err("untrusted CA")
            .code,
        "TRANSPORT"
    );
    let mut config = options(&server);
    config.tls.ca_certificates = vec![ca.cert.pem().into_bytes()];
    let wrong_ip = Client::new(config).expect("config");
    assert_eq!(
        wrong_ip
            .get(ID, CallOptions::default())
            .await
            .expect_err("IP is not signed")
            .code,
        "TRANSPORT"
    );
    let mut config = options(&server);
    config.tls.ca_certificates = vec![ca.cert.pem().into_bytes()];
    config.tls.server_name = Some("wrong.example".to_owned());
    let wrong_name = Client::new(config).expect("config");
    assert_eq!(
        wrong_name
            .get(ID, CallOptions::default())
            .await
            .expect_err("wrong configured name")
            .code,
        "TRANSPORT"
    );
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "no bearer request before trust and name verify"
    );
    let mut config = options(&server);
    config.tls.ca_certificates = vec![ca.cert.pem().into_bytes()];
    config.tls.server_name = Some("localhost".to_owned());
    Client::new(config)
        .expect("config")
        .get(ID, CallOptions::default())
        .await
        .expect("verified TLS");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn mtls_requires_the_explicit_client_identity_and_redacts_its_key() {
    let ca = Authority::new();
    let server = server(&ca, true).await;
    let mut config = options(&server);
    config.tls.ca_certificates = vec![ca.cert.pem().into_bytes()];
    config.tls.server_name = Some("localhost".to_owned());
    assert!(
        Client::new(config.clone())
            .expect("config")
            .get(ID, CallOptions::default())
            .await
            .is_err()
    );
    let (certificate, key) = ca.issue(true);
    let identity = TlsIdentity::from_pem(certificate, key.as_bytes()).expect("identity");
    config.tls.identity = Some(identity);
    assert!(!format!("{config:?}").contains(&key));
    assert!(!format!("{:?}", config.tls).contains("PRIVATE KEY"));
    Client::new(config)
        .expect("config")
        .get(ID, CallOptions::default())
        .await
        .expect("mTLS");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}
#[test]
fn invalid_identity_material_has_no_echo_in_errors() {
    let error = TlsIdentity::from_pem(b"not-a-certificate", b"private-key-canary")
        .expect_err("invalid config");
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(!format!("{error:?} {error}").contains("private-key-canary"));
}

#[test]
fn plaintext_warning_names_only_the_host_once_per_client() {
    use std::{collections::BTreeMap, sync::Mutex};
    use tracing_subscriber::{layer::Context, prelude::*};
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<BTreeMap<String, String>>>>);
    struct Fields(BTreeMap<String, String>);
    impl tracing::field::Visit for Fields {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            if event.metadata().level() == &tracing::Level::WARN
                && event.metadata().target() == "sagat_octopus_orchestrator_sdk::client"
            {
                let mut fields = Fields(BTreeMap::new());
                event.record(&mut fields);
                self.0.lock().expect("capture").push(fields.0);
            }
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(seen.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let client = Client::new(ClientOptions::new(
            "http://api.example/private-path",
            "payments",
            TokenSource::bearer("fixture-secret-token"),
        ))
        .expect("client");
        let _clone = client.clone();
        Client::new(ClientOptions::new(
            "http://127.0.0.1/v1",
            "payments",
            TokenSource::bearer("fixture-secret-token"),
        ))
        .expect("loopback");
    });
    let logs = seen.lock().expect("logs");
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["host"], "api.example");
    assert!(!format!("{logs:?}").contains("private-path"));
    assert!(!format!("{logs:?}").contains("fixture-secret-token"));
}
