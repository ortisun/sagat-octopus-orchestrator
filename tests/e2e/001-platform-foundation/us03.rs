//! US03 — Bring up a complete local environment (FR-001-004, FR-001-005).
//!
//! Given the repository and its prerequisites, when the developer migrates, seeds and starts the
//! control plane, then the API and PostgreSQL are healthy and migrations are idempotent.
//!
//! These tests run the same code paths as the `control-plane` executable (`run_migrations`,
//! `seed_dev`, `serve`) against a real PostgreSQL. The container composition itself
//! (`infra/docker/compose.yaml`) is verified by the procedure in `quickstart.md`.

use std::time::Duration;

use reqwest::Client;
use sagat_api_http::{AuthConfig, HttpLimits};
use sagat_control_plane::config::{Config, DatabaseConfig, Environment, SeedConfig, ServeConfig};
use sagat_control_plane::{RunError, SeedOutcome, healthcheck, run_migrations, seed_dev, serve};
use sagat_infra_postgres::LeastPrivilegeError;
use sagat_telemetry::LogFormat;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::support::oidc::{AUDIENCE, ISSUER, TestIdp};
use crate::support::{TestDb, fresh_db, migrated_db};

fn config(db: &TestDb, environment: Environment) -> Config {
    Config {
        environment,
        log_format: LogFormat::Json,
        database: DatabaseConfig {
            url: db.url.clone(),
            max_connections: 5,
            statement_timeout: Duration::from_secs(3),
        },
    }
}

fn serve_config(jwks_uri: &str) -> ServeConfig {
    let mut auth = AuthConfig::new(ISSUER, AUDIENCE, jwks_uri);
    auth.allow_insecure_jwks_http = true;
    ServeConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        auth,
        limits: HttpLimits::default(),
        server: sagat_api_http::ServerConfig::default(),
        rate_limit: sagat_api_http::RateLimitConfig::default(),
        subject_rate_limit: sagat_api_http::RateLimitConfig::subject_default(),
        admin_addr: None,
        migrate_on_start: false,
        secrets: None,
        scripts: None,
    }
}

/// A running control plane that can be stopped gracefully.
struct Running {
    addr: std::net::SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    handle: JoinHandle<Result<(), RunError>>,
}

impl Running {
    async fn start(config: Config, serve_config: ServeConfig) -> Self {
        let listener = tokio::net::TcpListener::bind(serve_config.bind_addr)
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stop, stopped) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            serve(&config, &serve_config, listener, async {
                let _ = stopped.await;
            })
            .await
        });
        // Wait until it answers, so tests do not race the start-up.
        for _ in 0..100 {
            if healthcheck(&addr.to_string()).await.is_ok() {
                return Self {
                    addr,
                    stop: Some(stop),
                    handle,
                };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the control plane never became healthy");
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}/v1{path}", self.addr)
    }

    async fn stop(mut self) -> Result<(), RunError> {
        let _ = self.stop.take().expect("not stopped yet").send(());
        tokio::time::timeout(Duration::from_secs(10), self.handle)
            .await
            .expect("graceful shutdown must finish")
            .expect("task joined")
    }
}

