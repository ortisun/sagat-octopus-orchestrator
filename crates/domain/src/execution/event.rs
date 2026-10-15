//! The journal vocabulary: every change to a run is one [`RunEvent`].

use std::fmt;

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::instance::{ExecutionInstanceId, InstancePath};
use super::state::{RunId, RunStatus};
use crate::identity::clip;
use crate::saga::{ReconcileVerdict, SagaOutcome};
use crate::workflow::{Digest, NodeId};

const MAX_CODE_LEN: usize = 64;
const MAX_KEY_LEN: usize = 256;

/// A machine-readable failure code: `[A-Za-z0-9_.:-]`, 1–64 characters. Free text from a worker
/// never reaches the journal through this type; callers map unknown input to `WORKER_ERROR`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FailureCode(String);

impl FailureCode {
    /// # Errors
    /// [`ExecutionError::InvalidFailureCode`] for an empty, long or non-conforming value.
    pub fn parse(value: &str) -> Result<Self, ExecutionError> {
        let ok = !value.is_empty()
            && value.len() <= MAX_CODE_LEN
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(ExecutionError::InvalidFailureCode(clip(value)))
        }
    }

    fn system(code: &'static str) -> Self {
        Self(code.to_owned())
    }

    /// The code a worker error maps to when it supplies none that conforms.
    #[must_use]
    pub fn worker_error() -> Self {
        Self::system("WORKER_ERROR")
    }

    #[must_use]
    pub fn lease_expired() -> Self {
        Self::system("LEASE_EXPIRED")
    }

    #[must_use]
    pub fn attempt_timeout() -> Self {
        Self::system("ATTEMPT_TIMEOUT")
    }

    #[must_use]
    pub fn expression_error() -> Self {
        Self::system("EXPRESSION_ERROR")
    }

    #[must_use]
    pub fn wait_timed_out() -> Self {
        Self::system("WAIT_TIMED_OUT")
    }

    #[must_use]
    pub fn branch_failed() -> Self {
        Self::system("BRANCH_FAILED")
    }

    /// The child run failed.
    #[must_use]
    pub fn child_failed() -> Self {
        Self::system("CHILD_FAILED")
    }

    /// The child run was cancelled.
    #[must_use]
    pub fn child_cancelled() -> Self {
        Self::system("CHILD_CANCELLED")
    }

    /// The child run ran past its deadline.
    #[must_use]
    pub fn child_timed_out() -> Self {
        Self::system("CHILD_TIMED_OUT")
    }

    /// Whether a step that ended with this code may have had its effect: the attempt was lost, timed
    /// out, said it did not know, its successful report could not be retained, or its lookup could
    /// not say. (A worker that answered with an ordinary failure knows the effect did not happen; a
    /// lookup that said so is `RECONCILED_NOT_APPLIED`.)
    #[must_use]
    pub fn leaves_outcome_unknown(&self) -> bool {
        [
            Self::lease_expired(),
            Self::attempt_timeout(),
            Self::outcome_unknown(),
            Self::storage_quota_exceeded(),
            Self::reconciliation_failed(),
            Self::reconciliation_unknown(),
        ]
        .contains(self)
    }

    /// The child run failed and its saga undid what it had done (feature 003).
    #[must_use]
    pub fn child_compensated() -> Self {
        Self::system("CHILD_COMPENSATED")
    }

    /// The child run failed and its saga could not undo everything: something was left in place.
    #[must_use]
    pub fn child_partial() -> Self {
        Self::system("CHILD_PARTIAL")
    }

    /// The child run failed and a compensation of its saga failed for good.
    #[must_use]
    pub fn child_compensation_failed() -> Self {
        Self::system("CHILD_COMPENSATION_FAILED")
    }

    /// The child workflow (or the pinned version) does not exist in the parent's namespace.
    #[must_use]
    pub fn child_not_found() -> Self {
        Self::system("CHILD_NOT_FOUND")
    }

    /// Starting the child would nest runs deeper than the limit.
    #[must_use]
    pub fn child_depth_exceeded() -> Self {
        Self::system("CHILD_DEPTH_EXCEEDED")
    }

    /// The tree of runs this child would join has reached its size limit.
    #[must_use]
    pub fn child_limit_exceeded() -> Self {
        Self::system("CHILD_LIMIT_EXCEEDED")
    }

    /// The namespace or the tenant is at its quota of active runs, so the child could not start
    /// (feature 009). Distinct from [`Self::child_limit_exceeded`], which is about the size of one
    /// tree: this one clears when other runs end, and a workflow may route on it.
    #[must_use]
    pub fn child_quota_exceeded() -> Self {
        Self::system("CHILD_QUOTA_EXCEEDED")
    }

    /// The outcome of an attempt is not known: the worker says the effect may or may not have
    /// happened (or lost its connection before it could tell). Never retried or compensated blindly.
    #[must_use]
    pub fn outcome_unknown() -> Self {
        Self::system("OUTCOME_UNKNOWN")
    }

    /// The reconciliation could not be completed, so nobody knows whether the effect happened.
    #[must_use]
    pub fn reconciliation_failed() -> Self {
        Self::system("RECONCILIATION_FAILED")
    }

    /// The provider cannot say whether the effect happened.
    #[must_use]
    pub fn reconciliation_unknown() -> Self {
        Self::system("RECONCILIATION_UNKNOWN")
    }

    /// The reconciliation found that the effect did not happen, and the step has no attempt left.
    /// Unlike the code of the lost attempt it is *not* an unknown outcome: nothing is compensated.
    #[must_use]
    pub fn reconciled_not_applied() -> Self {
        Self::system("RECONCILED_NOT_APPLIED")
    }

    /// A reconciliation answered something that is not an answer.
    #[must_use]
    pub fn reconciliation_answer_invalid() -> Self {
        Self::system("RECONCILIATION_ANSWER_INVALID")
    }

    /// The compensation failed after its retries.
    #[must_use]
    pub fn compensation_failed() -> Self {
        Self::system("COMPENSATION_FAILED")
    }

    /// The run was cancelled while a saga was running: the cause of its compensation.
    #[must_use]
    pub fn run_cancelled() -> Self {
        Self::system("RUN_CANCELLED")
    }

    /// The run ran past its deadline while a saga was running.
    #[must_use]
    pub fn run_timed_out() -> Self {
        Self::system("RUN_TIMED_OUT")
    }

    /// The step's output contained the value of a secret the step was given.
    #[must_use]
    pub fn secret_in_output() -> Self {
        Self::system("SECRET_IN_OUTPUT")
    }

    /// The attempt's otherwise successful terminal record could not fit in retained storage.
    #[must_use]
    pub fn storage_quota_exceeded() -> Self {
        Self::system("STORAGE_QUOTA_EXCEEDED")
    }

    /// An operator held the step for a person to decide, before it ran.
    #[must_use]
    pub fn marked_for_review() -> Self {
        Self::system("MARKED_FOR_REVIEW")
    }

    /// Completing the step would make the run retain more output than allowed.
    #[must_use]
    pub fn run_data_limit() -> Self {
        Self::system("RUN_DATA_LIMIT")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FailureCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The value a `waitEvent` node correlates on (the evaluated correlation expression).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CorrelationKey(String);

impl CorrelationKey {
    /// # Errors
    /// [`ExecutionError::InvalidCorrelationKey`] for an empty, long or control-character value.
    pub fn parse(value: &str) -> Result<Self, ExecutionError> {
        if value.is_empty() || value.len() > MAX_KEY_LEN || value.chars().any(char::is_control) {
            Err(ExecutionError::InvalidCorrelationKey)
        } else {
            Ok(Self(value.to_owned()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a run is being wound down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Cancelled,
    TimedOut,
    Failed,
    /// A failure that a saga compensated completely: the run ends `COMPENSATED`.
    Compensated,
    /// A failure that a saga could only compensate in part: the run ends `PARTIAL`.
    Partial,
    /// A failure whose compensation failed for good: the run ends `COMPENSATION_FAILED`.
    CompensationFailed,
}

impl StopReason {
    /// The terminal status the run reaches once nothing is in flight any more, before the outcome of
    /// the sagas that ran while it was stopping is taken into account
    /// ([`RunState::final_status_for`](super::state::RunState::final_status_for)).
    #[must_use]
    pub const fn final_status(self) -> RunStatus {
        match self {
            Self::Cancelled => RunStatus::Cancelled,
            Self::TimedOut => RunStatus::TimedOut,
            Self::Failed => RunStatus::Failed,
            Self::Compensated => RunStatus::Compensated,
            Self::Partial => RunStatus::Partial,
            Self::CompensationFailed => RunStatus::CompensationFailed,
        }
    }

    /// The stop reason of a failure that a saga ended with `outcome`.
    #[must_use]
    pub const fn of_outcome(outcome: SagaOutcome) -> Self {
        match outcome {
            SagaOutcome::Compensated => Self::Compensated,
            SagaOutcome::Partial => Self::Partial,
            SagaOutcome::CompensationFailed => Self::CompensationFailed,
        }
    }

    /// The name stored and shown.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "CANCELLED",
            Self::TimedOut => "TIMED_OUT",
            Self::Failed => "FAILED",
            Self::Compensated => "COMPENSATED",
            Self::Partial => "PARTIAL",
            Self::CompensationFailed => "COMPENSATION_FAILED",
        }
    }
}

impl std::str::FromStr for StopReason {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "CANCELLED" => Ok(Self::Cancelled),
            "TIMED_OUT" => Ok(Self::TimedOut),
            "FAILED" => Ok(Self::Failed),
            "COMPENSATED" => Ok(Self::Compensated),
            "PARTIAL" => Ok(Self::Partial),
            "COMPENSATION_FAILED" => Ok(Self::CompensationFailed),
            _ => Err(()),
        }
    }
}

/// One compensation of a saga's plan as the run records it when the saga starts to compensate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEntry {
    /// The forward node whose effect is being undone; its compensation is `node.compensation()`.
    pub node: NodeId,
    /// The forward nodes whose compensations must have finished first.
    pub after: Vec<NodeId>,
}

