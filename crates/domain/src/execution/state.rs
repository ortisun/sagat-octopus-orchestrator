//! The state of one run and the reducer that folds journal events into it.
//!
//! [`RunState::apply`] is the only way state changes. It needs no workflow definition, so a run can
//! be rebuilt from its journal alone ([`RunState::replay`]) and the stored projection can be checked
//! against that rebuild. Every event is validated before anything is mutated: a rejected event
//! leaves the state exactly as it was.

#[path = "state_effects.rs"]
pub(super) mod effects;

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use time::OffsetDateTime;

use super::continuation::{
    ContinuationTransfer, MAX_CHAIN_SEGMENTS, SegmentCompletionKind, SegmentIndex,
};
use super::error::ExecutionError;
use super::event::{
    AddressedRunEvent, ChildOutcome, CollectionDrainReason, CollectionOutcome, CorrelationKey,
    FailureCode, JoinOutcome, LoopContinuation, LoopDrainReason, LoopKind, LoopOutcome,
    ReconcileResult, ReviewOutcome, RunEvent, StopReason, SwitchSelection,
};
use super::instance::{ExecutionInstanceId, InstancePath, MAX_ITERATION_INDEX};
use crate::identity::IdentityError;
use crate::saga::{ReconcileVerdict, SagaOutcome};
use crate::workflow::{Digest, NodeId};

uuid_id!(
    /// Identifier of a run.
    RunId
);

/// Status of a run (`docs/architecture/execution-semantics.md`). `CREATED` and `READY` are never
/// persisted: a run is created and first scheduled in one transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    Running,
    /// Nothing is runnable; every active step waits for a timer or an event.
    Waiting,
    Paused,
    /// Winding down: pending work is cancelled and in-flight attempts are being told to stop.
    Cancelling,
    /// A saga is undoing the effects of the steps that completed (feature 003).
    Compensating,
    /// Nothing can move until an operator decides: a compensation or an unknown outcome is waiting.
    ManualReview,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    /// A failure was compensated completely.
    Compensated,
    /// A failure was compensated, but not everything could be undone (an irreversible effect, or a
    /// compensation an operator skipped).
    Partial,
    /// A compensation failed for good.
    CompensationFailed,
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    use crate::workflow::NodeId;

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("node")
    }

    #[test]
    fn continuation_closes_a_quiescent_segment_and_links_the_successor() {
        let predecessor = RunId::generate();
        let successor = RunId::generate();
        let entry = node("work");
        let deadline = OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1);
        let input = Digest::new([1; 32]);
        let state = Digest::new([2; 32]);
        let predecessor_events = [
            RunEvent::RunStarted {
                entrypoint: entry.clone(),
            },
            RunEvent::ExecutionChainStarted {
                max_segments: 3,
                original_deadline: deadline,
            },
            RunEvent::StepReleased {
                node: entry.clone(),
            },
            RunEvent::StepLeased {
                node: entry.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepStarted {
                node: entry.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepCompleted {
                node: entry.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepRouted {
                node: entry.clone(),
                activated: Vec::new(),
            },
            RunEvent::RunContinued {
                successor,
                input_digest: input,
                state_digest: Some(state),
            },
        ];
        let old = RunState::replay(predecessor, &predecessor_events).expect("predecessor replay");
        assert_eq!(old.status(), RunStatus::Succeeded);
        let old_chain = old.chain().expect("chain");
        assert_eq!(old_chain.chain_id(), predecessor);
        assert_eq!(old_chain.successor(), Some(successor));
        assert_eq!(
            old_chain.completion_kind(),
            Some(SegmentCompletionKind::Continued)
        );
        assert_eq!(
            old_chain.transfer(),
            Some(&ContinuationTransfer::new(input, Some(state)))
        );

        let successor_events = [
            RunEvent::RunStarted { entrypoint: entry },
            RunEvent::ExecutionChainSuccessorLinked {
                chain_id: predecessor,
                predecessor,
                segment_index: 1,
                max_segments: 3,
                original_deadline: deadline,
                input_digest: input,
                state_digest: Some(state),
            },
        ];
        let new = RunState::replay(successor, &successor_events).expect("successor replay");
        let new_chain = new.chain().expect("successor chain");
        assert_eq!(new_chain.chain_id(), predecessor);
        assert_eq!(new_chain.predecessor(), Some(predecessor));
        assert_eq!(new_chain.segment_index().get(), 1);
        assert_eq!(new_chain.original_deadline(), deadline);
    }

    #[test]
    fn continuation_refuses_an_active_step() {
        let run = RunId::generate();
        let entry = node("work");
        let mut state = RunState::new(run);
        state
            .apply(&RunEvent::RunStarted { entrypoint: entry })
            .expect("start");
        state
            .apply(&RunEvent::ExecutionChainStarted {
                max_segments: 2,
                original_deadline: OffsetDateTime::UNIX_EPOCH,
            })
            .expect("chain");
        let before = state.clone();
        assert!(
            state
                .apply(&RunEvent::RunContinued {
                    successor: RunId::generate(),
                    input_digest: Digest::new([3; 32]),
                    state_digest: None,
                })
                .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn typed_continuation_closes_its_pending_boundary() {
        let predecessor = RunId::generate();
        let successor = RunId::generate();
        let work = node("work");
        let boundary = node("rollover");
        let input = Digest::new([7; 32]);
        let events = [
            RunEvent::RunStarted {
                entrypoint: work.clone(),
            },
            RunEvent::ExecutionChainStarted {
                max_segments: 3,
                original_deadline: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
            },
            RunEvent::StepReleased { node: work.clone() },
            RunEvent::StepLeased {
                node: work.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepStarted {
                node: work.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepCompleted {
                node: work.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepRouted {
                node: work,
                activated: vec![boundary.clone()],
            },
            RunEvent::ContinuationSelected {
                node: boundary.clone(),
                successor,
                input_digest: input,
                state_digest: None,
            },
        ];

        let state = RunState::replay(predecessor, &events).expect("typed continuation replay");
        assert_eq!(state.status(), RunStatus::Succeeded);
        let step = state.step(&boundary).expect("continuation boundary");
        assert_eq!(step.status(), StepStatus::Succeeded);
        assert!(step.routed());
        let chain = state.chain().expect("chain");
        assert_eq!(chain.successor(), Some(successor));
        assert_eq!(
            chain.transfer(),
            Some(&ContinuationTransfer::new(input, None))
        );
    }

    #[test]
    fn typed_continuation_refuses_an_unsettled_sibling_without_mutation() {
        let run = RunId::generate();
        let work = node("work");
        let boundary = node("rollover");
        let blocker = node("still-open");
        let mut state = RunState::replay(
            run,
            &[
                RunEvent::RunStarted {
                    entrypoint: work.clone(),
                },
                RunEvent::ExecutionChainStarted {
                    max_segments: 3,
                    original_deadline: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
                },
                RunEvent::StepReleased { node: work.clone() },
                RunEvent::StepLeased {
                    node: work.clone(),
                    attempt: 1,
                    generation: 1,
                },
                RunEvent::StepStarted {
                    node: work.clone(),
                    attempt: 1,
                    generation: 1,
                },
                RunEvent::StepCompleted {
                    node: work.clone(),
                    attempt: 1,
                    generation: 1,
                },
                RunEvent::StepRouted {
                    node: work,
                    activated: vec![boundary.clone(), blocker],
                },
            ],
        )
        .expect("pending continuation and sibling");
        let before = state.clone();
        assert!(
            state
                .apply(&RunEvent::ContinuationSelected {
                    node: boundary,
                    successor: RunId::generate(),
                    input_digest: Digest::new([9; 32]),
                    state_digest: None,
                })
                .is_err()
        );
        assert_eq!(state, before);
    }
}

impl RunStatus {
    /// Every status, in the order of the enumeration.
    pub const ALL: [Self; 13] = [
        Self::Running,
        Self::Waiting,
        Self::Paused,
        Self::Cancelling,
        Self::Compensating,
        Self::ManualReview,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
        Self::TimedOut,
        Self::Compensated,
        Self::Partial,
        Self::CompensationFailed,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING",
            Self::Paused => "PAUSED",
            Self::Cancelling => "CANCELLING",
            Self::Compensating => "COMPENSATING",
            Self::ManualReview => "MANUAL_REVIEW",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::TimedOut => "TIMED_OUT",
            Self::Compensated => "COMPENSATED",
            Self::Partial => "PARTIAL",
            Self::CompensationFailed => "COMPENSATION_FAILED",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Compensated
                | Self::Partial
                | Self::CompensationFailed
        )
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RunStatus {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "RUNNING" => Ok(Self::Running),
            "WAITING" => Ok(Self::Waiting),
            "PAUSED" => Ok(Self::Paused),
            "CANCELLING" => Ok(Self::Cancelling),
            "COMPENSATING" => Ok(Self::Compensating),
            "MANUAL_REVIEW" => Ok(Self::ManualReview),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "TIMED_OUT" => Ok(Self::TimedOut),
            "COMPENSATED" => Ok(Self::Compensated),
            "PARTIAL" => Ok(Self::Partial),
            "COMPENSATION_FAILED" => Ok(Self::CompensationFailed),
            _ => Err(IdentityError::MalformedId),
        }
    }
}

/// Run-wide work facts. A bounded path retains only the facts about other paths, so local
/// transitions can derive the same global status without reading their individual steps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunWorkSummary {
    active: bool,
    live: bool,
    waiting: bool,
    reviewing: bool,
}

impl RunWorkSummary {
    /// Restores trusted existential facts from one consistent storage snapshot.
    ///
    /// # Errors
    /// Live work must be active, and a review must be waiting.
    pub fn restore(
        active: bool,
        live: bool,
        waiting: bool,
        reviewing: bool,
    ) -> Result<Self, ExecutionError> {
        if (live && !active) || (reviewing && !waiting) {
            return Err(ExecutionError::Invariant("inconsistent run work summary"));
        }
        Ok(Self {
            active,
            live,
            waiting,
            reviewing,
        })
    }

    pub(super) fn from_steps<'a>(steps: impl Iterator<Item = &'a StepState>) -> Self {
        let mut summary = Self::default();
        for step in steps {
            let live = matches!(
                step.status,
                StepStatus::Ready
                    | StepStatus::Leased
                    | StepStatus::Running
                    | StepStatus::RetryWait
            );
            summary.active |= live || step.status == StepStatus::Pending;
            summary.live |= live;
            summary.waiting |= step.status == StepStatus::Waiting;
            summary.reviewing |= step.status == StepStatus::Waiting
                && matches!(step.waiting, Some(WaitingOn::Operator { .. }));
        }
        summary
    }

    pub(super) fn union(self, other: Self) -> Self {
        Self {
            active: self.active || other.active,
            live: self.live || other.live,
            waiting: self.waiting || other.waiting,
            reviewing: self.reviewing || other.reviewing,
        }
    }

    pub(super) fn status(self, compensating: bool, stopping: bool, paused: bool) -> RunStatus {
        if self.reviewing && !self.live {
            RunStatus::ManualReview
        } else if compensating {
            RunStatus::Compensating
        } else if stopping {
            RunStatus::Cancelling
        } else if paused {
            RunStatus::Paused
        } else if !self.active && self.waiting {
            RunStatus::Waiting
        } else {
            RunStatus::Running
        }
    }
}

/// Status of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepStatus {
    /// Created by routing; the planner decides what it needs next.
    Pending,
    /// An activity awaiting a worker.
    Ready,
    Leased,
    Running,
    /// Parked on a timer, an event or the branches of a parallel node; holds no process.
    Waiting,
    /// A failed attempt is backing off until `next_attempt_at`.
    RetryWait,
    Succeeded,
    Failed,
    Cancelled,
    /// A compensation an operator gave up on.
    Skipped,
}

impl StepStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Ready => "READY",
            Self::Leased => "LEASED",
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING",
            Self::RetryWait => "RETRY_WAIT",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Skipped => "SKIPPED",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }

    /// A worker holds the lease.
    #[must_use]
    pub const fn is_leased(self) -> bool {
        matches!(self, Self::Leased | Self::Running)
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StepStatus {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PENDING" => Ok(Self::Pending),
            "READY" => Ok(Self::Ready),
            "LEASED" => Ok(Self::Leased),
            "RUNNING" => Ok(Self::Running),
            "WAITING" => Ok(Self::Waiting),
            "RETRY_WAIT" => Ok(Self::RetryWait),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "SKIPPED" => Ok(Self::Skipped),
            _ => Err(IdentityError::MalformedId),
        }
    }
}

/// What a `Waiting` step is parked on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitingOn {
    Timer {
        due_at: OffsetDateTime,
    },
    Event {
        correlation_key: CorrelationKey,
        deadline: OffsetDateTime,
    },
    Approval {
        request_id: super::ApprovalRequestId,
    },
    Branches,
    /// A bounded dynamic collection whose accepted iterations are tracked separately.
    Collection,
    /// A sequential loop whose current iteration or delay is tracked separately.
    Loop,
    /// A child run, reserved by `child.started`; it may not exist yet.
    Child {
        run: RunId,
    },
    /// The body of a `saga` node.
    Saga,
    /// The reconciliation of this step, which asks the provider what happened to an attempt whose
    /// outcome is unknown.
    Reconciliation,
    /// An operator: the step cannot go on by itself, for the reason in `code`.
    Operator {
        code: FailureCode,
    },
}

/// Durable lifecycle of one collection scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionPhase {
    Open,
    Draining,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CollectionIterationState {
    Admitted,
    Returned {
        return_instance: ExecutionInstanceId,
    },
    Checkpointed,
    Drained,
}

/// Durable reference to one body result; the value itself remains in bounded run data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionResult {
    return_instance: ExecutionInstanceId,
    digest: Digest,
}

/// One collection iteration reconstructed from durable projection rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionIterationRestore {
    index: u32,
    entry: ExecutionInstanceId,
    item_digest: Digest,
    returned: Option<(ExecutionInstanceId, Option<Digest>)>,
    drained: bool,
}

impl CollectionIterationRestore {
    #[must_use]
    pub const fn admitted(index: u32, entry: ExecutionInstanceId, item_digest: Digest) -> Self {
        Self {
            index,
            entry,
            item_digest,
            returned: None,
            drained: false,
        }
    }

    #[must_use]
    pub fn returned(
        index: u32,
        entry: ExecutionInstanceId,
        item_digest: Digest,
        return_instance: ExecutionInstanceId,
    ) -> Self {
        Self {
            index,
            entry,
            item_digest,
            returned: Some((return_instance, None)),
            drained: false,
        }
    }

    #[must_use]
    pub fn checkpointed(
        index: u32,
        entry: ExecutionInstanceId,
        item_digest: Digest,
        return_instance: ExecutionInstanceId,
        result_digest: Digest,
    ) -> Self {
        Self {
            index,
            entry,
            item_digest,
            returned: Some((return_instance, Some(result_digest))),
            drained: false,
        }
    }

    #[must_use]
    pub const fn drained(index: u32, entry: ExecutionInstanceId, item_digest: Digest) -> Self {
        Self {
            index,
            entry,
            item_digest,
            returned: None,
            drained: true,
        }
    }

    fn is_active(&self) -> bool {
        !self.drained
            && self
                .returned
                .as_ref()
                .is_none_or(|(_, result_digest)| result_digest.is_none())
    }
}

impl CollectionResult {
    #[must_use]
    pub const fn return_instance(&self) -> &ExecutionInstanceId {
        &self.return_instance
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}

impl CollectionIterationState {
    const fn is_active(&self) -> bool {
        matches!(self, Self::Admitted { .. } | Self::Returned { .. })
    }

