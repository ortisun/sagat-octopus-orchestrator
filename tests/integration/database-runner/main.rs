#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod ad_hoc;
mod crash;
mod fairness;
mod performance;
mod proxy;
#[path = "../../support/mod.rs"]
mod support;

use reqwest::Method;
use sagat_application::{blobs::BlobLimits, worker::WorkerSettings};
use sagat_domain::identity::Role;
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use support::{
    api::Options,
    engine::document,
    worker::{Platform, platform_with},
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner},
};
use uuid::Uuid;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Process {
    async fn stop(&mut self) {
        assert!(
            Command::new("kill")
                .args(["-TERM", &self.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
    }
}
async fn finished(p: &Platform, run: &str, expected: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let status = p.status(run).await;
            if status != "RUNNING" {
                assert_eq!(status, expected, "run {run}");
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("runner must finish the durable run");
}
async fn start(p: &Platform, name: &str, operation: &str, sql: &str, parameters: Value) -> String {
    let id = p
        .publish(
            name,
            &document(
                name,
                "database",
                vec![json!({
                    "id":"database","type":"database","connectionRef":{"name":"target"},
                    "operation":operation,"statement":sql,"parameters":parameters
                })],
            ),
        )
        .await;
    p.start(&id, &json!({})).await
}
async fn result(p: &Platform, ns: &str, run: &str) -> Value {
    let (status, value) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/runs/{run}/database-results/database"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{value}");
    value
}
async fn failure(p: &Platform, run: &str, expected: &str) {
    finished(p, run, "FAILED").await;
    let events: Vec<Value> =
        sqlx::query_scalar("SELECT detail FROM run_events WHERE run_id=$1 ORDER BY seq")
            .bind(run.parse::<Uuid>().unwrap())
            .fetch_all(&p.api.db.pool)
            .await
            .unwrap();
    assert!(
        serde_json::to_string(&events).unwrap().contains(expected),
        "{events:?}"
    );
}

#[tokio::test]
async fn isolated_process_executes_openbao_capabilities_and_reports_bounded_results() {
    scenario(false).await;
}

#[tokio::test]
#[ignore = "measurement: run explicitly on a quiet machine with --ignored"]
async fn measure_managed_database_phase_latency() {
    scenario(true).await;
}

async fn scenario(benchmark: bool) {
    let binary = std::env::var_os("SAGAT_DATABASE_RUNNER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| support::built_binary("database-runner"));
    assert!(
        binary.is_file(),
        "first run cargo build -p sagat-database-runner --bin database-runner"
    );
    let dir = tempfile::tempdir().unwrap();
    let collector = support::collector::Collector::start().await;
    let bao = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", "runner-bootstrap")
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .unwrap();
    let bao_token = dir.path().join("bao-token");
    std::fs::write(&bao_token, "runner-bootstrap").unwrap();
    let provider = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!(
                "http://127.0.0.1:{}",
                bao.get_host_port_ipv4(8200).await.unwrap()
            ),
            mount: "secret".into(),
            token_file: bao_token,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .unwrap(),
    );
    let p = platform_with(Options {
        secret_provider: Some(provider),
        runtime_role: true,
        blobs: Some(BlobLimits::default()),
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(8),
            ..Default::default()
        },
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    let proxy = proxy::start(&p.api.grpc_url).await;
    let target = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .unwrap();
    let port = target.get_host_port_ipv4(5432).await.unwrap();
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
        ))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE ROLE managed_reader LOGIN PASSWORD 'runner-password-canary'; CREATE TABLE effects(id bigint PRIMARY KEY, value text); GRANT SELECT,INSERT,UPDATE,DELETE ON effects TO managed_reader;")
        .execute(&admin).await.unwrap();
    let secret =
        json!({"username":"managed_reader","password":"runner-password-canary"}).to_string();
    assert_eq!(p.rest(Method::PUT,&format!("/namespaces/{ns}/secrets/credential"),Some(&json!({
        "value":secret,"requestId":Uuid::new_v4().to_string(),"expectedVersion":0,"requiredProvider":"openbao"
    }))).await.0,200);
    assert_eq!(
        p.rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/target"),
            Some(&json!({
                "requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"status":"ACTIVE","config":{
                    "driver":"postgresql","host":"127.0.0.1","port":port,"database":"postgres",
                    "tls":{"mode":"development-plaintext"},"secretName":"credential"
                }
            }))
        )
        .await
        .0,
        200
    );
    p.grant(p.tenant, "isolated-runner", Role::DatabaseRunner)
        .await;
    p.bind_everything("isolated-runner").await;
    let jwt = p.api.idp.token("isolated-runner", "acme").build();
    let token_file = dir.path().join("runner-token");
    std::fs::write(&token_file, &jwt).unwrap();
    let limits = json!({"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,
        "maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":1,"idleTimeoutMs":30000});
    let mut config = json!({"grpcUrl":proxy.url,"tokenFile":token_file,"name":"process-fixture",
        "maxConcurrent":2,"pollIntervalMs":100,"rpcTimeoutMs":5000,"developmentLoopback":true,
        "deployment":{"allowCidrs":["127.0.0.0/8"],"allowPorts":[port],"denyCidrs":[],"limits":limits},
        "namespaces":[{"id":ns,"slug":"payments","allowCidrs":["127.0.0.0/8"],"allowPorts":[port],"limits":limits}]
    });
    let config_file = dir.path().join("runner.json");
    std::fs::write(&config_file, config.to_string()).unwrap();
    let metrics_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let metrics_addr = metrics_listener.local_addr().unwrap();
    drop(metrics_listener);
    let log_file = dir.path().join("runner.log");
    let logs = std::fs::File::create(&log_file).unwrap();
    let mut child = Process(
        Command::new(&binary)
            .env_clear()
            .env("SAGAT_DATABASE_RUNNER_CONFIG", &config_file)
            .env(
                "SAGAT_DATABASE_RUNNER_METRICS_ADDR",
                metrics_addr.to_string(),
            )
            .env("RUST_LOG", if benchmark { "info" } else { "trace" })
            .env("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", &collector.url)
            .stdout(Stdio::from(logs.try_clone().unwrap()))
            .stderr(Stdio::from(logs))
            .spawn()
            .unwrap(),
    );
    if benchmark {
        performance::run(&p, &ns, &proxy, &collector).await;
        child.stop().await;
        admin.close().await;
        return;
    }
    ad_hoc::run(&p, &ns).await;
    let run = start(
        &p,
        "typed-process",
        "query",
        "SELECT $1::bigint AS integer, $2::numeric AS decimal, $3::text AS text",
        json!([{"literal":{"type":"integer","value":"9223372036854775807"}},
            {"literal":{"type":"decimal","value":"12345678901234567890.123456789"}},
            {"literal":{"type":"text","value":"parameter-value-canary"}}]),
    )
    .await;
    finished(&p, &run, "SUCCEEDED").await;
    let output = result(&p, &ns, &run).await;
    assert_eq!(output["rows"][0][0]["value"], "9223372036854775807");
    assert_eq!(
        output["rows"][0][1]["value"],
        "12345678901234567890.123456789"
    );
    assert_eq!(output["rows"][0][2]["value"], "parameter-value-canary");
    let large = start(
        &p,
        "blob-process",
        "query",
        "SELECT repeat('x',350000) AS text",
        json!([]),
    )
    .await;
    finished(&p, &large, "SUCCEEDED").await;
    assert_eq!(
        result(&p, &ns, &large).await["rows"][0][0]["value"]
            .as_str()
            .unwrap()
            .len(),
        350000
    );
    proxy.faults.lose_complete_ack.store(true, Ordering::SeqCst);
    let write = start(
        &p,
        "write-process",
        "execute",
        "INSERT INTO effects VALUES (1, $1) RETURNING id",
        json!([{"literal":{"type":"text","value":"sql-result-canary"}}]),
    )
    .await;
    finished(&p, &write, "SUCCEEDED").await;
    assert_eq!(result(&p, &ns, &write).await["affectedRows"], "1");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM effects")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    tokio::time::timeout(Duration::from_secs(5), async {
        while proxy.faults.receipts.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("lost committed ack must replay the durable receipt");
    // A statement still running after the first heartbeat retains its lease.
    let slow = start(
        &p,
        "heartbeat-process",
        "query",
        "SELECT 1::bigint AS value FROM pg_sleep(4)",
        json!([]),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(8),async {
        loop {
            let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE usename='managed_reader' AND state='active')").fetch_one(&admin).await.unwrap();
            if busy {break;} tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.unwrap();
    let crowded = start(
        &p,
        "pool-policy-process",
        "query",
        "SELECT 1::bigint AS value",
        json!([]),
    )
    .await;
    failure(&p, &crowded, "DATABASE_CAPACITY_EXHAUSTED").await;
    finished(&p, &slow, "SUCCEEDED").await;
    assert!(proxy.faults.heartbeats.load(Ordering::SeqCst) > 0);
    proxy.faults.wrong_namespace.store(true, Ordering::SeqCst);
    let invalid = start(
        &p,
        "scope-process",
        "execute",
        "INSERT INTO effects VALUES (2,'must-not-execute')",
        json!([]),
    )
    .await;
    failure(&p, &invalid, "DATABASE_CAPABILITY_UNAVAILABLE").await;
    proxy.faults.wrong_namespace.store(false, Ordering::SeqCst);
    proxy.faults.fail_heartbeat.store(true, Ordering::SeqCst);
    let interrupted = start(
        &p,
        "heartbeat-loss-process",
        "query",
        "SELECT 1::bigint AS value FROM pg_sleep(6)",
        json!([]),
    )
    .await;
    failure(&p, &interrupted, "DATABASE_CANCELLED").await;
    proxy.faults.fail_heartbeat.store(false, Ordering::SeqCst);
    let busy: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE usename='managed_reader'")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(
        busy, 0,
        "capacity returns only after the cancelled target session closes"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM effects")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // Policy is reread before each capability; changing it requires no process restart.
    config["namespaces"][0]["allowCidrs"] = json!(["192.0.2.0/24"]);
    std::fs::write(&config_file, config.to_string()).unwrap();
    let denied = start(
        &p,
        "egress-process",
        "query",
        "SELECT 1::bigint AS value",
        json!([]),
    )
    .await;
    failure(&p, &denied, "DATABASE_DESTINATION_DENIED").await;
    let metrics = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(metrics.contains("database_runner_attempt_seconds"));
    assert!(metrics.contains("DATABASE_CAPACITY_EXHAUSTED"));
    child.stop().await;
    crash::run(&p, &proxy, &admin, &mut config, &binary, &config_file).await;
    let spans = collector.named("database.runner.attempt");
    let first = spans
        .iter()
        .find(|span| span.attr("run_id") == Some(run.as_str()))
        .expect("isolated process exports the run attempt span");
    assert_eq!(first.attr("tenant_id"), Some(p.tenant.to_string().as_str()));
    assert_eq!(first.attr("namespace_id"), Some(ns.as_str()));
    assert!(first.parent.is_some());
    let exported = format!("{:?}", collector.spans());
    for sensitive in [
        &jwt,
        "runner-password-canary",
        "parameter-value-canary",
        "sql-result-canary",
        "SELECT $1",
    ] {
        assert!(!exported.contains(sensitive));
    }
    fairness::run(&p, &mut config, &binary, &config_file, &log_file).await;
    let logs = std::fs::read_to_string(log_file).unwrap();
    for sensitive in [
        &jwt,
        "runner-password-canary",
        "managed_reader",
        "parameter-value-canary",
        "sql-result-canary",
        "SELECT $1",
        "INSERT INTO effects",
    ] {
        assert!(
            !logs.contains(sensitive),
            "TRACE logs exposed protected data"
        );
    }
    assert!(logs.contains("database execution finished"));
    admin.close().await;
}
