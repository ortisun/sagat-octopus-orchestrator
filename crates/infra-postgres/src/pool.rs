//! Connection pool construction with safe defaults.

use std::str::FromStr;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Pool settings. The connection URL may contain credentials and is never logged.
#[derive(Clone)]
pub struct PoolConfig {
    pub url: String,
    pub max_connections: u32,
    /// How long a request waits for a free connection before failing as unavailable.
    pub acquire_timeout: Duration,
    /// Server-side cap per statement; a stuck query is cancelled instead of pinning a connection.
    pub statement_timeout: Duration,
}

impl PoolConfig {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 10,
            acquire_timeout: Duration::from_secs(3),
            statement_timeout: Duration::from_secs(5),
        }
    }
}

impl std::fmt::Debug for PoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolConfig")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .field("statement_timeout", &self.statement_timeout)
            .finish()
    }
}

/// Opens the pool.
///
/// # Errors
/// An invalid URL or a failure to establish the first connection.
pub async fn connect(config: &PoolConfig) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(&config.url)?
        .application_name("sagat-octopus-orchestrator")
        .options([(
            "statement_timeout",
            format!("{}ms", config.statement_timeout.as_millis()),
        )]);
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options)
        .await
}
