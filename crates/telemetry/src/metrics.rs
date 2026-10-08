//! Process-wide Prometheus metrics.
//!
//! Code records with the `metrics` facade (`counter!`, `histogram!`, `gauge!`); this module owns the
//! recorder that turns those into the Prometheus text format. Label values must be low-cardinality
//! (route *templates*, status codes, fixed reason names): never identifiers, tenants or subjects.

use std::sync::OnceLock;

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

/// Bucket boundaries (seconds) for every `*_duration_seconds` histogram.
const DURATION_BUCKETS: [f64; 12] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];

/// Handle to the installed recorder.
#[derive(Debug)]
pub struct Metrics {
    handle: PrometheusHandle,
}

static GLOBAL: OnceLock<Metrics> = OnceLock::new();

impl Metrics {
    /// The process-wide metrics, installing the recorder on first use. Safe to call from many
    /// threads and from tests that start several servers in one process.
    ///
    /// If another recorder was already installed by someone else, rendering yields an empty page
    /// rather than failing: metrics must never take the service down.
    pub fn global() -> &'static Self {
        GLOBAL.get_or_init(|| {
            let builder = PrometheusBuilder::new()
                .set_buckets_for_metric(Matcher::Suffix("duration_seconds".to_owned()), &DURATION_BUCKETS)
                .unwrap_or_else(|_| PrometheusBuilder::new());
            let handle = builder.install_recorder().unwrap_or_else(|error| {
                tracing::warn!(%error, "a metrics recorder is already installed; /metrics will be empty");
                PrometheusBuilder::new().build_recorder().handle()
            });
            Self { handle }
        })
    }

    /// The current metrics in the Prometheus text exposition format.
    #[must_use]
    pub fn render(&self) -> String {
        self.handle.render()
    }
}
