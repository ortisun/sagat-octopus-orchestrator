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
    /// Server-side cap on how long a transaction may sit open between two statements; zero turns it
    /// off. A process that dies or is cut off in the middle of an action leaves its transaction open
    /// on the server until the server notices the connection is gone (hours, by default), and the
    /// locks it holds with it: among them the `FOR SHARE` that fences a scheduler action, which a
    /// takeover of the shard waits for.
    pub idle_in_transaction_timeout: Duration,
    /// What `pg_stat_activity.application_name` shows for every connection of the pool: the replica or the process it
    /// serves ([`PoolName`]), so that a step-log quota lease's backend is attributable to a replica and the step-log
    /// wait slots are per replica (design 041, task T041-5; decisions D041-125 and D041-134).
    pub application_name: String,
}

/// The prefix every pool's `application_name` keeps, which `scripts/smoke-compose.sh` matches.
pub const APPLICATION_NAME_PREFIX: &str = "sagat-octopus-orchestrator/";

/// Who a pool serves, which names its connections: a long-running instance by its registry id (one name per replica,
/// so replicas never share wait slots), or a one-shot process by its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolName {
    Instance(uuid::Uuid),
    Process(&'static str),
}

impl PoolName {
    /// `sagat-octopus-orchestrator/<instance id>` (27 + 36 = 63 bytes, PostgreSQL's limit) or
    /// `sagat-octopus-orchestrator/<process kind>`.
    #[must_use]
    pub fn application_name(self) -> String {
        match self {
            Self::Instance(id) => format!("{APPLICATION_NAME_PREFIX}{}", id.hyphenated()),
            Self::Process(kind) => format!("{APPLICATION_NAME_PREFIX}{kind}"),
        }
    }
}

impl PoolConfig {
    /// A pool named `sagat-octopus-orchestrator/unnamed` until the caller names it, so that no pool keeps the old
    /// shared name by accident.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 10,
            acquire_timeout: Duration::from_secs(3),
            statement_timeout: Duration::from_secs(5),
            idle_in_transaction_timeout: Duration::from_secs(60),
            application_name: format!("{APPLICATION_NAME_PREFIX}unnamed"),
        }
    }

    /// The same settings, named for `name`.
    #[must_use]
    pub fn named(mut self, name: PoolName) -> Self {
        self.application_name = name.application_name();
        self
    }
}

/// *Q*, the step-log wait slots of a replica whose pool holds at most `max_connections` (design 041 section 2b,
/// decision D041-137): max(1, ⌊N / 4⌋), so 5 at a pool of 20, 2 at 10 and at 8. It is never configured apart from the
/// pool; the store computes it once and sends it per transaction as `sagat.step_log_wait_slots`.
#[must_use]
pub const fn step_log_wait_slots_for(max_connections: u32) -> u32 {
    let quarter = max_connections / 4;
    if quarter == 0 { 1 } else { quarter }
}

/// *Q* of a store over `pool`: [`step_log_wait_slots_for`] of the pool's maximum size.
#[must_use]
pub fn step_log_wait_slots(pool: &PgPool) -> u32 {
    step_log_wait_slots_for(pool.options().get_max_connections())
}

impl std::fmt::Debug for PoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolConfig")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .field("statement_timeout", &self.statement_timeout)
            .field(
                "idle_in_transaction_timeout",
                &self.idle_in_transaction_timeout,
            )
            .field("application_name", &self.application_name)
            .finish()
    }
}

/// Opens the pool.
///
/// # Errors
/// An invalid URL or a failure to establish the first connection.
pub async fn connect(config: &PoolConfig) -> Result<PgPool, sqlx::Error> {
    let mut settings = vec![(
        "statement_timeout",
        format!("{}ms", config.statement_timeout.as_millis()),
    )];
    if !config.idle_in_transaction_timeout.is_zero() {
        settings.push((
            "idle_in_transaction_session_timeout",
            format!("{}ms", config.idle_in_transaction_timeout.as_millis()),
        ));
    }
    let options = PgConnectOptions::from_str(&config.url)?
        .application_name(&config.application_name)
        .options(settings);
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
/// Remove this when `sqlx` begins cancel-safely (`tests/fault/events-webhooks-messaging/
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

#[cfg(test)]
mod tests {
    use super::{PoolConfig, PoolName, step_log_wait_slots_for};

    #[test]
    fn the_wait_slots_are_a_quarter_of_the_pool_and_never_none() {
        assert_eq!(step_log_wait_slots_for(20), 5);
        assert_eq!(step_log_wait_slots_for(10), 2);
        assert_eq!(step_log_wait_slots_for(8), 2);
        assert_eq!(step_log_wait_slots_for(7), 1);
        assert_eq!(step_log_wait_slots_for(1), 1);
        assert_eq!(step_log_wait_slots_for(0), 1);
    }

    #[test]
    fn a_pool_is_named_for_its_replica_or_its_process_within_the_limit() {
        assert_eq!(
            PoolConfig::new("postgres://x").application_name,
            "sagat-octopus-orchestrator/unnamed"
        );
        let id = uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let name = PoolName::Instance(id).application_name();
        assert_eq!(
            name,
            "sagat-octopus-orchestrator/01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(
            name.len(),
            63,
            "PostgreSQL keeps 63 bytes of an application_name"
        );
        assert_eq!(
            PoolName::Process("migrate").application_name(),
            "sagat-octopus-orchestrator/migrate"
        );
    }
}
