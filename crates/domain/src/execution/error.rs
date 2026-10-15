//! Typed errors of the execution model. None of them carries payload data.

use super::state::{RunStatus, StepStatus};
use crate::workflow::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    /// The run already reached a terminal status; nothing can change any more.
    #[error("the run is already {0}")]
    RunFinished(RunStatus),
    #[error("the run has no step '{0}'")]
    UnknownStep(NodeId),
    /// The step is not in a status from which this event is legal.
    #[error("step '{node}' is {status}, which does not allow '{event}'")]
    IllegalTransition {
        node: NodeId,
        status: StepStatus,
        event: &'static str,
    },
    /// The attempt or generation is not the one that holds the lease: the caller is stale.
    #[error("step '{node}': the attempt/generation is stale")]
    StaleAttempt { node: NodeId },
    #[error("step '{0}' has already been scheduled")]
    AlreadyScheduled(NodeId),
    /// A timer, wait deadline or retry delay has not elapsed yet.
    #[error("step '{0}' is not due yet")]
    NotYetDue(NodeId),
    #[error("invalid failure code '{0}'")]
    InvalidFailureCode(String),
    #[error("invalid correlation key")]
    InvalidCorrelationKey,
    /// The run is paused, stopping or finished, so no new attempt may start.
    #[error("the run is {0} and does not accept new work")]
    RunNotRunnable(RunStatus),
    #[error("the run is already paused")]
    AlreadyPaused,
    #[error("the run is not paused")]
    NotPaused,
    #[error("the run is already being stopped")]
    AlreadyStopping,
    /// A reconciliation is completed with its verdict, not like an ordinary step.
    #[error("step '{0}' is a reconciliation: complete it with its verdict")]
    NeedsVerdict(NodeId),
    /// A rule the reducer proves was broken; indicates a bug, never bad input.
    #[error("internal invariant violated: {0}")]
    Invariant(&'static str),
    /// A structured node has a valid definition but its execution capability is not enabled.
    #[error("structured node '{0}' is not executable by this runtime")]
    UnsupportedStructuredNode(NodeId),
}
