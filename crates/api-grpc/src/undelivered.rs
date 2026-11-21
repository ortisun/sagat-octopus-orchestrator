//! Leases that were committed and not handed to the transport (feature 006, decisions D006-001, D006-002, D006-016 and
//! D006-017 of `specs/006-sdk-multilanguage/research.md`; `stream-completion.md`, "Leases that never reach a host").
//!
//! Every lease a claim commits travels in a [`PendingLeases`] until the frame (or the unary answer) that carries it is
//! handed to the transport. A guard that is dropped before that, however it is dropped (a frame discarded at the window
//! deadline, a queue drained at token expiry, a body dropped by its client, a frame sent into a closed queue, a unary
//! handler dropped before it returned), hands its leases to the withdrawal. The withdrawals run on a tracker that a
//! graceful shutdown waits for.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sagat_application::clock::Clock;
use sagat_application::definitions::DefinitionStore;
use sagat_application::entropy::Entropy;
use sagat_application::execution::{ExpressionEvaluator, PathExecutionStore};
use sagat_application::runs::RunStore;
use sagat_application::worker::{
    ClaimedTask, UndeliveredLease, WithdrawalOutcome, WorkerService, WorkerStore, count_undelivered,
};
use sagat_domain::execution::WithdrawalCause;
use sagat_domain::identity::TenantId;
use tokio_util::task::TaskTracker;

/// A point a test holds a claim at, after each run's transaction committed leases and before the next run is claimed
/// (feature 006, decisions D006-008 and D006-035: a barrier, never a sleep). It is given the [`Abandonment`] of the consumer, so a test
/// can release the claim exactly when the server knows its consumer is gone. Production never sets one.
pub type DeliveryProbe =
    Arc<dyn Fn(Abandonment) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Resolves when the consumer of a claim is gone: the body of a task stream was dropped or ended its window, or the
/// handler of a unary poll was dropped.
#[derive(Clone, Debug)]
pub struct Abandonment(pub(crate) tokio_util::sync::CancellationToken);

impl Abandonment {
    /// Waits until the consumer is gone.
    pub async fn wait(self) {
        self.0.cancelled().await;
    }
}

type SinkFn = Arc<dyn Fn(Vec<UndeliveredLease>) + Send + Sync>;

/// Where dropped leases go: a withdrawal on the tracker of the worker API.
#[derive(Clone)]
pub(crate) struct WithdrawalSink(SinkFn);

impl WithdrawalSink {
    pub(crate) fn new<S, C, E, R>(
        workers: Arc<WorkerService<S, C, E, R>>,
        withdrawals: Withdrawals,
    ) -> Self
    where
        S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore + 'static,
        C: Clock + 'static,
        E: ExpressionEvaluator + 'static,
        R: Entropy + 'static,
    {
        Self(Arc::new(move |leases: Vec<UndeliveredLease>| {
            let workers = Arc::clone(&workers);
            let n = leases.len() as u64;
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let permits = Arc::clone(&withdrawals.permits);
                withdrawals.tracker.spawn_on(
                    async move {
                        // at most a quarter of the pool at once (decision D006-038); a waiting one holds no connection
                        let Ok(_permit) = permits.acquire_owned().await else {
                            return;
                        };
                        workers
                            .withdraw_undelivered(leases, WithdrawalCause::NotSent)
                            .await;
                    },
                    &handle,
                );
                withdrawals.handed(n);
            } else {
                // the runtime is gone (the process is stopping past its grace): the leases lapse (gap G006-002)
                count_undelivered(WithdrawalCause::NotSent, WithdrawalOutcome::Error, n);
            }
        }))
    }

    /// A sink that keeps what it is given, for the tests of the body.
    #[cfg(test)]
    pub(crate) fn recording() -> (Self, Arc<std::sync::Mutex<Vec<UndeliveredLease>>>) {
        let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = Arc::clone(&kept);
        (
            Self(Arc::new(move |leases| {
                into.lock().expect("lock").extend(leases);
            })),
            kept,
        )
    }
}

/// The leases of one frame (or one unary answer) not yet handed to the transport.
pub(crate) struct PendingLeases {
    leases: Vec<UndeliveredLease>,
    sink: Option<WithdrawalSink>,
}

impl PendingLeases {
    /// A guard that carries no lease (a hint, an error, an empty answer).
    pub(crate) const fn none() -> Self {
        Self {
            leases: Vec::new(),
            sink: None,
        }
    }

