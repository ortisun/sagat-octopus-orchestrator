//! Typed errors of the execution model. None of them carries payload data.

use super::instance::ExecutionInstanceId;
use super::state::{CollectionPhase, RunStatus, StepStatus};
use crate::workflow::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    /// The run already reached a terminal status; nothing can change any more.
    #[error("the run is already {0}")]
    RunFinished(RunStatus),
    #[error("the run has no step '{0}'")]
    UnknownStep(NodeId),
    #[error("the run has no execution instance '{0:?}'")]
    UnknownInstance(ExecutionInstanceId),
    /// The step is not in a status from which this event is legal.
    #[error("step '{node}' is {status}, which does not allow '{event}'")]
    IllegalTransition {
        node: NodeId,
        status: StepStatus,
        event: &'static str,
    },
    #[error("execution instance '{instance:?}' is {status}, which does not allow '{event}'")]
    IllegalInstanceTransition {
        instance: ExecutionInstanceId,
        status: StepStatus,
        event: &'static str,
    },
    /// The attempt or generation is not the one that holds the lease: the caller is stale.
    #[error("step '{node}': the attempt/generation is stale")]
    StaleAttempt { node: NodeId },
    #[error("execution instance '{instance:?}': the attempt/generation is stale")]
    StaleInstanceAttempt { instance: ExecutionInstanceId },
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
    /// A scoped transition is valid journal vocabulary but its reducer path is not enabled yet.
    #[error("scoped event '{0}' is not executable by this runtime")]
    UnsupportedScopedEvent(&'static str),
    #[error("the run has no collection scope '{0:?}'")]
    UnknownCollection(ExecutionInstanceId),
    #[error("collection scope '{0:?}' has already entered")]
    CollectionAlreadyEntered(ExecutionInstanceId),
    #[error("collection scope '{collection:?}' has no admitted iteration {index}")]
    UnknownCollectionIteration {
        collection: ExecutionInstanceId,
        index: u32,
    },
    #[error("collection scope '{collection:?}' already admitted iteration {index}")]
    DuplicateCollectionAdmission {
        collection: ExecutionInstanceId,
        index: u32,
    },
    #[error("collection scope '{collection:?}' expected admission {expected}, got {actual}")]
    UnexpectedCollectionAdmission {
        collection: ExecutionInstanceId,
        expected: u32,
        actual: u32,
    },
    #[error("collection scope '{collection:?}' reached concurrency {limit}")]
    CollectionConcurrencyExceeded {
        collection: ExecutionInstanceId,
        limit: u16,
    },
    #[error("collection scope '{collection:?}' is {phase:?} and does not admit work")]
    CollectionNotOpen {
        collection: ExecutionInstanceId,
        phase: CollectionPhase,
    },
    #[error("collection scope '{collection:?}' received an invalid instance '{instance:?}'")]
    InvalidCollectionInstance {
        collection: ExecutionInstanceId,
        instance: ExecutionInstanceId,
    },
    #[error("collection scope '{collection:?}' has invalid limits or cardinality")]
    InvalidCollectionLimits { collection: ExecutionInstanceId },
    #[error("collection scope '{collection:?}' cannot apply '{event}' in its current progress")]
    IllegalCollectionTransition {
        collection: ExecutionInstanceId,
        event: &'static str,
    },
}
