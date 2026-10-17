//! Pure rules for bounded continuation chains and durable child-chain links.
//!
//! This module decides whether a proposed transition is valid and records the resulting logical
//! state. It does not claim durability: an adapter must serialize the chain before its current run
//! and commit the run transition, successor, lineage, quota charges, audit and outbox atomically.

use time::OffsetDateTime;

use crate::definition::WorkflowVersionId;
use crate::identity::{NamespaceId, Subject, TenantId};
use crate::workflow::Digest;

use super::{ExecutionInstanceId, RunId, RunState, SagaPhase, StepState, StepStatus, WaitingOn};
use crate::workflow::NodeId;

/// Absolute definition ceiling, including the initial run at index zero.
pub const MAX_CHAIN_SEGMENTS: u16 = 10_000;

/// Most runs one tree may hold (a root run and all its descendants). The successors of a child
/// chain are runs of the parent's tree and spend the same budget (ADR-0030).
pub const MAX_TREE_RUNS: usize = 1_024;

/// Tenant and namespace coordinates that every member of one chain retains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainScope {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
}

impl ChainScope {
    #[must_use]
    pub const fn new(tenant_id: TenantId, namespace_id: NamespaceId) -> Self {
        Self {
            tenant_id,
            namespace_id,
        }
    }

    #[must_use]
    pub const fn tenant_id(self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(self) -> NamespaceId {
        self.namespace_id
    }
}

/// Per-definition chain bound captured when the initial run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainPolicy {
    max_segments: u16,
}

impl ChainPolicy {
    /// Validates a configured segment limit.
    ///
    /// # Errors
    /// Returns [`ChainError::InvalidSegmentLimit`] unless the limit is between one and 10,000.
    pub const fn new(max_segments: u16) -> Result<Self, ChainError> {
        if max_segments == 0 || max_segments > MAX_CHAIN_SEGMENTS {
            Err(ChainError::InvalidSegmentLimit)
        } else {
            Ok(Self { max_segments })
        }
    }

    #[must_use]
    pub const fn max_segments(self) -> u16 {
        self.max_segments
    }
}

/// Zero-based position of one immutable run in its chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegmentIndex(u16);

impl SegmentIndex {
    /// Validates an index against the absolute chain ceiling.
    ///
    /// # Errors
    /// Returns [`ChainError::InvalidSegmentIndex`] for an index greater than 9,999.
    pub const fn new(index: u16) -> Result<Self, ChainError> {
        if index >= MAX_CHAIN_SEGMENTS {
            Err(ChainError::InvalidSegmentIndex)
        } else {
            Ok(Self(index))
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Explicit values selected for a successor. Omitted state remains omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationTransfer {
    input: Digest,
    state: Option<Digest>,
}

impl ContinuationTransfer {
    #[must_use]
    pub const fn new(input: Digest, state: Option<Digest>) -> Self {
        Self { input, state }
    }

    #[must_use]
    pub const fn input(&self) -> Digest {
        self.input
    }

    #[must_use]
    pub const fn state(&self) -> Option<Digest> {
        self.state
    }
}

/// Outstanding work that makes a continuation boundary unsafe.
///
/// A blocker is a reason to wait. The boundary stays pending until the work settles or the run's
/// original deadline ends it; it is never recorded as a terminal outcome (D296).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuiescenceBlocker {
    SiblingWork,
    Child,
    Lease,
    Timer,
    Subscription,
    Approval,
    UnknownEffect,
    Compensation,
}

impl QuiescenceBlocker {
    /// Every blocker, most significant first. When a run holds several kinds of open work, the
    /// first of this list is the one reported, so the answer does not depend on map order.
    pub const ALL: [Self; 8] = [
        Self::Compensation,
        Self::UnknownEffect,
        Self::Approval,
        Self::Child,
        Self::Lease,
        Self::Timer,
        Self::Subscription,
        Self::SiblingWork,
    ];

    /// A bounded, stable word for logs and metric labels.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SiblingWork => "sibling_work",
            Self::Child => "child",
            Self::Lease => "lease",
            Self::Timer => "timer",
            Self::Subscription => "subscription",
            Self::Approval => "approval",
            Self::UnknownEffect => "unknown_effect",
            Self::Compensation => "compensation",
        }
    }

