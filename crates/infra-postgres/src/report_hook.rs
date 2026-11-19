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
