//! Real browser, PostgreSQL, OpenBao and the separate managed database runner.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "../../support/mod.rs"]
mod support;
use sagat_api_http::studio::{StudioConfig, StudioLimits};
use sagat_application::blobs::BlobLimits;
use sagat_domain::identity::{Membership, Role};
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use serde_json::json;
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires Node, frontend dependencies and Playwright Chromium; execute explicitly with --ignored"]
async fn catalog_and_secrets_in_a_real_browser() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    if std::env::var("SAGAT_STUDIO_REBUILD").as_deref() != Ok("0") {
        let mut build = Command::new("npm");
        build
            .args(["run", "build"])
            .current_dir(repo.join("frontend"))
            .stdin(Stdio::null());
        assert!(
            support::bounded::exit_status(
                "build the Studio",
                support::bounded::PROCESS_LIMIT,
                build
            )
            .await
            .success()
        );
    }
    let container = support::bounded::start_openbao("sagat-browser-bootstrap").await;
    let port = container.get_host_port_ipv4(8200).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let token_file = dir.path().join("token");
    std::fs::write(&token_file, "sagat-browser-bootstrap").unwrap();
    let provider = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            token_file,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .unwrap(),
    );
    let api = support::api::start_with(support::api::Options {
        secret_provider: Some(provider),
        runtime_role: true,
        blobs: Some(BlobLimits::default()),
        studio: Some(StudioConfig {
            dir: repo.join("frontend/apps/studio/dist"),
            oidc: None,
            trace_url: None,
            limits: StudioLimits::default(),
        }),
        ..Default::default()
    })
    .await;
    let tenant = api.tenant("database-browser", "owner").await;
    let workspace = api
        .create_workspace("owner", "database-browser", "workspace")
        .await;
    let namespace = api
        .create_namespace("owner", "database-browser", &workspace, "database", "dev")
        .await;
    let target = support::bounded::start_postgres_target().await;
    let target_port = target.get_host_port_ipv4(5432).await.unwrap();
    let target_admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://postgres:postgres@127.0.0.1:{target_port}/postgres"
        ))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE ROLE browser_reader LOGIN PASSWORD 'browser-target-password'; CREATE TABLE browser_effects(id bigint PRIMARY KEY); GRANT SELECT,INSERT,UPDATE,DELETE ON browser_effects TO browser_reader;")
        .execute(&target_admin).await.unwrap();
    for (subject, role) in [
        ("browser-runner", Role::DatabaseRunner),
        ("browser-developer", Role::Developer),
        ("browser-viewer", Role::Viewer),
    ] {
        api.owner_store
            .insert_membership(&Membership::restore(
                tenant,
                support::subject(subject),
                role,
                support::now(),
            ))
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO worker_bindings (tenant_id,namespace_id,subject,pattern,updated_by) VALUES ($1,$2,'browser-runner','*','owner')")
        .bind(tenant.as_uuid()).bind(namespace.parse::<uuid::Uuid>().unwrap()).execute(&api.db.pool).await.unwrap();
    let runner_token = dir.path().join("runner-token");
    std::fs::write(
        &runner_token,
        api.idp.token("browser-runner", "database-browser").build(),
    )
    .unwrap();
    let limits = json!({"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,"maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":2,"idleTimeoutMs":30000});
    let config = json!({"grpcUrl":api.grpc_url,"tokenFile":runner_token,"name":"browser-runner","maxConcurrent":2,"pollIntervalMs":100,"rpcTimeoutMs":5000,"developmentLoopback":true,
        "deployment":{"allowCidrs":["127.0.0.0/8"],"allowPorts":[target_port],"denyCidrs":[],"limits":limits},
        "namespaces":[{"id":namespace,"slug":"database","allowCidrs":["127.0.0.0/8"],"allowPorts":[target_port],"limits":limits}]});
    let config_file = dir.path().join("runner.json");
    std::fs::write(&config_file, config.to_string()).unwrap();
    let logs = std::fs::File::create(dir.path().join("runner.log")).unwrap();
    let binary = std::env::var_os("SAGAT_DATABASE_RUNNER_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| support::built_binary("database-runner"));
    let mut runner = Command::new(binary)
        .env_clear()
        .env("SAGAT_DATABASE_RUNNER_CONFIG", config_file)
        .env("SAGAT_DATABASE_RUNNER_METRICS_ADDR", "127.0.0.1:0")
        .stdout(Stdio::from(logs.try_clone().unwrap()))
        .stderr(Stdio::from(logs))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let token = api.idp.token("owner", "database-browser").build();
    let mut journeys = Command::new("npx");
    journeys
        .args(["playwright", "test", "database-connections.spec.ts"])
        .current_dir(repo.join("frontend/e2e"))
        .env("STUDIO_DATABASE_CONNECTIONS", "1")
        .env("STUDIO_ORIGIN", api.origin())
        .env("STUDIO_TOKEN_ADMIN", token)
        .env("STUDIO_NAMESPACE_ID", &namespace)
        .env("STUDIO_TARGET_PORT", target_port.to_string())
        .env(
            "STUDIO_TOKEN_DEVELOPER",
            api.idp
                .token("browser-developer", "database-browser")
                .build(),
        )
        .env(
            "STUDIO_TOKEN_VIEWER",
            api.idp.token("browser-viewer", "database-browser").build(),
        )
        .stdin(Stdio::null());
    let status = support::bounded::exit_status(
        "Playwright database journeys",
        support::bounded::PROCESS_LIMIT,
        journeys,
    )
    .await;
    let _ = runner.kill().await;
    let _ = runner.wait().await;
    assert!(
        status.success(),
        "database browser journeys failed; inspect frontend/e2e/test-results"
    );
    // Only the metadata may be in Sagat, despite credentials having traversed the form.
    let row: (i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM secret_versions WHERE provider_operation IS NOT NULL),(SELECT count(*) FROM database_connections)")
        .fetch_one(&api.db.pool).await.unwrap();
    assert!(row.0 >= 1 && row.1 >= 1);
    let effects: i64 =
        sqlx::query_scalar("SELECT count(*) FROM browser_effects WHERE id=9007199254740993")
            .fetch_one(&target_admin)
            .await
            .unwrap();
    assert_eq!(
        effects, 1,
        "lost acknowledgement retry must reuse the accepted run"
    );
}
