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
/// series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveKind {
    Switch,
    Collection,
    Loop,
    Approval,
    Query,
    Update,
    Continuation,
    /// Anything the projector cannot name: a kind written by a newer version.
    Unsupported,
}

impl WorkflowPrimitiveKind {
    pub const ALL: [Self; 8] = [
        Self::Switch,
        Self::Collection,
        Self::Loop,
        Self::Approval,
        Self::Query,
        Self::Update,
        Self::Continuation,
        Self::Unsupported,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Collection => "collection",
            Self::Loop => "loop",
            Self::Approval => "approval",
            Self::Query => "query",
            Self::Update => "update",
            Self::Continuation => "continuation",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Persisted outcomes allowed as the `outcome` label of workflow primitive metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveOutcome {
    Case,
    Default,
    Entered,
    Admitted,
    Committed,
    Draining,
    Succeeded,
    Failed,
    Cancelled,
    Approved,
    Rejected,
    Expired,
    Applied,
    Continued,
    Unsupported,
}

impl WorkflowPrimitiveOutcome {
    pub const ALL: [Self; 15] = [
        Self::Case,
        Self::Default,
        Self::Entered,
        Self::Admitted,
        Self::Committed,
        Self::Draining,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
        Self::Approved,
        Self::Rejected,
        Self::Expired,
        Self::Applied,
        Self::Continued,
        Self::Unsupported,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Case => "case",
            Self::Default => "default",
            Self::Entered => "entered",
            Self::Admitted => "admitted",
            Self::Committed => "committed",
            Self::Draining => "draining",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
            Self::Applied => "applied",
            Self::Continued => "continued",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Stable rejection codes allowed as the `code` label of workflow primitive metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveRejectionCode {
    UnsupportedNode,
    InvalidScope,
    IterationLimit,
    InstanceLimit,
    ChainLimit,
    ExpressionError,
    RunDataLimit,
    InvalidOperationState,
    NotEligible,
    InitiatorCannotDecide,
    AlreadyDecided,
    IdempotencyKeyReused,
    RevisionConflict,
    ChainHeadChanged,
    UpdateRejected,
    RunNotMutable,
    RunContinued,
    StateConflict,
    ContinuationNotQuiescent,
    QuotaExceeded,
    RateLimited,
    StaleFence,
}

impl WorkflowPrimitiveRejectionCode {
    pub const ALL: [Self; 22] = [
        Self::UnsupportedNode,
        Self::InvalidScope,
        Self::IterationLimit,
        Self::InstanceLimit,
        Self::ChainLimit,
        Self::ExpressionError,
        Self::RunDataLimit,
        Self::InvalidOperationState,
        Self::NotEligible,
        Self::InitiatorCannotDecide,
        Self::AlreadyDecided,
        Self::IdempotencyKeyReused,
        Self::RevisionConflict,
        Self::ChainHeadChanged,
        Self::UpdateRejected,
        Self::RunNotMutable,
        Self::RunContinued,
        Self::StateConflict,
        Self::ContinuationNotQuiescent,
        Self::QuotaExceeded,
        Self::RateLimited,
        Self::StaleFence,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedNode => "UNSUPPORTED_NODE",
            Self::InvalidScope => "INVALID_SCOPE",
            Self::IterationLimit => "ITERATION_LIMIT",
            Self::InstanceLimit => "INSTANCE_LIMIT",
            Self::ChainLimit => "CHAIN_LIMIT",
            Self::ExpressionError => "EXPRESSION_ERROR",
            Self::RunDataLimit => "RUN_DATA_LIMIT",
            Self::InvalidOperationState => "INVALID_OPERATION_STATE",
            Self::NotEligible => "NOT_ELIGIBLE",
            Self::InitiatorCannotDecide => "INITIATOR_CANNOT_DECIDE",
            Self::AlreadyDecided => "ALREADY_DECIDED",
            Self::IdempotencyKeyReused => "IDEMPOTENCY_KEY_REUSED",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::ChainHeadChanged => "CHAIN_HEAD_CHANGED",
            Self::UpdateRejected => "UPDATE_REJECTED",
            Self::RunNotMutable => "RUN_NOT_MUTABLE",
            Self::RunContinued => "RUN_CONTINUED",
            Self::StateConflict => "STATE_CONFLICT",
            Self::ContinuationNotQuiescent => "CONTINUATION_NOT_QUIESCENT",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::RateLimited => "RATE_LIMITED",
            Self::StaleFence => "STALE_FENCE",
        }
    }

    /// The code whose text equals a `details.reason`, if it is one of this closed set. Any other
    /// reason yields `None`, so a reason can never widen the label set.
    #[must_use]
    pub fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == reason)
    }
}

/// Metric instruments and their only permitted label keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveMetric {
    Transitions,
    Active,
    AdmissionWait,
    ApprovalWait,
    OperationDuration,
    Rejections,
    Continuation,
}

impl WorkflowPrimitiveMetric {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transitions => "workflow_primitive_transitions_total",
            Self::Active => "workflow_primitive_active",
            Self::AdmissionWait => "workflow_primitive_admission_wait_seconds",
            Self::ApprovalWait => "workflow_approval_wait_seconds",
            Self::OperationDuration => "workflow_operation_duration_seconds",
            Self::Rejections => "workflow_primitive_rejections_total",
            Self::Continuation => "workflow_continuation_total",
        }
    }

    #[must_use]
    pub const fn label_keys(self) -> &'static [&'static str] {
        match self {
            Self::Transitions | Self::OperationDuration => &["kind", "outcome"],
            Self::Active | Self::AdmissionWait => &["kind"],
            Self::ApprovalWait | Self::Continuation => &["outcome"],
            Self::Rejections => &["kind", "code"],
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
