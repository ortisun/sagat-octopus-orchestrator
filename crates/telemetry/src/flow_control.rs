//! Closed telemetry vocabulary of flow control (feature 022, release 0.3.0 slice; FR-022-033, FR-022-049; plan
//! "Observability" and "Observability (rate limits)").
//!
//! Every label takes its values from a small closed set; none takes a tenant, a namespace, a run, a workflow, a group or
//! a key (they belong in the spans and the audit, never in a series). Spans carry the group's *name*
//! (`sagat.concurrency.group`, `sagat.rate_limit.group`) and never a key value or its label.
//!
//! The gate records them after each commit: the decisions, grants, timeouts, dequeues and returns of both levels are
//! counted since decision D022-045 (the readiness gate, the grant and timeout routines, T022-096..098) and the run's
//! start since T022-022 and T022-072. The wait histograms and the grant lag are observed at each grant from the queue,
//! after its commit, and the waiter and unleased-grant gauges are set by the engine's monitor every look (task T022-082,
//! decision D022-083), which also raises the four flow-control alerts.

/// The `scope` label: a run's start or a step's attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlScope {
    Run,
    Step,
}

impl FlowControlScope {
    pub const ALL: [Self; 2] = [Self::Run, Self::Step];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Step => "step",
        }
    }
}

/// The `outcome` label of `concurrency_acquisitions_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyOutcome {
    Acquired,
    Queued,
    RefusedFull,
    RefusedReset,
    Granted,
    SkippedAhead,
    TimedOut,
    Dequeued,
}

impl ConcurrencyOutcome {
    pub const ALL: [Self; 8] = [
        Self::Acquired,
        Self::Queued,
        Self::RefusedFull,
        Self::RefusedReset,
        Self::Granted,
        Self::SkippedAhead,
        Self::TimedOut,
        Self::Dequeued,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acquired => "acquired",
            Self::Queued => "queued",
            Self::RefusedFull => "refused_full",
            Self::RefusedReset => "refused_reset",
            Self::Granted => "granted",
            Self::SkippedAhead => "skipped_ahead",
            Self::TimedOut => "timed_out",
            Self::Dequeued => "dequeued",
        }
    }
}

/// The `outcome` label of `rate_limit_decisions_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitOutcome {
    Granted,
    Queued,
    GrantedFromQueue,
    RefusedFull,
    RefusedReset,
    SkippedAhead,
    TimedOut,
    Dequeued,
    SlotReturned,
}

impl RateLimitOutcome {
    pub const ALL: [Self; 9] = [
        Self::Granted,
        Self::Queued,
        Self::GrantedFromQueue,
        Self::RefusedFull,
        Self::RefusedReset,
        Self::SkippedAhead,
        Self::TimedOut,
        Self::Dequeued,
        Self::SlotReturned,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Queued => "queued",
            Self::GrantedFromQueue => "granted_from_queue",
            Self::RefusedFull => "refused_full",
            Self::RefusedReset => "refused_reset",
            Self::SkippedAhead => "skipped_ahead",
            Self::TimedOut => "timed_out",
            Self::Dequeued => "dequeued",
            Self::SlotReturned => "slot_returned",
        }
    }
}

/// The metric families of the slice and their only label keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlMetric {
    ConcurrencyAcquisitions,
    ConcurrencyWaitSeconds,
    ConcurrencyWaiters,
    RateLimitDecisions,
    RateLimitWaitSeconds,
    RateLimitWaiters,
    RateLimitGrantLagSeconds,
    RateLimitUnleasedGrants,
}