async fn applied_migrations(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn migrating_twice_changes_nothing_and_the_api_comes_up_healthy() {
    // Given an empty database
    let db = fresh_db().await;
    let cfg = config(&db, Environment::Dev);

    // When the developer runs the migrations twice (and several runners start at once)
    run_migrations(&cfg.database).await.expect("first run");
    let after_first = applied_migrations(&db).await;
    run_migrations(&cfg.database)
        .await
        .expect("second run is a no-op");
    let concurrent: Vec<_> = (0..4)
        .map(|_| {
            let database = cfg.database.clone();
            tokio::spawn(async move { run_migrations(&database).await })
        })
        .collect();
    for run in concurrent {
        run.await
            .expect("join")
            .expect("concurrent runners succeed");
    }
    assert_eq!(
        applied_migrations(&db).await,
        after_first,
        "nothing was applied twice"
    );
    assert!(after_first >= 1);

    // And starts the API
    let api = Running::start(cfg, serve_config("http://127.0.0.1:1/jwks.json")).await;
    let client = Client::new();

    // Then the API and PostgreSQL report healthy
    let health = client.get(api.url("/health")).send().await.expect("send");
    assert_eq!(health.status(), 200);
    let ready = client.get(api.url("/ready")).send().await.expect("send");
    assert_eq!(ready.status(), 200, "readiness includes the database");
    assert!(
        healthcheck(&api.addr.to_string()).await.is_ok(),
        "the container probe agrees"
    );

    api.stop().await.expect("clean shutdown");
}

#[tokio::test]
async fn the_development_seed_yields_a_tenant_that_can_really_sign_in_and_is_idempotent() {
    let db = migrated_db().await;
    let cfg = config(&db, Environment::Dev);
    let seed = SeedConfig {
        tenant_slug: "dev".into(),
        admin_subject: "dev-admin".into(),
    };

    assert_eq!(
        seed_dev(&cfg, &seed).await.expect("first seed"),
        SeedOutcome::Created
    );
    assert_eq!(
        seed_dev(&cfg, &seed).await.expect("second seed"),
        SeedOutcome::AlreadyPresent
    );

    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(&db.pool)
        .await
        .expect("n");
    let admins: Vec<(String, String)> = sqlx::query_as("SELECT subject, role FROM memberships")
        .fetch_all(&db.pool)
        .await
        .expect("rows");
    assert_eq!(tenants, 1);
    assert_eq!(admins, vec![("dev-admin".to_owned(), "admin".to_owned())]);

    // The seeded administrator can authenticate against the running API with a real token.
    let idp = TestIdp::start().await;
    let api = Running::start(cfg, serve_config(&idp.jwks_url())).await;
    let response = Client::new()
        .get(api.url("/auth/context"))
        .bearer_auth(idp.token("dev-admin", "dev").build())
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(
        (body["tenant"].as_str(), body["role"].as_str()),
        (Some("dev"), Some("admin"))
    );
    api.stop().await.expect("clean shutdown");
}

#[tokio::test]
async fn the_seed_is_refused_outside_development_and_leaves_the_database_untouched() {
    let db = migrated_db().await;
    let seed = SeedConfig {
        tenant_slug: "dev".into(),
        admin_subject: "dev-admin".into(),
    };

    for environment in [Environment::Prod, Environment::Staging] {
        let result = seed_dev(&config(&db, environment), &seed).await;
        assert!(
            matches!(result, Err(RunError::Refused(_))),
            "{environment:?}: {result:?}"
        );
    }

    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(&db.pool)
        .await
        .expect("n");
    assert_eq!(tenants, 0, "no data was written");
}

#[tokio::test]
async fn an_invalid_seed_identity_is_a_configuration_error() {
    let db = migrated_db().await;
    let cfg = config(&db, Environment::Dev);
    let bad_slug = SeedConfig {
        tenant_slug: "NOT A SLUG".into(),
        admin_subject: "dev-admin".into(),
    };
    let bad_subject = SeedConfig {
        tenant_slug: "dev".into(),
        admin_subject: String::new(),
    };

    for seed in [bad_slug, bad_subject] {
        let error = seed_dev(&cfg, &seed).await.expect_err("invalid");
        assert!(matches!(error, RunError::Config(_)), "{error:?}");
        assert_eq!(error.exit_code(), 2);
    }
}

#[tokio::test]
async fn migrating_on_start_prepares_an_empty_database_before_listening() {
    let db = fresh_db().await; // no schema at all
    let mut serve_cfg = serve_config("http://127.0.0.1:1/jwks.json");
    serve_cfg.migrate_on_start = true;

    let api = Running::start(config(&db, Environment::Dev), serve_cfg).await;

    let ready = Client::new()
        .get(api.url("/ready"))
        .send()
        .await
        .expect("send");
    assert_eq!(
        ready.status(),
        200,
        "the schema existed before the first request"
    );
    api.stop().await.expect("clean shutdown");
}

/// Serve settings that are legal outside development: an https key URL and no insecure opt-in.
fn production_serve_config() -> ServeConfig {
    let mut serve_cfg = serve_config("https://idp.example.invalid/.well-known/jwks.json");
    serve_cfg.auth.allow_insecure_jwks_http = false;
    serve_cfg
}

/// A config that connects with the given database user instead of the owner.
fn config_as(db: &TestDb, environment: Environment, user: &str, password: &str) -> Config {
    let mut cfg = config(db, environment);
    cfg.database.url = db.url_for(user, password);
    cfg
}

#[tokio::test]
async fn outside_development_the_process_refuses_to_serve_as_the_database_owner() {
    // Review finding 10: nothing stopped an operator from pointing DATABASE_URL at the owner role,
    // which can disable the audit triggers that make the trail append-only.
    let db = migrated_db().await;
    for environment in [Environment::Prod, Environment::Staging] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");

        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            serve(
                &config(&db, environment), // the owner (a superuser) in the test cluster
                &production_serve_config(),
                listener,
                std::future::pending(),
            ),
        )
        .await
        .expect("it must refuse at once instead of starting to serve");

        let error = outcome.expect_err("refused");
        assert!(
            matches!(
                error,
                RunError::LeastPrivilege(LeastPrivilegeError::TooPrivileged { .. })
            ),
            "{error:?}"
        );
        assert_eq!(
            error.exit_code(),
            2,
            "{environment:?}: a configuration problem"
        );
        let message = error.to_string();
        assert!(
            message.contains("superuser") && message.contains("audit"),
            "{message}"
        );
        assert!(
            !message.contains("postgres://") && !message.contains("@127.0.0.1"),
            "no connection string in: {message}"
        );
    }
}