/// What a reconciliation led to for the step it was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileResult {
    /// The effect happened: the step succeeded.
    Succeeded,
    /// The effect did not happen and the retry budget allows another attempt, not before the time.
    RetryAt(OffsetDateTime),
    /// The effect did not happen and the retry budget is spent: the step failed.
    Failed(FailureCode),
    /// Nobody can say: an operator decides.
    Review(FailureCode),
}

/// How an operator resolved a step that waited for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    Succeeded,
    Failed(FailureCode),
}

/// How a `parallel` node ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Succeeded,
    Failed,
}

/// The one recorded branch chosen by an ordered switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchSelection {
    Case(NodeId),
    Default,
}

/// How a child run ended, as the step of the parent that waited for it sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildOutcome {
    Succeeded,
    /// The child did not succeed (or could not be started); the code says why.
    Failed(FailureCode),
}

/// Why a collection stops admitting new iterations and drains the ones already accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionDrainReason {
    /// One admitted iteration failed permanently.
    Failed { iteration: u32, code: FailureCode },
    /// The enclosing run asked the collection to stop.
    Cancelled,
}

/// The terminal result of a collection scope after all accepted iterations have settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionOutcome {
    Succeeded,
    Failed(FailureCode),
    Cancelled,
}

/// Why an event happened: a closed `kind` (a failure, a retry, a review, a stop, an operator's
/// decision, a request, the outcome of the run) and the `code` that goes with it. What a timeline
/// shows next to a transition (US01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cause {
    pub kind: &'static str,
    pub code: String,
}

