//! The image's healthcheck verifies native TLS without depending on a shell or external client.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{Router, http::StatusCode, routing::get};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use sagat_api_http::{HttpTlsConfig, ServerConfig, serve_tls};
use sagat_control_plane::{HealthcheckTlsConfig, healthcheck_tls};
use tokio::net::TcpListener;

struct Authority {
    cert: rcgen::Certificate,
    key: KeyPair,
}

impl Authority {
    fn new() -> Self {
        let key = KeyPair::generate().expect("CA key");
        let mut params = CertificateParams::new(Vec::new()).expect("CA params");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        Self {
            cert: params.self_signed(&key).expect("CA"),
            key,
        }
    }

    fn issue(&self, name: &str) -> (rcgen::Certificate, KeyPair) {
        let key = KeyPair::generate().expect("server key");
        let params = CertificateParams::new(vec![name.to_owned()]).expect("server params");
        (
            params
                .signed_by(&key, &self.cert, &self.key)
                .expect("server certificate"),
            key,
        )
    }
}

async fn native_tls_server(
    authority: &Authority,
) -> (
    SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (certificate, key) = authority.issue("localhost");
    let tls = HttpTlsConfig::from_pem(
        certificate.pem().as_bytes(),
        key.serialize_pem().as_bytes(),
        authority.cert.pem().as_bytes(),
    )
    .expect("TLS config");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let addr = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(serve_tls(
        listener,
        Router::new().route("/v1/health", get(|| async { StatusCode::OK })),
        ServerConfig::default(),
        tls,
        async {
            let _ = stopped.await;
        },
    ));
    (addr, stop, task)
}

fn config(authority: &Authority, name: &str) -> HealthcheckTlsConfig {
    HealthcheckTlsConfig {
        server_name: name.to_owned(),
        ca_pem: Some(authority.cert.pem().into_bytes()),
    }
}

#[tokio::test]
async fn the_healthcheck_verifies_the_ca_and_sni_at_the_exact_local_socket() {
    let authority = Authority::new();
    let (addr, stop, task) = native_tls_server(&authority).await;
    healthcheck_tls(addr, &config(&authority, "localhost"))
        .await
        .expect("trusted name at the pinned socket");

    let alien = Authority::new();
    assert!(
        healthcheck_tls(addr, &config(&alien, "localhost"))
            .await
            .is_err(),
        "another trust root must fail"
    );
    assert!(
        healthcheck_tls(addr, &config(&authority, "other.example"))
            .await
            .is_err(),
        "the local address must not bypass hostname verification"
    );
    stop.send(()).expect("stop server");
    task.await.expect("join server").expect("serve TLS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_healthcheck_command_selects_verified_tls_from_its_environment() {
    let authority = Authority::new();
    let (addr, stop, task) = native_tls_server(&authority).await;
    let path = temporary_file("trusted-ca");
    std::fs::write(&path, authority.cert.pem()).expect("write trust roots");
    let ca_path = path.to_string_lossy().into_owned();
    let address = addr.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_control-plane"))
            .arg("healthcheck")
            .env_clear()
            .env("SAGAT_HTTP_TLS_CERT", "/configured/native/tls.crt")
            .env("SAGAT_HEALTHCHECK_ADDR", address)
            .env("SAGAT_HEALTHCHECK_TLS_SERVER_NAME", "localhost")
            .env("SAGAT_HEALTHCHECK_TLS_CA_FILE", ca_path)
            .output()
            .expect("run healthcheck command")
    })
    .await
    .expect("join healthcheck command");
    assert!(
        output.status.success(),
        "healthcheck stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_file(path).expect("remove test CA");
    stop.send(()).expect("stop server");
    task.await.expect("join server").expect("serve TLS");
}

#[tokio::test]
async fn the_tls_healthcheck_rejects_plain_http_and_bounds_the_whole_probe() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener");
    let plain_addr = listener.local_addr().expect("HTTP address");
    let plain = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/health", get(|| async { StatusCode::OK })),
        )
        .await
    });
    let authority = Authority::new();
    assert!(
        healthcheck_tls(plain_addr, &config(&authority, "localhost"))
            .await
            .is_err(),
        "plain HTTP cannot satisfy a TLS healthcheck"
    );
    plain.abort();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stall listener");
    let stalled_addr = listener.local_addr().expect("stall address");
    let stalled = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept");
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let started = Instant::now();
    let error = healthcheck_tls(stalled_addr, &config(&authority, "localhost"))
        .await
        .expect_err("stalled handshake");
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the complete probe has a three-second deadline"
    );
    stalled.abort();
}

