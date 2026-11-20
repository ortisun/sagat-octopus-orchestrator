//! The report hook of design 041's spike T041-35 (compiled only with the `perf-spike` feature, which only
//! `sagat-tests --features perf-spike` enables): a benchmark runs the step-log carry's statements inside the real
//! report transaction of a root attempt, right before its `COMMIT`. It is a measurement seam, not the carry:
//! T041-36 builds the carry and removes this module.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use sqlx::PgConnection;
use uuid::Uuid;

/// The attempt a report transaction recorded, as its locked state names it.
#[derive(Debug, Clone)]
pub struct ReportedAttempt {
    pub tenant: Uuid,
    pub namespace: Uuid,
    pub run: Uuid,
    pub step: Uuid,
    pub node: String,
    pub attempt: i32,
    pub generation: i64,
}

/// What the benchmark runs in a report transaction before its commit.
pub trait ReportHook: Send + Sync + 'static {
    /// Runs on the report transaction's own connection. An error makes the commit fail.
    fn on_report_commit<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        attempt: &'a ReportedAttempt,
    ) -> Pin<Box<dyn Future<Output = Result<(), sqlx::Error>> + Send + 'a>>;

    /// Whether the store times this report's `COMMIT` for the hook (the hot-row diagnosis of T041-35's second run).
    fn times_commit(&self) -> bool {
        false
    }

    /// Called after a timed `COMMIT` returned: the transaction id, and the wall-clock microseconds since the Unix
    /// epoch at which the `COMMIT` was sent and at which it returned (the clock `clock_timestamp()` reads on the host).
    fn on_committed(&self, _xid: i64, _sent_us: i64, _returned_us: i64) {}
}

/// Wall-clock microseconds since the Unix epoch.
#[must_use]
pub fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
}

/// The store's optional hook, printable and clonable with the store.
#[derive(Clone, Default)]
pub(crate) struct HookSlot(pub(crate) Option<Arc<dyn ReportHook>>);

impl std::fmt::Debug for HookSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "HookSlot(set)"
        } else {
            "HookSlot(none)"
        })
    }
}