/// Names one attempt of one step. A completion or failure is only valid for the attempt and
/// generation that currently holds the lease (fencing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fence {
    pub attempt: u32,
    pub generation: u64,
}

/// One entry of the run's journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    RunStarted {
        entrypoint: NodeId,
    },
    /// The successors of a finished step were chosen and now exist as pending steps.
    StepRouted {
        node: NodeId,
        activated: Vec<NodeId>,
    },
    /// An activity is ready for a worker.
    StepReleased {
        node: NodeId,
    },
    StepLeased {
        node: NodeId,
        attempt: u32,
        generation: u64,
    },
    /// The worker confirmed it started (first heartbeat).
    StepStarted {
        node: NodeId,
        attempt: u32,
        generation: u64,
    },
    StepCompleted {
        node: NodeId,
        attempt: u32,
        generation: u64,
    },
    /// An attempt failed and the retry budget allows another one.
    StepRetryScheduled {
        node: NodeId,
        attempt: u32,
        generation: u64,
        code: FailureCode,
        not_before: OffsetDateTime,
    },
    StepRetryDue {
        node: NodeId,
    },
    /// The step failed for good. `fence` is present when an attempt held the lease.
    StepFailed {
        node: NodeId,
        code: FailureCode,
        fence: Option<Fence>,
    },
    ConditionEvaluated {
        node: NodeId,
        outcome: bool,
    },
    SwitchEvaluated {
        node: NodeId,
        selection: SwitchSelection,
    },
    TimerStarted {
        node: NodeId,
        due_at: OffsetDateTime,
    },
    TimerFired {
        node: NodeId,
    },
    WaitStarted {
        node: NodeId,
        correlation_key: CorrelationKey,
        deadline: OffsetDateTime,
    },
    WaitSatisfied {
        node: NodeId,
    },
    /// The wait's deadline passed. `handled` means the node defines an `onTimeout` route.
    WaitTimedOut {
        node: NodeId,
        handled: bool,
    },
    ParallelForked {
        node: NodeId,
        branches: Vec<NodeId>,
    },
    ParallelJoined {
        node: NodeId,
        outcome: JoinOutcome,
    },
    /// Captures immutable collection membership and its effective scope limits before admission.
    CollectionEntered {
        node: NodeId,
        item_count: u32,
        max_items: u32,
        max_concurrency: u16,
        snapshot_digest: Digest,
    },
    /// Allocates the first body instance for exactly one collection index.
    CollectionIterationAdmitted {
        node: NodeId,
        index: u32,
        entry: ExecutionInstanceId,
        item_digest: Digest,
    },
    /// Records the explicit body return instance for an admitted index.
    CollectionIterationReturned {
        node: NodeId,
        index: u32,
        return_instance: ExecutionInstanceId,
    },
    /// Makes one returned result durable and releases its collection concurrency slot.
    CollectionIterationCheckpointed {
        node: NodeId,
        index: u32,
        return_instance: ExecutionInstanceId,
        result_digest: Digest,
    },
    /// Stops new admission while accepted iterations settle.
    CollectionDrainStarted {
        node: NodeId,
        reason: CollectionDrainReason,
    },
    /// Records that one accepted iteration ended without a result during drain.
    CollectionIterationDrained {
        node: NodeId,
        index: u32,
    },
    /// Closes the scope only after every required result or drain record exists.
    CollectionFinished {
        node: NodeId,
        outcome: CollectionOutcome,
    },
    /// A child run was reserved for the step: its id is journaled before the child exists, so a
    /// crash between the two is repaired by starting exactly that run (`FR-002-009`).
    ChildStarted {
        node: NodeId,
        child: RunId,
    },
    /// The child run reached a terminal state (or could not be started).
    ChildFinished {
        node: NodeId,
        child: RunId,
        outcome: ChildOutcome,
    },
    EndReached {
        node: NodeId,
    },
    /// The worker holding the lease must stop (heartbeats report it).
    StepCancelRequested {
        node: NodeId,
    },
    StepCancelled {
        node: NodeId,
    },
    /// A `saga` node started: its body begins at `begin`.
    SagaEntered {
        node: NodeId,
        begin: NodeId,
    },
    /// The body of the saga finished without an unhandled failure.
    SagaCompleted {
        node: NodeId,
    },
    /// Something inside the saga failed for good, or the run is being stopped: the saga's forward
    /// work is wound down so its completed steps can be compensated.
    SagaAborting {
        node: NodeId,
        cause: FailureCode,
    },
    /// The forward work is over: the compensations to run, in the dependencies they wait for, and
    /// the completed steps whose effect cannot be undone.
    CompensationPlanned {
        node: NodeId,
        entries: Vec<PlannedEntry>,
        irreversible: Vec<NodeId>,
    },
    /// Every compensation of the saga has finished (or been given up on): how it ended.
    SagaCompensated {
        node: NodeId,
        outcome: SagaOutcome,
    },
    /// An attempt ended without a known outcome and the step says how to find it out: its
    /// reconciliation is scheduled and the step waits for the answer.
    ReconciliationRequested {
        node: NodeId,
        fence: Fence,
        cause: FailureCode,
    },
    /// The reconciliation answered, and what that means for the step.
    ReconciliationConcluded {
        node: NodeId,
        verdict: ReconcileVerdict,
        result: ReconcileResult,
    },
    /// The step cannot go on by itself and waits for an operator. `fence` is present when an
    /// attempt held the lease.
    StepNeedsReview {
        node: NodeId,
        code: FailureCode,
        fence: Option<Fence>,
    },
    /// An operator asked for another attempt of a step that waited for a decision.
    StepRetryRequested {
        node: NodeId,
    },
    /// An operator gave up on a compensation, with a justification (kept in the audit trail).
    StepSkipped {
        node: NodeId,
    },
    /// An operator settled a step that waited for a decision.
    StepResolved {
        node: NodeId,
        outcome: ReviewOutcome,
    },
    RunPaused,
    RunResumed,
    RunStopRequested {
        reason: StopReason,
        code: Option<FailureCode>,
    },
    RunFinished {
        status: RunStatus,
    },
}

