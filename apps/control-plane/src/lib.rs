//! The control plane process: HTTP API, schema migrations, development seed and a probe.
//!
//! `main.rs` only parses arguments and the environment; everything testable lives here.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_api_http::{
    AppState, ExecState, GovState, HttpJwksFetcher, HttpLimits, ServerConfig, TokenVerifier,
    protect, router_with_governance,
};
use sagat_application::bindings::BindingService;
use sagat_application::clock::SystemClock;
use sagat_application::connectors::ConnectorService;
use sagat_application::definitions::{DefinitionService, IrCodec};
use sagat_application::egress::EgressService;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::events::EventService;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::IdentityService;
use sagat_application::operations::OperationsService;
use sagat_application::outbound::OutboundService;
use sagat_application::publishing::PublishingService;
use sagat_application::quotas::QuotaService;
use sagat_application::runs::RunService;
use sagat_application::scripts::{RunnerService, ScriptService};
use sagat_application::secrets::SecretService;
use sagat_domain::identity::{Membership, Slug, Subject, Tenant, TenantId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{
    LeastPrivilegeError, PgIdentityStore, PgPool, PgRunStore, PoolConfig, connect, migrate,
    spawn_pool_metrics,
};
use sagat_telemetry::Metrics;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub mod cli;
pub mod config;

/// Parsed workflow versions kept in memory (they are immutable, so they never go stale).
const DEFINITION_CACHE_ENTRIES: usize = 256;

use config::{
    CeilingConfig, Config, ConfigError, DatabaseConfig, Environment, NamespaceSeed,
    ProvisionConfig, SeedConfig, ServeConfig,
};

/// A command failed. `exit_code` follows the convention: 2 for a problem the operator must fix in
/// the invocation or configuration, 1 for a runtime failure.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The command was understood but deliberately not executed.
    #[error("{0}")]
    Refused(&'static str),
    /// Database errors never include the connection URL.
    #[error("database error: {0}")]
    Database(String),
    #[error("migration failed: {0}")]
    Migrate(String),
    #[error("authentication setup failed: {0}")]
    Auth(String),
    /// The Studio directory or its sign-in settings are unusable; the process does not start.
    #[error("the Studio cannot be served: {0}")]
    Studio(#[from] sagat_api_http::studio::StudioError),
    /// Outside development the API refuses to run with a database role that could rewrite the
    /// audit trail. The message names the problem, never a connection string.
    #[error("refusing to serve: {0}")]
    LeastPrivilege(#[from] LeastPrivilegeError),
    #[error("a tenant with that slug already exists")]
    AlreadyExists,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("health check failed: {0}")]
    Unhealthy(String),
}

impl RunError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_)
            | Self::Refused(_)
            | Self::Auth(_)
            | Self::Studio(_)
            | Self::LeastPrivilege(_) => 2,
            Self::Database(_)
            | Self::Migrate(_)
            | Self::Io(_)
            | Self::Unhealthy(_)
            | Self::AlreadyExists => 1,
        }
    }
}

fn pool_config(db: &DatabaseConfig, statement_timeout: Duration) -> PoolConfig {
    let mut config = PoolConfig::new(db.url.clone());
    config.max_connections = db.max_connections;
    config.statement_timeout = statement_timeout;
    config
}

/// Connects a pool with the given per-statement cap.
///
/// # Errors
/// [`RunError::Database`] (never containing the connection URL).
pub async fn open_pool(
    db: &DatabaseConfig,
    statement_timeout: Duration,
) -> Result<PgPool, RunError> {
    connect(&pool_config(db, statement_timeout))
        .await
        .map_err(|e| RunError::Database(e.to_string()))
}

/// Applies pending migrations. Idempotent, and safe to run from several instances at once.
///
/// Uses a pool **without** a statement timeout: a legitimate DDL statement may run far longer than
/// the limit that protects normal traffic.
///
/// # Errors
/// [`RunError::Database`] if PostgreSQL is unreachable, [`RunError::Migrate`] if a migration fails.
pub async fn run_migrations(db: &DatabaseConfig) -> Result<(), RunError> {
    let pool = open_pool(db, Duration::ZERO).await?;
    migrate(&pool)
        .await
        .map_err(|e| RunError::Migrate(e.to_string()))?;
    pool.close().await;
    Ok(())
}