    /// The obligation one step still holds, or `None` when it is finished and routed.
    #[must_use]
    pub const fn of_step(step: &StepState) -> Option<Self> {
        if matches!(step.status(), StepStatus::Succeeded) && step.routed() {
            return None;
        }
        Some(match (step.status(), step.waiting_on()) {
            (StepStatus::Leased | StepStatus::Running, _) => Self::Lease,
            (StepStatus::Waiting, Some(WaitingOn::Timer { .. })) => Self::Timer,
            (StepStatus::Waiting, Some(WaitingOn::Event { .. })) => Self::Subscription,
            (StepStatus::Waiting, Some(WaitingOn::Approval { .. })) => Self::Approval,
            (StepStatus::Waiting, Some(WaitingOn::Child { .. })) => Self::Child,
            (StepStatus::Waiting, Some(WaitingOn::Reconciliation)) => Self::UnknownEffect,
            _ => Self::SiblingWork,
        })
    }
}

/// Result of the boundary's bounded obligation check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quiescence {
    blocker: Option<QuiescenceBlocker>,
}

impl Quiescence {
    #[must_use]
    pub const fn ready() -> Self {
        Self { blocker: None }
    }

    #[must_use]
    pub const fn blocked(blocker: QuiescenceBlocker) -> Self {
        Self {
            blocker: Some(blocker),
        }
    }

    #[must_use]
    pub const fn blocker(self) -> Option<QuiescenceBlocker> {
        self.blocker
    }

    /// Reads the run's own state: every step other than the boundary must be finished and routed,
    /// and every saga must be done. The most significant open obligation is the blocker.
    #[must_use]
    pub fn of_run(state: &RunState, boundary: &NodeId) -> Self {
        let open = state
            .instances()
            .filter(|(instance, _)| !(instance.path().is_root() && instance.node() == boundary))
            .filter_map(|(_, step)| QuiescenceBlocker::of_step(step))
            .chain(
                state
                    .sagas()
                    .filter(|(_, saga)| saga.phase() != SagaPhase::Done)
                    .map(|_| QuiescenceBlocker::Compensation),
            );
        let most_significant = open.min_by_key(|blocker| {
            QuiescenceBlocker::ALL
                .iter()
                .position(|candidate| candidate == blocker)
        });
        most_significant.map_or_else(Self::ready, Self::blocked)
    }
}

/// A handoff proposal evaluated against the serialized chain head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationProposal {
    scope: ChainScope,
    expected_head: RunId,
    expected_revision: u64,
    successor: RunId,
    definition_version: WorkflowVersionId,
    deadline: OffsetDateTime,
    transfer: ContinuationTransfer,
    quiescence: Quiescence,
}

impl ContinuationProposal {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        scope: ChainScope,
        expected_head: RunId,
        expected_revision: u64,
        successor: RunId,
        definition_version: WorkflowVersionId,
        deadline: OffsetDateTime,
        transfer: ContinuationTransfer,
        quiescence: Quiescence,
    ) -> Self {
        Self {
            scope,
            expected_head,
            expected_revision,
            successor,
            definition_version,
            deadline,
            transfer,
            quiescence,
        }
    }

    pub fn set_scope(&mut self, scope: ChainScope) {
        self.scope = scope;
    }

    pub fn set_expected_head(&mut self, expected_head: RunId) {
        self.expected_head = expected_head;
    }

    pub const fn set_expected_revision(&mut self, expected_revision: u64) {
        self.expected_revision = expected_revision;
    }

    pub fn set_definition_version(&mut self, definition_version: WorkflowVersionId) {
        self.definition_version = definition_version;
    }

    pub const fn set_deadline(&mut self, deadline: OffsetDateTime) {
        self.deadline = deadline;
    }

    pub const fn set_quiescence(&mut self, quiescence: Quiescence) {
        self.quiescence = quiescence;
    }
}

/// How an immutable segment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentCompletionKind {
    Final,
    Continued,
}

/// Terminal process outcome. A continued segment has no final process outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFinalOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Compensated,
    Partial,
    CompensationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SegmentCompletion {
    Continued {
        successor: RunId,
        transfer: ContinuationTransfer,
    },
    Final(ChainFinalOutcome),
}