#[test]
fn healthcheck_tls_configuration_is_narrow_and_bounded() {
    let lookup = |values: &BTreeMap<&str, String>, name: &str| values.get(name).cloned();
    assert_eq!(
        HealthcheckTlsConfig::from_lookup(|name| lookup(&BTreeMap::new(), name))
            .expect("plain config"),
        None
    );
    let native = BTreeMap::from([("SAGAT_HTTP_TLS_CERT", "/tls/tls.crt".to_owned())]);
    assert_eq!(
        HealthcheckTlsConfig::from_lookup(|name| lookup(&native, name))
            .expect("native TLS")
            .expect("TLS config")
            .server_name,
        "localhost"
    );
    let invalid = BTreeMap::from([
        ("SAGAT_HTTP_TLS_CERT", "/tls/tls.crt".to_owned()),
        (
            "SAGAT_HEALTHCHECK_TLS_SERVER_NAME",
            "https://localhost".to_owned(),
        ),
    ]);
    assert!(HealthcheckTlsConfig::from_lookup(|name| lookup(&invalid, name)).is_err());

    let path = temporary_file("oversized-ca");
    std::fs::write(&path, vec![b'x'; 256 * 1024 + 1]).expect("oversize CA");
    let oversized = BTreeMap::from([
        ("SAGAT_HTTP_TLS_CERT", "/tls/tls.crt".to_owned()),
        (
            "SAGAT_HEALTHCHECK_TLS_CA_FILE",
            path.to_string_lossy().into_owned(),
        ),
    ]);
    let error =
        HealthcheckTlsConfig::from_lookup(|name| lookup(&oversized, name)).expect_err("bounded CA");
    assert!(error.to_string().contains("256 KiB"), "{error}");
    std::fs::remove_file(path).expect("remove test CA");
}

fn temporary_file(purpose: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("sagat-healthcheck-{purpose}-{unique}.pem"))
}

#[tokio::test]
async fn direct_tls_config_cannot_bypass_local_resolution_or_bounds() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let addr = listener.local_addr().expect("address");
    for name in [
        "127.0.0.1",
        "2130706433",
        "0x7f000001",
        "localhost@elsewhere",
        "localhost/path",
    ] {
        let config = HealthcheckTlsConfig {
            server_name: name.to_owned(),
            ca_pem: None,
        };
        let error = healthcheck_tls(addr, &config)
            .await
            .expect_err("invalid name");
        assert!(
            error.to_string().contains("DNS hostname"),
            "{name}: {error}"
        );
        assert!(
            HealthcheckTlsConfig::from_lookup(|key| match key {
                "SAGAT_HTTP_TLS_CERT" => Some("/tls/cert.pem".into()),
                "SAGAT_HEALTHCHECK_TLS_SERVER_NAME" => Some(name.into()),
                _ => None,
            })
            .is_err()
        );
    }
    let oversized = HealthcheckTlsConfig {
        server_name: "localhost".into(),
        ca_pem: Some(vec![b'x'; 256 * 1024 + 1]),
    };
    let error = healthcheck_tls(addr, &oversized)
        .await
        .expect_err("bounded direct config");
    assert!(error.to_string().contains("256 KiB"));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err(),
        "invalid configuration must not contact a socket"
    );
}

#[cfg(unix)]
#[test]
fn tls_ca_configuration_refuses_a_fifo_without_opening_it() {
    let path = temporary_file("fifo-ca");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo")
            .success()
    );
    let result = HealthcheckTlsConfig::from_lookup(|key| match key {
        "SAGAT_HTTP_TLS_CERT" => Some("/tls/cert.pem".into()),
        "SAGAT_HEALTHCHECK_TLS_CA_FILE" => Some(path.to_string_lossy().into_owned()),
        _ => None,
    });
    std::fs::remove_file(path).expect("remove fifo");
    assert!(
        result
            .expect_err("regular file required")
            .to_string()
            .contains("regular file")
    );
}