/// What `seed-dev` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    Created,
    /// The tenant already existed; nothing was changed.
    AlreadyPresent,
}

/// Creates a development tenant and its first administrator. Idempotent.
///
/// The environment is checked **before** any connection is opened, so a misconfigured production
/// process cannot reach the database through this command.
///
/// # Errors
/// [`RunError::Refused`] unless `SAGAT_ENV=dev`.
pub async fn seed_dev(config: &Config, seed: &SeedConfig) -> Result<SeedOutcome, RunError> {
    if config.environment != Environment::Dev {
        return Err(RunError::Refused("seed-dev only runs when SAGAT_ENV=dev"));
    }
    let slug = Slug::parse(&seed.tenant_slug).map_err(|e| ConfigError::Invalid {
        name: "SAGAT_SEED_TENANT",
        reason: e.to_string(),
    })?;
    let subject = Subject::parse(&seed.admin_subject).map_err(|e| ConfigError::Invalid {
        name: "SAGAT_SEED_ADMIN_SUBJECT",
        reason: e.to_string(),
    })?;

    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let store = PgIdentityStore::new(pool.clone());
    let now = OffsetDateTime::now_utc();
    let tenant = Tenant::provision(TenantId::generate(), slug, now);
    let admin = Membership::bootstrap_admin(&tenant, subject, now);
    let outcome = match store.provision_tenant(&tenant, &admin).await {
        Ok(()) => SeedOutcome::Created,
        Err(StoreError::Conflict(ConflictKind::TenantSlug)) => SeedOutcome::AlreadyPresent,
        Err(error) => return Err(RunError::Database(error.to_string())),
    };
    pool.close().await;
    Ok(outcome)
}

/// Leaves a workspace and a namespace for the tenant `seed-dev` made, as its administrator would
/// (so it is audited like any other creation). Idempotent: what exists is kept. Run it after
/// [`seed_dev`].
///
/// # Errors
/// [`RunError::Refused`] unless `SAGAT_ENV=dev` and when the tenant or its administrator is not
/// there; [`RunError::Config`] for a slug or environment that is not valid.
pub async fn seed_dev_namespace(
    config: &Config,
    seed: &SeedConfig,
    wanted: &NamespaceSeed,
) -> Result<SeedOutcome, RunError> {
    use sagat_application::identity::{PageRequest, RequestContext};
    if config.environment != Environment::Dev {
        return Err(RunError::Refused("seed-dev only runs when SAGAT_ENV=dev"));
    }
    let invalid = |name: &'static str, reason: String| ConfigError::Invalid { name, reason };
    let workspace_slug = Slug::parse(&wanted.workspace_slug)
        .map_err(|e| invalid("SAGAT_SEED_WORKSPACE", e.to_string()))?;
    let namespace_slug = Slug::parse(&wanted.namespace_slug)
        .map_err(|e| invalid("SAGAT_SEED_NAMESPACE", e.to_string()))?;

    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let service = IdentityService::new(PgIdentityStore::new(pool.clone()), SystemClock);
    let result = async {
        let session = service
            .authenticate(&seed.tenant_slug, &seed.admin_subject)
            .await
            .map_err(|_| {
                RunError::Refused("run seed-dev first: the tenant has no such administrator")
            })?;
        let principal = session.principal;
        let context = RequestContext::default();
        let page = PageRequest {
            limit: Some(200),
            cursor: None,
        };
        let database = |e: sagat_application::error::AppError| RunError::Database(e.to_string());
        let namespaces = service
            .list_namespaces(&principal, &page)
            .await
            .map_err(database)?;
        if namespaces.items.iter().any(|n| n.slug() == &namespace_slug) {
            return Ok(SeedOutcome::AlreadyPresent);
        }
        let workspaces = service
            .list_workspaces(&principal, &page)
            .await
            .map_err(database)?;
        let workspace = match workspaces
            .items
            .into_iter()
            .find(|w| w.slug() == &workspace_slug)
        {
            Some(existing) => existing,
            None => service
                .create_workspace(&principal, workspace_slug.as_str(), &context)
                .await
                .map_err(database)?,
        };
        service
            .create_namespace(
                &principal,
                workspace.id(),
                namespace_slug.as_str(),
                &wanted.environment,
                &context,
            )
            .await
            .map_err(|e| match e {
                sagat_application::error::AppError::Invalid(_) => RunError::Config(invalid(
                    "SAGAT_SEED_ENVIRONMENT",
                    "not an environment (dev, staging or prod)".to_owned(),
                )),
                other => database(other),
            })?;
        Ok(SeedOutcome::Created)
    }
    .await;
    pool.close().await;
    result
}

