//! The control plane process: HTTP API, schema migrations, development seed and a probe.
//!
//! `main.rs` only parses arguments and the environment; everything testable lives here.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_api_http::{AppState, HttpJwksFetcher, TokenVerifier, router};
use sagat_application::clock::SystemClock;
use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::identity::IdentityService;
use sagat_domain::identity::{Membership, Slug, Subject, Tenant, TenantId};
use sagat_infra_postgres::{PgIdentityStore, PgPool, PoolConfig, connect, migrate};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub mod cli;
pub mod config;

use config::{Config, ConfigError, DatabaseConfig, Environment, SeedConfig, ServeConfig};

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
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("health check failed: {0}")]
    Unhealthy(String),
}

impl RunError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) | Self::Refused(_) | Self::Auth(_) => 2,
            Self::Database(_) | Self::Migrate(_) | Self::Io(_) | Self::Unhealthy(_) => 1,
        }
    }
}

fn pool_config(db: &DatabaseConfig, statement_timeout: Duration) -> PoolConfig {
    let mut config = PoolConfig::new(db.url.clone());
    config.max_connections = db.max_connections;
    config.statement_timeout = statement_timeout;
    config
}

async fn open_pool(db: &DatabaseConfig, statement_timeout: Duration) -> Result<PgPool, RunError> {
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
    if serve.migrate_on_start {
        run_migrations(&config.database).await?;
    }
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let fetcher = HttpJwksFetcher::new(&serve.auth).map_err(|e| RunError::Auth(e.to_string()))?;
    let verifier = TokenVerifier::new(Arc::new(fetcher), &serve.auth);
    let service = IdentityService::new(PgIdentityStore::new(pool.clone()), SystemClock);
    let app = router(AppState::new(service, verifier), serve.limits);

    // Public settings only; never the database URL.
    tracing::info!(
        addr = %listener.local_addr()?,
        environment = ?config.environment,
        issuer = %serve.auth.issuer,
        audience = %serve.auth.audience,
        jwks_uri = %serve.auth.jwks_uri,
        "control plane listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    tracing::info!("control plane stopped");
    pool.close().await;
    Ok(())
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

/// Probes `GET /v1/health` on `addr` with a plain HTTP/1.1 request; used as the container health
/// check so the image needs neither a shell nor `curl`.
///
/// # Errors
/// [`RunError::Unhealthy`] unless a `200` arrives within three seconds.
pub async fn healthcheck(addr: &str) -> Result<(), RunError> {
    let probe = async {
        let mut stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        stream
            .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
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
