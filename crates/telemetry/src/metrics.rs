//! Process-wide Prometheus metrics.
//!
//! Code records with the `metrics` facade (`counter!`, `histogram!`, `gauge!`); this module owns the
//! recorder that turns those into the Prometheus text format. Label values must be low-cardinality
//! (route *templates*, status codes, fixed reason names): never identifiers, tenants or subjects,
//! and never anything a client chooses freely (such as an HTTP method token).
//!
//! The recorder is also a memory-safety backstop: a series that is not updated for
//! [`Metrics::IDLE_TIMEOUT`] is forgotten, so a label that slipped through unbounded stays a bounded
//! leak instead of a permanent one. The recorder does that work **when the page is rendered**, so a
//! process whose metrics listener is off (or never scraped) must call [`Metrics::prune_idle`]
//! periodically.

use std::sync::OnceLock;
use std::time::Duration;

use metrics_exporter_prometheus::{
    Matcher, PrometheusBuilder, PrometheusHandle, PrometheusRecorder,
};
use metrics_util::MetricKindMask;

/// Primitive families allowed as the `kind` label of workflow primitive metrics.
///
/// This enum is intentionally closed so a definition-supplied node kind cannot create a metric
/// series. Add a variant only when its runtime telemetry is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveKind {
    Switch,
}

impl WorkflowPrimitiveKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Switch => "switch",
        }
    }
}

/// Persisted outcomes allowed as the `outcome` label of workflow primitive metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveOutcome {
    Case,
    Default,
}

impl WorkflowPrimitiveOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Case => "case",
            Self::Default => "default",
        }
    }
}

/// Bucket boundaries (seconds) for every `*_seconds` histogram: a request, an attempt, a lag. From a
/// millisecond to an hour, because the same rule serves the time a request takes and the time a
/// delivery has waited. A histogram with no rule is rendered as a summary, whose quantiles cannot
/// be added across replicas, so the dashboard could not compute a percentile from it.
const DURATION_BUCKETS: [f64; 20] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
    120.0, 300.0, 600.0, 1800.0, 3600.0,
];

/// Handle to the installed recorder.
#[derive(Debug)]
pub struct Metrics {
    handle: PrometheusHandle,
}

static GLOBAL: OnceLock<Metrics> = OnceLock::new();

/// A builder with the project's histogram buckets and the given idle expiry for every metric kind.
fn builder(idle_timeout: Duration) -> PrometheusBuilder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_seconds".to_owned()), &DURATION_BUCKETS)
        .unwrap_or_else(|_| PrometheusBuilder::new())
        .idle_timeout(MetricKindMask::ALL, Some(idle_timeout))
}

impl Metrics {
    /// How long a series may go without an update before it is dropped from the page. Longer than
    /// any realistic scrape gap (so a quiet counter does not flicker), short enough to bound memory.
    pub const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

    /// The process-wide metrics, installing the recorder on first use. Safe to call from many
    /// threads and from tests that start several servers in one process.
    ///
    /// If another recorder was already installed by someone else, rendering yields an empty page
    /// rather than failing: metrics must never take the service down.
    pub fn global() -> &'static Self {
        GLOBAL.get_or_init(|| {
            let handle = builder(Self::IDLE_TIMEOUT)
                .install_recorder()
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "a metrics recorder is already installed; /metrics will be empty");
                    PrometheusBuilder::new().build_recorder().handle()
                });
            Self { handle }
        })
    }

    /// A recorder that is **not** installed process-wide, with its own idle expiry. For tests that
    /// need to observe expiry quickly; record into it with `metrics::with_local_recorder`.
    #[must_use]
    pub fn detached(idle_timeout: Duration) -> (Self, PrometheusRecorder) {
        let recorder = builder(idle_timeout).build_recorder();
        (
            Self {
                handle: recorder.handle(),
            },
            recorder,
        )
    }

    /// The current metrics in the Prometheus text exposition format.
    #[must_use]
    pub fn render(&self) -> String {
        self.handle.render()
    }

    /// Forgets series idle for longer than [`Self::IDLE_TIMEOUT`] (and settles histogram buffers).
    /// Rendering does this too; call it on a timer so a process that nobody scrapes does not keep
    /// every series it has ever created.
    pub fn prune_idle(&self) {
        self.handle.run_upkeep();
        drop(self.handle.render());
    }
}