/// Creates a tenant and its first administrator in any environment. Audited atomically with the
/// change (`tenant.provisioned`, `membership.granted`, actor `system:provisioning`).
///
/// # Errors
/// [`RunError::AlreadyExists`] when the slug is taken; [`RunError::Config`] for an invalid slug or
/// subject.
pub async fn provision_tenant(config: &Config, request: &ProvisionConfig) -> Result<(), RunError> {
    let slug = Slug::parse(&request.tenant_slug).map_err(|e| ConfigError::Invalid {
        name: "SAGAT_PROVISION_TENANT",
        reason: e.to_string(),
    })?;
    let subject = Subject::parse(&request.admin_subject).map_err(|e| ConfigError::Invalid {
        name: "SAGAT_PROVISION_ADMIN",
        reason: e.to_string(),
    })?;
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let store = PgIdentityStore::new(pool.clone());
    let now = OffsetDateTime::now_utc();
    let tenant = Tenant::provision(TenantId::generate(), slug, now);
    let admin = Membership::bootstrap_admin(&tenant, subject, now);
    let result = store.provision_tenant(&tenant, &admin).await;
    pool.close().await;
    match result {
        Ok(()) => Ok(()),
        Err(StoreError::Conflict(ConflictKind::TenantSlug)) => Err(RunError::AlreadyExists),
        Err(error) => Err(RunError::Database(error.to_string())),
    }
}

/// Grants a tenant a quota ceiling, or withdraws it (`request.value == None`), and audits it.
///
/// A tenant's administrators set their own limits only up to this ceiling; nothing in the API
/// raises it, so this command (run by someone with access to the database) is how capacity is
/// sold or granted.
///
/// # Errors
/// [`RunError::Config`] for an identifier or resource that is not valid, or a value out of range;
/// [`RunError::Refused`] when the tenant does not exist; [`RunError::Database`] otherwise.
pub async fn grant_quota_ceiling(config: &Config, request: &CeilingConfig) -> Result<(), RunError> {
    let invalid = |name: &'static str, reason: &str| ConfigError::Invalid {
        name,
        reason: reason.to_owned(),
    };
    let tenant = request
        .tenant_id
        .parse::<TenantId>()
        .map_err(|_| invalid("SAGAT_CEILING_TENANT_ID", "expected a tenant id (a UUID)"))?;
    let resource = request
        .resource
        .parse::<sagat_domain::quota::QuotaResource>()
        .map_err(|_| invalid("SAGAT_CEILING_RESOURCE", "not a quota resource"))?;
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 1);
    let service = QuotaService::new(PgRunStore::new(pool.clone(), cache), SystemClock);
    let result = service.grant_ceiling(tenant, resource, request.value).await;
    pool.close().await;
    match result {
        Ok(()) => Ok(()),
        Err(sagat_application::error::AppError::NotFound) => {
            Err(RunError::Refused("no tenant has that id"))
        }
        Err(sagat_application::error::AppError::NotAllowed(reason)) => {
            Err(invalid("SAGAT_CEILING_VALUE", reason).into())
        }
        Err(error) => Err(RunError::Database(error.to_string())),
    }
}