/// A journal event together with the scope path that addresses its logical execution instance.
///
/// Existing journals use [`Self::root`]. A scoped address is only valid for a step event; run-level
/// events remain properties of the run and therefore always use the empty path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressedRunEvent {
    path: InstancePath,
    event: RunEvent,
}

/// Why a journal event cannot use the requested scope path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventAddressError {
    #[error("a run-level event must use the root path")]
    RunEventInScope,
}

impl AddressedRunEvent {
    #[must_use]
    pub fn root(event: RunEvent) -> Self {
        Self {
            path: InstancePath::root(),
            event,
        }
    }

    /// Addresses a step event in one scope path.
    ///
    /// # Errors
    /// Returns [`EventAddressError::RunEventInScope`] when a run-level event is assigned to a
    /// non-root path.
    pub fn in_path(path: InstancePath, event: RunEvent) -> Result<Self, EventAddressError> {
        if !path.is_root() && event.node().is_none() {
            return Err(EventAddressError::RunEventInScope);
        }
        Ok(Self { path, event })
    }

    #[must_use]
    pub const fn path(&self) -> &InstancePath {
        &self.path
    }

    #[must_use]
    pub const fn event(&self) -> &RunEvent {
        &self.event
    }

    #[must_use]
    pub fn into_event(self) -> RunEvent {
        self.event
    }