    const fn is_settled(&self) -> bool {
        matches!(self, Self::Checkpointed { .. } | Self::Drained)
    }
}

/// Journal-derived progress for one bounded collection activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionState {
    phase: CollectionPhase,
    item_count: u32,
    max_items: u32,
    max_concurrency: u16,
    snapshot_digest: Digest,
    next_admission: u32,
    entries: BTreeMap<u32, ExecutionInstanceId>,
    item_digests: BTreeMap<u32, Digest>,
    iterations: BTreeMap<u32, CollectionIterationState>,
    results: BTreeMap<u32, CollectionResult>,
    result_digest: Option<Digest>,
    drain_reason: Option<CollectionDrainReason>,
}

impl CollectionState {
    #[must_use]
    pub const fn phase(&self) -> CollectionPhase {
        self.phase
    }

    #[must_use]
    pub const fn item_count(&self) -> u32 {
        self.item_count
    }

    #[must_use]
    pub const fn max_items(&self) -> u32 {
        self.max_items
    }

    #[must_use]
    pub const fn max_concurrency(&self) -> u16 {
        self.max_concurrency
    }

    #[must_use]
    pub const fn snapshot_digest(&self) -> Digest {
        self.snapshot_digest
    }

    #[must_use]
    pub const fn next_admission(&self) -> u32 {
        self.next_admission
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.iterations
            .values()
            .filter(|iteration| iteration.is_active())
            .count()
    }

    /// Iterations whose result was checkpointed or whose drain completed.
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.iterations
            .values()
            .filter(|iteration| iteration.is_settled())
            .count()
    }

    /// Admitted members that still hold a durable tenant slot.
    pub fn active_iterations(&self) -> impl Iterator<Item = u32> + '_ {
        self.iterations
            .iter()
            .filter_map(|(index, iteration)| iteration.is_active().then_some(*index))
    }

    /// Result references in immutable input-index order, independent of completion order.
    pub fn ordered_results(&self) -> impl Iterator<Item = (u32, &CollectionResult)> {
        self.results.iter().map(|(index, result)| (*index, result))
    }

    #[must_use]
    pub const fn result_digest(&self) -> Option<Digest> {
        self.result_digest
    }

    #[must_use]
    pub fn item_digest(&self, index: u32) -> Option<Digest> {
        self.item_digests.get(&index).copied()
    }

    #[must_use]
    pub fn entry_instance(&self, index: u32) -> Option<&ExecutionInstanceId> {
        self.entries.get(&index)
    }

    #[must_use]
    pub fn drain_reason(&self) -> Option<&CollectionDrainReason> {
        self.drain_reason.as_ref()
    }
}

/// Durable lifecycle of one sequential loop scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopPhase {
    Ready,
    Running,
    Delaying,
    Draining,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopActiveIteration {
    index: u32,
    entry: ExecutionInstanceId,
    return_instance: Option<ExecutionInstanceId>,
}

/// Immutable evidence committed after one loop body returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCheckpoint {
    return_instance: ExecutionInstanceId,
    result_digest: Digest,
    carry_digest: Digest,
    condition: bool,
    checked_at: OffsetDateTime,
    continuation: LoopContinuation,
}

/// One sequential-loop iteration reconstructed from durable projection rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopIterationRestore {
    index: u32,
    entry: ExecutionInstanceId,
    checkpoint: Option<LoopCheckpoint>,
    drained: bool,
}

impl LoopIterationRestore {
    #[must_use]
    pub const fn admitted(index: u32, entry: ExecutionInstanceId) -> Self {
        Self {
            index,
            entry,
            checkpoint: None,
            drained: false,
        }
    }

    #[must_use]
    pub const fn drained(index: u32, entry: ExecutionInstanceId) -> Self {
        Self {
            index,
            entry,
            checkpoint: None,
            drained: true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn checkpointed(
        index: u32,
        entry: ExecutionInstanceId,
        return_instance: ExecutionInstanceId,
        result_digest: Digest,
        carry_digest: Digest,
        condition: bool,
        checked_at: OffsetDateTime,
        continuation: LoopContinuation,
    ) -> Self {
        Self {
            index,
            entry,
            checkpoint: Some(LoopCheckpoint {
                return_instance,
                result_digest,
                carry_digest,
                condition,
                checked_at,
                continuation,
            }),
            drained: false,
        }
    }
}

impl LoopCheckpoint {
    #[must_use]
    pub const fn return_instance(&self) -> &ExecutionInstanceId {
        &self.return_instance
    }

    #[must_use]
    pub const fn result_digest(&self) -> Digest {
        self.result_digest
    }

    #[must_use]
    pub const fn carry_digest(&self) -> Digest {
        self.carry_digest
    }

    #[must_use]
    pub const fn condition(&self) -> bool {
        self.condition
    }

    #[must_use]
    pub const fn checked_at(&self) -> OffsetDateTime {
        self.checked_at
    }

    #[must_use]
    pub const fn continuation(&self) -> LoopContinuation {
        self.continuation
    }
}

/// Journal-derived progress for one sequential loop activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopState {
    kind: LoopKind,
    phase: LoopPhase,
    max_iterations: u32,
    entered_at: OffsetDateTime,
    deadline: OffsetDateTime,
    initial_carry_digest: Digest,
    current_carry_digest: Digest,
    next_iteration: u32,
    active: Option<LoopActiveIteration>,
    checkpoints: BTreeMap<u32, LoopCheckpoint>,
    delay_until: Option<OffsetDateTime>,
    drain_reason: Option<LoopDrainReason>,
    failure: Option<FailureCode>,
}

impl LoopState {
    #[must_use]
    pub const fn kind(&self) -> LoopKind {
        self.kind
    }

    #[must_use]
    pub const fn phase(&self) -> LoopPhase {
        self.phase
    }

    #[must_use]
    pub const fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    #[must_use]
    pub const fn entered_at(&self) -> OffsetDateTime {
        self.entered_at
    }

    #[must_use]
    pub const fn deadline(&self) -> OffsetDateTime {
        self.deadline
    }

    #[must_use]
    pub const fn initial_carry_digest(&self) -> Digest {
        self.initial_carry_digest
    }

    #[must_use]
    pub const fn current_carry_digest(&self) -> Digest {
        self.current_carry_digest
    }

    #[must_use]
    pub const fn next_iteration(&self) -> u32 {
        self.next_iteration
    }

    #[must_use]
    pub fn active_iteration(&self) -> Option<u32> {
        self.active.as_ref().map(|active| active.index)
    }

    #[must_use]
    pub fn checkpoint(&self, index: u32) -> Option<&LoopCheckpoint> {
        self.checkpoints.get(&index)
    }

    #[must_use]
    pub const fn delay_until(&self) -> Option<OffsetDateTime> {
        self.delay_until
    }

    #[must_use]
    pub const fn drain_reason(&self) -> Option<&LoopDrainReason> {
        self.drain_reason.as_ref()
    }

    #[must_use]
    pub const fn failure(&self) -> Option<&FailureCode> {
        self.failure.as_ref()
    }
}

/// Where a saga is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SagaPhase {
    /// The body runs.
    Forward,
    /// The body is being wound down so what completed can be compensated.
    Aborting,
    /// The compensations run.
    Compensating,
    /// Over; `outcome` says how it ended, unless the saga completed.
    Done,
}

impl SagaPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "FORWARD",
            Self::Aborting => "ABORTING",
            Self::Compensating => "COMPENSATING",
            Self::Done => "DONE",
        }
    }
}

impl FromStr for SagaPhase {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FORWARD" => Ok(Self::Forward),
            "ABORTING" => Ok(Self::Aborting),
            "COMPENSATING" => Ok(Self::Compensating),
            "DONE" => Ok(Self::Done),
            _ => Err(IdentityError::MalformedId),
        }
    }
}

/// What the run knows about one saga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaState {
    phase: SagaPhase,
    cause: Option<FailureCode>,
    /// The forward nodes being compensated and the forward nodes each waits for.
    entries: BTreeMap<NodeId, Vec<NodeId>>,
    /// Forward nodes that completed and cannot be undone.
    irreversible: Vec<NodeId>,
    outcome: Option<SagaOutcome>,
    /// Whether the saga ended (or was aborted) while the run was being stopped: its outcome then
    /// counts in how the run ends.
    during_stop: bool,
}

impl SagaState {
    const fn forward() -> Self {
        Self {
            phase: SagaPhase::Forward,
            cause: None,
            entries: BTreeMap::new(),
            irreversible: Vec::new(),
            outcome: None,
            during_stop: false,
        }
    }

    #[must_use]
    pub const fn phase(&self) -> SagaPhase {
        self.phase
    }

    /// Why the saga is compensating (or was).
    #[must_use]
    pub const fn cause(&self) -> Option<&FailureCode> {
        self.cause.as_ref()
    }

    /// The forward nodes being compensated, with the forward nodes each of them waits for.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<NodeId, Vec<NodeId>> {
        &self.entries
    }

    #[must_use]
    pub fn irreversible(&self) -> &[NodeId] {
        &self.irreversible
    }

    #[must_use]
    pub const fn outcome(&self) -> Option<SagaOutcome> {
        self.outcome
    }

    #[must_use]
    pub const fn during_stop(&self) -> bool {
        self.during_stop
    }

    /// Rebuilds a saga from trusted storage.
    #[must_use]
    pub const fn restore(
        phase: SagaPhase,
        cause: Option<FailureCode>,
        entries: BTreeMap<NodeId, Vec<NodeId>>,
        irreversible: Vec<NodeId>,
        outcome: Option<SagaOutcome>,
        during_stop: bool,
    ) -> Self {
        Self {
            phase,
            cause,
            entries,
            irreversible,
            outcome,
            during_stop,
        }
    }
}

/// What the run knows about the compensation of one collection or loop scope (D380..D385). The
/// scope's own drain is tracked by [`CollectionState`] or [`LoopState`]; this starts when every
/// iteration has settled and ends when every compensation it planned has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSagaState {
    phase: SagaPhase,
    cause: FailureCode,
    /// The forward instances being compensated, with the forward instances each waits for.
    entries: BTreeMap<ExecutionInstanceId, Vec<ExecutionInstanceId>>,
    /// Forward instances that completed and cannot be undone.
    irreversible: Vec<ExecutionInstanceId>,
    outcome: Option<SagaOutcome>,
    /// Whether the scope ended (or was planned) while the run was being stopped: its outcome then
    /// counts in how the run ends.
    during_stop: bool,
}

impl ScopeSagaState {
    /// `Compensating` while a compensation can still run, `Done` when the scope has an outcome.
    #[must_use]
    pub const fn phase(&self) -> SagaPhase {
        self.phase
    }

    /// Why the scope compensates: the failure that drained it, or the stop of the run.
    #[must_use]
    pub const fn cause(&self) -> &FailureCode {
        &self.cause
    }

    /// The forward instances being compensated, with the forward instances each waits for.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<ExecutionInstanceId, Vec<ExecutionInstanceId>> {
        &self.entries
    }

    #[must_use]
    pub fn irreversible(&self) -> &[ExecutionInstanceId] {
        &self.irreversible
    }

    #[must_use]
    pub const fn outcome(&self) -> Option<SagaOutcome> {
        self.outcome
    }

    #[must_use]
    pub const fn during_stop(&self) -> bool {
        self.during_stop
    }

    /// Rebuilds a scope saga from trusted storage.
    #[must_use]
    pub const fn restore(
        phase: SagaPhase,
        cause: FailureCode,
        entries: BTreeMap<ExecutionInstanceId, Vec<ExecutionInstanceId>>,
        irreversible: Vec<ExecutionInstanceId>,
        outcome: Option<SagaOutcome>,
        during_stop: bool,
    ) -> Self {
        Self {
            phase,
            cause,
            entries,
            irreversible,
            outcome,
            during_stop,
        }
    }
}

/// Everything the engine tracks about one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepState {
    status: StepStatus,
    attempt: u32,
    generation: u64,
    cancel_requested: bool,
    routed: bool,
    next_attempt_at: Option<OffsetDateTime>,
    waiting: Option<WaitingOn>,
    failure: Option<FailureCode>,
    condition: Option<bool>,
    switch_selection: Option<SwitchSelection>,
    timed_out: bool,
}

impl StepState {
    pub(super) const fn pending() -> Self {
        Self {
            status: StepStatus::Pending,
            attempt: 0,
            generation: 0,
            cancel_requested: false,
            routed: false,
            next_attempt_at: None,
            waiting: None,
            failure: None,
            condition: None,
            switch_selection: None,
            timed_out: false,
        }
    }

    #[must_use]
    pub const fn status(&self) -> StepStatus {
        self.status
    }

    /// Attempts started so far (0 until the first lease).
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The generation of the latest lease grant.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    /// Whether the successors of this (finished) step have been chosen.
    #[must_use]
    pub const fn routed(&self) -> bool {
        self.routed
    }

    pub(super) fn mark_routed(&mut self) {
        self.routed = true;
    }

    #[must_use]
    pub const fn next_attempt_at(&self) -> Option<OffsetDateTime> {
        self.next_attempt_at
    }

    #[must_use]
    pub const fn waiting_on(&self) -> Option<&WaitingOn> {
        self.waiting.as_ref()
    }

    /// The code of the last failed attempt, or of the terminal failure.
    #[must_use]
    pub const fn failure(&self) -> Option<&FailureCode> {
        self.failure.as_ref()
    }

    /// A terminal failure that has not taken its own declared error route. The complete
    /// instance identity resolves equal local node names in different bodies independently.
    /// A missing declaration or an unrouted failure remains unhandled.
    #[must_use]
    pub fn unhandled_instance_failure(
        &self,
        definition: &crate::workflow::WorkflowDefinition,
        instance: &ExecutionInstanceId,
    ) -> Option<&FailureCode> {
        if self.status != StepStatus::Failed
            || (self.routed
                && definition
                    .node_for_instance(instance)
                    .is_some_and(|node| node.on_error().is_some()))
        {
            None
        } else {
            self.failure.as_ref()
        }
    }

    /// The branch a condition chose, once evaluated.
    #[must_use]
    pub const fn condition(&self) -> Option<bool> {
        self.condition
    }

    /// The case or default branch recorded for a completed switch.
    #[must_use]
    pub const fn switch_selection(&self) -> Option<&SwitchSelection> {
        self.switch_selection.as_ref()
    }

    /// A wait that ended by deadline rather than by its event.
    #[must_use]
    pub const fn timed_out(&self) -> bool {
        self.timed_out
    }