    pub(crate) fn of(tenant: TenantId, tasks: &[ClaimedTask], sink: &WithdrawalSink) -> Self {
        Self {
            leases: tasks
                .iter()
                .map(|task| UndeliveredLease::of(tenant, task))
                .collect(),
            sink: Some(sink.clone()),
        }
    }

    /// The frame that carries these leases was handed to the transport: from here only delivery acknowledgement can
    /// recover them (gap G006-001).
    pub(crate) fn handed(mut self) {
        self.leases.clear();
    }

    /// Leases built by a test, guarded as a claim's would be.
    #[cfg(test)]
    pub(crate) fn recorded(leases: Vec<UndeliveredLease>, sink: &WithdrawalSink) -> Self {
        Self {
            leases,
            sink: Some(sink.clone()),
        }
    }
}

impl Drop for PendingLeases {
    fn drop(&mut self) {
        if self.leases.is_empty() {
            return;
        }
        if let Some(sink) = &self.sink {
            (sink.0)(std::mem::take(&mut self.leases));
        }
    }
}

/// How many withdrawals run at once when the composition root says nothing: a quarter of the default pool of 20
/// (decision D006-038).
pub const DEFAULT_WITHDRAWAL_CONCURRENCY: usize = 5;

/// The most withdrawals an engine runs at once for a pool of `max_connections`: a quarter of it, at least one (decision
/// D006-038). The rest of the pool stays for the scheduler, the lease keeper, the claims and the reports.
#[must_use]
pub const fn withdrawal_concurrency(max_connections: u32) -> usize {
    let quarter = (max_connections / 4) as usize;
    if quarter == 0 { 1 } else { quarter }
}

/// The withdrawals of a worker API in flight, and the claims that may hand leases to them; a graceful shutdown waits for
/// them (decisions D006-016 and D006-036).
#[derive(Clone, Debug)]
pub struct Withdrawals {
    tracker: TaskTracker,
    permits: Arc<tokio::sync::Semaphore>,
    handed: Arc<(std::sync::atomic::AtomicU64, tokio::sync::Notify)>,
}

impl Default for Withdrawals {
    fn default() -> Self {
        Self::bounded(DEFAULT_WITHDRAWAL_CONCURRENCY)
    }
}

impl Withdrawals {
    /// At most `concurrency` withdrawals at once (at least one).
    #[must_use]
    pub fn bounded(concurrency: usize) -> Self {
        Self {
            tracker: TaskTracker::new(),
            permits: Arc::new(tokio::sync::Semaphore::new(concurrency.max(1))),
            handed: Arc::default(),
        }
    }

    /// Runs `task` (a stream's producer, a unary claim) on the tracker a graceful shutdown waits for.
    pub(crate) fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tracker.spawn(task)
    }

    /// How many withdrawals may run at once now.
    #[must_use]
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    fn handed(&self, n: u64) {
        self.handed
            .0
            .fetch_add(n, std::sync::atomic::Ordering::AcqRel);
        self.handed.1.notify_waiters();
    }

    /// How many leases were handed to the withdrawal since the worker API was built (a withdrawal of each is spawned
    /// before the count moves).
    #[must_use]
    pub fn handed_over(&self) -> u64 {
        self.handed.0.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Waits until at least `n` leases were handed to the withdrawal: a test's barrier, never a sleep.
    pub async fn until_handed_over(&self, n: u64) {
        loop {
            let notified = self.handed.1.notified();
            if self.handed_over() >= n {
                return;
            }
            notified.await;
        }
    }

    /// Waits, at most `within`, for every withdrawal handed over until now and those that start meanwhile. Whether they
    /// all ended.
    pub async fn drain(&self, within: Duration) -> bool {
        self.tracker.close();
        let ended = tokio::time::timeout(within, self.tracker.wait())
            .await
            .is_ok();
        self.tracker.reopen();
        ended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter of the pool, at least one (decision D006-038), at the edges of the division.
    #[test]
    fn a_quarter_of_the_pool_withdraws_at_once() {
        for (pool, expected) in [
            (0, 1),
            (1, 1),
            (3, 1),
            (4, 1),
            (7, 1),
            (8, 2),
            (20, 5),
            (21, 5),
            (100, 25),
        ] {
            assert_eq!(withdrawal_concurrency(pool), expected, "pool {pool}");
        }
        assert_eq!(withdrawal_concurrency(20), DEFAULT_WITHDRAWAL_CONCURRENCY);
        assert_eq!(Withdrawals::bounded(0).available(), 1);
    }
}
