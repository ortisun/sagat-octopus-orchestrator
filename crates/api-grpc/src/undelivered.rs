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
use sagat_application::worker::{ClaimedTask, UndeliveredLease, WorkerService, WorkerStore};
use sagat_domain::execution::WithdrawalCause;
use sagat_domain::identity::TenantId;
use tokio_util::task::TaskTracker;

/// A point a test holds the delivery of a claim at, after its leases committed and before they are queued or answered
/// (feature 006, decision D006-008: a barrier, never a sleep). It is given the [`Abandonment`] of the consumer, so a test
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
                withdrawals.tracker.spawn_on(
                    async move {
                        workers
                            .withdraw_undelivered(leases, WithdrawalCause::NotSent)
                            .await;
                    },
                    &handle,
                );
                withdrawals.handed(n);
            } else {
                // the runtime is gone (the process is stopping past its grace): the leases lapse (gap G006-002)
                metrics::counter!(
                    "worker_leases_undelivered_total",
                    "cause" => WithdrawalCause::NotSent.as_str(),
                    "outcome" => "error"
                )
                .increment(n);
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

/// The withdrawals of a worker API in flight; a graceful shutdown waits for them (decision D006-016).
#[derive(Clone, Debug, Default)]
pub struct Withdrawals {
    tracker: TaskTracker,
    handed: Arc<(std::sync::atomic::AtomicU64, tokio::sync::Notify)>,
}

impl Withdrawals {
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