    /// Rebuilds a step from trusted storage (a projection row). The reducer's checks do not run:
    /// the rows were written from states the reducer produced, and tests compare the result with a
    /// replay of the journal.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn restore(
        status: StepStatus,
        attempt: u32,
        generation: u64,
        cancel_requested: bool,
        routed: bool,
        next_attempt_at: Option<OffsetDateTime>,
        waiting: Option<WaitingOn>,
        failure: Option<FailureCode>,
        condition: Option<bool>,
        timed_out: bool,
    ) -> Self {
        Self {
            status,
            attempt,
            generation,
            cancel_requested,
            routed,
            next_attempt_at,
            waiting,
            failure,
            condition,
            switch_selection: None,
            timed_out,
        }
    }

    /// Restores a switch choice from its journal-derived projection.
    #[must_use]
    pub fn with_restored_switch_selection(mut self, selection: Option<SwitchSelection>) -> Self {
        self.switch_selection = selection;
        self
    }

    const fn fenced_by(&self, attempt: u32, generation: u64) -> bool {
        self.attempt == attempt && self.generation == generation
    }

    pub(super) fn apply_instance_lifecycle(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), ExecutionError> {
        use StepStatus::{
            Cancelled, Failed, Leased, Pending, Ready, RetryWait, Running, Succeeded, Waiting,
        };
        let name = event.name();
        let require = |step: &Self, allowed: &[StepStatus]| {
            if allowed.contains(&step.status) {
                Ok(())
            } else {
                Err(illegal_transition(instance, step.status, name))
            }
        };
        match event {
            RunEvent::StepReleased { .. } => {
                require(self, &[Pending])?;
                self.status = Ready;
            }
            RunEvent::StepLeased {
                attempt,
                generation,
                ..
            } => {
                require(self, &[Ready])?;
                if *attempt != self.attempt + 1 || *generation != self.generation + 1 {
                    return Err(stale_attempt(instance));
                }
                self.status = Leased;
                self.attempt = *attempt;
                self.generation = *generation;
                self.cancel_requested = false;
                self.next_attempt_at = None;
            }
            RunEvent::StepStarted {
                attempt,
                generation,
                ..
            } => {
                require(self, &[Leased])?;
                if !self.fenced_by(*attempt, *generation) {
                    return Err(stale_attempt(instance));
                }
                self.status = Running;
            }
            RunEvent::StepCompleted {
                attempt,
                generation,
                ..
            } => {
                require(self, &[Leased, Running])?;
                if !self.fenced_by(*attempt, *generation) {
                    return Err(stale_attempt(instance));
                }
                self.status = Succeeded;
                self.cancel_requested = false;
            }
            RunEvent::StepRetryScheduled {
                attempt,
                generation,
                code,
                not_before,
                ..
            } => {
                require(self, &[Leased, Running])?;
                if !self.fenced_by(*attempt, *generation) {
                    return Err(stale_attempt(instance));
                }
                self.status = RetryWait;
                self.failure = Some(code.clone());
                self.next_attempt_at = Some(*not_before);
            }
            RunEvent::StepRetryDue { .. } => {
                require(self, &[RetryWait])?;
                self.status = Ready;
                self.next_attempt_at = None;
            }
            RunEvent::StepFailed { code, fence, .. } => {
                require(self, &[Leased, Running, Pending, Waiting])?;
                match (self.status.is_leased(), fence) {
                    (true, Some(fence)) if self.fenced_by(fence.attempt, fence.generation) => {}
                    (true, _) => return Err(stale_attempt(instance)),
                    (false, None) => {}
                    (false, Some(_)) => {
                        return Err(illegal_transition(instance, self.status, name));
                    }
                }
                self.status = Failed;
                self.failure = Some(code.clone());
                self.waiting = None;
                self.cancel_requested = false;
            }
            RunEvent::ConditionEvaluated { outcome, .. } => {
                require(self, &[Pending])?;
                self.status = Succeeded;
                self.condition = Some(*outcome);
            }
            RunEvent::SwitchEvaluated { selection, .. } => {
                require(self, &[Pending])?;
                self.status = Succeeded;
                self.switch_selection = Some(selection.clone());
            }
            RunEvent::StepCancelRequested { .. } => {
                require(self, &[Leased, Running, Waiting])?;
                if (self.status == Waiting
                    && !matches!(self.waiting, Some(WaitingOn::Child { .. })))
                    || self.cancel_requested
                {
                    return Err(illegal_transition(instance, self.status, name));
                }
                self.cancel_requested = true;
            }
            RunEvent::StepCancelled { .. } => {
                require(self, &[Pending, Ready, Leased, Running, Waiting, RetryWait])?;
                self.status = Cancelled;
                self.routed = true;
                self.waiting = None;
                self.next_attempt_at = None;
                self.cancel_requested = false;
            }
            _ => return Err(ExecutionError::UnsupportedScopedEvent(name)),
        }
        Ok(())
    }
}

/// Chain membership and segment completion retained by one run's journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunChainState {
    chain_id: RunId,
    predecessor: Option<RunId>,
    successor: Option<RunId>,
    segment_index: SegmentIndex,
    max_segments: u16,
    original_deadline: OffsetDateTime,
    completion_kind: Option<SegmentCompletionKind>,
    transfer: Option<ContinuationTransfer>,
}

impl RunChainState {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn restore(
        chain_id: RunId,
        predecessor: Option<RunId>,
        successor: Option<RunId>,
        segment_index: SegmentIndex,
        max_segments: u16,
        original_deadline: OffsetDateTime,
        completion_kind: Option<SegmentCompletionKind>,
        transfer: Option<ContinuationTransfer>,
    ) -> Self {
        Self {
            chain_id,
            predecessor,
            successor,
            segment_index,
            max_segments,
            original_deadline,
            completion_kind,
            transfer,
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> RunId {
        self.chain_id
    }

    #[must_use]
    pub const fn predecessor(&self) -> Option<RunId> {
        self.predecessor
    }

    #[must_use]
    pub const fn successor(&self) -> Option<RunId> {
        self.successor
    }

    #[must_use]
    pub const fn segment_index(&self) -> SegmentIndex {
        self.segment_index
    }

    #[must_use]
    pub const fn max_segments(&self) -> u16 {
        self.max_segments
    }

    #[must_use]
    pub const fn original_deadline(&self) -> OffsetDateTime {
        self.original_deadline
    }

    #[must_use]
    pub const fn completion_kind(&self) -> Option<SegmentCompletionKind> {
        self.completion_kind
    }

    #[must_use]
    pub const fn transfer(&self) -> Option<&ContinuationTransfer> {
        self.transfer.as_ref()
    }
}

/// The terminal status a stopped run reaches: the status of the reason, worsened by the sagas that
/// ran while it was stopping. `failure_compensated` says that the failure that stopped the run was
/// itself the failure a compensating scope ran into, so undoing it completely is what ends the run
/// `COMPENSATED` rather than `FAILED`.
fn end_status(
    reason: StopReason,
    failure_compensated: bool,
    outcomes: impl Iterator<Item = SagaOutcome>,
) -> RunStatus {
    let own = match reason {
        StopReason::Compensated => Some(SagaOutcome::Compensated),
        StopReason::Partial => Some(SagaOutcome::Partial),
        StopReason::CompensationFailed => Some(SagaOutcome::CompensationFailed),
        StopReason::Failed if failure_compensated => Some(SagaOutcome::Compensated),
        StopReason::Cancelled | StopReason::TimedOut | StopReason::Failed => None,
    };
    match own.into_iter().chain(outcomes).max() {
        Some(SagaOutcome::CompensationFailed) => RunStatus::CompensationFailed,
        Some(SagaOutcome::Partial) => RunStatus::Partial,
        Some(SagaOutcome::Compensated) if own.is_some() => RunStatus::Compensated,
        _ => reason.final_status(),
    }
}

/// The terminal status of a run that one compensating scope stopped or ended while it was stopping:
/// the same answer as [`RunState::final_status_for`] for a run with that one scope, for the adapters
/// that finish the run in the transaction that finishes the scope.
#[must_use]
pub fn scope_final_status(
    reason: StopReason,
    stop_code: Option<&FailureCode>,
    cause: &FailureCode,
    outcome: SagaOutcome,
    during_stop: bool,
) -> RunStatus {
    end_status(
        reason,
        Some(cause) == stop_code,
        during_stop.then_some(outcome).into_iter(),
    )
}

fn illegal_transition(
    instance: &ExecutionInstanceId,
    status: StepStatus,
    event: &'static str,
) -> ExecutionError {
    if instance.path().is_root() {
        ExecutionError::IllegalTransition {
            node: instance.node().clone(),
            status,
            event,
        }
    } else {
        ExecutionError::IllegalInstanceTransition {
            instance: instance.clone(),
            status,
            event,
        }
    }
}

fn stale_attempt(instance: &ExecutionInstanceId) -> ExecutionError {
    if instance.path().is_root() {
        ExecutionError::StaleAttempt {
            node: instance.node().clone(),
        }
    } else {
        ExecutionError::StaleInstanceAttempt {
            instance: instance.clone(),
        }
    }
}

/// The state of a run, rebuilt by folding its journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunState {
    run_id: RunId,
    status: RunStatus,
    stop: Option<(StopReason, Option<FailureCode>)>,
    paused: bool,
    steps: BTreeMap<ExecutionInstanceId, StepState>,
    collections: BTreeMap<ExecutionInstanceId, CollectionState>,
    loops: BTreeMap<ExecutionInstanceId, LoopState>,
    sagas: BTreeMap<NodeId, SagaState>,
    scope_sagas: BTreeMap<ExecutionInstanceId, ScopeSagaState>,
    chain: Option<RunChainState>,
    applied: u64,
}

impl RunState {
    /// An empty run: nothing has been journaled yet.
    #[must_use]
    pub const fn new(run_id: RunId) -> Self {
        Self {
            run_id,
            status: RunStatus::Running,
            stop: None,
            paused: false,
            steps: BTreeMap::new(),
            collections: BTreeMap::new(),
            loops: BTreeMap::new(),
            sagas: BTreeMap::new(),
            scope_sagas: BTreeMap::new(),
            chain: None,
            applied: 0,
        }
    }

    /// Rebuilds a state by applying `events` in order.
    ///
    /// # Errors
    /// The first event the reducer rejects.
    pub fn replay<'a>(
        run_id: RunId,
        events: impl IntoIterator<Item = &'a RunEvent>,
    ) -> Result<Self, ExecutionError> {
        let mut state = Self::new(run_id);
        for event in events {
            state.apply(event)?;
        }
        Ok(state)
    }