    #[must_use]
    pub fn instance(&self) -> Option<ExecutionInstanceId> {
        self.event
            .node()
            .cloned()
            .map(|node| ExecutionInstanceId::in_path(self.path.clone(), node))
    }

    /// Every instance created or changed by the event. Child activations inherit the addressed
    /// path; opening a new dynamic iteration remains the responsibility of its scope event.
    #[must_use]
    pub fn changed_instances(&self) -> Vec<ExecutionInstanceId> {
        let mut instances = self
            .event
            .changed_nodes()
            .into_iter()
            .map(|node| ExecutionInstanceId::in_path(self.path.clone(), node))
            .collect::<Vec<_>>();
        match &self.event {
            RunEvent::CollectionIterationAdmitted { entry, .. } => {
                instances.push(entry.clone());
            }
            RunEvent::CollectionIterationReturned {
                return_instance, ..
            }
            | RunEvent::CollectionIterationCheckpointed {
                return_instance, ..
            } => {
                instances.push(return_instance.clone());
            }
            _ => {}
        }
        instances
    }
}

impl RunEvent {
    /// Stable dotted name stored in the journal (`event_type`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run.started",
            Self::StepRouted { .. } => "step.routed",
            Self::StepReleased { .. } => "step.released",
            Self::StepLeased { .. } => "step.leased",
            Self::StepStarted { .. } => "step.started",
            Self::StepCompleted { .. } => "step.completed",
            Self::StepRetryScheduled { .. } => "step.retry_scheduled",
            Self::StepRetryDue { .. } => "step.retry_due",
            Self::StepFailed { .. } => "step.failed",
            Self::ConditionEvaluated { .. } => "condition.evaluated",
            Self::SwitchEvaluated { .. } => "switch.evaluated",
            Self::TimerStarted { .. } => "timer.started",
            Self::TimerFired { .. } => "timer.fired",
            Self::WaitStarted { .. } => "wait.started",
            Self::WaitSatisfied { .. } => "wait.satisfied",
            Self::WaitTimedOut { .. } => "wait.timed_out",
            Self::ParallelForked { .. } => "parallel.forked",
            Self::ParallelJoined { .. } => "parallel.joined",
            Self::CollectionEntered { .. } => "collection.entered",
            Self::CollectionIterationAdmitted { .. } => "collection.iteration_admitted",
            Self::CollectionIterationReturned { .. } => "collection.iteration_returned",
            Self::CollectionIterationCheckpointed { .. } => "collection.iteration_checkpointed",
            Self::CollectionDrainStarted { .. } => "collection.drain_started",
            Self::CollectionIterationDrained { .. } => "collection.iteration_drained",
            Self::CollectionFinished { .. } => "collection.finished",
            Self::ChildStarted { .. } => "child.started",
            Self::ChildFinished { .. } => "child.finished",
            Self::EndReached { .. } => "end.reached",
            Self::StepCancelRequested { .. } => "step.cancel_requested",
            Self::StepCancelled { .. } => "step.cancelled",
            Self::SagaEntered { .. } => "saga.entered",
            Self::SagaCompleted { .. } => "saga.completed",
            Self::SagaAborting { .. } => "saga.aborting",
            Self::CompensationPlanned { .. } => "compensation.planned",
            Self::SagaCompensated { .. } => "saga.compensated",
            Self::ReconciliationRequested { .. } => "reconciliation.requested",
            Self::ReconciliationConcluded { .. } => "reconciliation.concluded",
            Self::StepNeedsReview { .. } => "step.needs_review",
            Self::StepRetryRequested { .. } => "step.retry_requested",
            Self::StepSkipped { .. } => "step.skipped",
            Self::StepResolved { .. } => "step.resolved",
            Self::RunPaused => "run.paused",
            Self::RunResumed => "run.resumed",
            Self::RunStopRequested { .. } => "run.stop_requested",
            Self::RunFinished { .. } => "run.finished",
        }
    }

    /// The step this event is about, if any.
    #[must_use]
    pub const fn node(&self) -> Option<&NodeId> {
        match self {
            Self::StepRouted { node, .. }
            | Self::StepReleased { node }
            | Self::StepLeased { node, .. }
            | Self::StepStarted { node, .. }
            | Self::StepCompleted { node, .. }
            | Self::StepRetryScheduled { node, .. }
            | Self::StepRetryDue { node }
            | Self::StepFailed { node, .. }
            | Self::ConditionEvaluated { node, .. }
            | Self::SwitchEvaluated { node, .. }
            | Self::TimerStarted { node, .. }
            | Self::TimerFired { node }
            | Self::WaitStarted { node, .. }
            | Self::WaitSatisfied { node }
            | Self::WaitTimedOut { node, .. }
            | Self::ParallelForked { node, .. }
            | Self::ParallelJoined { node, .. }
            | Self::CollectionEntered { node, .. }
            | Self::CollectionIterationAdmitted { node, .. }
            | Self::CollectionIterationReturned { node, .. }
            | Self::CollectionIterationCheckpointed { node, .. }
            | Self::CollectionDrainStarted { node, .. }
            | Self::CollectionIterationDrained { node, .. }
            | Self::CollectionFinished { node, .. }
            | Self::ChildStarted { node, .. }
            | Self::ChildFinished { node, .. }
            | Self::EndReached { node }
            | Self::StepCancelRequested { node }
            | Self::StepCancelled { node }
            | Self::SagaEntered { node, .. }
            | Self::SagaCompleted { node }
            | Self::SagaAborting { node, .. }
            | Self::CompensationPlanned { node, .. }
            | Self::SagaCompensated { node, .. }
            | Self::ReconciliationRequested { node, .. }
            | Self::ReconciliationConcluded { node, .. }
            | Self::StepNeedsReview { node, .. }
            | Self::StepRetryRequested { node }
            | Self::StepSkipped { node }
            | Self::StepResolved { node, .. } => Some(node),
            Self::RunStarted { .. }
            | Self::RunPaused
            | Self::RunResumed
            | Self::RunStopRequested { .. }
            | Self::RunFinished { .. } => None,
        }
    }

    /// Every step this event creates or changes: the one it names and the ones it activates (the
    /// next steps of a routing, the branches of a fork, the first step of a saga, the undo of each
    /// planned entry, the lookup of a step that may have happened). The runtime graph a client holds
    /// stays equal to the platform's only if a change is reported for each of them (D141).
    #[must_use]
    pub fn changed_nodes(&self) -> Vec<NodeId> {
        let mut nodes: Vec<NodeId> = self.node().cloned().into_iter().collect();
        match self {
            Self::RunStarted { entrypoint } => nodes.push(entrypoint.clone()),
            Self::StepRouted { activated, .. } => nodes.extend(activated.iter().cloned()),
            Self::ParallelForked { branches, .. } => nodes.extend(branches.iter().cloned()),
            Self::SagaEntered { begin, .. } => nodes.push(begin.clone()),
            Self::CompensationPlanned { entries, .. } => {
                nodes.extend(entries.iter().map(|entry| entry.node.compensation()));
            }
            Self::ReconciliationRequested { node, .. } => nodes.push(node.reconciliation()),
            _ => {}
        }
        nodes
    }

    /// Why this happened, when the event is the result of something going wrong, an operator's
    /// choice or the clock: the cause a timeline shows next to it.
    #[must_use]
    pub fn cause(&self) -> Option<Cause> {
        let cause = |kind: &'static str, code: &str| {
            Some(Cause {
                kind,
                code: code.to_owned(),
            })
        };
        match self {
            Self::StepFailed { code, .. } => cause("failure", code.as_str()),
            Self::StepRetryScheduled { code, .. } => cause("retry", code.as_str()),
            Self::StepNeedsReview { code, .. } => cause("review", code.as_str()),
            Self::SagaAborting { cause: code, .. } => cause("abort", code.as_str()),
            Self::ReconciliationRequested { cause: code, .. } => {
                cause("unknown_outcome", code.as_str())
            }
            Self::ReconciliationConcluded { verdict, .. } => {
                cause("reconciliation", verdict.as_str())
            }
            Self::RunStopRequested { reason, code } => cause(
                "stop",
                code.as_ref()
                    .map_or_else(|| reason.as_str(), FailureCode::as_str),
            ),
            Self::WaitTimedOut { .. } => cause("timeout", "WAIT_TIMED_OUT"),
            Self::StepCancelRequested { .. } => cause("cancel", "REQUESTED"),
            Self::StepRetryRequested { .. } => cause("decision", "RETRY"),
            Self::StepSkipped { .. } => cause("decision", "SKIP"),
            Self::StepResolved { outcome, .. } => match outcome {
                ReviewOutcome::Succeeded => cause("decision", "SUCCEEDED"),
                ReviewOutcome::Failed(code) => cause("decision", code.as_str()),
            },
            Self::ChildFinished {
                outcome: ChildOutcome::Failed(code),
                ..
            } => cause("failure", code.as_str()),
            Self::ParallelJoined {
                outcome: JoinOutcome::Failed,
                ..
            } => cause("failure", "BRANCH_FAILED"),
            Self::CollectionDrainStarted {
                reason: CollectionDrainReason::Failed { code, .. },
                ..
            }
            | Self::CollectionFinished {
                outcome: CollectionOutcome::Failed(code),
                ..
            } => cause("failure", code.as_str()),
            Self::CollectionDrainStarted {
                reason: CollectionDrainReason::Cancelled,
                ..
            }
            | Self::CollectionFinished {
                outcome: CollectionOutcome::Cancelled,
                ..
            } => cause("cancel", "REQUESTED"),
            Self::RunPaused => cause("request", "PAUSE"),
            Self::RunResumed => cause("request", "RESUME"),
            Self::RunFinished { status } => cause("outcome", status.as_str()),
            _ => None,
        }
    }

    /// The attempt number this event refers to, if it is attempt-bound.
    #[must_use]
    pub const fn attempt(&self) -> Option<u32> {
        match self {
            Self::StepLeased { attempt, .. }
            | Self::StepStarted { attempt, .. }
            | Self::StepCompleted { attempt, .. }
            | Self::StepRetryScheduled { attempt, .. } => Some(*attempt),
            Self::StepFailed {
                fence: Some(fence), ..
            }
            | Self::ReconciliationRequested { fence, .. }
            | Self::StepNeedsReview {
                fence: Some(fence), ..
            } => Some(fence.attempt),
            _ => None,
        }
    }
}
