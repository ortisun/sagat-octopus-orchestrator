//! Real browser, PostgreSQL and OpenBao catalog/credential journeys. No managed SQL claim here.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "../../support/mod.rs"]
mod support;
use sagat_api_http::studio::{StudioConfig, StudioLimits};
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires Node, frontend dependencies and Playwright Chromium; execute explicitly with --ignored"]
async fn catalog_and_secrets_in_a_real_browser() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    if std::env::var("SAGAT_STUDIO_REBUILD").as_deref() != Ok("0") {
        assert!(
            Command::new("npm")
                .args(["run", "build"])
                .current_dir(repo.join("frontend"))
                .stdin(Stdio::null())
                .status()
                .await
                .unwrap()
                .success()
        );
    }
    let container = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", "sagat-browser-bootstrap")
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .unwrap();
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
        studio: Some(StudioConfig {
            dir: repo.join("frontend/apps/studio/dist"),
            oidc: None,
            trace_url: None,
            limits: StudioLimits::default(),
        }),
        ..Default::default()
    })
    .await;
    api.tenant("database-browser", "owner").await;
    let workspace = api
        .create_workspace("owner", "database-browser", "workspace")
        .await;
    let namespace = api
        .create_namespace("owner", "database-browser", &workspace, "database", "dev")
        .await;
    let token = api.idp.token("owner", "database-browser").build();
    let status = Command::new("npx")
        .args(["playwright", "test", "database-connections.spec.ts"])
        .current_dir(repo.join("frontend/e2e"))
        .env("STUDIO_DATABASE_CONNECTIONS", "1")
        .env("STUDIO_ORIGIN", api.origin())
        .env("STUDIO_TOKEN_ADMIN", token)
        .env("STUDIO_NAMESPACE_ID", &namespace)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .unwrap();
    assert!(
        status.success(),
        "database browser journeys failed; inspect frontend/e2e/test-results"
    );
    // Only the metadata may be in Sagat, despite credentials having traversed the form.
    let row: (i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM secret_versions WHERE provider_operation IS NOT NULL),(SELECT count(*) FROM database_connections)")
        .fetch_one(&api.db.pool).await.unwrap();
    assert!(row.0 >= 1 && row.1 >= 1);
}
