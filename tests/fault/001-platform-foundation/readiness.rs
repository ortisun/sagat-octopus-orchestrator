//! `GET /v1/ready` under load and failure (second security review, findings 4 and 14): a flood of
//! probes costs one database ping per interval, a hanging ping cannot hang the probe, and the
//! reason a probe failed is logged without being returned to the caller.

use std::time::{Duration, Instant};

use reqwest::Method;
use sagat_api_http::{AuthConfig, HttpLimits};
use sagat_control_plane::config::{Config, DatabaseConfig, Environment, ServeConfig};
use sagat_control_plane::serve;
use sagat_telemetry::LogFormat;
use tokio::sync::oneshot;

use crate::support::api::{Options, json_of, start_with};
use crate::support::fresh_db;
use crate::support::logs::capture;
use crate::support::oidc::{AUDIENCE, ISSUER};

#[tokio::test]
async fn a_burst_of_concurrent_readiness_probes_issues_one_ping() {
    // Before the fix the cache was only consulted, never held: 300 concurrent probes that all saw
    // an empty cache issued 300 pings (review: 300 pings).
    let api = start_with(Options {
        ping_delay: Duration::from_millis(300),
        ..Options::default()
    })
    .await;
    let mut probes = Vec::new();
    for _ in 0..100 {
        let request = api.request(Method::GET, "/ready");
        probes.push(tokio::spawn(async move {
            request.send().await.expect("send").status().as_u16()
        }));
    }

    for probe in probes {
        assert_eq!(probe.await.expect("task"), 200);
    }

    assert_eq!(api.pings(), 1, "one refresh served every waiting probe");
}

#[tokio::test]
async fn a_ping_that_never_returns_makes_the_instance_not_ready_instead_of_hanging_the_probe() {
    let api = start_with(Options {
        ping_delay: Duration::from_secs(30),
        ..Options::default()
    })
    .await;

    let started = Instant::now();
    let response = api
        .request(Method::GET, "/ready")
        .send()
        .await
        .expect("send");

    assert_eq!(
        response.status(),
        503,
        "a database that does not answer is not ready"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the probe gives up after about 500 ms, not after the request timeout: {:?}",
        started.elapsed()
    );
    let body = json_of(response).await;
    assert_eq!(body["code"], "UNAVAILABLE");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
async fn the_reason_a_probe_failed_is_logged_but_not_returned() {
    // An unmigrated database: the schema check used to bypass the store's error logging, and the
    // handler threw the error away with `.is_ok()`, so an operator saw a bare 503 and no cause.
    let db = fresh_db().await; // no schema at all
    let mut auth = AuthConfig::new(ISSUER, AUDIENCE, "http://127.0.0.1:1/jwks.json");
    auth.allow_insecure_jwks_http = true;
    let config = Config {
        environment: Environment::Dev,
        log_format: LogFormat::Json,
        database: DatabaseConfig {
            url: db.url.clone(),
            max_connections: 2,
            statement_timeout: Duration::from_secs(3),
        },
    };
    let serve_config = ServeConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        auth,
        limits: HttpLimits::default(),
        server: sagat_api_http::ServerConfig::default(),
        rate_limit: sagat_api_http::RateLimitConfig::default(),
        subject_rate_limit: sagat_api_http::RateLimitConfig::subject_default(),
        admin_addr: None,
        migrate_on_start: false,
        secrets: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        serve(&config, &serve_config, listener, async {
            let _ = stopped.await;
        })
        .await
    });
    let client = reqwest::Client::new();
    let mut alive = false;
    for _ in 0..100 {
        if client
            .get(format!("http://{addr}/v1/health"))
            .send()
            .await
            .is_ok_and(|r| r.status() == 200)
        {
            alive = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(alive, "the server never came up");

    // Installed only now, right before the action under test (see `capture`).
    let (_logs, captured) = capture();
    let response = client
        .get(format!("http://{addr}/v1/ready"))
        .send()
        .await
        .expect("send");

    assert_eq!(response.status(), 503);
    let body = response.text().await.expect("body");
    assert!(
        !body.contains("migrat") && !body.contains("table"),
        "the cause is for operators, not for callers: {body}"
    );
    let logs = captured.text();
    assert!(
        logs.contains("WARN") && logs.contains("the database schema has not been migrated"),
        "the cause must be in the log: {logs:?}"
    );
    assert!(
        logs.contains("readiness probe failed"),
        "and the probe itself reports the failure: {logs:?}"
    );

    let _ = stop.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), server).await;
}