/// Re-seals every secret version that is not sealed under the active key of `secrets`, in batches,
/// until none is left. Safe to repeat and to run next to a serving instance. Returns how many
/// versions were moved.
///
/// A version that no key of the ring opens does not stop the others: all the rest are moved first,
/// and then the command fails, saying how many could not be and how many were.
///
/// # Errors
/// [`RunError::Database`] when the store fails, or when some versions could not be opened (the ring
/// no longer holds the key they were sealed under); versions already moved stay moved.
pub async fn reseal_secrets(
    config: &Config,
    secrets: &sagat_infra_secrets::SecretsConfig,
) -> Result<u64, RunError> {
    const BATCH: u32 = 500;
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 1);
    let service = SecretService::new(
        PgRunStore::new(pool.clone(), cache),
        SystemClock,
        secrets.cipher(),
    );
    let (mut moved, mut failed) = (0u64, 0u64);
    let mut after = None;
    let result = loop {
        match service.reseal(after, BATCH).await {
            Ok(batch) => {
                moved += u64::from(batch.moved);
                failed += u64::from(batch.failed);
                match batch.next {
                    Some(next) => after = Some(next),
                    None => break Ok(()),
                }
            }
            Err(error) => break Err(RunError::Database(error.to_string())),
        }
    };
    pool.close().await;
    result?;
    if failed > 0 {
        return Err(RunError::Database(format!(
            "{failed} secret versions could not be opened with any key of the ring and were left \
             as they are ({moved} others were re-sealed); restore the key they were sealed under"
        )));
    }
    Ok(moved)
}

