#![allow(clippy::unwrap_used, clippy::panic)]
use sagat_application::database::DatabaseCredentials;
use sagat_domain::{
    database::*,
    identity::Slug,
    secrets::{SecretName, SecretValue},
};
use sagat_infra_database::{DestinationScope, ExecutionError, NetworkPolicy, PostgresExecutor};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn hostile_server_announcing_a_gigabyte_row_is_closed_without_receiving_the_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let length = socket.read_u32().await.unwrap();
        assert!((4..65536).contains(&length));
        let mut startup = vec![0; length as usize - 4];
        socket.read_exact(&mut startup).await.unwrap();
        // AuthenticationOk, then ReadyForQuery. The client begins its fixed role verification.
        socket
            .write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 0, b'Z', 0, 0, 0, 5, b'I'])
            .await
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        // Only a header, claiming a huge DataRow. Waiting for its body would hang this test.
        socket
            .write_all(&[b'D', 0x7f, 0xff, 0xff, 0xff])
            .await
            .unwrap();
        // Drain any already queued client messages; connection must close promptly.
        while socket.read(&mut request).await.unwrap_or(0) > 0 {}
    });
    let limits = DatabaseLimits::default();
    let config = ConnectionConfig::postgres(
        "127.0.0.1",
        port,
        "postgres",
        TlsPolicy::DevelopmentPlaintext,
        SecretName::parse("credential").unwrap(),
        limits,
    )
    .unwrap();
    let scope = DestinationScope::new(vec!["127.0.0.0/8".parse().unwrap()], vec![port]).unwrap();
    let network = NetworkPolicy::new(scope.clone(), vec![], true).unwrap();
    let executor = PostgresExecutor::new(network, 1).unwrap();
    let operation = DatabaseOperation::new(
        ConnectionRef::new(Slug::parse("test-db").unwrap(), None).unwrap(),
        OperationKind::Query,
        "SELECT 1",
        vec![],
        limits,
    )
    .unwrap();
    let credentials = DatabaseCredentials {
        username: SecretValue::new(b"fixture-user".to_vec()).unwrap(),
        password: SecretValue::new(b"fixture-password".to_vec()).unwrap(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        executor.execute(
            &config,
            &scope,
            credentials,
            operation,
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.err(), Some(ExecutionError::ResultLimit));
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
