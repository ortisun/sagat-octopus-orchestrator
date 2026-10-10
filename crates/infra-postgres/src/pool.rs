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

/// Publishes `db_pool_connections{state="idle"|"in_use"}` every `interval` until the pool closes.
///
/// A pool that stays saturated (`in_use` at the maximum) is the earliest visible sign of a slow
/// database or of abuse, well before requests start timing out.
pub fn spawn_pool_metrics(pool: PgPool, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if pool.is_closed() {
                return;
            }
            let size = f64::from(pool.size());
            let idle = f64::from(u32::try_from(pool.num_idle()).unwrap_or(u32::MAX));
            metrics::gauge!("db_pool_connections", "state" => "idle").set(idle);
            metrics::gauge!("db_pool_connections", "state" => "in_use").set((size - idle).max(0.0));
        }
    })
}

/// Starts a transaction in a way nothing can cut off halfway.
///
/// `sqlx` 0.9.0 sends `BEGIN` and only afterwards records that the connection is in a transaction.
/// A future dropped between the two (a timeout, a client that went away, a task that was
/// cancelled) leaves the server in a transaction that `sqlx` does not know about: the guard that
/// rolls back on drop does nothing at depth zero, the connection goes back to the pool, and the next
/// statements on it run inside a transaction that is never committed: their writes are lost and the
/// locks they take are held. Measured: 205 of 3000 cut-off `begin`s did it. Here the beginning runs
/// in a task of its own, which finishes whatever the caller does; if the caller is gone, the
/// transaction it returns is dropped there and rolled back as it should be.
///
/// Remove this when `sqlx` begins cancel-safely (`tests/fault/004-events-webhooks-messaging/
/// cancellation.rs` has a test that will say so).
pub trait BeginSafely {
    /// # Errors
    /// The connection could not be taken or the transaction not begun.
    fn begin_safely(
        &self,
    ) -> impl std::future::Future<
        Output = Result<sqlx::Transaction<'static, sqlx::Postgres>, sqlx::Error>,
    > + Send;
}

impl BeginSafely for sqlx::PgPool {
    async fn begin_safely(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, sqlx::Error> {
        let pool = self.clone();
        tokio::spawn(async move { pool.begin().await })
            .await
            .map_err(|_| {
                sqlx::Error::Protocol("the task that begins a transaction failed".into())
            })?
    }
}
