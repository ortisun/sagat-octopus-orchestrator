//! US03 under failure (spec edge case "migration interrupted and repeated"): an instance must not
//! be considered ready while its schema is missing, and must become ready by itself once migrated.

use std::time::Duration;

use reqwest::Client;
use sagat_api_http::{AuthConfig, HttpLimits};
use sagat_control_plane::config::{Config, DatabaseConfig, Environment, ServeConfig};
use sagat_control_plane::{healthcheck, run_migrations, serve};
use sagat_telemetry::LogFormat;
use tokio::sync::oneshot;

use crate::support::fresh_db;
use crate::support::oidc::{AUDIENCE, ISSUER};

fn cfg(url: &str) -> (Config, ServeConfig) {
    let mut auth = AuthConfig::new(ISSUER, AUDIENCE, "http://127.0.0.1:1/jwks.json");
    auth.allow_insecure_jwks_http = true;
    (
        Config {
            environment: Environment::Dev,
            log_format: LogFormat::Json,
            database: DatabaseConfig {
                url: url.to_owned(),
                max_connections: 4,
                statement_timeout: Duration::from_secs(3),
            },
        },
        ServeConfig {
            redis_callbacks: false,
            schedules_enabled: false,
            openbao: Default::default(),
            bind_addr: "127.0.0.1:0".parse().expect("addr"),
            tls: None,
            webhook_oidc: Vec::new(),
            webhook_oidc_development: true,
            blobs: None,
            blob_transfer: sagat_api_http::HttpBlobTransfer::default(),
            auth,
            limits: HttpLimits::default(),
            server: sagat_api_http::ServerConfig::default(),
            rate_limit: sagat_api_http::RateLimitConfig::default(),
            subject_rate_limit: sagat_api_http::RateLimitConfig::subject_default(),
            admin_addr: None,
            migrate_on_start: false,
            secrets: None,
            scripts: None,
            studio: None,
            streams: sagat_api_http::streams::StreamLimits::default(),
            runbook_base: "docs/runbooks/alerts.md".to_owned(),
            telemetry_lag_budget: std::time::Duration::from_secs(30),
        },
    )
}

#[tokio::test]
async fn an_instance_pointed_at_an_unmigrated_database_is_alive_but_not_ready_until_migrated() {
    let db = fresh_db().await; // a database with no schema at all
    let (config, serve_cfg) = cfg(&db.url);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        serve(&config, &serve_cfg, listener, async {
            let _ = stopped.await;
        })
        .await
    });
    for _ in 0..100 {
        if healthcheck(&addr.to_string()).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let client = Client::new();
    let url = |path: &str| format!("http://{addr}/v1{path}");

    // Alive (do not restart it), but NOT ready (do not route traffic to it).
    assert_eq!(
        client
            .get(url("/health"))
            .send()
            .await
            .expect("send")
            .status(),
        200
    );
    let not_ready = client.get(url("/ready")).send().await.expect("send");
    assert_eq!(
        not_ready.status(),
        503,
        "the schema is missing, so the instance cannot serve"
    );
    let body: serde_json::Value = not_ready.json().await.expect("json");
    assert_eq!(body["code"], "UNAVAILABLE");
    assert_eq!(body["retryable"], true);

    // The operator (or the migrate job) finishes the migration; no restart of the API is needed.
    let (config, _) = cfg(&db.url);
    run_migrations(&config.database).await.expect("migrate");
    let mut ready = false;
    for _ in 0..40 {
        if client
            .get(url("/ready"))
            .send()
            .await
            .expect("send")
            .status()
            == 200
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        ready,
        "the instance must become ready by itself once the schema exists"
    );

    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("shutdown")
        .expect("join")
        .expect("clean");
}
