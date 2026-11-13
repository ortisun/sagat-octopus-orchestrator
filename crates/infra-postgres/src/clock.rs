//! The clock of a process that acts on the database's time (feature 018, `FR-018-005`: "due-ness
//! uses the database clock").
//!
//! A schedule is due when the store says so, but the firing plans the times it acts on, and decides
//! whether a retry has waited long enough, with the clock it is given. A process whose own clock runs
//! hours ahead would start (or fold into a summary) times the database has not reached, and one that
//! runs behind would leave owed times alone. [`DatabaseClock`] removes the disagreement: it reads the
//! database's clock now and then, remembers by how much the process clock is off, and answers with
//! the process clock corrected by that difference. Between two readings it runs on the process clock,
//! which is steady enough for the seconds a poll interval lasts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use sagat_application::clock::{Clock, truncate_to_micros};
use sagat_application::error::StoreError;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use crate::identity::store_error;

/// The process clock `L`, moved by what the database says the time is.
///
/// Clones share the correction, so the one that is kept in step by [`DatabaseClock::sync`] is the
/// one the services read.
#[derive(Clone)]
pub struct DatabaseClock<L> {
    pool: PgPool,
    local: L,
    /// Database time minus process time, in microseconds. Zero until the first reading.
    offset_micros: Arc<AtomicI64>,
    /// Whether a reading has ever succeeded: until then the offset is a guess, not a measurement.
    synced: Arc<AtomicBool>,
}

impl<L: Clock> DatabaseClock<L> {
    pub fn new(pool: PgPool, local: L) -> Self {
        Self {
            pool,
            local,
            offset_micros: Arc::new(AtomicI64::new(0)),
            synced: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether the database has been read at least once. Before that the clock is the process
    /// clock, which is exactly what a caller that must act on the database's time cannot use.
    #[must_use]
    pub fn is_synced(&self) -> bool {
        self.synced.load(Ordering::Relaxed)
    }

    /// Reads the database's clock and keeps the difference to the process clock; returns it.
    ///
    /// The connection is taken from the pool first, so the wait for one is not part of the interval
    /// that is measured; the process clock is then read before the question and after the answer,
    /// and the database's answer is taken to belong to the middle of that interval, so the time the
    /// question took does not count as drift. When the database cannot be reached the last
    /// difference is kept.
    ///
    /// # Errors
    /// The store could not be reached.
    pub async fn sync(&self) -> Result<Duration, StoreError> {
        let mut connection = self.pool.acquire().await.map_err(store_error)?;
        let before = self.local.now();
        let database: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *connection)
            .await
            .map_err(store_error)?;
        let after = self.local.now();
        let offset = database - (before + (after - before) / 2_i32);
        let micros =
            i64::try_from(offset.whole_microseconds()).unwrap_or(if offset.is_negative() {
                i64::MIN
            } else {
                i64::MAX
            });
        self.offset_micros.store(micros, Ordering::Relaxed);
        self.synced.store(true, Ordering::Relaxed);
        Ok(offset)
    }
}

impl<L: Clock> Clock for DatabaseClock<L> {
    fn now(&self) -> OffsetDateTime {
        let offset = Duration::microseconds(self.offset_micros.load(Ordering::Relaxed));
        truncate_to_micros(self.local.now().saturating_add(offset))
    }
}