/// Immutable chain membership and the completion link of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    index: SegmentIndex,
    run_id: RunId,
    predecessor: Option<RunId>,
    completion: Option<SegmentCompletion>,
}

impl Segment {
    #[must_use]
    pub const fn index(&self) -> SegmentIndex {
        self.index
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn predecessor(&self) -> Option<RunId> {
        self.predecessor
    }

    #[must_use]
    pub const fn successor(&self) -> Option<RunId> {
        match &self.completion {
            Some(SegmentCompletion::Continued { successor, .. }) => Some(*successor),
            Some(SegmentCompletion::Final(_)) | None => None,
        }
    }

    #[must_use]
    pub const fn completion_kind(&self) -> Option<SegmentCompletionKind> {
        match self.completion {
            Some(SegmentCompletion::Continued { .. }) => Some(SegmentCompletionKind::Continued),
            Some(SegmentCompletion::Final(_)) => Some(SegmentCompletionKind::Final),
            None => None,
        }
    }

    #[must_use]
    pub const fn transfer(&self) -> Option<&ContinuationTransfer> {
        match &self.completion {
            Some(SegmentCompletion::Continued { transfer, .. }) => Some(transfer),
            Some(SegmentCompletion::Final(_)) | None => None,
        }
    }

    #[must_use]
    pub const fn final_outcome(&self) -> Option<ChainFinalOutcome> {
        match self.completion {
            Some(SegmentCompletion::Final(outcome)) => Some(outcome),
            Some(SegmentCompletion::Continued { .. }) | None => None,
        }
    }
}

/// Result of one accepted or replayed handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainHandoff {
    predecessor: RunId,
    successor: RunId,
    successor_index: SegmentIndex,
    replayed: bool,
}

impl ChainHandoff {
    #[must_use]
    pub const fn predecessor(self) -> RunId {
        self.predecessor
    }

    #[must_use]
    pub const fn successor(self) -> RunId {
        self.successor
    }

    #[must_use]
    pub const fn successor_index(self) -> SegmentIndex {
        self.successor_index
    }

    #[must_use]
    pub const fn replayed(self) -> bool {
        self.replayed
    }
}

/// Result of setting or replaying the chain cancellation latch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelResolution {
    target_run_id: RunId,
    changed: bool,
}

impl CancelResolution {
    #[must_use]
    pub const fn target_run_id(self) -> RunId {
        self.target_run_id
    }

    #[must_use]
    pub const fn changed(self) -> bool {
        self.changed
    }
}

/// Result of committing or replaying a final chain outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationResolution {
    outcome: ChainFinalOutcome,
    replayed: bool,
}

impl FinalizationResolution {
    #[must_use]
    pub const fn outcome(self) -> ChainFinalOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn replayed(self) -> bool {
        self.replayed
    }
}

/// One bounded chain, identified by its initial run id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionChain {
    scope: ChainScope,
    chain_id: RunId,
    current_run_id: RunId,
    revision: u64,
    policy: ChainPolicy,
    original_deadline: OffsetDateTime,
    definition_version: WorkflowVersionId,
    initiator: Subject,
    cancel_requested: bool,
    final_outcome: Option<ChainFinalOutcome>,
    segments: Vec<Segment>,
}