#[tokio::test]
async fn outside_development_the_runtime_role_serves_normally() {
    let db = migrated_db().await;
    let cfg = config_as(
        &db,
        Environment::Staging,
        crate::support::RUNTIME_ROLE,
        crate::support::RUNTIME_ROLE_PASSWORD,
    );

    let api = Running::start(cfg, production_serve_config()).await;

    let ready = Client::new()
        .get(api.url("/ready"))
        .send()
        .await
        .expect("send");
    assert_eq!(ready.status(), 200);
    api.stop().await.expect("clean shutdown");
}

#[tokio::test]
async fn in_development_the_role_is_not_checked() {
    // The development stack and most tests run as the owner; only real environments are guarded.
    let db = migrated_db().await;

    let api = Running::start(
        config(&db, Environment::Dev),
        serve_config("http://127.0.0.1:1/jwks.json"),
    )
    .await;

    api.stop().await.expect("clean shutdown");
}

#[tokio::test]
async fn outside_development_an_unmigrated_database_is_refused_at_start() {
    let db = fresh_db().await;
    let cfg = config_as(
        &db,
        Environment::Prod,
        crate::support::RUNTIME_ROLE,
        crate::support::RUNTIME_ROLE_PASSWORD,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");

    let error = tokio::time::timeout(
        Duration::from_secs(10),
        serve(
            &cfg,
            &production_serve_config(),
            listener,
            std::future::pending(),
        ),
    )
    .await
    .expect("refused at once")
    .expect_err("no schema");

    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("migrate"), "{error}");
}

#[tokio::test]
async fn migrating_on_start_is_refused_outside_development() {
    // It needs the schema-owning role, which is exactly what the guard above refuses to serve with.
    let db = migrated_db().await;
    let mut serve_cfg = production_serve_config();
    serve_cfg.migrate_on_start = true;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");

    let error = tokio::time::timeout(
        Duration::from_secs(10),
        serve(
            &config(&db, Environment::Prod),
            &serve_cfg,
            listener,
            std::future::pending(),
        ),
    )
    .await
    .expect("refused at once instead of migrating and serving")
    .expect_err("refused");

    assert!(matches!(error, RunError::Refused(_)), "{error:?}");
    assert_eq!(error.exit_code(), 2);
    assert!(
        error.to_string().contains("SAGAT_MIGRATE_ON_START"),
        "{error}"
    );
}