/// Runs the HTTP API on `listener` until `shutdown` completes, then drains in-flight requests.
///
/// # Errors
/// Fails fast, before listening, on an unusable database or authentication setup.
pub async fn serve(
    config: &Config,
    serve: &ServeConfig,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RunError> {
    serve_with_admin(config, serve, listener, None, shutdown).await
}

/// [`serve`] plus an optional administrative listener that exposes `GET /metrics` only. It is a
/// separate socket so operational data never shares a port (or an exposure policy) with the API.
///
/// # Errors
/// Same as [`serve`].
pub async fn serve_with_admin(
    config: &Config,
    serve: &ServeConfig,
    listener: TcpListener,
    admin_listener: Option<TcpListener>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RunError> {
    // A plain-http key URL lets anyone on the path substitute their own signing keys and forge any
    // identity, so it is a development convenience only and is refused everywhere else.
    if serve.auth.allow_insecure_jwks_http && config.environment != Environment::Dev {
        return Err(RunError::Refused(
            "SAGAT_ALLOW_INSECURE_JWKS_HTTP is only allowed when SAGAT_ENV=dev",
        ));
    }
    // Migrating needs the schema-owning role, which is exactly what the check below refuses to serve
    // with; say so up front instead of letting the operator discover it as a privilege error.
    if serve.migrate_on_start && config.environment != Environment::Dev {
        return Err(RunError::Refused(
            "SAGAT_MIGRATE_ON_START needs the schema-owning database role and is only allowed when \
             SAGAT_ENV=dev; run `control-plane migrate` as a separate step",
        ));
    }
    // Everything that can be checked without the database comes first.
    let fetcher = HttpJwksFetcher::new(&serve.auth).map_err(|e| RunError::Auth(e.to_string()))?;
    let verifier = TokenVerifier::new(Arc::new(fetcher), &serve.auth);
    if serve.migrate_on_start {
        run_migrations(&config.database).await?;
    }
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let store = PgIdentityStore::new(pool.clone());
    if config.environment == Environment::Dev {
        tracing::debug!("database role privileges are not checked when SAGAT_ENV=dev");
    } else {
        // The audit trail is only append-only if this process cannot undo it: refuse to serve as
        // a superuser, as the owner of `audit_records`, or with write access to it.
        if let Err(error) = store.verify_least_privilege().await {
            pool.close().await;
            return Err(error.into());
        }
        tracing::info!("database role verified: it cannot rewrite the audit trail");
    }
    let service = IdentityService::new(store, SystemClock);
    // Install the metrics recorder before the first request so nothing is missed. Idle series are
    // forgotten when the page is rendered, so a process nobody scrapes needs this timer.
    let metrics_upkeep = spawn_metrics_upkeep(Metrics::global(), METRICS_UPKEEP_INTERVAL);
    // Already validated by `ServeConfig`; checked again because the limiter must never run with a
    // budget that could panic or never admit a request.
    let state = AppState::with_rate_limit(service, verifier, serve.rate_limit)
        .map_err(|e| config::rate_limit_error(config::TENANT_RATE_VARS, e))?
        .with_subject_rate_limit(serve.subject_rate_limit)
        .map_err(|e| config::rate_limit_error(config::SUBJECT_RATE_VARS, e))?;
    // Workflow documents are validated and parsed once per immutable version, then kept.
    let codec: Arc<dyn IrCodec> = Arc::new(JsonIrCodec);
    let cache = DefinitionCache::new(codec, DEFINITION_CACHE_ENTRIES);
    let executions = PgRunStore::new(pool.clone(), cache.clone());
    let mut governance = GovState::new(
        state.clone(),
        QuotaService::new(executions.clone(), SystemClock),
        BindingService::new(executions.clone(), SystemClock),
        EgressService::new(executions.clone(), SystemClock),
    )
    .with_operations(OperationsService::new(
        executions.clone(),
        SystemClock,
        serve.runbook_base.clone(),
    ));
    if let Some(secrets) = &serve.secrets {
        governance = governance
            .with_secrets(SecretService::new(
                executions.clone(),
                SystemClock,
                secrets.cipher(),
            ))
            // Webhooks are verified with secrets of the namespace, so they need the key ring too.
            .with_events(EventService::new(
                executions.clone(),
                SystemClock,
                secrets.cipher(),
                Arc::new(JsonEvaluator),
            ))
            // Deliveries are signed with a secret of the namespace, so they need the key ring too.
            .with_outbound(OutboundService::new(
                executions.clone(),
                SystemClock,
                EgressService::new(executions.clone(), SystemClock),
            ))
            .with_connectors(ConnectorService::new(
                executions.clone(),
                SystemClock,
                EgressService::new(executions.clone(), SystemClock),
                Arc::new(JsonEvaluator),
            ));
    }
    // An artifact is only trusted if the platform signed it, so without a key there are no scripts.
    if let Some(scripts) = &serve.scripts {
        governance = governance.with_scripts(
            ScriptService::new(
                executions.clone(),
                SystemClock,
                sagat_infra_schema::Schemas,
                scripts.ceiling,
            ),
            RunnerService::new(
                executions.clone(),
                SystemClock,
                SystemEntropy,
                scripts.signer()?,
                sagat_infra_schema::bundle::Bundles,
                scripts.ceiling,
            ),
        );
    }
    let publishing = PublishingService::new(executions.clone(), SystemClock, JsonIrCodec);
    let exec = ExecState::new(
        state,
        DefinitionService::new(executions.clone(), SystemClock, JsonIrCodec),
        RunService::new(executions, SystemClock, Engine::new(JsonEvaluator), cache)
            .with_lag_budget(serve.telemetry_lag_budget),
    )
    .with_stream_limits(serve.streams)
    .with_publishing(publishing);
    let mut app = router_with_governance(exec, governance, serve.limits);
    // The Studio is static files on the API's own origin: no CORS, no second listener.
    if let Some(studio) = serve.studio.clone() {
        let dir = studio.dir.display().to_string();
        app = app.merge(sagat_api_http::studio::studio_router(studio)?);
        tracing::info!(%dir, "serving the Studio under /studio/");
    }
    let pool_metrics = spawn_pool_metrics(pool.clone(), Duration::from_secs(5));

    // One shutdown signal drives both listeners.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop_tx.send(true);
    });

    // Public settings only; never the database URL.
    tracing::info!(
        addr = %listener.local_addr()?,
        environment = ?config.environment,
        issuer = %config::redact_url(&serve.auth.issuer),
        audience = %serve.auth.audience,
        jwks_uri = %config::redact_url(&serve.auth.jwks_uri),
        "control plane listening"
    );
    let admin = match admin_listener {
        Some(admin_listener) => {
            let addr = admin_listener.local_addr()?;
            tracing::info!(%addr, "metrics listener");
            if config::admin_addr_is_exposed(addr) {
                tracing::warn!(
                    %addr,
                    "the metrics endpoint is unauthenticated and listens on a non-loopback \
                     address; restrict it with a NetworkPolicy or firewall rule, or bind \
                     SAGAT_ADMIN_ADDR to 127.0.0.1"
                );
            }
            Some(tokio::spawn(sagat_api_http::serve(
                admin_listener,
                admin_router(),
                ServerConfig::default(),
                stopped(stop_rx.clone()),
            )))
        }
        None => None,
    };
    sagat_api_http::serve(listener, app, serve.server, stopped(stop_rx)).await?;
    if let Some(admin) = admin {
        let _ = admin.await;
    }
    tracing::info!("control plane stopped");
    pool_metrics.abort();
    metrics_upkeep.abort();
    pool.close().await;
    Ok(())
}

