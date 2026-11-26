//! Operational alerts: the conditions that mean the platform, not the workflow, needs a person
//! (FR-008-005).
//!
//! A condition is about a **namespace** (or the tenant, when it cannot be attributed to one), has a
//! closed kind, a severity, and a runbook that says what to do. What an alert carries is counts and
//! ages: never a payload, a run's data or a secret.

use std::fmt;
use std::str::FromStr;

/// What is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AlertKind {
    /// Messages in the outbox have waited too long: runs are not being announced to workers or
    /// subscribers (the broker or the dispatcher is down).
    OutboxStuck,
    /// An attempt's lease ran out and nobody has taken it back: the scheduler is not running.
    LeaseNotReaped,
    /// A step has been ready for a long time and no worker takes it (the subject is the activity).
    StepUnclaimed,
    /// A step waits for a person to decide, and has for a long time.
    ReviewPending,
    /// Events, deliveries or connector messages ran out of attempts and wait to be redriven or
    /// discarded (the subject says which).
    DeadItems,
    /// A concurrency key has no room and its oldest eligible waiter was passed over by every holder it has (feature
    /// 022; the subject is the key's group, never a key).
    ConcurrencyQueueStuck,
    /// A rate-limit key with waiters has been due for long and no grant of it was made: the grant pass is not serving
    /// it (feature 022; the subject is the key's group).
    RateLimitGrantLagging,
    /// A rate-limit key's buffer of run starts is more than 80% full (feature 022; the subject is the key's group).
    RateLimitQueueSaturated,
    /// A flow-control waiter belongs to a run that ended: an invariant breach (feature 022; the subject is the
    /// waiter's group).
    FlowControlWaiterOrphaned,
}

impl AlertKind {
    pub const ALL: [Self; 9] = [
        Self::OutboxStuck,
        Self::LeaseNotReaped,
        Self::StepUnclaimed,
        Self::ReviewPending,
        Self::DeadItems,
        Self::ConcurrencyQueueStuck,
        Self::RateLimitGrantLagging,
        Self::RateLimitQueueSaturated,
        Self::FlowControlWaiterOrphaned,
    ];

    /// The kinds of feature 022 (flow control), whose subject is a group.
    pub const FLOW_CONTROL: [Self; 4] = [
        Self::ConcurrencyQueueStuck,
        Self::RateLimitGrantLagging,
        Self::RateLimitQueueSaturated,
        Self::FlowControlWaiterOrphaned,
    ];

    /// The name stored, shown and put in metrics: a closed set.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OutboxStuck => "OUTBOX_STUCK",
            Self::LeaseNotReaped => "LEASE_NOT_REAPED",
            Self::StepUnclaimed => "STEP_UNCLAIMED",
            Self::ReviewPending => "REVIEW_PENDING",
            Self::DeadItems => "DEAD_ITEMS",
            Self::ConcurrencyQueueStuck => "CONCURRENCY_QUEUE_STUCK",
            Self::RateLimitGrantLagging => "RATE_LIMIT_GRANT_LAGGING",
            Self::RateLimitQueueSaturated => "RATE_LIMIT_QUEUE_SATURATED",
            Self::FlowControlWaiterOrphaned => "FLOW_CONTROL_WAITER_ORPHANED",
        }
    }

    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::OutboxStuck
            | Self::LeaseNotReaped
            | Self::RateLimitGrantLagging
            | Self::FlowControlWaiterOrphaned => Severity::Critical,
            Self::StepUnclaimed
            | Self::ReviewPending
            | Self::DeadItems
            | Self::ConcurrencyQueueStuck
            | Self::RateLimitQueueSaturated => Severity::Warning,
        }
    }

    /// The section of `docs/runbooks/alerts.md` that says what to do.
    #[must_use]
    pub const fn runbook_anchor(self) -> &'static str {
        match self {
            Self::OutboxStuck => "outbox-stuck",
            Self::LeaseNotReaped => "lease-not-reaped",
            Self::StepUnclaimed => "step-unclaimed",
            Self::ReviewPending => "review-pending",
            Self::DeadItems => "dead-items",
            Self::ConcurrencyQueueStuck => "concurrency-queue-stuck",
            Self::RateLimitGrantLagging => "rate-limit-grant-lagging",
            Self::RateLimitQueueSaturated => "rate-limit-queue-saturated",
            Self::FlowControlWaiterOrphaned => "flow-control-waiter-orphaned",
        }
    }

    /// What the condition is, in a sentence that names no data.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::OutboxStuck => {
                "messages of the outbox have waited too long: workers and subscribers are not being told"
            }
            Self::LeaseNotReaped => {
                "attempts whose lease ran out have not been taken back: the scheduler is not running"
            }
            Self::StepUnclaimed => "a step has been ready for a long time and no worker takes it",
            Self::ReviewPending => "a step has waited a long time for a person to decide",
            Self::DeadItems => "items ran out of attempts and wait to be redriven or discarded",
            Self::ConcurrencyQueueStuck => {
                "a concurrency key has no room and its oldest waiter was passed over by every holder it has"
            }
            Self::RateLimitGrantLagging => {
                "a rate-limit key with waiters has been due for long and the grant pass has not served it"
            }
            Self::RateLimitQueueSaturated => {
                "a rate-limit key's buffer of run starts is more than 80% full"
            }
            Self::FlowControlWaiterOrphaned => {
                "a flow-control waiter belongs to a run that ended: an invariant breach"
            }
        }
    }
}

impl fmt::Display for AlertKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AlertKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or(())
    }
}

/// How urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Needs attention soon.
    Warning,
    /// The platform is not doing its job.
    Critical,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// Where an alert is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertState {
    Firing,
    Resolved,
}

impl AlertState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "FIRING",
            Self::Resolved => "RESOLVED",
        }
    }
}

impl FromStr for AlertState {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "FIRING" => Ok(Self::Firing),
            "RESOLVED" => Ok(Self::Resolved),
            _ => Err(()),
        }
    }
}

/// The longest a subject (an activity name, the kind of dead item) is kept on an alert.
pub const MAX_SUBJECT_CHARS: usize = 120;
