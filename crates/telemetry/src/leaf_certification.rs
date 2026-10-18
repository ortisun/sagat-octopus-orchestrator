//! Closed operational attempt observations, not a projection of durable source-fact totals.

use std::time::Duration;

pub const LEAF_CERTIFICATION_SPAN: &str = "workflow.collection.leaf.certify";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafCertificationOutcome {
    Certified,
    Replayed,
    Refused,
    StorageError,
    Unknown,
}

impl LeafCertificationOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Certified => "certified",
            Self::Replayed => "replayed",
            Self::Refused => "refused",
            Self::StorageError => "storage_error",
            Self::Unknown => "unknown",
        }
    }

    /// Returned attempts only: cancellation has no returned outcome. The confirmed counter
    /// counts acknowledged first commits, never replay or Unknown. Durable canonical records
    /// remain authoritative: an unacknowledged server commit can be absent from this counter.
    pub fn record(self, duration: Duration) {
        metrics::counter!("workflow_collection_leaf_certification_attempts_total", "outcome" => self.as_str()).increment(1);
        metrics::histogram!("workflow_collection_leaf_certification_duration_seconds", "outcome" => self.as_str()).record(duration.as_secs_f64());
        if self == Self::Certified {
            metrics::counter!("workflow_collection_leaf_certified_total").increment(1);
        }
    }
}