#[tokio::test]
async fn a_bad_setup_stops_the_process_before_it_listens() {
    let db = migrated_db().await;

    // An http JWKS URL without the explicit development opt-in.
    let mut insecure = serve_config("http://idp.example.test/jwks.json");
    insecure.auth.allow_insecure_jwks_http = false;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let error = serve(
        &config(&db, Environment::Prod),
        &insecure,
        listener,
        std::future::pending(),
    )
    .await
    .expect_err("insecure JWKS URL");
    assert!(matches!(error, RunError::Auth(_)), "{error:?}");
    assert_eq!(error.exit_code(), 2);

    // A database that is not there.
    // Development: the insecure JWKS URL of `serve_config` is only legal there, and this case is
    // about the database being unreachable, not about that flag.
    let mut unreachable = config(&db, Environment::Dev);
    unreachable.database.url = "postgres://nobody:none@127.0.0.1:1/missing".into();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let error = serve(
        &unreachable,
        &serve_config("http://127.0.0.1:1/j"),
        listener,
        std::future::pending(),
    )
    .await
    .expect_err("unreachable database");
    assert!(matches!(error, RunError::Database(_)), "{error:?}");
    assert!(
        !error.to_string().contains("none"),
        "credentials must not leak: {error}"
    );
}

#[tokio::test]
async fn migrations_are_not_subject_to_the_statement_timeout_that_protects_normal_traffic() {
    // A legitimate DDL statement can outlast the per-statement limit used for requests.
    let db = fresh_db().await;
    let mut cfg = config(&db, Environment::Dev);
    cfg.database.statement_timeout = Duration::from_millis(1);

    run_migrations(&cfg.database)
        .await
        .expect("migrations ignore the request statement timeout");

    assert!(applied_migrations(&db).await >= 1);
}

