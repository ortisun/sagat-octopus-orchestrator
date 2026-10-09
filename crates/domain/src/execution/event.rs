//! The journal vocabulary: every change to a run is one [`RunEvent`].

use std::fmt;

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::state::{RunId, RunStatus};
use crate::identity::clip;
use crate::workflow::NodeId;

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
}

impl StopReason {
    /// The terminal status the run reaches once nothing is in flight any more.
    #[must_use]
    pub const fn final_status(self) -> RunStatus {
        match self {
            Self::Cancelled => RunStatus::Cancelled,
            Self::TimedOut => RunStatus::TimedOut,
            Self::Failed => RunStatus::Failed,
        }
    }
}

/// How a `parallel` node ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Succeeded,
    Failed,
}

/// How a child run ended, as the step of the parent that waited for it sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildOutcome {
    Succeeded,
    /// The child did not succeed (or could not be started); the code says why.
    Failed(FailureCode),
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
            Self::TimerStarted { .. } => "timer.started",
            Self::TimerFired { .. } => "timer.fired",
            Self::WaitStarted { .. } => "wait.started",
            Self::WaitSatisfied { .. } => "wait.satisfied",
            Self::WaitTimedOut { .. } => "wait.timed_out",
            Self::ParallelForked { .. } => "parallel.forked",
            Self::ParallelJoined { .. } => "parallel.joined",
            Self::ChildStarted { .. } => "child.started",
            Self::ChildFinished { .. } => "child.finished",
            Self::EndReached { .. } => "end.reached",
            Self::StepCancelRequested { .. } => "step.cancel_requested",
            Self::StepCancelled { .. } => "step.cancelled",
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
            | Self::TimerStarted { node, .. }
            | Self::TimerFired { node }
            | Self::WaitStarted { node, .. }
            | Self::WaitSatisfied { node }
            | Self::WaitTimedOut { node, .. }
            | Self::ParallelForked { node, .. }
            | Self::ParallelJoined { node, .. }
            | Self::ChildStarted { node, .. }
            | Self::ChildFinished { node, .. }
            | Self::EndReached { node }
            | Self::StepCancelRequested { node }
            | Self::StepCancelled { node } => Some(node),
            Self::RunStarted { .. }
            | Self::RunPaused
            | Self::RunResumed
            | Self::RunStopRequested { .. }
            | Self::RunFinished { .. } => None,
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
            } => Some(fence.attempt),
            _ => None,
        }
    }
}
