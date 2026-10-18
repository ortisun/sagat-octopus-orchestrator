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