#[tokio::test]
async fn shutdown_lets_a_request_that_is_still_waiting_on_the_identity_provider_finish() {
    // This request holds no database connection, so only the server's own graceful shutdown (not
    // the pool drain) can keep it alive.
    let db = migrated_db().await;
    let idp = TestIdp::start().await;
    idp.set_mode(crate::support::oidc::Mode::Hang); // key fetch blocks until the client timeout
    let cfg = config(&db, Environment::Dev);
    seed_dev(
        &cfg,
        &SeedConfig {
            tenant_slug: "dev".into(),
            admin_subject: "dev-admin".into(),
        },
    )
    .await
    .expect("seed");
    let api = Running::start(cfg, serve_config(&idp.jwks_url())).await;
    let token = idp.token("dev-admin", "dev").build();
    let url = api.url("/auth/context");
    let in_flight = tokio::spawn(async move {
        Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .expect("in-flight request completes")
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    let stopping = tokio::spawn(api.stop());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !stopping.is_finished(),
        "shutdown must wait for the request that is still running"
    );

    let response = in_flight.await.expect("join");
    assert_eq!(
        response.status(),
        503,
        "it received its real (fail-closed) answer, not a reset"
    );
    stopping
        .await
        .expect("join")
        .expect("clean shutdown after draining");
}

#[tokio::test]
async fn shutdown_waits_for_database_work_that_is_still_running() {
    let db = migrated_db().await;
    let idp = TestIdp::start().await;
    let cfg = config(&db, Environment::Dev);
    seed_dev(
        &cfg,
        &SeedConfig {
            tenant_slug: "dev".into(),
            admin_subject: "dev-admin".into(),
        },
    )
    .await
    .expect("seed");
    let api = Running::start(cfg, serve_config(&idp.jwks_url())).await;
    let token = idp.token("dev-admin", "dev").build();

    // Hold a lock so the next request is blocked in the database when shutdown is requested.
    let mut blocker = db.pool.acquire().await.expect("conn");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE workspaces IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");
    let url = api.url("/workspaces");
    let in_flight = tokio::spawn(async move {
        Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .expect("in-flight request completes")
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let stopping = tokio::spawn(api.stop());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !stopping.is_finished(),
        "shutdown must wait for the request that is still running"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");

    let response = in_flight.await.expect("join");
    assert_eq!(
        response.status(),
        200,
        "the in-flight request got its real answer"
    );
    stopping
        .await
        .expect("join")
        .expect("clean shutdown after draining");
}

#[tokio::test]
async fn the_insecure_jwks_flag_is_refused_outside_development() {
    // A plain-http key URL would let anyone on the network path substitute signing keys.
    let db = migrated_db().await;
    for environment in [Environment::Prod, Environment::Staging] {
        let mut serve_cfg = serve_config("http://idp.example.test/jwks.json");
        serve_cfg.auth.allow_insecure_jwks_http = true;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");

        // Bounded: if the refusal regressed, the server would start and never return.
        let error = tokio::time::timeout(
            Duration::from_secs(10),
            serve(
                &config(&db, environment),
                &serve_cfg,
                listener,
                std::future::pending(),
            ),
        )
        .await
        .expect("it must refuse to start, not start serving")
        .expect_err("must be refused");

        assert!(
            matches!(error, RunError::Refused(_)),
            "{environment:?}: {error:?}"
        );
        assert_eq!(error.exit_code(), 2);
    }
}

/// True if some line of a Prometheus page is the series `name` with all the given labels.
fn has_series(page: &str, name: &str, labels: &[(&str, &str)]) -> bool {
    page.lines().any(|line| {
        line.starts_with(name)
            && line[name.len()..].starts_with('{')
            && labels
                .iter()
                .all(|(k, v)| line.contains(&format!("{k}=\"{v}\"")))
    })
}

/// Starts the control plane with an admin (metrics) listener bound to `admin_bind`, waits until it is
/// healthy, stops it, and returns what was logged at warn level or above meanwhile.
async fn logs_of_a_run_with_admin_listener_on(admin_bind: &str) -> String {
    let db = migrated_db().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let admin = tokio::net::TcpListener::bind(admin_bind)
        .await
        .expect("bind admin");
    let api_addr = listener.local_addr().expect("addr");
    // Installed after the slow set-up, right before the action under test (see `capture`).
    let (_guard, captured) = crate::support::logs::capture();
    let cfg = config(&db, Environment::Dev);
    let serve_cfg = serve_config("http://127.0.0.1:1/jwks.json");
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        sagat_control_plane::serve_with_admin(&cfg, &serve_cfg, listener, Some(admin), async {
            let _ = stopped.await;
        })
        .await
    });
    for _ in 0..100 {
        if healthcheck(&api_addr.to_string()).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("shutdown")
        .expect("join")
        .expect("clean");
    captured.text()
}

#[tokio::test]
async fn a_metrics_listener_reachable_from_a_network_logs_a_warning() {
    // /metrics has no authentication by design (D15); the only protection is where it listens.
    let logs = logs_of_a_run_with_admin_listener_on("0.0.0.0:0").await;

    assert!(
        logs.contains("WARN") && logs.contains("unauthenticated") && logs.contains("NetworkPolicy"),
        "{logs}"
    );
}

#[tokio::test]
async fn a_loopback_metrics_listener_is_silent() {
    let logs = logs_of_a_run_with_admin_listener_on("127.0.0.1:0").await;

    assert!(!logs.contains("unauthenticated"), "{logs}");
}

#[tokio::test]
async fn metrics_are_served_on_a_separate_listener_and_never_carry_identifiers() {
    let db = migrated_db().await;
    let idp = TestIdp::start().await;
    let cfg = config(&db, Environment::Dev);
    seed_dev(
        &cfg,
        &SeedConfig {
            tenant_slug: "dev".into(),
            admin_subject: "dev-admin".into(),
        },
    )
    .await
    .expect("seed");
    let serve_cfg = serve_config(&idp.jwks_url());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let admin = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind admin");
    let (api_addr, admin_addr) = (
        listener.local_addr().expect("addr"),
        admin.local_addr().expect("addr"),
    );
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        sagat_control_plane::serve_with_admin(&cfg, &serve_cfg, listener, Some(admin), async {
            let _ = stopped.await;
        })
        .await
    });
    for _ in 0..100 {
        if healthcheck(&api_addr.to_string()).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let client = Client::new();
    let api = |path: &str| format!("http://{api_addr}/v1{path}");

    // Traffic of every kind: success, anonymous, expired token, unknown route, a path with an id.
    let good = idp.token("dev-admin", "dev").build();
    assert_eq!(
        client
            .get(api("/auth/context"))
            .bearer_auth(&good)
            .send()
            .await
            .expect("send")
            .status(),
        200
    );
    assert_eq!(
        client
            .get(api("/auth/context"))
            .send()
            .await
            .expect("send")
            .status(),
        401
    );
    let expired = idp.token("dev-admin", "dev").issued(900, 300).build();
    assert_eq!(
        client
            .get(api("/auth/context"))
            .bearer_auth(expired)
            .send()
            .await
            .expect("send")
            .status(),
        401
    );
    assert_eq!(
        client
            .get(api("/no-such-route"))
            .send()
            .await
            .expect("send")
            .status(),
        404
    );
    let secret_id = "019b5d6e-1111-7222-8333-444455556666";
    assert_eq!(
        client
            .get(api(&format!("/workspaces/{secret_id}")))
            .bearer_auth(&good)
            .send()
            .await
            .expect("send")
            .status(),
        404
    );

    let page = client
        .get(format!("http://{admin_addr}/metrics"))
        .send()
        .await
        .expect("metrics");
    assert_eq!(page.status(), 200);
    assert!(
        page.headers()["content-type"]
            .to_str()
            .expect("ascii")
            .starts_with("text/plain")
    );
    let page = page.text().await.expect("body");

    assert!(
        has_series(
            &page,
            "http_requests_total",
            &[
                ("method", "GET"),
                ("route", "/v1/auth/context"),
                ("status", "200")
            ]
        ),
        "{page}"
    );
    assert!(has_series(
        &page,
        "http_requests_total",
        &[("route", "/v1/auth/context"), ("status", "401")]
    ));
    assert!(
        has_series(
            &page,
            "http_requests_total",
            &[("route", "unmatched"), ("status", "404")]
        ),
        "unknown routes share one label"
    );
    assert!(
        has_series(
            &page,
            "http_requests_total",
            &[("route", "/v1/workspaces/{workspace_id}")]
        ),
        "route TEMPLATES, not paths"
    );
    assert!(has_series(
        &page,
        "auth_failures_total",
        &[("reason", "expired")]
    ));
    assert!(has_series(
        &page,
        "jwks_refresh_total",
        &[("outcome", "ok")]
    ));
    assert!(page.contains("http_request_duration_seconds_bucket"));

    // Nothing that identifies a person, a tenant or a resource may become a label.
    for forbidden in ["dev-admin", secret_id, "tenant=", "subject=", "sub="] {
        assert!(!page.contains(forbidden), "metrics leak {forbidden:?}");
    }

    // The operational surface is not on the public port, and the API is not on the admin port.
    assert_eq!(
        client
            .get(api("/metrics"))
            .send()
            .await
            .expect("send")
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("http://{admin_addr}/v1/health"))
            .send()
            .await
            .expect("send")
            .status(),
        404
    );

    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("shutdown")
        .expect("join")
        .expect("clean");
}