    /// Rebuilds root and scoped state by folding explicitly addressed events in order.
    ///
    /// # Errors
    /// The first event the reducer rejects.
    pub fn replay_addressed<'a>(
        run_id: RunId,
        events: impl IntoIterator<Item = &'a AddressedRunEvent>,
    ) -> Result<Self, ExecutionError> {
        let mut state = Self::new(run_id);
        for event in events {
            state.apply_addressed(event)?;
        }
        Ok(state)
    }

    /// Rebuilds a run from trusted storage (the `runs` and `steps` rows). `version` is the number of
    /// journal events the rows reflect.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        run_id: RunId,
        status: RunStatus,
        stop: Option<(StopReason, Option<FailureCode>)>,
        paused: bool,
        steps: BTreeMap<NodeId, StepState>,
        sagas: BTreeMap<NodeId, SagaState>,
        version: u64,
    ) -> Self {
        Self::restore_instances(
            run_id,
            status,
            stop,
            paused,
            steps
                .into_iter()
                .map(|(node, state)| (ExecutionInstanceId::root(node), state))
                .collect(),
            sagas,
            version,
        )
    }

    /// Rebuilds every logical instance from a trusted projection, retaining distinct scope paths.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn restore_instances(
        run_id: RunId,
        status: RunStatus,
        stop: Option<(StopReason, Option<FailureCode>)>,
        paused: bool,
        steps: BTreeMap<ExecutionInstanceId, StepState>,
        sagas: BTreeMap<NodeId, SagaState>,
        version: u64,
    ) -> Self {
        Self {
            run_id,
            status,
            stop,
            paused,
            steps,
            collections: BTreeMap::new(),
            loops: BTreeMap::new(),
            sagas,
            scope_sagas: BTreeMap::new(),
            chain: None,
            applied: version,
        }
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn status(&self) -> RunStatus {
        self.status
    }

    /// Number of events applied: the run's version and the sequence of its last journal entry.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.applied
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.status.is_terminal()
    }

    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    #[must_use]
    pub const fn chain(&self) -> Option<&RunChainState> {
        self.chain.as_ref()
    }

    /// Restores journal-derived chain membership without advancing the journal sequence.
    pub fn restore_chain(&mut self, chain: RunChainState) {
        self.chain = Some(chain);
    }

    /// Why the run is winding down, if it is.
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.stop.as_ref().map(|(reason, _)| *reason)
    }

    /// The failure code recorded when the run was stopped by a failure.
    #[must_use]
    pub fn stop_code(&self) -> Option<&FailureCode> {
        self.stop.as_ref().and_then(|(_, code)| code.as_ref())
    }

    #[must_use]
    pub fn step(&self, node: &NodeId) -> Option<&StepState> {
        self.steps.get(&ExecutionInstanceId::root(node.clone()))
    }

    #[must_use]
    pub fn instance(&self, id: &ExecutionInstanceId) -> Option<&StepState> {
        self.steps.get(id)
    }

    #[must_use]
    pub fn collection(&self, id: &ExecutionInstanceId) -> Option<&CollectionState> {
        self.collections.get(id)
    }

    #[must_use]
    pub fn loop_state(&self, id: &ExecutionInstanceId) -> Option<&LoopState> {
        self.loops.get(id)
    }

    /// Every entered collection scope, ordered by its complete instance identity.
    pub fn collections(&self) -> impl Iterator<Item = (&ExecutionInstanceId, &CollectionState)> {
        self.collections.iter()
    }

    /// Restores collection progress from its trusted projection without advancing the journal
    /// sequence. The owner, admitted entries and recorded returns must already exist in the
    /// restored step map.
    ///
    /// # Errors
    /// Returns an execution invariant error when the projection is internally inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_collection(
        &mut self,
        collection: ExecutionInstanceId,
        phase: CollectionPhase,
        item_count: u32,
        max_items: u32,
        max_concurrency: u16,
        snapshot_digest: Digest,
        result_digest: Option<Digest>,
        admitted: Vec<CollectionIterationRestore>,
    ) -> Result<(), ExecutionError> {
        self.restore_collection_progress(
            collection,
            phase,
            item_count,
            max_items,
            max_concurrency,
            snapshot_digest,
            result_digest,
            None,
            admitted,
        )
    }

    /// Restores collection progress that carries a durable drain reason without advancing the
    /// journal sequence. Every drained member must already be terminal in the restored step map.
    ///
    /// # Errors
    /// Returns an execution invariant error when the projection is internally inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_collection_progress(
        &mut self,
        collection: ExecutionInstanceId,
        phase: CollectionPhase,
        item_count: u32,
        max_items: u32,
        max_concurrency: u16,
        snapshot_digest: Digest,
        result_digest: Option<Digest>,
        drain_reason: Option<CollectionDrainReason>,
        admitted: Vec<CollectionIterationRestore>,
    ) -> Result<(), ExecutionError> {
        let max_cardinality = MAX_ITERATION_INDEX + 1;
        let active = admitted
            .iter()
            .filter(|iteration| iteration.is_active())
            .count();
        let has_drained = admitted.iter().any(|iteration| iteration.drained);
        let lifecycle_matches = matches!(
            (phase, result_digest, &drain_reason),
            (CollectionPhase::Open, None, None)
                | (CollectionPhase::Succeeded, Some(_), None)
                | (CollectionPhase::Draining, None, Some(_))
                | (
                    CollectionPhase::Failed,
                    None,
                    Some(
                        CollectionDrainReason::Failed { .. }
                            | CollectionDrainReason::RunDataLimit
                            | CollectionDrainReason::StorageQuotaExceeded
                    )
                )
                | (
                    CollectionPhase::Cancelled,
                    None,
                    Some(CollectionDrainReason::Cancelled)
                )
        );
        if max_items == 0
            || max_items > max_cardinality
            || item_count > max_items
            || !(1..=256).contains(&max_concurrency)
            || active > usize::from(max_concurrency)
            || admitted.len() > usize::try_from(item_count).unwrap_or(usize::MAX)
            || (has_drained && drain_reason.is_none())
            || !lifecycle_matches
        {
            return Err(ExecutionError::InvalidCollectionLimits { collection });
        }
        if self.collections.contains_key(&collection) {
            return Err(ExecutionError::CollectionAlreadyEntered(collection));
        }
        let owner = self
            .steps
            .get(&collection)
            .ok_or_else(|| ExecutionError::UnknownInstance(collection.clone()))?;
        let owner_matches = match phase {
            CollectionPhase::Open => {
                owner.status == StepStatus::Waiting
                    && matches!(owner.waiting_on(), Some(WaitingOn::Collection))
            }
            CollectionPhase::Succeeded => {
                owner.status == StepStatus::Succeeded && owner.waiting_on().is_none()
            }
            CollectionPhase::Draining => {
                owner.status == StepStatus::Waiting
                    && matches!(owner.waiting_on(), Some(WaitingOn::Collection))
            }
            CollectionPhase::Failed => {
                let storage_quota = FailureCode::storage_quota_exceeded();
                let run_data_limit = FailureCode::run_data_limit();
                let expected = match &drain_reason {
                    Some(CollectionDrainReason::Failed { code, .. }) => Some(code),
                    Some(CollectionDrainReason::RunDataLimit) => Some(&run_data_limit),
                    Some(CollectionDrainReason::StorageQuotaExceeded) => Some(&storage_quota),
                    _ => None,
                };
                owner.status == StepStatus::Failed
                    && owner.waiting_on().is_none()
                    && owner.failure() == expected
            }
            CollectionPhase::Cancelled => {
                owner.status == StepStatus::Cancelled && owner.waiting_on().is_none()
            }
        };
        if !owner_matches {
            return Err(ExecutionError::IllegalInstanceTransition {
                instance: collection,
                status: owner.status,
                event: "collection.restore",
            });
        }

        let mut entries = BTreeMap::new();
        let mut item_digests = BTreeMap::new();
        let mut iterations = BTreeMap::new();
        let mut results = BTreeMap::new();
        for (position, restored) in admitted.into_iter().enumerate() {
            let CollectionIterationRestore {
                index,
                entry,
                item_digest,
                returned,
                drained,
            } = restored;
            let expected =
                u32::try_from(position).map_err(|_| ExecutionError::InvalidCollectionLimits {
                    collection: collection.clone(),
                })?;
            if index != expected || index >= item_count {
                return Err(ExecutionError::UnexpectedCollectionAdmission {
                    collection,
                    expected,
                    actual: index,
                });
            }
            Self::validate_collection_instance(&collection, index, &entry)?;
            if !self.steps.contains_key(&entry) {
                return Err(ExecutionError::UnknownInstance(entry));
            }
            let iteration = if drained {
                if returned.is_some() {
                    return Err(ExecutionError::IllegalCollectionTransition {
                        collection,
                        event: "collection.restore",
                    });
                }
                let path = entry.path();
                // The compensations of a scope that stopped are created after its iterations
                // drain, so they are no part of what a drained iteration had settled.
                let mut scoped = self.steps.iter().filter(|(instance, _)| {
                    instance.path() == path && !instance.node().is_compensation()
                });
                if !scoped.any(|(instance, step)| {
                    step.status.is_terminal()
                        && (!instance.node().is_derived()
                            || matches!(step.status, StepStatus::Succeeded | StepStatus::Skipped))
                }) || self.steps.iter().any(|(instance, step)| {
                    instance.path() == path
                        && !instance.node().is_compensation()
                        && (!step.status.is_terminal()
                            || (instance.node().is_derived()
                                && !matches!(
                                    step.status,
                                    StepStatus::Succeeded | StepStatus::Skipped
                                )))
                }) {
                    return Err(ExecutionError::IllegalCollectionTransition {
                        collection,
                        event: "collection.restore",
                    });
                }
                CollectionIterationState::Drained
            } else {
                match returned {
                    None => CollectionIterationState::Admitted,
                    Some((return_instance, result_digest)) => {
                        Self::validate_collection_instance(&collection, index, &return_instance)?;
                        let returned_step = self.steps.get(&return_instance).ok_or_else(|| {
                            ExecutionError::UnknownInstance(return_instance.clone())
                        })?;
                        if returned_step.status != StepStatus::Succeeded || !returned_step.routed {
                            return Err(ExecutionError::IllegalInstanceTransition {
                                instance: return_instance,
                                status: returned_step.status,
                                event: "collection.restore",
                            });
                        }
                        match result_digest {
                            None => CollectionIterationState::Returned { return_instance },
                            Some(digest) => {
                                results.insert(
                                    index,
                                    CollectionResult {
                                        return_instance: return_instance.clone(),
                                        digest,
                                    },
                                );
                                CollectionIterationState::Checkpointed
                            }
                        }
                    }
                }
            };
            entries.insert(index, entry);
            item_digests.insert(index, item_digest);
            iterations.insert(index, iteration);
        }
        let complete = entries.len() == usize::try_from(item_count).unwrap_or(usize::MAX)
            && results.len() == usize::try_from(item_count).unwrap_or(usize::MAX)
            && iterations
                .values()
                .all(|iteration| matches!(iteration, CollectionIterationState::Checkpointed));
        if phase == CollectionPhase::Succeeded && !complete {
            return Err(ExecutionError::IllegalCollectionTransition {
                collection,
                event: "collection.restore",
            });
        }
        let all_settled = iterations
            .values()
            .all(CollectionIterationState::is_settled);
        if matches!(phase, CollectionPhase::Failed | CollectionPhase::Cancelled) && !all_settled {
            return Err(ExecutionError::IllegalCollectionTransition {
                collection,
                event: "collection.restore",
            });
        }
        if let Some(CollectionDrainReason::Failed { iteration, .. }) = &drain_reason
            && !matches!(
                iterations.get(iteration),
                Some(CollectionIterationState::Admitted | CollectionIterationState::Drained)
            )
        {
            return Err(ExecutionError::IllegalCollectionTransition {
                collection,
                event: "collection.restore",
            });
        }
        self.collections.insert(
            collection,
            CollectionState {
                phase,
                item_count,
                max_items,
                max_concurrency,
                snapshot_digest,
                next_admission: u32::try_from(entries.len()).unwrap_or(u32::MAX),
                entries,
                item_digests,
                iterations,
                results,
                result_digest,
                drain_reason,
            },
        );
        Ok(())
    }

    /// Restores a sequential loop from its trusted projection without advancing the journal.
    /// Admitted entry and returned instances must already exist in the restored step map.
    ///
    /// # Errors
    /// Returns an execution invariant error when the projection cannot be produced by loop events.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub fn restore_loop(
        &mut self,
        loop_instance: ExecutionInstanceId,
        kind: LoopKind,
        phase: LoopPhase,
        max_iterations: u32,
        entered_at: OffsetDateTime,
        deadline: OffsetDateTime,
        initial_carry_digest: Digest,
        current_carry_digest: Digest,
        initial_condition: Option<bool>,
        next_iteration: u32,
        delay_until: Option<OffsetDateTime>,
        drain_reason: Option<LoopDrainReason>,
        iterations: Vec<LoopIterationRestore>,
    ) -> Result<(), ExecutionError> {
        let timeout_ms = (deadline - entered_at).whole_milliseconds();
        let valid_condition = matches!(
            (kind, initial_condition),
            (LoopKind::While, Some(_)) | (LoopKind::RepeatUntil, None)
        );
        if !(1..=MAX_ITERATION_INDEX + 1).contains(&max_iterations)
            || !(1..=31_622_400_000).contains(&timeout_ms)
            || !valid_condition
            || self.loops.contains_key(&loop_instance)
        {
            return Err(ExecutionError::InvalidLoopEntry(loop_instance));
        }
        let owner = self
            .steps
            .get(&loop_instance)
            .ok_or_else(|| ExecutionError::UnknownInstance(loop_instance.clone()))?;
        // A loop that failed without draining failed at its last checkpoint, on the budget that
        // checkpoint recorded.
        let checkpoint_failure = iterations
            .iter()
            .rev()
            .find_map(|restored| restored.checkpoint.as_ref())
            .and_then(|checkpoint| checkpoint.continuation().limit_failure());
        let expected_failure = match &drain_reason {
            Some(LoopDrainReason::Failed { code, .. }) => code.clone(),
            Some(LoopDrainReason::TimedOut) => FailureCode::run_timed_out(),
            Some(LoopDrainReason::Cancelled) | None => {
                checkpoint_failure.unwrap_or_else(FailureCode::iteration_limit)
            }
        };
        let owner_matches = match phase {
            LoopPhase::Ready | LoopPhase::Running | LoopPhase::Delaying => {
                owner.status == StepStatus::Waiting
                    && matches!(owner.waiting_on(), Some(WaitingOn::Loop))
            }
            LoopPhase::Succeeded => {
                owner.status == StepStatus::Succeeded && owner.waiting_on().is_none()
            }
            LoopPhase::Failed => {
                owner.status == StepStatus::Failed
                    && owner.waiting_on().is_none()
                    && owner.failure() == Some(&expected_failure)
            }
            LoopPhase::Draining => {
                owner.status == StepStatus::Waiting
                    && matches!(owner.waiting_on(), Some(WaitingOn::Loop))
            }
            LoopPhase::Cancelled => {
                owner.status == StepStatus::Cancelled && owner.waiting_on().is_none()
            }
        };
        if !owner_matches {
            return Err(ExecutionError::IllegalInstanceTransition {
                instance: loop_instance,
                status: owner.status,
                event: "loop.restore",
            });
        }

        let mut active = None;
        let mut drained = None;
        let mut checkpoints = BTreeMap::new();
        for (position, restored) in iterations.into_iter().enumerate() {
            let expected_index = u32::try_from(position).unwrap_or(u32::MAX);
            if restored.index != expected_index || restored.index >= max_iterations {
                return Err(ExecutionError::UnexpectedLoopIteration {
                    loop_instance,
                    expected: expected_index,
                    actual: restored.index,
                });
            }
            Self::validate_loop_instance(&loop_instance, restored.index, &restored.entry)?;
            if !self.steps.contains_key(&restored.entry) {
                return Err(ExecutionError::UnknownInstance(restored.entry));
            }
            if restored.drained {
                if restored.checkpoint.is_some() {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance,
                        event: "loop.restore",
                    });
                }
                if drained.replace(restored.index).is_some() {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance,
                        event: "loop.restore",
                    });
                }
            } else if let Some(checkpoint) = restored.checkpoint {
                Self::validate_loop_instance(
                    &loop_instance,
                    restored.index,
                    checkpoint.return_instance(),
                )?;
                let returned = self
                    .steps
                    .get(checkpoint.return_instance())
                    .ok_or_else(|| {
                        ExecutionError::UnknownInstance(checkpoint.return_instance().clone())
                    })?;
                if returned.status != StepStatus::Succeeded || !returned.routed {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance: checkpoint.return_instance().clone(),
                        status: returned.status,
                        event: "loop.restore",
                    });
                }
                checkpoints.insert(restored.index, checkpoint);
            } else if active
                .replace(LoopActiveIteration {
                    index: restored.index,
                    entry: restored.entry,
                    return_instance: None,
                })
                .is_some()
            {
                return Err(ExecutionError::IllegalLoopTransition {
                    loop_instance,
                    event: "loop.restore",
                });
            }
        }

        let expected_next = u32::try_from(checkpoints.len()).unwrap_or(u32::MAX);
        let last_checkpoint = checkpoints
            .last_key_value()
            .map(|(_, checkpoint)| checkpoint);
        let carry_matches = last_checkpoint
            .map_or(current_carry_digest == initial_carry_digest, |checkpoint| {
                checkpoint.carry_digest() == current_carry_digest
            });
        if drained.is_some_and(|index| index != expected_next) {
            return Err(ExecutionError::IllegalLoopTransition {
                loop_instance,
                event: "loop.restore",
            });
        }
        let phase_matches = match phase {
            LoopPhase::Ready => active.is_none() && drained.is_none() && delay_until.is_none(),
            LoopPhase::Running => {
                matches!(active.as_ref(), Some(value) if value.index == expected_next)
                    && drained.is_none()
                    && delay_until.is_none()
            }
            LoopPhase::Delaying => {
                active.is_none()
                    && drained.is_none()
                    && matches!(
                        (last_checkpoint.map(LoopCheckpoint::continuation), delay_until),
                        (Some(LoopContinuation::DelayUntil(expected)), Some(actual))
                            if expected == actual
                    )
            }
            LoopPhase::Succeeded => {
                active.is_none()
                    && drained.is_none()
                    && delay_until.is_none()
                    && (matches!(
                        last_checkpoint.map(LoopCheckpoint::continuation),
                        Some(LoopContinuation::Complete)
                    ) || (checkpoints.is_empty()
                        && kind == LoopKind::While
                        && initial_condition == Some(false)))
            }
            LoopPhase::Failed => {
                active.is_none()
                    && delay_until.is_none()
                    && match &drain_reason {
                        None => {
                            matches!(
                                last_checkpoint.map(LoopCheckpoint::continuation),
                                Some(
                                    LoopContinuation::IterationLimit
                                        | LoopContinuation::InstanceLimit
                                )
                            ) && drained.is_none()
                        }
                        Some(LoopDrainReason::Failed { .. } | LoopDrainReason::TimedOut) => true,
                        Some(LoopDrainReason::Cancelled) => false,
                    }
            }
            LoopPhase::Draining => {
                delay_until.is_none()
                    && match &drain_reason {
                        Some(LoopDrainReason::Failed { iteration, .. }) => {
                            matches!(active.as_ref(), Some(value) if value.index == *iteration)
                                || drained == Some(*iteration)
                        }
                        Some(LoopDrainReason::TimedOut | LoopDrainReason::Cancelled) => true,
                        None => false,
                    }
            }
            LoopPhase::Cancelled => {
                active.is_none()
                    && delay_until.is_none()
                    && matches!(drain_reason, Some(LoopDrainReason::Cancelled))
            }
        };
        if next_iteration != expected_next || !carry_matches || !phase_matches {
            return Err(ExecutionError::IllegalLoopTransition {
                loop_instance,
                event: "loop.restore",
            });
        }
        self.loops.insert(
            loop_instance,
            LoopState {
                kind,
                phase,
                max_iterations,
                entered_at,
                deadline,
                initial_carry_digest,
                current_carry_digest,
                next_iteration,
                active,
                checkpoints,
                delay_until,
                drain_reason,
                failure: (phase == LoopPhase::Failed).then_some(expected_failure),
            },
        );
        Ok(())
    }

    /// Every entered loop scope, ordered by its complete instance identity.
    pub fn loops(&self) -> impl Iterator<Item = (&ExecutionInstanceId, &LoopState)> {
        self.loops.iter()
    }

    /// All root steps, ordered by node id. Scoped instances have their own iterator.
    pub fn steps(&self) -> impl Iterator<Item = (&NodeId, &StepState)> {
        self.steps
            .iter()
            .filter(|(id, _)| id.path().is_root())
            .map(|(id, step)| (id.node(), step))
    }

    /// Every logical step, including repeated definition nodes in distinct scopes.
    pub fn instances(&self) -> impl Iterator<Item = (&ExecutionInstanceId, &StepState)> {
        self.steps.iter()
    }

    /// How many instances the run has charged against its lifetime budget: every scoped instance
    /// (an admission charges its whole body up front) and each root instance that left `Pending`
    /// other than by cancellation, as the store counts them.
    #[must_use]
    pub fn charged_instances(&self) -> usize {
        self.steps
            .iter()
            .filter(|(instance, step)| {
                !instance.path().is_root()
                    || !matches!(step.status(), StepStatus::Pending | StepStatus::Cancelled)
            })
            .count()
    }

    /// The sagas that have started, ordered by node id.
    pub fn sagas(&self) -> impl Iterator<Item = (&NodeId, &SagaState)> {
        self.sagas.iter()
    }

    #[must_use]
    pub fn saga(&self, node: &NodeId) -> Option<&SagaState> {
        self.sagas.get(node)
    }

    /// The compensation of every collection or loop scope that started one, by owner instance.
    pub fn scope_sagas(&self) -> impl Iterator<Item = (&ExecutionInstanceId, &ScopeSagaState)> {
        self.scope_sagas.iter()
    }

    #[must_use]
    pub fn scope_saga(&self, owner: &ExecutionInstanceId) -> Option<&ScopeSagaState> {
        self.scope_sagas.get(owner)
    }

    /// Restores a scope's compensation from its trusted projection without advancing the journal.
    ///
    /// # Errors
    /// An invariant error when the scope is not a draining collection or loop of this run.
    pub fn restore_scope_saga(
        &mut self,
        owner: ExecutionInstanceId,
        saga: ScopeSagaState,
    ) -> Result<(), ExecutionError> {
        if !self.collections.contains_key(&owner) && !self.loops.contains_key(&owner) {
            return Err(ExecutionError::UnknownInstance(owner));
        }
        if self.scope_sagas.insert(owner, saga).is_some() {
            return Err(ExecutionError::Invariant(
                "a scope compensation is restored once",
            ));
        }
        Ok(())
    }

    /// The status the run reaches when it finishes after being stopped for `reason`: the sagas that
    /// ran *because* the run was stopping can make it worse than the reason alone says (a
    /// cancellation whose compensation failed is not a clean cancellation, and a run that ended
    /// because one saga was compensated is not `COMPENSATED` if a sibling saga could not be).
    #[must_use]
    pub fn final_status_for(&self, reason: StopReason) -> RunStatus {
        // A scope whose own failure stopped the run (D385) is the saga that failure ran into: if it
        // undid everything the run ends `COMPENSATED`, as a failure inside a `saga` node does.
        let scope_failure = self
            .scope_sagas
            .values()
            .any(|saga| Some(&saga.cause) == self.stop_code() && saga.outcome.is_some());
        let worst = self
            .sagas
            .values()
            .filter(|saga| saga.during_stop)
            .filter_map(|saga| saga.outcome)
            .chain(
                self.scope_sagas
                    .values()
                    .filter(|saga| saga.during_stop)
                    .filter_map(|saga| saga.outcome),
            );
        end_status(reason, scope_failure, worst)
    }

    /// How `saga` ends, judged from its compensations: failed or cancelled ones make it
    /// `COMPENSATION_FAILED`, skipped ones or an irreversible effect make it `PARTIAL`.
    pub(super) fn expected_outcome(&self, saga: &SagaState) -> SagaOutcome {
        let (mut failed, mut skipped) = (false, false);
        for node in saga.entries.keys() {
            match self
                .steps
                .get(&ExecutionInstanceId::root(node.compensation()))
                .map(|s| s.status)
            {
                Some(StepStatus::Failed | StepStatus::Cancelled) => failed = true,
                Some(StepStatus::Skipped) => skipped = true,
                _ => {}
            }
        }
        if failed {
            SagaOutcome::CompensationFailed
        } else if skipped || !saga.irreversible.is_empty() {
            SagaOutcome::Partial
        } else {
            SagaOutcome::Compensated
        }
    }

    #[must_use]
    pub fn has_started(&self) -> bool {
        self.applied > 0
    }

    fn step_mut(
        &mut self,
        node: &NodeId,
        event: &'static str,
        allowed: &[StepStatus],
    ) -> Result<&mut StepState, ExecutionError> {
        let step = self
            .steps
            .get_mut(&ExecutionInstanceId::root(node.clone()))
            .ok_or_else(|| ExecutionError::UnknownStep(node.clone()))?;
        if allowed.contains(&step.status) {
            Ok(step)
        } else {
            Err(ExecutionError::IllegalTransition {
                node: node.clone(),
                status: step.status,
                event,
            })
        }
    }

    fn fenced_step(
        &mut self,
        node: &NodeId,
        event: &'static str,
        allowed: &[StepStatus],
        attempt: u32,
        generation: u64,
    ) -> Result<&mut StepState, ExecutionError> {
        let step = self.step_mut(node, event, allowed)?;
        if step.fenced_by(attempt, generation) {
            Ok(step)
        } else {
            Err(ExecutionError::StaleAttempt { node: node.clone() })
        }
    }

    fn instance_mut(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &'static str,
        allowed: &[StepStatus],
    ) -> Result<&mut StepState, ExecutionError> {
        let step = self
            .steps
            .get_mut(instance)
            .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
        if allowed.contains(&step.status) {
            Ok(step)
        } else {
            Err(ExecutionError::IllegalInstanceTransition {
                instance: instance.clone(),
                status: step.status,
                event,
            })
        }
    }

    fn activate(&mut self, nodes: &[NodeId]) -> Result<(), ExecutionError> {
        for (i, node) in nodes.iter().enumerate() {
            if self
                .steps
                .contains_key(&ExecutionInstanceId::root(node.clone()))
                || nodes[..i].contains(node)
            {
                return Err(ExecutionError::AlreadyScheduled(node.clone()));
            }
        }
        for node in nodes {
            self.steps.insert(
                ExecutionInstanceId::root(node.clone()),
                StepState::pending(),
            );
        }
        Ok(())
    }

    fn activate_in_path(
        &mut self,
        path: &InstancePath,
        nodes: &[NodeId],
    ) -> Result<(), ExecutionError> {
        for (position, node) in nodes.iter().enumerate() {
            let instance = ExecutionInstanceId::in_path(path.clone(), node.clone());
            if self.steps.contains_key(&instance) || nodes[..position].contains(node) {
                return Err(ExecutionError::AlreadyScheduled(node.clone()));
            }
        }
        for node in nodes {
            self.steps.insert(
                ExecutionInstanceId::in_path(path.clone(), node.clone()),
                StepState::pending(),
            );
        }
        Ok(())
    }

    fn require_saga(
        &self,
        node: &NodeId,
        event: &'static str,
        phase: SagaPhase,
    ) -> Result<(), ExecutionError> {
        match self.sagas.get(node) {
            Some(saga) if saga.phase == phase => Ok(()),
            Some(_) => Err(ExecutionError::Invariant(
                "a saga event arrived in a phase that does not allow it",
            )),
            None => {
                let _ = event;
                Err(ExecutionError::UnknownStep(node.clone()))
            }
        }
    }

    fn collection_iteration_path(
        collection: &ExecutionInstanceId,
        index: u32,
    ) -> Result<InstancePath, ExecutionError> {
        collection
            .path()
            .child(collection.node().clone(), index)
            .map_err(|_| ExecutionError::InvalidCollectionLimits {
                collection: collection.clone(),
            })
    }

    fn validate_collection_instance(
        collection: &ExecutionInstanceId,
        index: u32,
        instance: &ExecutionInstanceId,
    ) -> Result<(), ExecutionError> {
        let expected = Self::collection_iteration_path(collection, index)?;
        if instance.path() == &expected {
            Ok(())
        } else {
            Err(ExecutionError::InvalidCollectionInstance {
                collection: collection.clone(),
                instance: instance.clone(),
            })
        }
    }

    fn apply_collection_event(
        &mut self,
        collection: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), ExecutionError> {
        let name = event.name();
        match event {
            RunEvent::CollectionEntered {
                item_count,
                max_items,
                max_concurrency,
                snapshot_digest,
                ..
            } => {
                let max_cardinality = MAX_ITERATION_INDEX + 1;
                if *max_items == 0
                    || *max_items > max_cardinality
                    || *item_count > *max_items
                    || !(1..=256).contains(max_concurrency)
                {
                    return Err(ExecutionError::InvalidCollectionLimits {
                        collection: collection.clone(),
                    });
                }
                if self.collections.contains_key(collection) {
                    return Err(ExecutionError::CollectionAlreadyEntered(collection.clone()));
                }
                let step = self.instance_mut(collection, name, &[StepStatus::Pending])?;
                step.status = StepStatus::Waiting;
                step.waiting = Some(WaitingOn::Collection);
                self.collections.insert(
                    collection.clone(),
                    CollectionState {
                        phase: CollectionPhase::Open,
                        item_count: *item_count,
                        max_items: *max_items,
                        max_concurrency: *max_concurrency,
                        snapshot_digest: *snapshot_digest,
                        next_admission: 0,
                        entries: BTreeMap::new(),
                        item_digests: BTreeMap::new(),
                        iterations: BTreeMap::new(),
                        results: BTreeMap::new(),
                        result_digest: None,
                        drain_reason: None,
                    },
                );
            }
            RunEvent::CollectionIterationAdmitted {
                index,
                entry,
                item_digest,
                ..
            } => {
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                if scope.iterations.contains_key(index) {
                    return Err(ExecutionError::DuplicateCollectionAdmission {
                        collection: collection.clone(),
                        index: *index,
                    });
                }
                if scope.phase != CollectionPhase::Open {
                    return Err(ExecutionError::CollectionNotOpen {
                        collection: collection.clone(),
                        phase: scope.phase,
                    });
                }
                if *index != scope.next_admission || *index >= scope.item_count {
                    return Err(ExecutionError::UnexpectedCollectionAdmission {
                        collection: collection.clone(),
                        expected: scope.next_admission,
                        actual: *index,
                    });
                }
                if scope.active_count() >= usize::from(scope.max_concurrency) {
                    return Err(ExecutionError::CollectionConcurrencyExceeded {
                        collection: collection.clone(),
                        limit: scope.max_concurrency,
                    });
                }
                Self::validate_collection_instance(collection, *index, entry)?;
                if self.steps.contains_key(entry) {
                    return Err(ExecutionError::InvalidCollectionInstance {
                        collection: collection.clone(),
                        instance: entry.clone(),
                    });
                }
                self.steps.insert(entry.clone(), StepState::pending());
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                scope
                    .iterations
                    .insert(*index, CollectionIterationState::Admitted);
                scope.entries.insert(*index, entry.clone());
                scope.item_digests.insert(*index, *item_digest);
                scope.next_admission += 1;
            }
            RunEvent::CollectionIterationReturned {
                index,
                return_instance,
                ..
            } => {
                Self::validate_collection_instance(collection, *index, return_instance)?;
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                let Some(CollectionIterationState::Admitted) = scope.iterations.get(index) else {
                    return if scope.iterations.contains_key(index) {
                        Err(ExecutionError::IllegalCollectionTransition {
                            collection: collection.clone(),
                            event: name,
                        })
                    } else {
                        Err(ExecutionError::UnknownCollectionIteration {
                            collection: collection.clone(),
                            index: *index,
                        })
                    };
                };
                let returned = self.instance_mut(return_instance, name, &[StepStatus::Pending])?;
                returned.status = StepStatus::Succeeded;
                returned.routed = true;
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                scope.iterations.insert(
                    *index,
                    CollectionIterationState::Returned {
                        return_instance: return_instance.clone(),
                    },
                );
            }
            RunEvent::CollectionIterationCheckpointed {
                index,
                return_instance,
                result_digest,
                ..
            } => {
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                match scope.iterations.get(index) {
                    Some(CollectionIterationState::Returned {
                        return_instance: recorded,
                    }) if recorded == return_instance => {}
                    Some(_) => {
                        return Err(ExecutionError::IllegalCollectionTransition {
                            collection: collection.clone(),
                            event: name,
                        });
                    }
                    None => {
                        return Err(ExecutionError::UnknownCollectionIteration {
                            collection: collection.clone(),
                            index: *index,
                        });
                    }
                }
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                let result = CollectionResult {
                    return_instance: return_instance.clone(),
                    digest: *result_digest,
                };
                scope.results.insert(*index, result.clone());
                scope
                    .iterations
                    .insert(*index, CollectionIterationState::Checkpointed);
            }
            RunEvent::CollectionDrainStarted { reason, .. } => {
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                if scope.phase != CollectionPhase::Open {
                    return Err(ExecutionError::CollectionNotOpen {
                        collection: collection.clone(),
                        phase: scope.phase,
                    });
                }
                if let CollectionDrainReason::Failed { iteration, .. } = reason
                    && !matches!(
                        scope.iterations.get(iteration),
                        Some(CollectionIterationState::Admitted)
                    )
                {
                    return if scope.iterations.contains_key(iteration) {
                        Err(ExecutionError::IllegalCollectionTransition {
                            collection: collection.clone(),
                            event: name,
                        })
                    } else {
                        Err(ExecutionError::UnknownCollectionIteration {
                            collection: collection.clone(),
                            index: *iteration,
                        })
                    };
                }
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                scope.phase = CollectionPhase::Draining;
                scope.drain_reason = Some(reason.clone());
            }
            RunEvent::CollectionIterationDrained { index, .. } => {
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                if scope.phase != CollectionPhase::Draining {
                    return Err(ExecutionError::IllegalCollectionTransition {
                        collection: collection.clone(),
                        event: name,
                    });
                }
                if !matches!(
                    scope.iterations.get(index),
                    Some(CollectionIterationState::Admitted)
                ) {
                    return if scope.iterations.contains_key(index) {
                        Err(ExecutionError::IllegalCollectionTransition {
                            collection: collection.clone(),
                            event: name,
                        })
                    } else {
                        Err(ExecutionError::UnknownCollectionIteration {
                            collection: collection.clone(),
                            index: *index,
                        })
                    };
                }
                let path = Self::collection_iteration_path(collection, *index)?;
                let reason = scope.drain_reason.clone().ok_or(ExecutionError::Invariant(
                    "a draining collection has no reason",
                ))?;
                let entry = scope.entries.get(index).cloned().ok_or(
                    ExecutionError::UnknownCollectionIteration {
                        collection: collection.clone(),
                        index: *index,
                    },
                )?;
                let mut found = false;
                for (instance, step) in self.steps.iter().filter(|(id, _)| id.path() == &path) {
                    found = true;
                    let unresolved_derived = instance.node().is_derived()
                        && !matches!(step.status, StepStatus::Succeeded | StepStatus::Skipped);
                    // Nothing is in flight: a step that has not started, or one that waits to retry
                    // (or is ready to) in a run that is stopping and can no longer lease it.
                    let safe_to_stop = (step.attempt == 0
                        && matches!(step.status, StepStatus::Pending | StepStatus::Ready))
                        || matches!(step.status, StepStatus::Ready | StepStatus::RetryWait);
                    if unresolved_derived || (!step.status.is_terminal() && !safe_to_stop) {
                        return Err(ExecutionError::IllegalCollectionTransition {
                            collection: collection.clone(),
                            event: name,
                        });
                    }
                }
                if !found {
                    return Err(ExecutionError::InvalidCollectionInstance {
                        collection: collection.clone(),
                        instance: entry,
                    });
                }
                for (instance, step) in self.steps.iter_mut().filter(|(id, _)| id.path() == &path) {
                    if !step.status.is_terminal() {
                        match &reason {
                            CollectionDrainReason::Failed {
                                iteration, code, ..
                            } if iteration == index && instance == &entry => {
                                step.status = StepStatus::Failed;
                                step.failure = Some(code.clone());
                            }
                            _ => step.status = StepStatus::Cancelled,
                        }
                        step.waiting = None;
                        step.next_attempt_at = None;
                        step.cancel_requested = false;
                        step.routed = true;
                    }
                }
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                scope
                    .iterations
                    .insert(*index, CollectionIterationState::Drained);
            }
            RunEvent::CollectionFinished {
                outcome,
                result_digest,
                ..
            } => {
                let scope = self
                    .collections
                    .get(collection)
                    .ok_or_else(|| ExecutionError::UnknownCollection(collection.clone()))?;
                let all_settled = scope
                    .iterations
                    .values()
                    .all(CollectionIterationState::is_settled)
                    && self
                        .scope_sagas
                        .get(collection)
                        .is_none_or(|saga| saga.phase == SagaPhase::Done);
                let valid = match (outcome, result_digest, &scope.drain_reason) {
                    (CollectionOutcome::Succeeded, Some(_), None) => {
                        scope.phase == CollectionPhase::Open
                            && scope.next_admission == scope.item_count
                            && scope.results.len() == scope.item_count as usize
                            && all_settled
                    }
                    (
                        CollectionOutcome::Failed(code),
                        None,
                        Some(CollectionDrainReason::Failed { code: recorded, .. }),
                    ) => {
                        scope.phase == CollectionPhase::Draining && code == recorded && all_settled
                    }
                    (
                        CollectionOutcome::Failed(code),
                        None,
                        Some(CollectionDrainReason::RunDataLimit),
                    ) => {
                        scope.phase == CollectionPhase::Draining
                            && code == &FailureCode::run_data_limit()
                            && all_settled
                    }
                    (
                        CollectionOutcome::Failed(code),
                        None,
                        Some(CollectionDrainReason::StorageQuotaExceeded),
                    ) => {
                        scope.phase == CollectionPhase::Draining
                            && code == &FailureCode::storage_quota_exceeded()
                            && all_settled
                    }
                    (
                        CollectionOutcome::Cancelled,
                        None,
                        Some(CollectionDrainReason::Cancelled),
                    ) => scope.phase == CollectionPhase::Draining && all_settled,
                    _ => false,
                };
                if !valid {
                    return Err(ExecutionError::IllegalCollectionTransition {
                        collection: collection.clone(),
                        event: name,
                    });
                }
                let step = self.instance_mut(collection, name, &[StepStatus::Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Collection)) {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance: collection.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                match outcome {
                    CollectionOutcome::Succeeded => step.status = StepStatus::Succeeded,
                    CollectionOutcome::Failed(code) => {
                        step.status = StepStatus::Failed;
                        step.failure = Some(code.clone());
                    }
                    CollectionOutcome::Cancelled => step.status = StepStatus::Cancelled,
                }
                let scope =
                    self.collections
                        .get_mut(collection)
                        .ok_or(ExecutionError::Invariant(
                            "validated collection disappeared",
                        ))?;
                scope.phase = match outcome {
                    CollectionOutcome::Succeeded => CollectionPhase::Succeeded,
                    CollectionOutcome::Failed(_) => CollectionPhase::Failed,
                    CollectionOutcome::Cancelled => CollectionPhase::Cancelled,
                };
                scope.result_digest = *result_digest;
            }
            _ => {
                return Err(ExecutionError::Invariant(
                    "a non-collection event reached the collection reducer",
                ));
            }
        }
        Ok(())
    }

    fn loop_iteration_path(
        loop_instance: &ExecutionInstanceId,
        index: u32,
    ) -> Result<InstancePath, ExecutionError> {
        loop_instance
            .path()
            .child(loop_instance.node().clone(), index)
            .map_err(|_| ExecutionError::InvalidLoopEntry(loop_instance.clone()))
    }

    fn validate_loop_instance(
        loop_instance: &ExecutionInstanceId,
        index: u32,
        instance: &ExecutionInstanceId,
    ) -> Result<(), ExecutionError> {
        let expected = Self::loop_iteration_path(loop_instance, index)?;
        if instance.path() == &expected {
            Ok(())
        } else {
            Err(ExecutionError::InvalidLoopInstance {
                loop_instance: loop_instance.clone(),
                instance: instance.clone(),
            })
        }
    }

    fn apply_loop_event(
        &mut self,
        loop_instance: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), ExecutionError> {
        let name = event.name();
        match event {
            RunEvent::LoopEntered {
                kind,
                max_iterations,
                entered_at,
                deadline,
                initial_carry_digest,
                initial_condition,
                ..
            } => {
                let timeout_ms = (*deadline - *entered_at).whole_milliseconds();
                let valid_condition = matches!(
                    (kind, initial_condition),
                    (LoopKind::While, Some(_)) | (LoopKind::RepeatUntil, None)
                );
                if !(1..=MAX_ITERATION_INDEX + 1).contains(max_iterations)
                    || !(1..=31_622_400_000).contains(&timeout_ms)
                    || !valid_condition
                {
                    return Err(ExecutionError::InvalidLoopEntry(loop_instance.clone()));
                }
                if self.loops.contains_key(loop_instance) {
                    return Err(ExecutionError::LoopAlreadyEntered(loop_instance.clone()));
                }
                let completes = *kind == LoopKind::While && *initial_condition == Some(false);
                let step = self.instance_mut(loop_instance, name, &[StepStatus::Pending])?;
                if completes {
                    step.status = StepStatus::Succeeded;
                } else {
                    step.status = StepStatus::Waiting;
                    step.waiting = Some(WaitingOn::Loop);
                }
                self.loops.insert(
                    loop_instance.clone(),
                    LoopState {
                        kind: *kind,
                        phase: if completes {
                            LoopPhase::Succeeded
                        } else {
                            LoopPhase::Ready
                        },
                        max_iterations: *max_iterations,
                        entered_at: *entered_at,
                        deadline: *deadline,
                        initial_carry_digest: *initial_carry_digest,
                        current_carry_digest: *initial_carry_digest,
                        next_iteration: 0,
                        active: None,
                        checkpoints: BTreeMap::new(),
                        delay_until: None,
                        drain_reason: None,
                        failure: None,
                    },
                );
            }
            RunEvent::LoopIterationAdmitted { index, entry, .. } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                if scope.phase != LoopPhase::Ready {
                    return Err(ExecutionError::LoopNotReady {
                        loop_instance: loop_instance.clone(),
                        phase: scope.phase,
                    });
                }
                if *index != scope.next_iteration || *index >= scope.max_iterations {
                    return Err(ExecutionError::UnexpectedLoopIteration {
                        loop_instance: loop_instance.clone(),
                        expected: scope.next_iteration,
                        actual: *index,
                    });
                }
                if scope.active.is_some() || scope.checkpoints.contains_key(index) {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                Self::validate_loop_instance(loop_instance, *index, entry)?;
                if self.steps.contains_key(entry) {
                    return Err(ExecutionError::InvalidLoopInstance {
                        loop_instance: loop_instance.clone(),
                        instance: entry.clone(),
                    });
                }
                self.steps.insert(entry.clone(), StepState::pending());
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                scope.phase = LoopPhase::Running;
                scope.active = Some(LoopActiveIteration {
                    index: *index,
                    entry: entry.clone(),
                    return_instance: None,
                });
            }
            RunEvent::LoopIterationReturned {
                index,
                return_instance,
                ..
            } => {
                Self::validate_loop_instance(loop_instance, *index, return_instance)?;
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                let Some(active) = scope.active.as_ref() else {
                    return Err(ExecutionError::UnknownLoopIteration {
                        loop_instance: loop_instance.clone(),
                        index: *index,
                    });
                };
                if scope.phase != LoopPhase::Running
                    || active.index != *index
                    || active.return_instance.is_some()
                {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                let returned = self.instance_mut(return_instance, name, &[StepStatus::Pending])?;
                returned.status = StepStatus::Succeeded;
                returned.routed = true;
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                let active = scope.active.as_mut().ok_or(ExecutionError::Invariant(
                    "validated loop iteration disappeared",
                ))?;
                active.return_instance = Some(return_instance.clone());
            }
            RunEvent::LoopIterationCheckpointed {
                index,
                return_instance,
                result_digest,
                carry_digest,
                condition,
                checked_at,
                continuation,
                ..
            } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                let Some(active) = scope.active.as_ref() else {
                    return Err(ExecutionError::UnknownLoopIteration {
                        loop_instance: loop_instance.clone(),
                        index: *index,
                    });
                };
                if scope.phase != LoopPhase::Running
                    || active.index != *index
                    || active.return_instance.as_ref() != Some(return_instance)
                    || scope.checkpoints.contains_key(index)
                    || *checked_at < scope.entered_at
                    || *checked_at > scope.deadline
                {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                let needs_another = match scope.kind {
                    LoopKind::While => *condition,
                    LoopKind::RepeatUntil => !*condition,
                };
                let next_iteration =
                    index
                        .checked_add(1)
                        .ok_or(ExecutionError::InvalidLoopContinuation {
                            loop_instance: loop_instance.clone(),
                            index: *index,
                        })?;
                let valid_continuation = if !needs_another {
                    // A finished loop whose successor the instance budget cannot pay for stops
                    // with INSTANCE_LIMIT instead of completing.
                    matches!(
                        continuation,
                        LoopContinuation::Complete | LoopContinuation::InstanceLimit
                    )
                } else if next_iteration >= scope.max_iterations {
                    matches!(continuation, LoopContinuation::IterationLimit)
                } else {
                    match continuation {
                        LoopContinuation::Continue => true,
                        LoopContinuation::DelayUntil(due_at) => {
                            (*due_at - *checked_at).whole_milliseconds() >= 1
                                && *due_at <= scope.deadline
                        }
                        LoopContinuation::InstanceLimit => true,
                        LoopContinuation::Complete | LoopContinuation::IterationLimit => false,
                    }
                };
                if !valid_continuation {
                    return Err(ExecutionError::InvalidLoopContinuation {
                        loop_instance: loop_instance.clone(),
                        index: *index,
                    });
                }
                let terminal = matches!(
                    continuation,
                    LoopContinuation::Complete
                        | LoopContinuation::IterationLimit
                        | LoopContinuation::InstanceLimit
                );
                if terminal {
                    let step = self
                        .steps
                        .get(loop_instance)
                        .ok_or_else(|| ExecutionError::UnknownInstance(loop_instance.clone()))?;
                    if step.status != StepStatus::Waiting
                        || !matches!(step.waiting, Some(WaitingOn::Loop))
                    {
                        return Err(ExecutionError::IllegalInstanceTransition {
                            instance: loop_instance.clone(),
                            status: step.status,
                            event: name,
                        });
                    }
                }
                let checkpoint = LoopCheckpoint {
                    return_instance: return_instance.clone(),
                    result_digest: *result_digest,
                    carry_digest: *carry_digest,
                    condition: *condition,
                    checked_at: *checked_at,
                    continuation: *continuation,
                };
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                scope.checkpoints.insert(*index, checkpoint);
                scope.current_carry_digest = *carry_digest;
                scope.next_iteration = next_iteration;
                scope.active = None;
                scope.delay_until = match continuation {
                    LoopContinuation::DelayUntil(due_at) => Some(*due_at),
                    _ => None,
                };
                scope.phase = match continuation {
                    LoopContinuation::Complete => LoopPhase::Succeeded,
                    LoopContinuation::Continue => LoopPhase::Ready,
                    LoopContinuation::DelayUntil(_) => LoopPhase::Delaying,
                    LoopContinuation::IterationLimit | LoopContinuation::InstanceLimit => {
                        LoopPhase::Failed
                    }
                };
                if let Some(code) = continuation.limit_failure() {
                    scope.failure = Some(code);
                }
                if terminal {
                    let step = self
                        .steps
                        .get_mut(loop_instance)
                        .ok_or(ExecutionError::Invariant("validated loop step disappeared"))?;
                    step.waiting = None;
                    if matches!(continuation, LoopContinuation::Complete) {
                        step.status = StepStatus::Succeeded;
                    } else {
                        step.status = StepStatus::Failed;
                        step.failure = continuation.limit_failure();
                    }
                }
            }
            RunEvent::LoopDelayElapsed { next_iteration, .. } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                if scope.phase != LoopPhase::Delaying
                    || scope.delay_until.is_none()
                    || scope.next_iteration != *next_iteration
                {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                scope.phase = LoopPhase::Ready;
                scope.delay_until = None;
            }
            RunEvent::LoopDrainStarted { reason, .. } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                if !matches!(
                    scope.phase,
                    LoopPhase::Ready | LoopPhase::Running | LoopPhase::Delaying
                ) {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                if let LoopDrainReason::Failed { iteration, .. } = reason
                    && !matches!(scope.active.as_ref(), Some(active) if active.index == *iteration)
                {
                    return Err(ExecutionError::UnknownLoopIteration {
                        loop_instance: loop_instance.clone(),
                        index: *iteration,
                    });
                }
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                scope.phase = LoopPhase::Draining;
                scope.delay_until = None;
                scope.drain_reason = Some(reason.clone());
            }
            RunEvent::LoopIterationDrained { index, .. } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                let Some(active) = scope.active.as_ref() else {
                    return Err(ExecutionError::UnknownLoopIteration {
                        loop_instance: loop_instance.clone(),
                        index: *index,
                    });
                };
                if scope.phase != LoopPhase::Draining || active.index != *index {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                let path = Self::loop_iteration_path(loop_instance, *index)?;
                let entry = active.entry.clone();
                let reason = scope
                    .drain_reason
                    .clone()
                    .ok_or(ExecutionError::Invariant("a draining loop has no reason"))?;
                let mut found = false;
                for (_, step) in self.steps.iter().filter(|(id, _)| id.path() == &path) {
                    found = true;
                    let safe_to_stop = step.attempt == 0
                        && matches!(step.status, StepStatus::Pending | StepStatus::Ready);
                    if !step.status.is_terminal() && !safe_to_stop {
                        return Err(ExecutionError::IllegalLoopTransition {
                            loop_instance: loop_instance.clone(),
                            event: name,
                        });
                    }
                }
                if !found {
                    return Err(ExecutionError::InvalidLoopInstance {
                        loop_instance: loop_instance.clone(),
                        instance: entry,
                    });
                }
                for (instance, step) in self.steps.iter_mut().filter(|(id, _)| id.path() == &path) {
                    if !step.status.is_terminal() {
                        match &reason {
                            LoopDrainReason::Failed {
                                iteration, code, ..
                            } if iteration == index && instance == &entry => {
                                step.status = StepStatus::Failed;
                                step.failure = Some(code.clone());
                            }
                            _ => step.status = StepStatus::Cancelled,
                        }
                        step.waiting = None;
                        step.next_attempt_at = None;
                        step.cancel_requested = false;
                        step.routed = true;
                    }
                }
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                scope.active = None;
            }
            RunEvent::LoopFinished { outcome, .. } => {
                let scope = self
                    .loops
                    .get(loop_instance)
                    .ok_or_else(|| ExecutionError::UnknownLoop(loop_instance.clone()))?;
                let matching_outcome = match (outcome, &scope.drain_reason) {
                    (
                        LoopOutcome::Failed(code),
                        Some(LoopDrainReason::Failed { code: recorded, .. }),
                    ) => code == recorded,
                    (LoopOutcome::Failed(code), Some(LoopDrainReason::TimedOut)) => {
                        code == &FailureCode::run_timed_out()
                    }
                    (LoopOutcome::Cancelled, Some(LoopDrainReason::Cancelled)) => true,
                    _ => false,
                };
                let valid = scope.phase == LoopPhase::Draining
                    && scope.active.is_none()
                    && self
                        .scope_sagas
                        .get(loop_instance)
                        .is_none_or(|saga| saga.phase == SagaPhase::Done)
                    && matching_outcome;
                if !valid {
                    return Err(ExecutionError::IllegalLoopTransition {
                        loop_instance: loop_instance.clone(),
                        event: name,
                    });
                }
                let step = self.instance_mut(loop_instance, name, &[StepStatus::Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Loop)) {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance: loop_instance.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                match outcome {
                    LoopOutcome::Failed(code) => {
                        step.status = StepStatus::Failed;
                        step.failure = Some(code.clone());
                    }
                    LoopOutcome::Cancelled => step.status = StepStatus::Cancelled,
                }
                let scope = self
                    .loops
                    .get_mut(loop_instance)
                    .ok_or(ExecutionError::Invariant("validated loop disappeared"))?;
                match outcome {
                    LoopOutcome::Failed(code) => {
                        scope.phase = LoopPhase::Failed;
                        scope.failure = Some(code.clone());
                    }
                    LoopOutcome::Cancelled => scope.phase = LoopPhase::Cancelled,
                }
            }
            _ => {
                return Err(ExecutionError::Invariant(
                    "a non-loop event reached the loop reducer",
                ));
            }
        }
        Ok(())
    }

    /// Applies one journal event.
    ///
    /// # Errors
    /// An [`ExecutionError`] when the event is not legal in the current state; the state is
    /// unchanged in that case.
    #[allow(clippy::too_many_lines)] // one arm per journal event: splitting would hide the table
    pub fn apply(&mut self, event: &RunEvent) -> Result<(), ExecutionError> {
        use StepStatus::{
            Cancelled, Failed, Leased, Pending, Ready, RetryWait, Running, Skipped, Succeeded,
            Waiting,
        };
        if self.status.is_terminal() {
            return Err(ExecutionError::RunFinished(self.status));
        }
        let name = event.name();
        match event {
            RunEvent::RunStarted { entrypoint } => {
                if self.applied != 0 || !self.steps.is_empty() {
                    return Err(ExecutionError::Invariant("a run starts exactly once"));
                }
                self.activate(std::slice::from_ref(entrypoint))?;
            }
            RunEvent::ExecutionChainStarted {
                max_segments,
                original_deadline,
            } => {
                if self.applied == 0
                    || self.steps.is_empty()
                    || self.chain.is_some()
                    || !(1..=MAX_CHAIN_SEGMENTS).contains(max_segments)
                {
                    return Err(ExecutionError::Invariant(
                        "an initial execution chain is recorded once after run start",
                    ));
                }
                self.chain = Some(RunChainState {
                    chain_id: self.run_id,
                    predecessor: None,
                    successor: None,
                    segment_index: SegmentIndex::new(0)
                        .map_err(|_| ExecutionError::Invariant("invalid initial chain segment"))?,
                    max_segments: *max_segments,
                    original_deadline: *original_deadline,
                    completion_kind: None,
                    transfer: None,
                });
            }
            RunEvent::ExecutionChainSuccessorLinked {
                chain_id,
                predecessor,
                segment_index,
                max_segments,
                original_deadline,
                input_digest,
                state_digest,
            } => {
                let index = SegmentIndex::new(*segment_index)
                    .map_err(|_| ExecutionError::Invariant("invalid successor chain segment"))?;
                if self.applied != 1
                    || self.chain.is_some()
                    || *chain_id == self.run_id
                    || *predecessor == self.run_id
                    || *segment_index == 0
                    || *segment_index >= *max_segments
                    || !(1..=MAX_CHAIN_SEGMENTS).contains(max_segments)
                {
                    return Err(ExecutionError::Invariant(
                        "a successor joins one bounded chain after run start",
                    ));
                }
                self.chain = Some(RunChainState {
                    chain_id: *chain_id,
                    predecessor: Some(*predecessor),
                    successor: None,
                    segment_index: index,
                    max_segments: *max_segments,
                    original_deadline: *original_deadline,
                    completion_kind: None,
                    transfer: Some(ContinuationTransfer::new(*input_digest, *state_digest)),
                });
            }
            RunEvent::StepRouted { node, activated } => {
                let step = self.step_mut(node, name, &[Succeeded, Failed, Skipped])?;
                if step.routed {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                self.activate(activated)?;
                if let Some(step) = self.steps.get_mut(&ExecutionInstanceId::root(node.clone())) {
                    step.routed = true;
                }
            }
            RunEvent::StepReleased { node } => {
                self.step_mut(node, name, &[Pending])?.status = Ready;
            }
            RunEvent::StepLeased {
                node,
                attempt,
                generation,
            } => {
                let step = self.step_mut(node, name, &[Ready])?;
                if *attempt != step.attempt + 1 || *generation != step.generation + 1 {
                    return Err(ExecutionError::StaleAttempt { node: node.clone() });
                }
                step.status = Leased;
                step.attempt = *attempt;
                step.generation = *generation;
                step.cancel_requested = false;
                step.next_attempt_at = None;
            }
            RunEvent::StepStarted {
                node,
                attempt,
                generation,
            } => {
                self.fenced_step(node, name, &[Leased], *attempt, *generation)?
                    .status = Running;
            }
            RunEvent::StepCompleted {
                node,
                attempt,
                generation,
            } => {
                let step =
                    self.fenced_step(node, name, &[Leased, Running], *attempt, *generation)?;
                step.status = Succeeded;
                step.cancel_requested = false;
            }
            RunEvent::StepRetryScheduled {
                node,
                attempt,
                generation,
                code,
                not_before,
            } => {
                let step =
                    self.fenced_step(node, name, &[Leased, Running], *attempt, *generation)?;
                step.status = RetryWait;
                step.failure = Some(code.clone());
                step.next_attempt_at = Some(*not_before);
            }
            RunEvent::StepRetryDue { node } => {
                let step = self.step_mut(node, name, &[RetryWait])?;
                step.status = Ready;
                step.next_attempt_at = None;
            }
            RunEvent::StepFailed { node, code, fence } => {
                let step = self.step_mut(node, name, &[Leased, Running, Pending, Waiting])?;
                match (step.status.is_leased(), fence) {
                    (true, Some(f)) if step.fenced_by(f.attempt, f.generation) => {}
                    (true, _) => return Err(ExecutionError::StaleAttempt { node: node.clone() }),
                    (false, None) => {}
                    (false, Some(_)) => {
                        return Err(ExecutionError::IllegalTransition {
                            node: node.clone(),
                            status: step.status,
                            event: name,
                        });
                    }
                }
                step.status = Failed;
                step.failure = Some(code.clone());
                step.waiting = None;
                step.cancel_requested = false;
            }
            RunEvent::ConditionEvaluated { node, outcome } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Succeeded;
                step.condition = Some(*outcome);
            }
            RunEvent::SwitchEvaluated { node, selection } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Succeeded;
                step.switch_selection = Some(selection.clone());
            }
            RunEvent::TimerStarted { node, due_at } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Timer { due_at: *due_at });
            }
            RunEvent::TimerFired { node } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Timer { .. })) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Succeeded;
                step.waiting = None;
            }
            RunEvent::WaitStarted {
                node,
                correlation_key,
                deadline,
            } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Event {
                    correlation_key: correlation_key.clone(),
                    deadline: *deadline,
                });
            }
            RunEvent::WaitSatisfied { node } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Event { .. })) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Succeeded;
                step.waiting = None;
            }
            RunEvent::WaitTimedOut { node, handled } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Event { .. })) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                if *handled {
                    step.status = Succeeded;
                    step.timed_out = true;
                } else {
                    step.status = Failed;
                    step.failure = Some(FailureCode::wait_timed_out());
                }
            }
            RunEvent::ApprovalOpened {
                node, request_id, ..
            } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Approval {
                    request_id: *request_id,
                });
            }
            RunEvent::ApprovalDecided {
                node, request_id, ..
            }
            | RunEvent::ApprovalExpired {
                node, request_id, ..
            } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(
                    step.waiting,
                    Some(WaitingOn::Approval {
                        request_id: waiting_request,
                    }) if waiting_request == *request_id
                ) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Succeeded;
                step.waiting = None;
            }
            RunEvent::ParallelForked { node, branches } => {
                // Validate everything first: `activate` inserts nothing unless all branches are new.
                self.step_mut(node, name, &[Pending])?;
                self.activate(branches)?;
                if let Some(step) = self.steps.get_mut(&ExecutionInstanceId::root(node.clone())) {
                    step.status = Waiting;
                    step.waiting = Some(WaitingOn::Branches);
                }
            }
            RunEvent::ParallelJoined { node, outcome } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Branches)) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                match outcome {
                    JoinOutcome::Succeeded => step.status = Succeeded,
                    JoinOutcome::Failed => {
                        step.status = Failed;
                        step.failure = Some(FailureCode::branch_failed());
                    }
                }
            }
            event @ (RunEvent::CollectionEntered { node, .. }
            | RunEvent::CollectionIterationAdmitted { node, .. }
            | RunEvent::CollectionIterationReturned { node, .. }
            | RunEvent::CollectionIterationCheckpointed { node, .. }
            | RunEvent::CollectionDrainStarted { node, .. }
            | RunEvent::CollectionIterationDrained { node, .. }
            | RunEvent::CollectionFinished { node, .. }) => {
                let collection = ExecutionInstanceId::root(node.clone());
                self.apply_collection_event(&collection, event)?;
            }
            event @ (RunEvent::LoopEntered { node, .. }
            | RunEvent::LoopIterationAdmitted { node, .. }
            | RunEvent::LoopIterationReturned { node, .. }
            | RunEvent::LoopIterationCheckpointed { node, .. }
            | RunEvent::LoopDelayElapsed { node, .. }
            | RunEvent::LoopDrainStarted { node, .. }
            | RunEvent::LoopIterationDrained { node, .. }
            | RunEvent::LoopFinished { node, .. }) => {
                let loop_instance = ExecutionInstanceId::root(node.clone());
                self.apply_loop_event(&loop_instance, event)?;
            }
            RunEvent::ChildStarted { node, child } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Child { run: *child });
            }
            RunEvent::ChildFinished {
                node,
                child,
                outcome,
            } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if step.waiting != Some(WaitingOn::Child { run: *child }) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                step.cancel_requested = false;
                match outcome {
                    ChildOutcome::Succeeded => step.status = Succeeded,
                    ChildOutcome::Failed(code) => {
                        step.status = Failed;
                        step.failure = Some(code.clone());
                    }
                }
            }
            RunEvent::EndReached { node } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Succeeded;
                step.routed = true;
            }
            RunEvent::StepCancelRequested { node } => {
                let step = self.step_mut(node, name, &[Leased, Running, Waiting])?;
                // Only a step that waits for a child can be asked to stop while it is parked.
                if step.status == Waiting && !matches!(step.waiting, Some(WaitingOn::Child { .. }))
                {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                if step.cancel_requested {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.cancel_requested = true;
            }
            RunEvent::StepCancelled { node } => {
                let step = self.step_mut(
                    node,
                    name,
                    &[Pending, Ready, Leased, Running, Waiting, RetryWait],
                )?;
                step.status = Cancelled;
                step.routed = true;
                step.waiting = None;
                step.next_attempt_at = None;
                step.cancel_requested = false;
            }
            RunEvent::SagaEntered { node, begin } => {
                if self.sagas.contains_key(node) {
                    return Err(ExecutionError::AlreadyScheduled(node.clone()));
                }
                self.step_mut(node, name, &[Pending])?;
                self.activate(std::slice::from_ref(begin))?;
                if let Some(step) = self.steps.get_mut(&ExecutionInstanceId::root(node.clone())) {
                    step.status = Waiting;
                    step.waiting = Some(WaitingOn::Saga);
                }
                self.sagas.insert(node.clone(), SagaState::forward());
            }
            RunEvent::SagaCompleted { node } => {
                self.require_saga(node, name, SagaPhase::Forward)?;
                let step = self.step_mut(node, name, &[Waiting])?;
                step.status = Succeeded;
                step.waiting = None;
                if let Some(saga) = self.sagas.get_mut(node) {
                    saga.phase = SagaPhase::Done;
                }
            }
            RunEvent::SagaAborting { node, cause } => {
                self.require_saga(node, name, SagaPhase::Forward)?;
                let during_stop = self.stop.is_some();
                if let Some(saga) = self.sagas.get_mut(node) {
                    saga.phase = SagaPhase::Aborting;
                    saga.cause = Some(cause.clone());
                    saga.during_stop = during_stop;
                }
            }
            RunEvent::CompensationPlanned {
                node,
                entries,
                irreversible,
            } => {
                self.require_saga(node, name, SagaPhase::Aborting)?;
                let named: std::collections::BTreeSet<&NodeId> =
                    entries.iter().map(|e| &e.node).collect();
                let coherent = named.len() == entries.len()
                    && entries
                        .iter()
                        .all(|e| e.after.iter().all(|a| named.contains(a) && *a != e.node));
                if !coherent {
                    return Err(ExecutionError::Invariant(
                        "a compensation plan names an entry twice or waits for one that is not in it",
                    ));
                }
                let companions: Vec<NodeId> =
                    entries.iter().map(|e| e.node.compensation()).collect();
                self.activate(&companions)?;
                if let Some(saga) = self.sagas.get_mut(node) {
                    saga.phase = SagaPhase::Compensating;
                    saga.entries = entries
                        .iter()
                        .map(|e| (e.node.clone(), e.after.clone()))
                        .collect();
                    saga.irreversible.clone_from(irreversible);
                }
            }
            RunEvent::SagaCompensated { node, outcome } => {
                self.require_saga(node, name, SagaPhase::Compensating)?;
                if let Some(saga) = self.sagas.get(node) {
                    let settled = saga.entries.keys().all(|forward| {
                        self.steps
                            .get(&ExecutionInstanceId::root(forward.compensation()))
                            .is_some_and(|s| s.status.is_terminal())
                    });
                    if !settled {
                        return Err(ExecutionError::Invariant(
                            "a saga cannot end while one of its compensations is still active",
                        ));
                    }
                    if self.expected_outcome(saga) != *outcome {
                        return Err(ExecutionError::Invariant(
                            "the outcome of a saga must follow from how its compensations ended",
                        ));
                    }
                }
                let cause = self.sagas.get(node).and_then(|s| s.cause.clone());
                let stopping = self.stop.is_some();
                let step = self.step_mut(node, name, &[Waiting])?;
                step.status = Failed;
                step.waiting = None;
                step.failure = cause;
                if let Some(saga) = self.sagas.get_mut(node) {
                    saga.phase = SagaPhase::Done;
                    saga.outcome = Some(*outcome);
                    // A saga that ends while the run is being stopped is part of how the run ends,
                    // whichever of the two started it.
                    saga.during_stop |= stopping;
                }
            }
            RunEvent::ReconciliationRequested { node, .. }
            | RunEvent::ReconciliationConcluded { node, .. }
            | RunEvent::StepNeedsReview { node, .. }
            | RunEvent::StepRetryRequested { node }
            | RunEvent::StepSkipped { node }
            | RunEvent::StepResolved { node, .. } => {
                self.apply_effect_event(&ExecutionInstanceId::root(node.clone()), event)?;
            }
            RunEvent::ScopeCompensationPlanned { node, .. }
            | RunEvent::ScopeCompensated { node, .. } => {
                self.apply_scope_saga_event(&ExecutionInstanceId::root(node.clone()), event)?;
            }
            RunEvent::StateUpdated {
                before_revision,
                after_revision,
                ..
            } => {
                if !matches!(self.status, RunStatus::Running | RunStatus::Waiting)
                    || before_revision.checked_add(1) != Some(*after_revision)
                {
                    return Err(ExecutionError::Invariant(
                        "a state update advances one revision on a mutable run",
                    ));
                }
            }
            RunEvent::RunContinued {
                successor,
                input_digest,
                state_digest,
            } => {
                if self.stop.is_some()
                    || self.paused
                    || !self
                        .steps
                        .values()
                        .all(|step| step.status == Succeeded && step.routed)
                {
                    return Err(ExecutionError::Invariant(
                        "a run continues only after every obligation is quiescent",
                    ));
                }
                let chain = self.chain.as_mut().ok_or(ExecutionError::Invariant(
                    "a run must belong to a chain before it continues",
                ))?;
                if chain.completion_kind.is_some()
                    || *successor == self.run_id
                    || chain.segment_index.get() + 1 >= chain.max_segments
                {
                    return Err(ExecutionError::Invariant(
                        "a chain segment can continue exactly once within its bound",
                    ));
                }
                chain.successor = Some(*successor);
                chain.completion_kind = Some(SegmentCompletionKind::Continued);
                chain.transfer = Some(ContinuationTransfer::new(*input_digest, *state_digest));
            }
            RunEvent::ContinuationSelected {
                node,
                successor,
                input_digest,
                state_digest,
            } => {
                let boundary = ExecutionInstanceId::root(node.clone());
                if self.stop.is_some()
                    || self.paused
                    || self.steps.iter().any(|(instance, step)| {
                        instance != &boundary && (step.status != Succeeded || !step.routed)
                    })
                {
                    return Err(ExecutionError::Invariant(
                        "a continuation requires every other obligation to be quiescent",
                    ));
                }
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Succeeded;
                step.routed = true;
                let chain = self.chain.as_mut().ok_or(ExecutionError::Invariant(
                    "a run must belong to a chain before it continues",
                ))?;
                if chain.completion_kind.is_some()
                    || *successor == self.run_id
                    || chain.segment_index.get() + 1 >= chain.max_segments
                {
                    return Err(ExecutionError::Invariant(
                        "a chain segment can continue exactly once within its bound",
                    ));
                }
                chain.successor = Some(*successor);
                chain.completion_kind = Some(SegmentCompletionKind::Continued);
                chain.transfer = Some(ContinuationTransfer::new(*input_digest, *state_digest));
            }
            RunEvent::RunPaused => {
                if self.paused {
                    return Err(ExecutionError::AlreadyPaused);
                }
                if self.stop.is_some() {
                    return Err(ExecutionError::AlreadyStopping);
                }
                self.paused = true;
            }
            RunEvent::RunResumed => {
                if !self.paused {
                    return Err(ExecutionError::NotPaused);
                }
                self.paused = false;
            }
            RunEvent::RunStopRequested { reason, code } => {
                if self.stop.is_some() {
                    return Err(ExecutionError::AlreadyStopping);
                }
                self.stop = Some((*reason, code.clone()));
                // Stopping supersedes a pause: what the run still has to do (undo what it did) needs
                // its steps to be leased, and nobody is going to resume a run that is ending.
                self.paused = false;
            }
            RunEvent::RunFinished { status } => {
                self.check_finish(*status)?;
                if let Some(chain) = self.chain.as_mut() {
                    chain.completion_kind = Some(SegmentCompletionKind::Final);
                }
            }
        }
        self.applied += 1;
        self.refresh_status(event);
        Ok(())
    }

    /// Applies a journal event at its explicit logical instance address.
    ///
    /// Root events retain the complete existing reducer behavior. Scoped collection control and
    /// activity lifecycle use the same typed address; other scoped transitions stay gated behind
    /// durable planner integration.
    ///
    /// # Errors
    /// An [`ExecutionError`] when the addressed transition is illegal or not enabled.
    pub fn apply_addressed(&mut self, addressed: &AddressedRunEvent) -> Result<(), ExecutionError> {
        use StepStatus::{Failed, Pending, Skipped, Succeeded, Waiting};

        if addressed.path().is_root() {
            return self.apply(addressed.event());
        }
        if self.status.is_terminal() {
            return Err(ExecutionError::RunFinished(self.status));
        }
        let instance = addressed.instance().ok_or(ExecutionError::Invariant(
            "a scoped journal address has no execution instance",
        ))?;
        let name = addressed.event().name();
        match addressed.event() {
            RunEvent::StepRouted { activated, .. } => {
                {
                    let step = self.instance_mut(&instance, name, &[Succeeded, Failed, Skipped])?;
                    if step.routed {
                        return Err(ExecutionError::IllegalInstanceTransition {
                            instance,
                            status: step.status,
                            event: name,
                        });
                    }
                }
                self.activate_in_path(addressed.path(), activated)?;
                self.steps
                    .get_mut(&instance)
                    .ok_or(ExecutionError::Invariant(
                        "validated scoped step disappeared",
                    ))?
                    .routed = true;
            }
            event @ (RunEvent::StepReleased { .. }
            | RunEvent::StepLeased { .. }
            | RunEvent::StepStarted { .. }
            | RunEvent::StepCompleted { .. }
            | RunEvent::StepRetryScheduled { .. }
            | RunEvent::StepRetryDue { .. }
            | RunEvent::StepFailed { .. }
            | RunEvent::ConditionEvaluated { .. }
            | RunEvent::SwitchEvaluated { .. }
            | RunEvent::StepCancelRequested { .. }
            | RunEvent::StepCancelled { .. }) => {
                self.steps
                    .get_mut(&instance)
                    .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?
                    .apply_instance_lifecycle(&instance, event)?;
            }
            RunEvent::ApprovalOpened { request_id, .. } => {
                let step = self.instance_mut(&instance, name, &[Pending])?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Approval {
                    request_id: *request_id,
                });
            }
            RunEvent::ApprovalDecided { request_id, .. }
            | RunEvent::ApprovalExpired { request_id, .. } => {
                let step = self.instance_mut(&instance, name, &[Waiting])?;
                if !matches!(
                    step.waiting,
                    Some(WaitingOn::Approval {
                        request_id: waiting_request,
                    }) if waiting_request == *request_id
                ) {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance,
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Succeeded;
                step.waiting = None;
            }
            event @ (RunEvent::ReconciliationRequested { .. }
            | RunEvent::ReconciliationConcluded { .. }
            | RunEvent::StepNeedsReview { .. }
            | RunEvent::StepRetryRequested { .. }
            | RunEvent::StepSkipped { .. }
            | RunEvent::StepResolved { .. }) => {
                self.apply_effect_event(&instance, event)?;
            }
            event @ (RunEvent::ScopeCompensationPlanned { .. }
            | RunEvent::ScopeCompensated { .. }) => {
                self.apply_scope_saga_event(&instance, event)?;
            }
            event @ (RunEvent::CollectionEntered { .. }
            | RunEvent::CollectionIterationAdmitted { .. }
            | RunEvent::CollectionIterationReturned { .. }
            | RunEvent::CollectionIterationCheckpointed { .. }
            | RunEvent::CollectionDrainStarted { .. }
            | RunEvent::CollectionIterationDrained { .. }
            | RunEvent::CollectionFinished { .. }) => {
                self.apply_collection_event(&instance, event)?;
            }
            event @ (RunEvent::LoopEntered { .. }
            | RunEvent::LoopIterationAdmitted { .. }
            | RunEvent::LoopIterationReturned { .. }
            | RunEvent::LoopIterationCheckpointed { .. }
            | RunEvent::LoopDelayElapsed { .. }
            | RunEvent::LoopDrainStarted { .. }
            | RunEvent::LoopIterationDrained { .. }
            | RunEvent::LoopFinished { .. }) => {
                self.apply_loop_event(&instance, event)?;
            }
            _ => return Err(ExecutionError::UnsupportedScopedEvent(name)),
        }
        self.applied += 1;
        self.refresh_status(addressed.event());
        Ok(())
    }

    /// Reconciliation, operator holds and operator decisions about one logical instance. The root
    /// reducer and the scoped one are this function, so the two cannot drift apart: a scoped step
    /// differs only in the path its derived companions share.
    fn apply_effect_event(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), ExecutionError> {
        effects::apply_effect_event(self, instance, event)
    }

    /// The scope-level events of a compensating collection or loop (D380..D385).
    fn apply_scope_saga_event(
        &mut self,
        owner: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), ExecutionError> {
        let name = event.name();
        match event {
            RunEvent::ScopeCompensationPlanned {
                cause,
                entries,
                irreversible,
                ..
            } => {
                if self.scope_sagas.contains_key(owner) {
                    return Err(ExecutionError::Invariant(
                        "a scope plans its compensation once",
                    ));
                }
                // The plan is made after the drain has settled every iteration, never before.
                let settled = if let Some(collection) = self.collections.get(owner) {
                    collection.phase == CollectionPhase::Draining
                        && !collection
                            .iterations
                            .values()
                            .any(CollectionIterationState::is_active)
                } else if let Some(scope) = self.loops.get(owner) {
                    scope.phase == LoopPhase::Draining && scope.active.is_none()
                } else {
                    return Err(ExecutionError::UnknownInstance(owner.clone()));
                };
                if !settled {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance: owner.clone(),
                        status: self
                            .steps
                            .get(owner)
                            .map_or(StepStatus::Pending, |s| s.status),
                        event: name,
                    });
                }
                let named: std::collections::BTreeSet<&ExecutionInstanceId> =
                    entries.iter().map(|entry| &entry.instance).collect();
                let coherent = named.len() == entries.len()
                    && entries.iter().all(|entry| {
                        entry
                            .after
                            .iter()
                            .all(|earlier| named.contains(earlier) && earlier != &entry.instance)
                    });
                if !coherent {
                    return Err(ExecutionError::Invariant(
                        "a scope plan names an instance twice or waits for one that is not in it",
                    ));
                }
                // Every entry and every irreversible instance is a member of an iteration of this
                // scope that finished its effect, or may have: nothing else can be undone.
                for instance in entries
                    .iter()
                    .map(|entry| &entry.instance)
                    .chain(irreversible.iter())
                {
                    if !Self::is_member_of(owner, instance) {
                        return Err(ExecutionError::Invariant(
                            "a scope plan names an instance outside the scope",
                        ));
                    }
                    let step = self
                        .steps
                        .get(instance)
                        .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
                    let unknown = matches!(step.status, StepStatus::Cancelled | StepStatus::Failed)
                        && step
                            .failure
                            .as_ref()
                            .is_some_and(FailureCode::leaves_outcome_unknown);
                    if step.status != StepStatus::Succeeded && !unknown {
                        return Err(ExecutionError::Invariant(
                            "a scope plan names an instance that has no effect to undo",
                        ));
                    }
                }
                for entry in entries {
                    if self.steps.contains_key(&entry.instance.compensation()) {
                        return Err(ExecutionError::AlreadyScheduled(
                            entry.instance.node().compensation(),
                        ));
                    }
                }
                for entry in entries {
                    self.steps
                        .insert(entry.instance.compensation(), StepState::pending());
                }
                self.scope_sagas.insert(
                    owner.clone(),
                    ScopeSagaState {
                        phase: SagaPhase::Compensating,
                        cause: cause.clone(),
                        entries: entries
                            .iter()
                            .map(|entry| (entry.instance.clone(), entry.after.clone()))
                            .collect(),
                        irreversible: irreversible.clone(),
                        outcome: None,
                        during_stop: self.stop.is_some(),
                    },
                );
            }
            RunEvent::ScopeCompensated { outcome, .. } => {
                let saga = self
                    .scope_sagas
                    .get(owner)
                    .filter(|saga| saga.phase == SagaPhase::Compensating)
                    .ok_or(ExecutionError::Invariant(
                        "a scope ends its compensation only while it compensates",
                    ))?;
                let settled = saga.entries.keys().all(|forward| {
                    self.steps
                        .get(&forward.compensation())
                        .is_some_and(|step| step.status.is_terminal())
                });
                if !settled {
                    return Err(ExecutionError::Invariant(
                        "a scope cannot end while one of its compensations is still active",
                    ));
                }
                if self.expected_scope_outcome(saga) != *outcome {
                    return Err(ExecutionError::Invariant(
                        "the outcome of a scope must follow from how its compensations ended",
                    ));
                }
                let stopping = self.stop.is_some();
                if let Some(saga) = self.scope_sagas.get_mut(owner) {
                    saga.phase = SagaPhase::Done;
                    saga.outcome = Some(*outcome);
                    saga.during_stop |= stopping;
                }
            }
            _ => {
                return Err(ExecutionError::Invariant(
                    "a non-scope event reached the scope compensation reducer",
                ));
            }
        }
        Ok(())
    }

    /// Whether `instance` runs directly in an iteration of the scope `owner`.
    fn is_member_of(owner: &ExecutionInstanceId, instance: &ExecutionInstanceId) -> bool {
        let segments = instance.path().segments();
        segments.split_last().is_some_and(|(last, parents)| {
            last.scope_node() == owner.node() && parents == owner.path().segments()
        })
    }

    /// How a scope ends, judged from its compensations: failed or cancelled ones make it
    /// `COMPENSATION_FAILED`, skipped ones or an irreversible effect make it `PARTIAL`.
    pub(super) fn expected_scope_outcome(&self, saga: &ScopeSagaState) -> SagaOutcome {
        let (mut failed, mut skipped) = (false, false);
        for forward in saga.entries.keys() {
            match self.steps.get(&forward.compensation()).map(|s| s.status) {
                Some(StepStatus::Failed | StepStatus::Cancelled) => failed = true,
                Some(StepStatus::Skipped) => skipped = true,
                _ => {}
            }
        }
        if failed {
            SagaOutcome::CompensationFailed
        } else if skipped || !saga.irreversible.is_empty() {
            SagaOutcome::Partial
        } else {
            SagaOutcome::Compensated
        }
    }

    fn check_finish(&self, status: RunStatus) -> Result<(), ExecutionError> {
        if !self.steps.values().all(|s| s.status.is_terminal()) {
            return Err(ExecutionError::Invariant(
                "a run cannot finish while a step is still active",
            ));
        }
        match &self.stop {
            Some((reason, _)) if self.final_status_for(*reason) == status => Ok(()),
            Some(_) => Err(ExecutionError::Invariant(
                "the final status must match the reason the run was stopped",
            )),
            None if status == RunStatus::Succeeded => {
                if self.steps.values().all(|s| s.routed) {
                    Ok(())
                } else {
                    Err(ExecutionError::Invariant(
                        "a run cannot succeed while a finished step is not routed",
                    ))
                }
            }
            None => Err(ExecutionError::Invariant(
                "a run that was not stopped can only finish as succeeded",
            )),
        }
    }

    fn refresh_status(&mut self, event: &RunEvent) {
        match event {
            RunEvent::RunFinished { status } => {
                self.status = *status;
                return;
            }
            RunEvent::RunContinued { .. } | RunEvent::ContinuationSelected { .. } => {
                self.status = RunStatus::Succeeded;
                return;
            }
            _ => {}
        }
        self.status = RunWorkSummary::from_steps(self.steps.values()).status(
            self.compensation_in_progress(),
            self.stop.is_some(),
            self.paused,
        );
    }

    pub(super) fn compensation_in_progress(&self) -> bool {
        self.sagas
            .values()
            .any(|saga| matches!(saga.phase, SagaPhase::Aborting | SagaPhase::Compensating))
            || self
                .scope_sagas
                .values()
                .any(|saga| saga.phase == SagaPhase::Compensating)
    }
}
