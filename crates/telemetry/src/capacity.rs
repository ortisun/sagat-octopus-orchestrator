//! Closed observations for the dormant exact ChildRoot Finish port.

use std::time::Duration;

pub const CAPACITY_FINISH_SPAN: &str = "workflow.capacity.finish";

/// Returned attempt outcomes, never a caller-supplied reason or storage error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityFinishOutcome {
    Applied,
    Replayed,
    Busy,
    Refused,
    StorageError,
    Unknown,
}

impl CapacityFinishOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Replayed => "replayed",
            Self::Busy => "busy",
            Self::Refused => "refused",
            Self::StorageError => "storage_error",
            Self::Unknown => "unknown",
        }
    }

    /// Count a returned outcome. Cancellation has no returned outcome and is not classified.
    /// Applied means a first commit was acknowledged; receipt replay and unknown commits never
    /// increment the transition counter. An unknown commit can therefore remain uncounted here:
    /// the durable receipt, not a process metric, is authoritative.
    pub fn record(self, duration: Duration) {
        metrics::counter!("workflow_capacity_finish_attempts_total", "outcome" => self.as_str())
            .increment(1);
        metrics::histogram!("workflow_capacity_finish_duration_seconds", "outcome" => self.as_str())
            .record(duration.as_secs_f64());
        if self == Self::Applied {
            metrics::counter!("workflow_capacity_member_finished_total").increment(1);
        }
    }
}

/// Closed outcomes for dormant Vacate; no identifiers or caller-supplied messages in labels.
pub const CAPACITY_VACATE_SPAN: &str = "workflow.capacity.vacate";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityVacateOutcome {
    Applied,
    Replayed,
    Busy,
    NotQuiescent,
    NotHolder,
    Stale,
    Denied,
    Refused,
    StorageError,
    Unknown,
}
impl CapacityVacateOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Replayed => "replayed",
            Self::Busy => "busy",
            Self::NotQuiescent => "not_quiescent",
            Self::NotHolder => "not_holder",
            Self::Stale => "stale",
            Self::Denied => "denied",
            Self::Refused => "refused",
            Self::StorageError => "storage_error",
            Self::Unknown => "unknown",
        }
    }
    pub fn record(self, duration: Duration) {
        metrics::counter!("workflow_capacity_transfer_attempts_total", "operation"=>"vacate", "outcome"=>self.as_str()).increment(1);
        metrics::histogram!("workflow_capacity_transfer_duration_seconds", "operation"=>"vacate", "outcome"=>self.as_str()).record(duration.as_secs_f64());
        if self == Self::Applied {
            metrics::counter!("workflow_capacity_member_vacated_total").increment(1);
        }
    }
}

/// Closed outcomes for dormant Acquire; possession alone is not a runnable-work verdict.
pub const CAPACITY_ACQUIRE_SPAN: &str = "workflow.capacity.acquire";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityAcquireOutcome {
    Applied,
    Replayed,
    Busy,
    NotMovable,
    Occupied,
    Stale,
    Denied,
    Refused,
    StorageError,
    Unknown,
}

impl CapacityAcquireOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Replayed => "replayed",
            Self::Busy => "busy",
            Self::NotMovable => "not_movable",
            Self::Occupied => "occupied",
            Self::Stale => "stale",
            Self::Denied => "denied",
            Self::Refused => "refused",
            Self::StorageError => "storage_error",
            Self::Unknown => "unknown",
        }
    }

    pub fn record(self, duration: Duration) {
        metrics::counter!("workflow_capacity_transfer_attempts_total", "operation"=>"acquire", "outcome"=>self.as_str()).increment(1);
        metrics::histogram!("workflow_capacity_transfer_duration_seconds", "operation"=>"acquire", "outcome"=>self.as_str()).record(duration.as_secs_f64());
        if self == Self::Applied {
            metrics::counter!("workflow_capacity_member_acquired_total").increment(1);
        }
    }
}
