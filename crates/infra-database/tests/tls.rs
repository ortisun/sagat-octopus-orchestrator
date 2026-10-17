#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use sagat_application::database::DatabaseCredentials;
use sagat_domain::{
    database::*,
    identity::Slug,
    secrets::{SecretName, SecretValue},
};
use sagat_infra_database::{DestinationScope, ExecutionError, NetworkPolicy, PostgresExecutor};
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
};
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn verified_tls_preserves_original_hostname_and_rejects_wrong_trust_or_name() {
    let ca_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = params.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&key, &ca, &ca_key)
        .unwrap();
    let container=GenericImage::new("postgres","16-alpine")
        .with_entrypoint("/bin/sh")
        .with_exposed_port(5432.into())
        .with_wait_for(WaitFor::message_on_stderr("database system is ready to accept connections"))
        .with_wait_for(WaitFor::message_on_stdout("database system is ready to accept connections"))
        .with_env_var("POSTGRES_PASSWORD","fixture-password")
        .with_copy_to("/tmp/server.crt",cert.pem().into_bytes())
        .with_copy_to("/tmp/server.key",key.serialize_pem().into_bytes())
        .with_copy_to("/docker-entrypoint-initdb.d/role.sql",b"CREATE ROLE sagat_target LOGIN PASSWORD 'fixture-password';".to_vec())
        .with_cmd(["-c","chown postgres:postgres /tmp/server.key; chmod 600 /tmp/server.key; exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key"])
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let network = NetworkPolicy::new(
        DestinationScope::new(
            vec!["127.0.0.0/8".parse().unwrap(), "::1/128".parse().unwrap()],
            vec![port],
        )
        .unwrap(),
        vec![],
        true,
    )
    .unwrap();
    let ns = network.allowed().clone();
    let executor = PostgresExecutor::new(network, 1).unwrap();
    let execute = async |host: &str, pem: Option<String>| {
        let config = ConnectionConfig::postgres(
            host,
            port,
            "postgres",
            TlsPolicy::VerifyFull { ca_pem: pem },
            SecretName::parse("credential").unwrap(),
            DatabaseLimits::default(),
        )
        .unwrap();
        let operation = DatabaseOperation::new(
            ConnectionRef::new(Slug::parse("test-db").unwrap(), None).unwrap(),
            OperationKind::Query,
            "SELECT 1",
            vec![],
            DatabaseLimits::default(),
        )
        .unwrap();
        executor
            .execute(
                &config,
                &ns,
                DatabaseCredentials {
                    username: SecretValue::new(b"sagat_target".to_vec()).unwrap(),
                    password: SecretValue::new(b"fixture-password".to_vec()).unwrap(),
                },
                operation,
                Instant::now() + Duration::from_secs(15),
                &CancellationToken::new(),
            )
            .await
    };
    assert!(execute("localhost", Some(ca.pem())).await.is_ok());
    assert_eq!(
        execute("127.0.0.1", Some(ca.pem())).await.err(),
        Some(ExecutionError::Tls)
    );
    assert_eq!(
        execute("localhost", None).await.err(),
        Some(ExecutionError::Tls)
    );
}
