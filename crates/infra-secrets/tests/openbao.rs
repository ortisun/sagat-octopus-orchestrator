//! FR-017-005..008: provider boundary, immutable writes and credential-safe failures.
use sagat_application::secret_provider::{ProviderError, ProviderReference, SecretProvider};
use sagat_domain::{
    identity::{NamespaceId, TenantId},
    secrets::SecretValue,
};
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

fn config(address: &str) -> OpenBaoConfig {
    OpenBaoConfig {
        address: address.to_owned(),
        mount: "secret".to_owned(),
        token_file: PathBuf::from("/unread-test-token"),
        timeout: Duration::from_secs(5),
        development_loopback: false,
        ca_pem: None,
    }
}

#[test]
fn production_requires_tls_and_rejects_embedded_routing_and_credentials() {
    for address in [
        "http://bao:8200",
        "https://user:pass@bao:8200",
        "https://bao/v1/other",
        "https://bao/?token=canary",
        "https://bao/#fragment",
    ] {
        assert!(matches!(
            OpenBao::new(config(address)),
            Err(ProviderError::Configuration)
        ));
    }
    for mount in [
        "../secret",
        "secret/data",
        "secret%2fother",
        "",
        ".",
        "secret?x",
    ] {
        let mut c = config("https://bao:8200");
        c.mount = mount.into();
        assert!(matches!(OpenBao::new(c), Err(ProviderError::Configuration)));
    }
    assert!(OpenBao::new(config("https://bao:8200")).is_ok());
}

#[test]
fn development_exception_is_loopback_only_and_timeouts_are_finite() {
    for address in [
        "http://bao:8200",
        "http://169.254.169.254",
        "http://example.com",
    ] {
        let mut c = config(address);
        c.development_loopback = true;
        assert!(matches!(OpenBao::new(c), Err(ProviderError::Configuration)));
    }
    for timeout in [Duration::ZERO, Duration::from_secs(11)] {
        let mut c = config("https://bao:8200");
        c.timeout = timeout;
        assert!(matches!(OpenBao::new(c), Err(ProviderError::Configuration)));
    }
}

#[tokio::test]
async fn real_openbao_preserves_scope_cas_and_token_rotation() {
    use testcontainers_modules::testcontainers::{
        GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
    };
    let container = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", "sagat-test-bootstrap")
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .expect("real OpenBao container");
    let port = container.get_host_port_ipv4(8200).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let token_file = temp.path().join("token");
    std::fs::write(&token_file, "sagat-test-bootstrap").unwrap();
    let mut c = config(&format!("http://127.0.0.1:{port}"));
    c.development_loopback = true;
    c.token_file = token_file.clone();
    let provider = OpenBao::new(c).unwrap();
    let reference = ProviderReference {
        tenant: TenantId::new(Uuid::new_v4()),
        namespace: NamespaceId::new(Uuid::new_v4()),
        operation: Uuid::new_v4(),
    };
    let secret = SecretValue::new(b"openbao-binary-canary\0\xff".to_vec()).unwrap();
    assert!(provider.read(&reference).await.unwrap().is_none());
    provider.create(&reference, &secret).await.unwrap();
    assert_eq!(
        provider.read(&reference).await.unwrap().unwrap().expose(),
        secret.expose()
    );
    let replacement = SecretValue::new(b"replacement-canary".to_vec()).unwrap();
    assert_eq!(
        provider.create(&reference, &replacement).await.unwrap_err(),
        ProviderError::Conflict
    );
    assert_eq!(
        provider.read(&reference).await.unwrap().unwrap().expose(),
        secret.expose()
    );
    let foreign = ProviderReference {
        tenant: TenantId::new(Uuid::new_v4()),
        ..reference.clone()
    };
    assert!(provider.read(&foreign).await.unwrap().is_none());
    std::fs::write(&token_file, "expired-token-canary").unwrap();
    let error = provider.read(&reference).await.unwrap_err();
    assert_eq!(error, ProviderError::Denied);
    assert!(!format!("{error:?} {error}").contains("canary"));
    std::fs::write(&token_file, "sagat-test-bootstrap\n").unwrap();
    assert!(provider.read(&reference).await.unwrap().is_some());
}

#[tokio::test]
async fn provider_never_follows_redirects_or_returns_untrusted_error_bodies() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for (status, header, body, expected) in [
        (
            302,
            "Location: http://169.254.169.254/latest/meta-data/\r\n",
            "provider-token-canary",
            ProviderError::InvalidResponse,
        ),
        (403, "", "secret-value-canary", ProviderError::Denied),
        (503, "", "secret-value-canary", ProviderError::Unavailable),
        (
            200,
            "",
            "malformed-secret-canary",
            ProviderError::InvalidResponse,
        ),
        (200, "", "oversized", ProviderError::InvalidResponse),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let length = if body == "oversized" {
            65537
        } else {
            body.len()
        };
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            stream.write_all(format!("HTTP/1.1 {status} Response\r\n{header}Content-Length: {length}\r\nConnection: close\r\n\r\n{body}").as_bytes()).await.unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let token = temp.path().join("token");
        std::fs::write(&token, "redacted-token").unwrap();
        let mut c = config(&format!("http://{address}"));
        c.development_loopback = true;
        c.token_file = token;
        let provider = OpenBao::new(c).unwrap();
        let reference = ProviderReference {
            tenant: TenantId::new(Uuid::new_v4()),
            namespace: NamespaceId::new(Uuid::new_v4()),
            operation: Uuid::new_v4(),
        };
        let error = provider.read(&reference).await.unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{provider:?} {error:?} {error}").contains("canary"));
        server.await.unwrap();
    }
}