impl FlowControlMetric {
    pub const ALL: [Self; 8] = [
        Self::ConcurrencyAcquisitions,
        Self::ConcurrencyWaitSeconds,
        Self::ConcurrencyWaiters,
        Self::RateLimitDecisions,
        Self::RateLimitWaitSeconds,
        Self::RateLimitWaiters,
        Self::RateLimitGrantLagSeconds,
        Self::RateLimitUnleasedGrants,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConcurrencyAcquisitions => "concurrency_acquisitions_total",
            Self::ConcurrencyWaitSeconds => "concurrency_wait_seconds",
            Self::ConcurrencyWaiters => "concurrency_waiters",
            Self::RateLimitDecisions => "rate_limit_decisions_total",
            Self::RateLimitWaitSeconds => "rate_limit_wait_seconds",
            Self::RateLimitWaiters => "rate_limit_waiters",
            Self::RateLimitGrantLagSeconds => "rate_limit_grant_lag_seconds",
            Self::RateLimitUnleasedGrants => "rate_limit_unleased_grants",
        }
    }

    #[must_use]
    pub const fn label_keys(self) -> &'static [&'static str] {
        match self {
            Self::ConcurrencyAcquisitions | Self::RateLimitDecisions => &["scope", "outcome"],
            Self::ConcurrencyWaitSeconds
            | Self::ConcurrencyWaiters
            | Self::RateLimitWaitSeconds
            | Self::RateLimitWaiters => &["scope"],
            Self::RateLimitGrantLagSeconds | Self::RateLimitUnleasedGrants => &[],
        }
    }
}

/// The spans of the slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlSpan {
    ConcurrencyAcquire,
    ConcurrencyGrant,
    RateLimitAdmit,
    RateLimitGrant,
}

impl FlowControlSpan {
    pub const ALL: [Self; 4] = [
        Self::ConcurrencyAcquire,
        Self::ConcurrencyGrant,
        Self::RateLimitAdmit,
        Self::RateLimitGrant,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConcurrencyAcquire => "workflow.concurrency.acquire",
            Self::ConcurrencyGrant => "workflow.concurrency.grant",
            Self::RateLimitAdmit => "workflow.rate_limit.admit",
            Self::RateLimitGrant => "workflow.rate_limit.grant",
        }
    }

    /// The attribute that names the group a span is about: a name, never a key value or label.
    #[must_use]
    pub const fn group_attribute(self) -> &'static str {
        match self {
            Self::ConcurrencyAcquire | Self::ConcurrencyGrant => "sagat.concurrency.group",
            Self::RateLimitAdmit | Self::RateLimitGrant => "sagat.rate_limit.group",
        }
    }
}

pub fn record_concurrency(scope: FlowControlScope, outcome: ConcurrencyOutcome) {
    metrics::counter!(
        FlowControlMetric::ConcurrencyAcquisitions.as_str(),
        "scope" => scope.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_concurrency_wait(scope: FlowControlScope, seconds: f64) {
    metrics::histogram!(FlowControlMetric::ConcurrencyWaitSeconds.as_str(), "scope" => scope.as_str())
        .record(seconds);
}

pub fn set_concurrency_waiters(scope: FlowControlScope, waiters: f64) {
    metrics::gauge!(FlowControlMetric::ConcurrencyWaiters.as_str(), "scope" => scope.as_str())
        .set(waiters);
}

pub fn record_rate_limit(scope: FlowControlScope, outcome: RateLimitOutcome) {
    metrics::counter!(
        FlowControlMetric::RateLimitDecisions.as_str(),
        "scope" => scope.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_rate_limit_wait(scope: FlowControlScope, seconds: f64) {
    metrics::histogram!(FlowControlMetric::RateLimitWaitSeconds.as_str(), "scope" => scope.as_str())
        .record(seconds);
}

pub fn set_rate_limit_waiters(scope: FlowControlScope, waiters: f64) {
    metrics::gauge!(FlowControlMetric::RateLimitWaiters.as_str(), "scope" => scope.as_str())
        .set(waiters);
}

pub fn record_grant_lag(seconds: f64) {
    metrics::histogram!(FlowControlMetric::RateLimitGrantLagSeconds.as_str()).record(seconds);
}

pub fn set_unleased_grants(grants: f64) {
    metrics::gauge!(FlowControlMetric::RateLimitUnleasedGrants.as_str()).set(grants);
}