/// How often idle metric series are released when nobody scrapes `/metrics`.
pub const METRICS_UPKEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Runs [`Metrics::prune_idle`] every `interval` until aborted.
pub fn spawn_metrics_upkeep(
    metrics: &'static Metrics,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            metrics.prune_idle();
        }
    })
}

/// Completes once the shared shutdown flag is set.
pub async fn stopped(mut flag: tokio::sync::watch::Receiver<bool>) {
    while !*flag.borrow() {
        if flag.changed().await.is_err() {
            return;
        }
    }
}

/// The administrative surface: Prometheus metrics, nothing else.
pub fn admin_router() -> axum::Router {
    admin_router_with(axum::Router::new())
}

/// [`admin_router`] plus `extra` routes (a process's own probes), under the same protections.
pub fn admin_router_with(extra: axum::Router) -> axum::Router {
    use axum::http::{HeaderValue, header};
    use axum::routing::get;
    let metrics = || async {
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            )],
            Metrics::global().render(),
        )
    };
    protect(
        axum::Router::new()
            .route("/metrics", get(metrics))
            .merge(extra),
        HttpLimits::default(),
    )
}

/// Completes on SIGINT or SIGTERM.
pub async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
}

/// A new key for `SAGAT_SCRIPT_SIGNING_KEY`: an Ed25519 key in PKCS#8 form, base64 encoded.
///
/// # Errors
/// [`RunError::Refused`] if the system's randomness fails.
pub fn new_script_key() -> Result<String, RunError> {
    use base64::Engine as _;
    let pkcs8 = sagat_application::script_signing::Signer::generate_pkcs8()
        .map_err(|_| RunError::Refused("the system could not make a key"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(pkcs8))
}

/// Probes `GET /v1/health` on `addr` with a plain HTTP/1.1 request; used as the container health
/// check so the image needs neither a shell nor `curl`.
///
/// # Errors
/// [`RunError::Unhealthy`] unless a `200` arrives within three seconds.
pub async fn healthcheck(addr: &str) -> Result<(), RunError> {
    probe(addr, "/v1/health").await
}

/// Probes `GET <path>` on `addr` the same way; used by every process's container health check.
///
/// # Errors
/// [`RunError::Unhealthy`] unless a `200` arrives within three seconds.
pub async fn probe(addr: &str, path: &str) -> Result<(), RunError> {
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let probe = async {
        let mut stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut head = Vec::new();
        stream
            .take(1024)
            .read_to_end(&mut head)
            .await
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&head);
        let status_line = text.lines().next().unwrap_or_default();
        if status_line.split_whitespace().nth(1) == Some("200") {
            Ok(())
        } else {
            Err(format!("unexpected response '{status_line}'"))
        }
    };
    match tokio::time::timeout(Duration::from_secs(3), probe).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(reason)) => Err(RunError::Unhealthy(reason)),
        Err(_) => Err(RunError::Unhealthy("timed out".to_owned())),
    }
}