#[tokio::test]
async fn an_operator_can_provision_a_real_tenant_in_production_and_its_admin_can_sign_in() {
    use sagat_control_plane::config::ProvisionConfig;
    use sagat_control_plane::provision_tenant;

    let db = migrated_db().await;
    let prod = config(&db, Environment::Prod); // not gated on dev, unlike the seed
    let acme = ProvisionConfig {
        tenant_slug: "acme".into(),
        admin_subject: "alice".into(),
    };

    provision_tenant(&prod, &acme).await.expect("provisioned");

    // Audited atomically, attributed to the system, and the administrator really can authenticate.
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_records ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .expect("audit");
    assert_eq!(actions, ["tenant.provisioned", "membership.granted"]);
    let idp = TestIdp::start().await;
    let mut dev = config(&db, Environment::Dev);
    dev.database.url = db.url.clone();
    let api = Running::start(dev, serve_config(&idp.jwks_url())).await;
    let response = Client::new()
        .get(api.url("/auth/context"))
        .bearer_auth(idp.token("alice", "acme").build())
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), 200);
    api.stop().await.expect("clean shutdown");

    // Provisioning twice is reported, not silently ignored, and changes nothing.
    let again = provision_tenant(&prod, &acme).await.expect_err("duplicate");
    assert!(matches!(again, RunError::AlreadyExists), "{again:?}");
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(&db.pool)
        .await
        .expect("n");
    assert_eq!(tenants, 1);

    // Bad input is a configuration error.
    let bad = ProvisionConfig {
        tenant_slug: "Bad Slug".into(),
        admin_subject: "alice".into(),
    };
    assert!(matches!(
        provision_tenant(&prod, &bad).await,
        Err(RunError::Config(_))
    ));
}