impl ExecutionChain {
    #[must_use]
    pub fn start(
        scope: ChainScope,
        initial_run_id: RunId,
        definition_version: WorkflowVersionId,
        initiator: Subject,
        original_deadline: OffsetDateTime,
        policy: ChainPolicy,
    ) -> Self {
        Self {
            scope,
            chain_id: initial_run_id,
            current_run_id: initial_run_id,
            revision: 0,
            policy,
            original_deadline,
            definition_version,
            initiator,
            cancel_requested: false,
            final_outcome: None,
            segments: vec![Segment {
                index: SegmentIndex(0),
                run_id: initial_run_id,
                predecessor: None,
                completion: None,
            }],
        }
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn chain_id(&self) -> RunId {
        self.chain_id
    }

    #[must_use]
    pub const fn current_run_id(&self) -> RunId {
        self.current_run_id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn policy(&self) -> ChainPolicy {
        self.policy
    }

    #[must_use]
    pub const fn original_deadline(&self) -> OffsetDateTime {
        self.original_deadline
    }

    #[must_use]
    pub const fn definition_version(&self) -> WorkflowVersionId {
        self.definition_version
    }

    #[must_use]
    pub const fn initiator(&self) -> &Subject {
        &self.initiator
    }

    #[must_use]
    pub const fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    #[must_use]
    pub const fn final_outcome(&self) -> Option<ChainFinalOutcome> {
        self.final_outcome
    }

    #[must_use]
    pub fn segment_count(&self) -> u16 {
        self.segments.len() as u16
    }

    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Applies one logical handoff or returns the original decision for an identical retry.
    ///
    /// The caller must evaluate this while holding the durable chain serialization boundary. This
    /// method deliberately does not schedule work or imply that any record was committed.
    ///
    /// # Errors
    /// Returns a typed error when scope, head, revision, version, deadline, quiescence, cancellation
    /// or a finite limit rejects the handoff.
    pub fn continue_as_new(
        &mut self,
        proposal: ContinuationProposal,
    ) -> Result<ChainHandoff, ChainError> {
        self.validate_inherited_fields(&proposal)?;

        if proposal.expected_head != self.current_run_id {
            return self.replay_or_reject_handoff(&proposal);
        }
        if self.final_outcome.is_some() {
            return Err(ChainError::ChainFinal);
        }
        if self.cancel_requested {
            return Err(ChainError::CancellationRequested);
        }
        if proposal.expected_revision != self.revision {
            return Err(ChainError::RevisionChanged {
                expected: proposal.expected_revision,
                actual: self.revision,
            });
        }
        if let Some(blocker) = proposal.quiescence.blocker() {
            return Err(ChainError::NotQuiescent(blocker));
        }
        if self.segments.len() >= usize::from(self.policy.max_segments) {
            return Err(ChainError::SegmentLimitExceeded);
        }
        if self
            .segments
            .iter()
            .any(|segment| segment.run_id == proposal.successor)
        {
            return Err(ChainError::DuplicateRun);
        }

        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ChainError::RevisionExhausted)?;
        let index = SegmentIndex::new(self.segment_count())?;
        let predecessor = self.current_run_id;
        let Some(current) = self.segments.last_mut() else {
            return Err(ChainError::InvalidChainState);
        };
        current.completion = Some(SegmentCompletion::Continued {
            successor: proposal.successor,
            transfer: proposal.transfer,
        });
        self.segments.push(Segment {
            index,
            run_id: proposal.successor,
            predecessor: Some(predecessor),
            completion: None,
        });
        self.current_run_id = proposal.successor;
        self.revision = next_revision;

        Ok(ChainHandoff {
            predecessor,
            successor: proposal.successor,
            successor_index: index,
            replayed: false,
        })
    }

    /// Sets a durable cancellation latch for the current head.
    ///
    /// # Errors
    /// Returns a typed CAS or terminal error without changing the chain.
    pub fn request_cancel(
        &mut self,
        expected_head: RunId,
        expected_revision: u64,
    ) -> Result<CancelResolution, ChainError> {
        if expected_head != self.current_run_id {
            return Err(ChainError::HeadChanged {
                current: self.current_run_id,
            });
        }
        if self.final_outcome.is_some() {
            return Err(ChainError::ChainFinal);
        }
        if self.cancel_requested {
            return Ok(CancelResolution {
                target_run_id: self.current_run_id,
                changed: false,
            });
        }
        if expected_revision != self.revision {
            return Err(ChainError::RevisionChanged {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ChainError::RevisionExhausted)?;
        self.cancel_requested = true;
        Ok(CancelResolution {
            target_run_id: self.current_run_id,
            changed: true,
        })
    }

    /// Commits the first final process outcome for the current segment.
    ///
    /// # Errors
    /// Returns a typed CAS, terminal-conflict or revision-exhaustion error without mutation.
    pub fn finish(
        &mut self,
        expected_head: RunId,
        expected_revision: u64,
        outcome: ChainFinalOutcome,
    ) -> Result<FinalizationResolution, ChainError> {
        if expected_head != self.current_run_id {
            return Err(ChainError::HeadChanged {
                current: self.current_run_id,
            });
        }
        if let Some(committed) = self.final_outcome {
            return if committed == outcome {
                Ok(FinalizationResolution {
                    outcome,
                    replayed: true,
                })
            } else {
                Err(ChainError::FinalOutcomeConflict)
            };
        }
        if expected_revision != self.revision {
            return Err(ChainError::RevisionChanged {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ChainError::RevisionExhausted)?;
        let Some(current) = self.segments.last_mut() else {
            return Err(ChainError::InvalidChainState);
        };
        current.completion = Some(SegmentCompletion::Final(outcome));
        self.final_outcome = Some(outcome);
        self.revision = next_revision;
        Ok(FinalizationResolution {
            outcome,
            replayed: false,
        })
    }

    fn validate_inherited_fields(&self, proposal: &ContinuationProposal) -> Result<(), ChainError> {
        if proposal.scope != self.scope {
            return Err(ChainError::ScopeMismatch);
        }
        if proposal.definition_version != self.definition_version {
            return Err(ChainError::DefinitionVersionChanged);
        }
        if proposal.deadline != self.original_deadline {
            return Err(ChainError::DeadlineChanged);
        }
        Ok(())
    }

    fn replay_or_reject_handoff(
        &self,
        proposal: &ContinuationProposal,
    ) -> Result<ChainHandoff, ChainError> {
        let Some(predecessor) = self
            .segments
            .iter()
            .find(|segment| segment.run_id == proposal.expected_head)
        else {
            return Err(ChainError::HeadChanged {
                current: self.current_run_id,
            });
        };
        let Some(SegmentCompletion::Continued {
            successor,
            transfer,
        }) = &predecessor.completion
        else {
            return Err(ChainError::HeadChanged {
                current: self.current_run_id,
            });
        };
        if *successor != proposal.successor {
            return Err(ChainError::HeadChanged {
                current: self.current_run_id,
            });
        }
        if transfer != &proposal.transfer {
            return Err(ChainError::ContinuationConflict);
        }
        let successor_index = self
            .segments
            .iter()
            .find(|segment| segment.run_id == *successor)
            .map(|segment| segment.index)
            .ok_or(ChainError::InvalidChainState)?;
        Ok(ChainHandoff {
            predecessor: predecessor.run_id,
            successor: *successor,
            successor_index,
            replayed: true,
        })
    }
}

/// What the parent can infer from its linked child chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildObservation {
    Waiting { head: RunId },
    Final(ChainFinalOutcome),
}

/// Durable relationship between the original parent instance and a child chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildChainLink {
    scope: ChainScope,
    parent_run_id: RunId,
    parent_instance: ExecutionInstanceId,
    child_chain_id: RunId,
    cancellation_requested: bool,
    final_outcome: Option<ChainFinalOutcome>,
    settled: bool,
}

impl ChildChainLink {
    /// Creates a same-tenant, same-namespace link to the child's initial run.
    ///
    /// # Errors
    /// Returns [`ChainError::ScopeMismatch`] for a foreign child chain.
    pub fn new(
        scope: ChainScope,
        parent_run_id: RunId,
        parent_instance: ExecutionInstanceId,
        child: &ExecutionChain,
    ) -> Result<Self, ChainError> {
        if scope != child.scope {
            return Err(ChainError::ScopeMismatch);
        }
        Ok(Self {
            scope,
            parent_run_id,
            parent_instance,
            child_chain_id: child.chain_id,
            cancellation_requested: false,
            final_outcome: None,
            settled: false,
        })
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn parent_run_id(&self) -> RunId {
        self.parent_run_id
    }

    #[must_use]
    pub const fn parent_instance(&self) -> &ExecutionInstanceId {
        &self.parent_instance
    }

    #[must_use]
    pub const fn child_chain_id(&self) -> RunId {
        self.child_chain_id
    }

    #[must_use]
    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }

    #[must_use]
    pub const fn final_outcome(&self) -> Option<ChainFinalOutcome> {
        self.final_outcome
    }

    #[must_use]
    pub const fn is_settled(&self) -> bool {
        self.settled
    }

    /// Latches parent cancellation. The store must commit this with the corresponding parent
    /// transition and reconcile cancellation against the child's serialized current head.
    #[must_use]
    pub const fn request_cancel(&mut self) -> bool {
        if self.cancellation_requested {
            false
        } else {
            self.cancellation_requested = true;
            true
        }
    }

    /// Checks the link's cancellation latch before a child handoff.
    ///
    /// # Errors
    /// Returns a mismatch for a foreign chain or a cancellation error after the parent latch wins.
    pub fn permit_handoff(&self, child: &ExecutionChain) -> Result<(), ChainError> {
        self.check_child(child)?;
        if self.cancellation_requested {
            Err(ChainError::ChildCancellationRequested)
        } else {
            Ok(())
        }
    }

    /// Observes only the chain-level outcome. A continued segment remains waiting.
    ///
    /// # Errors
    /// Returns a mismatch for a foreign chain or an outcome conflict for corrupt restored state.
    pub fn observe(&mut self, child: &ExecutionChain) -> Result<ChildObservation, ChainError> {
        self.check_child(child)?;
        let Some(outcome) = child.final_outcome else {
            return Ok(ChildObservation::Waiting {
                head: child.current_run_id,
            });
        };
        if let Some(stored) = self.final_outcome {
            if stored != outcome {
                return Err(ChainError::FinalOutcomeConflict);
            }
        } else {
            self.final_outcome = Some(outcome);
        }
        Ok(ChildObservation::Final(outcome))
    }

    /// Marks the final child result as applied to the original parent instance exactly once.
    ///
    /// # Errors
    /// Returns an unavailable or conflicting outcome without changing settlement.
    pub fn settle_final(&mut self, outcome: ChainFinalOutcome) -> Result<bool, ChainError> {
        match self.final_outcome {
            None => Err(ChainError::ChildOutcomeUnavailable),
            Some(stored) if stored != outcome => Err(ChainError::FinalOutcomeConflict),
            Some(_) if self.settled => Ok(false),
            Some(_) => {
                self.settled = true;
                Ok(true)
            }
        }
    }

    fn check_child(&self, child: &ExecutionChain) -> Result<(), ChainError> {
        if child.scope != self.scope || child.chain_id != self.child_chain_id {
            Err(ChainError::ChildChainMismatch)
        } else {
            Ok(())
        }
    }
}

impl ChainError {
    /// The obligation a rejected handoff is waiting for, when the rejection is only that.
    #[must_use]
    pub const fn awaited_blocker(&self) -> Option<QuiescenceBlocker> {
        match self {
            Self::NotQuiescent(blocker) => Some(*blocker),
            _ => None,
        }
    }

    /// Whether the same handoff may succeed later without anyone changing the request: the
    /// boundary keeps waiting instead of ending the chain (D296).
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        self.awaited_blocker().is_some()
    }
}

/// Typed rejection of an in-memory continuation-chain transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("chain segment limit must be between one and 10,000")]
    InvalidSegmentLimit,
    #[error("chain segment index must be between zero and 9,999")]
    InvalidSegmentIndex,
    #[error("continuation scope does not match the chain tenant and namespace")]
    ScopeMismatch,
    #[error("continuation cannot change the pinned workflow version")]
    DefinitionVersionChanged,
    #[error("continuation cannot change the original chain deadline")]
    DeadlineChanged,
    #[error("chain head changed")]
    HeadChanged { current: RunId },
    #[error("chain revision changed from expected {expected} to {actual}")]
    RevisionChanged { expected: u64, actual: u64 },
    #[error("chain revision is exhausted")]
    RevisionExhausted,
    #[error("chain cancellation was already requested")]
    CancellationRequested,
    #[error("continuation boundary has an unresolved obligation: {0:?}")]
    NotQuiescent(QuiescenceBlocker),
    #[error("chain segment limit is exhausted")]
    SegmentLimitExceeded,
    #[error("the chain's original deadline has passed; the deadline ends it, not a hand-off")]
    DeadlineElapsed,
    #[error("the run tree that holds this chain has no room for another successor")]
    TreeBudgetExhausted,
    #[error("a run may belong to a chain only once")]
    DuplicateRun,
    #[error("the committed continuation has different transfer content")]
    ContinuationConflict,
    #[error("the chain already has a final outcome")]
    ChainFinal,
    #[error("a different final outcome is already retained")]
    FinalOutcomeConflict,
    #[error("stored chain links are inconsistent")]
    InvalidChainState,
    #[error("the link does not identify this child chain")]
    ChildChainMismatch,
    #[error("parent cancellation prevents another child handoff")]
    ChildCancellationRequested,
    #[error("the child chain has no final outcome to settle")]
    ChildOutcomeUnavailable,
}
