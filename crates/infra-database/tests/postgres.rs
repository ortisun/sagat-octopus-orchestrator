#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use sagat_application::database::DatabaseCredentials;
use sagat_domain::{
    database::*,
    identity::Slug,
    secrets::{SecretName, SecretValue},
};
use sagat_infra_database::{DestinationScope, ExecutionError, NetworkPolicy, PostgresExecutor};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ImageExt, runners::AsyncRunner},
};
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
#[derive(Clone, Default)]
struct LogSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for LogSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl tracing_subscriber::fmt::MakeWriter<'_> for LogSink {
    type Writer = Self;
    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn credentials(user: &str) -> DatabaseCredentials {
    DatabaseCredentials {
        username: SecretValue::new(user.as_bytes().to_vec()).unwrap(),
        password: SecretValue::new(b"fixture-password-canary".to_vec()).unwrap(),
    }
}
fn operation(
    sql: &str,
    kind: OperationKind,
    parameters: Vec<TypedValue>,
    limits: DatabaseLimits,
) -> DatabaseOperation {
    DatabaseOperation::new(
        ConnectionRef::new(Slug::parse("test-db").unwrap(), None).unwrap(),
        kind,
        sql,
        parameters.into_iter().map(Parameter::Literal).collect(),
        limits,
    )
    .unwrap()
}
#[tokio::test]
async fn real_postgres_prepared_types_read_only_limits_cancel_and_role_restriction() {
    let logs = LogSink::default();
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(
        sagat_telemetry::LogLayer::new(logs.clone(), "database-test", sagat_telemetry::Style::Json),
    ))
    .unwrap();
    tracing_log::LogTracer::init().unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    let container = Postgres::default()
        .with_password("fixture-password-canary")
        .with_tag("16-alpine")
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (admin, pump) = tokio_postgres::Config::new()
        .host("127.0.0.1")
        .port(port)
        .user("postgres")
        .password("fixture-password-canary")
        .dbname("postgres")
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    let pump = tokio::spawn(pump);
    admin.batch_execute("CREATE ROLE sagat_target LOGIN PASSWORD 'fixture-password-canary'; CREATE TABLE orders(id bigint, note text); GRANT SELECT, INSERT, UPDATE, DELETE ON orders TO sagat_target; CREATE FUNCTION bad_write() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER AS $$ BEGIN INSERT INTO orders VALUES(666,'bad'); RETURN 1; END $$; CREATE FUNCTION canary_notice() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RAISE NOTICE 'fixture-password-canary'; RETURN 1; END $$;").await.unwrap();
    // Fixture provisioning uses Docker credentials; capture only actual managed executions.
    logs.0.lock().unwrap().clear();
    let limits = DatabaseLimits::default();
    let config = ConnectionConfig::postgres(
        "127.0.0.1",
        port,
        "postgres",
        TlsPolicy::DevelopmentPlaintext,
        SecretName::parse("db-secret").unwrap(),
        limits,
    )
    .unwrap();
    let network = NetworkPolicy::new(
        DestinationScope::new(vec!["127.0.0.0/8".parse().unwrap()], vec![port]).unwrap(),
        vec![],
        true,
    )
    .unwrap();
    let ns = network.allowed().clone();
    let executor = PostgresExecutor::new(network, 2).unwrap();
    let cancel = CancellationToken::new();
    let deadline = || Instant::now() + Duration::from_secs(15);
    let run = |op| {
        executor.execute(
            &config,
            &ns,
            credentials("sagat_target"),
            op,
            deadline(),
            &cancel,
        )
    };
    let op = operation(
        "SELECT $1::bigint AS duplicate, $2::numeric AS duplicate, $3::bytea AS bytes, $4::jsonb AS json, $5::timestamptz AS moment, $6::date AS date, $7::float8 AS float, $8::boolean AS truth, $9::text AS text, $10::text AS nil",
        OperationKind::Query,
        vec![
            TypedValue::Integer("9223372036854775807".into()),
            TypedValue::Decimal("12345678901234567890.001230".into()),
            TypedValue::Binary(vec![0, 255, 42]),
            TypedValue::Json("{\"a\":[1,null]}".into()),
            TypedValue::Timestamp(
                time::OffsetDateTime::parse(
                    "2026-10-17T13:14:15.123Z",
                    &time::format_description::well_known::Rfc3339,
                )
                .unwrap(),
            ),
            TypedValue::Date(time::macros::date!(2026 - 10 - 17)),
            TypedValue::float(1.25).unwrap(),
            TypedValue::Boolean(true),
            TypedValue::Text("'; DROP TABLE orders; --".into()),
            TypedValue::Null,
        ],
        limits,
    );
    let result = run(op).await.unwrap();
    let result: serde_json::Value = serde_json::from_slice(result.as_bytes()).unwrap();
    assert_eq!(result["columns"][0]["name"], result["columns"][1]["name"]);
    assert_eq!(result["rows"][0][0]["value"], "9223372036854775807");
    assert_eq!(result["rows"][0][1]["value"], "12345678901234567890.001230");
    assert_eq!(result["rows"][0][2]["value"], "AP8q");
    assert_eq!(result["rows"][0][8]["value"], "'; DROP TABLE orders; --");
    assert_eq!(result["rows"][0][9]["type"], "null");
    assert_eq!(result["truncated"], false);
    assert!(
        run(operation(
            "SELECT canary_notice()",
            OperationKind::Query,
            vec![],
            limits
        ))
        .await
        .is_ok()
    );
    let mut bad = credentials("sagat_target");
    bad.password = SecretValue::new(b"incorrect-password".to_vec()).unwrap();
    assert_eq!(
        executor
            .execute(
                &config,
                &ns,
                bad,
                operation("SELECT 1", OperationKind::Query, vec![], limits),
                deadline(),
                &cancel
            )
            .await
            .err(),
        Some(ExecutionError::Authentication)
    );
    assert!(
        run(operation(
            "SELECT 'sql-literal-canary'::text",
            OperationKind::Query,
            vec![],
            limits
        ))
        .await
        .is_ok()
    );
    let insert = operation(
        "INSERT INTO orders VALUES($1,$2) RETURNING id",
        OperationKind::Execute,
        vec![
            TypedValue::Integer("42".into()),
            TypedValue::Text("ok".into()),
        ],
        limits,
    );
    assert!(run(insert).await.is_ok());
    assert_eq!(
        run(operation(
            "SELECT bad_write()",
            OperationKind::Query,
            vec![],
            limits
        ))
        .await
        .err(),
        Some(ExecutionError::Statement)
    );
    let count: i64 = admin
        .query_one("SELECT count(*) FROM orders", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    assert_eq!(
        executor
            .execute(
                &config,
                &ns,
                credentials("postgres"),
                operation("SELECT 1", OperationKind::Query, vec![], limits),
                deadline(),
                &cancel
            )
            .await
            .err(),
        Some(ExecutionError::PrivilegedRole)
    );
    for sql in [
        "SELECT repeat('x', 10000000)",
        "SELECT i FROM generate_series(1,1001) i",
    ] {
        assert_eq!(
            run(operation(sql, OperationKind::Query, vec![], limits))
                .await
                .err(),
            Some(ExecutionError::ResultLimit)
        );
    }
    assert_eq!(
        run(operation(
            "SELECT 'NaN'::numeric",
            OperationKind::Query,
            vec![],
            limits
        ))
        .await
        .err(),
        Some(ExecutionError::ValueType)
    );
    let short = DatabaseLimits {
        timeout_ms: 150,
        ..limits
    };
    assert_eq!(
        run(operation(
            "SELECT pg_sleep(10)::text",
            OperationKind::Query,
            vec![],
            short
        ))
        .await
        .err(),
        Some(ExecutionError::Timeout)
    );
    let cancelled = CancellationToken::new();
    let clone = cancelled.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        clone.cancel();
    });
    assert_eq!(
        executor
            .execute(
                &config,
                &ns,
                credentials("sagat_target"),
                operation(
                    "SELECT pg_sleep(10)::text",
                    OperationKind::Query,
                    vec![],
                    limits
                ),
                deadline(),
                &cancelled
            )
            .await
            .err(),
        Some(ExecutionError::Cancelled)
    );
    // A long-running write has an unknown external outcome; it must never be retried automatically.
    assert_eq!(
        run(operation(
            "INSERT INTO orders SELECT 99, pg_sleep(10)::text",
            OperationKind::Execute,
            vec![],
            short
        ))
        .await
        .err(),
        Some(ExecutionError::OutcomeUnknown)
    );
    let held = CancellationToken::new();
    let op = || {
        operation(
            "SELECT pg_sleep(10)::text",
            OperationKind::Query,
            vec![],
            limits,
        )
    };
    let a = executor.execute(
        &config,
        &ns,
        credentials("sagat_target"),
        op(),
        deadline(),
        &held,
    );
    let b = executor.execute(
        &config,
        &ns,
        credentials("sagat_target"),
        op(),
        deadline(),
        &held,
    );
    let check = async {
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                let busy:i64=admin.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name='sagat-database-runner' AND state='active' AND query LIKE 'SELECT pg_sleep%'",&[]).await.unwrap().get(0);
                if busy==2 {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert_eq!(
            run(operation("SELECT 1", OperationKind::Query, vec![], limits))
                .await
                .err(),
            Some(ExecutionError::Capacity)
        );
        held.cancel();
    };
    let (a, b, ()) = tokio::join!(a, b, check);
    assert_eq!(a.err(), Some(ExecutionError::Cancelled));
    assert_eq!(b.err(), Some(ExecutionError::Cancelled));
    assert!(
        run(operation("SELECT 1", OperationKind::Query, vec![], limits))
            .await
            .is_ok()
    );
    drop(admin);
    pump.abort();
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for canary in [
        "fixture-password-canary",
        "sql-literal-canary",
        "DROP TABLE orders",
        "12345678901234567890.001230",
    ] {
        let leaking_targets: Vec<_> = logs
            .lines()
            .filter(|line| line.contains(canary))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["target"].clone())
            .collect();
        assert!(
            leaking_targets.is_empty(),
            "sensitive diagnostic targets: {leaking_targets:?}"
        );
    }
}
